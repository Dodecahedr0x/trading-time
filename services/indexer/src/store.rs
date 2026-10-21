use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use solana_pubkey::Pubkey;
use tradingtime_interface::events::*;
use tradingtime_interface::{LimitOrder, Market, Operator, Trader};
use tradingtime_math::to_f64;

use crate::decode::{Event, TtAccount, decode_account_data};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts (pubkey TEXT PRIMARY KEY, kind TEXT NOT NULL, owner TEXT, slot INTEGER NOT NULL, data BLOB NOT NULL);
CREATE INDEX IF NOT EXISTS accounts_kind ON accounts(kind);
CREATE INDEX IF NOT EXISTS accounts_owner ON accounts(owner);
CREATE TABLE IF NOT EXISTS fills (signature TEXT NOT NULL, seq INTEGER NOT NULL, slot INTEGER NOT NULL, ts INTEGER NOT NULL,
  trader TEXT NOT NULL, operator TEXT NOT NULL, side INTEGER NOT NULL, kind INTEGER NOT NULL, size INTEGER NOT NULL,
  ln_price TEXT NOT NULL, fee INTEGER NOT NULL, PRIMARY KEY (signature, seq));
CREATE INDEX IF NOT EXISTS fills_trader ON fills(trader, ts);
CREATE INDEX IF NOT EXISTS fills_ts ON fills(ts);
CREATE TABLE IF NOT EXISTS index_events (signature TEXT NOT NULL, seq INTEGER NOT NULL, ts INTEGER NOT NULL, index_ln TEXT NOT NULL,
  var_ewma TEXT NOT NULL, pyth_price INTEGER NOT NULL, pyth_expo INTEGER NOT NULL, publish_time INTEGER NOT NULL, PRIMARY KEY (signature, seq));
CREATE TABLE IF NOT EXISTS funding_events (signature TEXT NOT NULL, seq INTEGER NOT NULL, ts INTEGER NOT NULL, funding_index TEXT NOT NULL,
  mark_ln TEXT NOT NULL, index_ln TEXT NOT NULL, PRIMARY KEY (signature, seq));
CREATE TABLE IF NOT EXISTS liquidations (signature TEXT NOT NULL, seq INTEGER NOT NULL, ts INTEGER NOT NULL, trader TEXT NOT NULL,
  liquidator TEXT NOT NULL, size INTEGER NOT NULL, bankruptcy_ln TEXT NOT NULL, penalty INTEGER NOT NULL, insurance_used INTEGER NOT NULL,
  PRIMARY KEY (signature, seq));
CREATE TABLE IF NOT EXISTS order_events (signature TEXT NOT NULL, seq INTEGER NOT NULL, ts INTEGER NOT NULL, owner TEXT NOT NULL,
  order_id INTEGER NOT NULL, kind INTEGER NOT NULL, side INTEGER NOT NULL, size INTEGER NOT NULL, filled INTEGER NOT NULL,
  limit_ln TEXT NOT NULL, PRIMARY KEY (signature, seq));
CREATE TABLE IF NOT EXISTS orders (owner TEXT NOT NULL, order_id INTEGER NOT NULL, side INTEGER NOT NULL, size INTEGER NOT NULL,
  filled INTEGER NOT NULL, limit_ln TEXT NOT NULL, expiry_ts INTEGER, created_slot INTEGER, status TEXT NOT NULL, updated_ts INTEGER NOT NULL,
  PRIMARY KEY (owner, order_id));
CREATE TABLE IF NOT EXISTS operator_events (signature TEXT NOT NULL, seq INTEGER NOT NULL, ts INTEGER NOT NULL, owner TEXT NOT NULL,
  kind INTEGER NOT NULL, bond INTEGER NOT NULL, PRIMARY KEY (signature, seq));
CREATE TABLE IF NOT EXISTS cash_events (signature TEXT NOT NULL, seq INTEGER NOT NULL, ts INTEGER NOT NULL, owner TEXT NOT NULL,
  kind INTEGER NOT NULL, amount INTEGER NOT NULL, PRIMARY KEY (signature, seq));
CREATE INDEX IF NOT EXISTS cash_owner ON cash_events(owner, kind, ts);
CREATE TABLE IF NOT EXISTS mark_points (ts INTEGER NOT NULL, ln REAL NOT NULL, source TEXT NOT NULL, PRIMARY KEY (ts, ln, source));
CREATE TABLE IF NOT EXISTS order_meta (owner TEXT NOT NULL, order_id INTEGER NOT NULL, expiry_ts INTEGER NOT NULL, created_slot INTEGER NOT NULL,
  PRIMARY KEY (owner, order_id));
";

pub fn b58(k: &[u8; 32]) -> String {
    Pubkey::new_from_array(*k).to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct FillRow {
    pub signature: String,
    pub slot: u64,
    pub ts: i64,
    pub trader: String,
    pub operator: String,
    pub side: u8,
    pub kind: u8,
    pub size: u64,
    pub ln_price: i128,
    pub fee: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrderRow {
    pub owner: String,
    pub order_id: u64,
    pub side: u8,
    pub size: u64,
    pub filled: u64,
    pub limit_ln: i128,
    pub expiry_ts: Option<i64>,
    pub created_slot: Option<u64>,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexPoint {
    pub ts: i64,
    pub index_ln: i128,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FillStats {
    pub volume: u64,
    pub trades: u64,
    pub fees: u64,
    pub last_ts: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FillTotals {
    pub trades: u64,
    pub volume: u64,
    pub fees: u64,
    pub adl_volume: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VolumeBucket {
    pub ts: i64,
    pub volume: u64,
    pub long: u64,
    pub short: u64,
    pub trades: u64,
    pub fees: u64,
    pub liquidation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiquidationRow {
    pub signature: String,
    pub ts: i64,
    pub trader: String,
    pub liquidator: String,
    pub size: u64,
    pub bankruptcy_ln: i128,
    pub penalty: u64,
    pub insurance_used: u64,
}

/// The wallet owner for a key that may be a trader PDA.
pub fn owner_of(pdas: &HashMap<String, String>, key: String) -> String {
    pdas.get(&key).cloned().unwrap_or(key)
}

/// Which snapshot kind changed; lets ingest decide what to broadcast.
#[derive(Clone, Debug, PartialEq)]
pub enum Changed {
    Market,
    Trader { owner: String },
    Operator,
    Order { owner: String },
}

pub struct Store {
    conn: Connection,
    slot: u64,
    chain_ts: Option<i64>,
}

fn parse_i128(s: String) -> i128 {
    s.parse().unwrap_or(0)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn, slot: 0, chain_ts: None })
    }

    pub fn set_chain(&mut self, slot: u64, unix_ts: i64) {
        self.slot = self.slot.max(slot);
        self.chain_ts = Some(unix_ts);
    }

    pub fn set_chain_slot(&mut self, slot: u64) {
        self.slot = self.slot.max(slot);
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    /// Chain time if known, else wall clock.
    pub fn now(&self) -> i64 {
        self.chain_ts.unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
        })
    }

    /// Stores the latest snapshot of a program account; older slots never overwrite newer ones.
    pub fn upsert_account(&mut self, pubkey: &[u8; 32], slot: u64, data: &[u8]) -> Result<Option<Changed>> {
        let Some(acct) = decode_account_data(data) else { return Ok(None) };
        self.slot = self.slot.max(slot);
        let (kind, owner, changed) = match &acct {
            TtAccount::Market(m) => {
                if m.mark_updated_ts > 0 {
                    self.add_mark_point(m.mark_updated_ts, m.mark_ln.get(), "mark")?;
                }
                ("market", None, Changed::Market)
            }
            TtAccount::Trader(t) => ("trader", Some(b58(&t.owner)), Changed::Trader { owner: b58(&t.owner) }),
            TtAccount::Operator(o) => ("operator", Some(b58(&o.owner)), Changed::Operator),
            TtAccount::Order(o) => {
                self.upsert_order_from_account(o)?;
                ("order", Some(b58(&o.owner)), Changed::Order { owner: b58(&o.owner) })
            }
        };
        let n = self.conn.execute(
            "INSERT INTO accounts (pubkey, kind, owner, slot, data) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(pubkey) DO UPDATE SET kind = excluded.kind, owner = excluded.owner, slot = excluded.slot, data = excluded.data
             WHERE excluded.slot >= accounts.slot",
            params![b58(pubkey), kind, owner, slot as i64, data],
        )?;
        Ok(if n > 0 { Some(changed) } else { None })
    }

    fn add_mark_point(&self, ts: i64, ln: i128, source: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO mark_points (ts, ln, source) VALUES (?1, ?2, ?3)",
            params![ts, to_f64(ln), source],
        )?;
        Ok(())
    }

    fn upsert_order_from_account(&self, o: &LimitOrder) -> Result<()> {
        self.conn.execute(
            "INSERT INTO orders (owner, order_id, side, size, filled, limit_ln, expiry_ts, created_slot, status, updated_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', 0)
             ON CONFLICT(owner, order_id) DO UPDATE SET side = excluded.side, size = excluded.size,
               filled = MAX(orders.filled, excluded.filled), limit_ln = excluded.limit_ln,
               expiry_ts = excluded.expiry_ts, created_slot = excluded.created_slot",
            params![
                b58(&o.owner),
                o.order_id as i64,
                o.side,
                o.size as i64,
                o.filled as i64,
                o.limit_ln.get().to_string(),
                o.expiry_ts,
                o.created_slot as i64
            ],
        )?;
        Ok(())
    }

    /// Expiry and slot come only from the PlaceLimitOrder instruction (OrderEvent lacks them).
    pub fn record_order_placement(&mut self, owner: &[u8; 32], order_id: u64, expiry_ts: i64, slot: u64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO order_meta (owner, order_id, expiry_ts, created_slot) VALUES (?1, ?2, ?3, ?4)",
            params![b58(owner), order_id as i64, expiry_ts, slot as i64],
        )?;
        self.conn.execute(
            "UPDATE orders SET expiry_ts = COALESCE(expiry_ts, ?3), created_slot = COALESCE(created_slot, ?4) WHERE owner = ?1 AND order_id = ?2",
            params![b58(owner), order_id as i64, expiry_ts, slot as i64],
        )?;
        Ok(())
    }

    /// Inserts one decoded event; returns false if (signature, seq) was already stored.
    pub fn insert_event(&mut self, signature: &str, seq: u32, slot: u64, ev: &Event) -> Result<bool> {
        let c = &self.conn;
        let n = match ev {
            Event::Trade(e) => {
                let n = c.execute(
                    "INSERT OR IGNORE INTO fills VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        signature,
                        seq,
                        (if e.slot > 0 { e.slot } else { slot }) as i64,
                        e.ts,
                        b58(&e.trader),
                        b58(&e.operator),
                        e.side,
                        e.kind,
                        e.size as i64,
                        e.ln_price.get().to_string(),
                        e.fee as i64
                    ],
                )?;
                if n > 0 {
                    self.add_mark_point(e.ts, e.ln_price.get(), "trade")?;
                }
                n
            }
            Event::Index(e) => c.execute(
                "INSERT OR IGNORE INTO index_events VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    signature,
                    seq,
                    e.ts,
                    e.index_ln.get().to_string(),
                    e.var_ewma.get().to_string(),
                    e.pyth_price,
                    e.pyth_expo,
                    e.publish_time
                ],
            )?,
            Event::Funding(e) => {
                let n = c.execute(
                    "INSERT OR IGNORE INTO funding_events VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        signature,
                        seq,
                        e.ts,
                        e.funding_index.get().to_string(),
                        e.mark_ln.get().to_string(),
                        e.index_ln.get().to_string()
                    ],
                )?;
                if n > 0 {
                    self.add_mark_point(e.ts, e.mark_ln.get(), "funding")?;
                }
                n
            }
            Event::Liquidation(e) => c.execute(
                "INSERT OR IGNORE INTO liquidations VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    signature,
                    seq,
                    e.ts,
                    b58(&e.trader),
                    b58(&e.liquidator),
                    e.size as i64,
                    e.bankruptcy_ln.get().to_string(),
                    e.penalty as i64,
                    e.insurance_used as i64
                ],
            )?,
            Event::Order(e) => {
                let n = c.execute(
                    "INSERT OR IGNORE INTO order_events VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        signature,
                        seq,
                        e.ts,
                        b58(&e.owner),
                        e.order_id as i64,
                        e.kind,
                        e.side,
                        e.size as i64,
                        e.filled as i64,
                        e.limit_ln.get().to_string()
                    ],
                )?;
                if n > 0 {
                    let status = match e.kind {
                        ORDER_CANCELLED => "cancelled",
                        ORDER_FILLED => "filled",
                        _ => "open",
                    };
                    let owner = b58(&e.owner);
                    let meta: Option<(i64, i64)> = c
                        .query_row(
                            "SELECT expiry_ts, created_slot FROM order_meta WHERE owner = ?1 AND order_id = ?2",
                            params![owner, e.order_id as i64],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    // Terminal states stick; among open updates the newest event wins.
                    c.execute(
                        "INSERT INTO orders (owner, order_id, side, size, filled, limit_ln, expiry_ts, created_slot, status, updated_ts)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                         ON CONFLICT(owner, order_id) DO UPDATE SET
                           filled = MAX(orders.filled, excluded.filled),
                           status = CASE WHEN orders.status != 'open' THEN orders.status ELSE excluded.status END,
                           updated_ts = MAX(orders.updated_ts, excluded.updated_ts),
                           expiry_ts = COALESCE(orders.expiry_ts, excluded.expiry_ts),
                           created_slot = COALESCE(orders.created_slot, excluded.created_slot)",
                        params![
                            owner,
                            e.order_id as i64,
                            e.side,
                            e.size as i64,
                            e.filled as i64,
                            e.limit_ln.get().to_string(),
                            meta.map(|m| m.0),
                            meta.map(|m| m.1).or(if e.kind == ORDER_PLACED { Some(slot as i64) } else { None }),
                            status,
                            e.ts
                        ],
                    )?;
                }
                n
            }
            Event::Operator(e) => c.execute(
                "INSERT OR IGNORE INTO operator_events VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![signature, seq, e.ts, b58(&e.owner), e.kind, e.bond as i64],
            )?,
            Event::Cash(e) => c.execute(
                "INSERT OR IGNORE INTO cash_events VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![signature, seq, e.ts, b58(&e.owner), e.kind, e.amount],
            )?,
        };
        Ok(n > 0)
    }

    /// Every program account seen so far (refreshed by address each second by the poller).
    pub fn known_pubkeys(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT pubkey FROM accounts")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn accounts_of(&self, kind: &str) -> Result<Vec<([u8; 32], Vec<u8>)>> {
        let mut st = self.conn.prepare("SELECT pubkey, data FROM accounts WHERE kind = ?1")?;
        let rows = st.query_map([kind], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (k, d) = row?;
            if let Ok(pk) = k.parse::<Pubkey>() {
                out.push((pk.to_bytes(), d));
            }
        }
        Ok(out)
    }

    pub fn market(&self) -> Result<Option<Market>> {
        Ok(self.accounts_of("market")?.into_iter().find_map(|(_, d)| match decode_account_data(&d) {
            Some(TtAccount::Market(m)) => Some(m),
            _ => None,
        }))
    }

    pub fn traders(&self) -> Result<Vec<([u8; 32], Trader)>> {
        Ok(self
            .accounts_of("trader")?
            .into_iter()
            .filter_map(|(k, d)| match decode_account_data(&d) {
                Some(TtAccount::Trader(t)) => Some((k, t)),
                _ => None,
            })
            .collect())
    }

    pub fn operators(&self) -> Result<Vec<([u8; 32], Operator)>> {
        Ok(self
            .accounts_of("operator")?
            .into_iter()
            .filter_map(|(k, d)| match decode_account_data(&d) {
                Some(TtAccount::Operator(o)) => Some((k, o)),
                _ => None,
            })
            .collect())
    }

    pub fn trader_by_owner(&self, owner: &str) -> Result<Option<([u8; 32], Trader)>> {
        Ok(self.traders()?.into_iter().find(|(_, t)| b58(&t.owner) == owner))
    }

    /// Trader PDA → wallet owner, for events that name the PDA.
    pub fn pda_owners(&self) -> Result<HashMap<String, String>> {
        Ok(self.traders()?.into_iter().map(|(k, t)| (b58(&k), b58(&t.owner))).collect())
    }

    /// The keys an owner may appear under in events: wallet and trader PDA.
    fn keys_for(&self, owner: &str) -> Result<Vec<String>> {
        let mut keys = vec![owner.to_string()];
        if let Some((k, _)) = self.trader_by_owner(owner)? {
            keys.push(b58(&k));
        }
        Ok(keys)
    }

    pub fn fills_for(&self, owner: &str, limit: usize) -> Result<Vec<FillRow>> {
        let keys = self.keys_for(owner)?;
        let k2 = keys.get(1).cloned().unwrap_or_else(|| keys[0].clone());
        self.query_fills("WHERE trader = ?1 OR trader = ?2", params![keys[0], k2, limit as i64])
    }

    pub fn recent_fills(&self, limit: usize) -> Result<Vec<FillRow>> {
        self.query_fills("WHERE ?1 = ?2", params![1, 1, limit as i64])
    }

    pub fn fill(&self, signature: &str, seq: u32) -> Result<Option<FillRow>> {
        Ok(self
            .query_fills("WHERE signature = ?1 AND seq = ?2", params![signature, seq, 1])?
            .into_iter()
            .next())
    }

    fn query_fills(&self, filter: &str, p: impl rusqlite::Params) -> Result<Vec<FillRow>> {
        let sql = format!(
            "SELECT signature, slot, ts, trader, operator, side, kind, size, ln_price, fee FROM fills {filter}
             ORDER BY ts DESC, slot DESC, seq DESC LIMIT ?3"
        );
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(p, |r| {
            Ok(FillRow {
                signature: r.get(0)?,
                slot: r.get::<_, i64>(1)? as u64,
                ts: r.get(2)?,
                trader: r.get(3)?,
                operator: r.get(4)?,
                side: r.get(5)?,
                kind: r.get(6)?,
                size: r.get::<_, i64>(7)? as u64,
                ln_price: parse_i128(r.get(8)?),
                fee: r.get::<_, i64>(9)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// (ts, amount) funding cash events for an owner, newest first.
    pub fn funding_for(&self, owner: &str, limit: usize) -> Result<Vec<(i64, i64)>> {
        let keys = self.keys_for(owner)?;
        let k2 = keys.get(1).cloned().unwrap_or_else(|| keys[0].clone());
        let mut st = self.conn.prepare(
            "SELECT ts, amount FROM cash_events WHERE kind = ?1 AND (owner = ?2 OR owner = ?3) ORDER BY ts DESC, seq DESC LIMIT ?4",
        )?;
        let rows = st.query_map(params![CASH_FUNDING, keys[0], k2, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn query_orders(&self, filter: &str, p: impl rusqlite::Params) -> Result<Vec<OrderRow>> {
        let sql = format!(
            "SELECT o.owner, o.order_id, o.side, o.size, o.filled, o.limit_ln, COALESCE(o.expiry_ts, m.expiry_ts),
                    COALESCE(o.created_slot, m.created_slot), o.status
             FROM orders o LEFT JOIN order_meta m ON m.owner = o.owner AND m.order_id = o.order_id {filter}
             ORDER BY o.order_id"
        );
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(p, |r| {
            Ok(OrderRow {
                owner: r.get(0)?,
                order_id: r.get::<_, i64>(1)? as u64,
                side: r.get(2)?,
                size: r.get::<_, i64>(3)? as u64,
                filled: r.get::<_, i64>(4)? as u64,
                limit_ln: parse_i128(r.get(5)?),
                expiry_ts: r.get(6)?,
                created_slot: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                status: r.get(8)?,
            })
        })?;
        let pdas = self.pda_owners()?;
        let mut out: Vec<OrderRow> = rows.collect::<rusqlite::Result<_>>()?;
        for o in &mut out {
            if let Some(w) = pdas.get(&o.owner) {
                o.owner = w.clone();
            }
        }
        Ok(out)
    }

    pub fn orders_for(&self, owner: &str) -> Result<Vec<OrderRow>> {
        let keys = self.keys_for(owner)?;
        let k2 = keys.get(1).cloned().unwrap_or_else(|| keys[0].clone());
        self.query_orders("WHERE o.owner = ?1 OR o.owner = ?2", params![keys[0], k2])
    }

    pub fn open_orders(&self) -> Result<Vec<OrderRow>> {
        self.query_orders("WHERE o.status = 'open'", [])
    }

    /// Mark/trade ln-time points (ts, ln as f64) with ts >= since, ascending.
    pub fn mark_points(&self, since: i64) -> Result<Vec<(i64, f64)>> {
        let mut st = self.conn.prepare("SELECT ts, ln FROM mark_points WHERE ts >= ?1 ORDER BY ts, rowid")?;
        let rows = st.query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn mark_before(&self, ts: i64) -> Result<Option<f64>> {
        Ok(self
            .conn
            .query_row("SELECT ln FROM mark_points WHERE ts < ?1 ORDER BY ts DESC LIMIT 1", [ts], |r| r.get(0))
            .optional()?)
    }

    /// Latest mark point at or before `ts`, else the earliest one.
    pub fn mark_near(&self, ts: i64) -> Result<Option<f64>> {
        let v: Option<f64> = self
            .conn
            .query_row("SELECT ln FROM mark_points WHERE ts <= ?1 ORDER BY ts DESC LIMIT 1", [ts], |r| r.get(0))
            .optional()?;
        if v.is_some() {
            return Ok(v);
        }
        Ok(self.conn.query_row("SELECT ln FROM mark_points ORDER BY ts LIMIT 1", [], |r| r.get(0)).optional()?)
    }

    /// (ts, size) of fills with ts >= since.
    pub fn trade_sizes(&self, since: i64) -> Result<Vec<(i64, u64)>> {
        let mut st = self.conn.prepare("SELECT ts, size FROM fills WHERE ts >= ?1 ORDER BY ts")?;
        let rows = st.query_map([since], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Last `limit` index events, ascending by time.
    /// Pyth SOL/USD price (USD) recorded by the last index crank published at or before `ts`; the earliest
    /// recorded price when none is that old.
    pub fn pyth_price_near(&self, ts: i64) -> Result<Option<f64>> {
        let q = |sql: &str, p: &[&dyn rusqlite::ToSql]| {
            self.conn.query_row(sql, p, |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i32>(1)?))).optional()
        };
        let row = match q("SELECT pyth_price, pyth_expo FROM index_events WHERE publish_time <= ?1 ORDER BY publish_time DESC LIMIT 1", &[&ts])? {
            Some(r) => Some(r),
            None => q("SELECT pyth_price, pyth_expo FROM index_events ORDER BY publish_time LIMIT 1", &[])?,
        };
        Ok(row.map(|(p, e)| p as f64 * 10f64.powi(e)))
    }

    pub fn index_points(&self, limit: usize) -> Result<Vec<IndexPoint>> {
        let mut st = self.conn.prepare(
            "SELECT ts, index_ln FROM (SELECT ts, index_ln, signature, seq FROM index_events ORDER BY ts DESC, signature, seq LIMIT ?1) ORDER BY ts",
        )?;
        let rows = st.query_map([limit as i64], |r| Ok(IndexPoint { ts: r.get(0)?, index_ln: parse_i128(r.get(1)?) }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Taker fill stats per wallet owner (PDA-named rows folded into their owner).
    pub fn taker_stats(&self) -> Result<HashMap<String, FillStats>> {
        self.grouped_fill_stats("SELECT trader, SUM(size), COUNT(*), SUM(fee), MAX(ts) FROM fills GROUP BY trader", [])
    }

    /// Maker fill stats per operator owner, swap and limit fills only.
    pub fn maker_stats(&self) -> Result<HashMap<String, FillStats>> {
        self.grouped_fill_stats(
            "SELECT operator, SUM(size), COUNT(*), SUM(fee), MAX(ts) FROM fills WHERE kind IN (?1, ?2) GROUP BY operator",
            params![TRADE_SWAP, TRADE_LIMIT],
        )
    }

    fn grouped_fill_stats(&self, sql: &str, p: impl rusqlite::Params) -> Result<HashMap<String, FillStats>> {
        let pdas = self.pda_owners()?;
        let mut st = self.conn.prepare(sql)?;
        let rows = st.query_map(p, |r| {
            Ok((
                r.get::<_, String>(0)?,
                FillStats {
                    volume: r.get::<_, i64>(1)? as u64,
                    trades: r.get::<_, i64>(2)? as u64,
                    fees: r.get::<_, i64>(3)? as u64,
                    last_ts: r.get(4)?,
                },
            ))
        })?;
        let mut out: HashMap<String, FillStats> = HashMap::new();
        for row in rows {
            let (k, s) = row?;
            let e = out.entry(owner_of(&pdas, k)).or_default();
            e.volume += s.volume;
            e.trades += s.trades;
            e.fees += s.fees;
            e.last_ts = e.last_ts.max(s.last_ts);
        }
        Ok(out)
    }

    /// Fill totals with ts >= since.
    pub fn fill_totals(&self, since: i64) -> Result<FillTotals> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0), COALESCE(SUM(fee), 0),
                    COALESCE(SUM(CASE WHEN kind = ?2 THEN size ELSE 0 END), 0)
             FROM fills WHERE ts >= ?1",
            params![since, TRADE_ADL],
            |r| {
                Ok(FillTotals {
                    trades: r.get::<_, i64>(0)? as u64,
                    volume: r.get::<_, i64>(1)? as u64,
                    fees: r.get::<_, i64>(2)? as u64,
                    adl_volume: r.get::<_, i64>(3)? as u64,
                })
            },
        )?)
    }

    /// Distinct taker owners with a fill at ts >= since.
    pub fn active_takers(&self, since: i64) -> Result<usize> {
        let pdas = self.pda_owners()?;
        let mut st = self.conn.prepare("SELECT DISTINCT trader FROM fills WHERE ts >= ?1")?;
        let keys = st.query_map([since], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(keys.into_iter().map(|k| owner_of(&pdas, k)).collect::<std::collections::HashSet<_>>().len())
    }

    /// Σ deposits − Σ withdrawals per wallet owner.
    pub fn net_deposits(&self) -> Result<HashMap<String, i64>> {
        let pdas = self.pda_owners()?;
        let mut st = self.conn.prepare(
            "SELECT owner, SUM(CASE WHEN kind = ?1 THEN ABS(amount) ELSE -ABS(amount) END)
             FROM cash_events WHERE kind IN (?1, ?2) GROUP BY owner",
        )?;
        let rows =
            st.query_map(params![CASH_DEPOSIT, CASH_WITHDRAW], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out: HashMap<String, i64> = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            *out.entry(owner_of(&pdas, k)).or_default() += v;
        }
        Ok(out)
    }

    /// Σ deposits across all owners.
    pub fn deposits_total(&self) -> Result<u64> {
        let v: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(ABS(amount)), 0) FROM cash_events WHERE kind = ?1",
            [CASH_DEPOSIT],
            |r| r.get(0),
        )?;
        Ok(v as u64)
    }

    /// Liquidation count per wallet owner.
    pub fn liquidation_counts(&self) -> Result<HashMap<String, u64>> {
        let pdas = self.pda_owners()?;
        let mut st = self.conn.prepare("SELECT trader, COUNT(*) FROM liquidations GROUP BY trader")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out: HashMap<String, u64> = HashMap::new();
        for row in rows {
            let (k, n) = row?;
            *out.entry(owner_of(&pdas, k)).or_default() += n as u64;
        }
        Ok(out)
    }

    pub fn liquidations_since(&self, since: i64) -> Result<u64> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM liquidations WHERE ts >= ?1", [since], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// Most recent liquidations, newest first, trader normalised to the wallet owner.
    pub fn recent_liquidations(&self, limit: usize) -> Result<Vec<LiquidationRow>> {
        let pdas = self.pda_owners()?;
        let mut st = self.conn.prepare(
            "SELECT signature, ts, trader, liquidator, size, bankruptcy_ln, penalty, insurance_used FROM liquidations
             ORDER BY ts DESC, signature DESC, seq DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit as i64], |r| {
            Ok(LiquidationRow {
                signature: r.get(0)?,
                ts: r.get(1)?,
                trader: r.get(2)?,
                liquidator: r.get(3)?,
                size: r.get::<_, i64>(4)? as u64,
                bankruptcy_ln: parse_i128(r.get(5)?),
                penalty: r.get::<_, i64>(6)? as u64,
                insurance_used: r.get::<_, i64>(7)? as u64,
            })
        })?;
        let mut out: Vec<LiquidationRow> = rows.collect::<rusqlite::Result<_>>()?;
        for l in &mut out {
            l.trader = owner_of(&pdas, std::mem::take(&mut l.trader));
        }
        Ok(out)
    }

    /// Non-empty fill buckets of `interval` seconds with start in [from, to), ascending.
    pub fn volume_buckets(&self, interval: i64, from: i64, to: i64) -> Result<Vec<VolumeBucket>> {
        let mut st = self.conn.prepare(
            "SELECT ts - (ts % ?1) AS b, SUM(size), SUM(CASE WHEN side = ?4 THEN 0 ELSE size END),
                    SUM(CASE WHEN side = ?4 THEN size ELSE 0 END), COUNT(*), SUM(fee),
                    SUM(CASE WHEN kind IN (?5, ?6) THEN size ELSE 0 END)
             FROM fills WHERE ts >= ?2 AND ts < ?3 GROUP BY b ORDER BY b",
        )?;
        let rows = st.query_map(
            params![interval, from, to, tradingtime_math::Side::Short as u8, TRADE_LIQUIDATION, TRADE_ADL],
            |r| {
                Ok(VolumeBucket {
                    ts: r.get(0)?,
                    volume: r.get::<_, i64>(1)? as u64,
                    long: r.get::<_, i64>(2)? as u64,
                    short: r.get::<_, i64>(3)? as u64,
                    trades: r.get::<_, i64>(4)? as u64,
                    fees: r.get::<_, i64>(5)? as u64,
                    liquidation: r.get::<_, i64>(6)? as u64,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count(&self, table: &str) -> Result<i64> {
        Ok(self.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use tradingtime_interface::{DISC_MARKET, DISC_OPERATOR, DISC_ORDER, DISC_TRADER, ONE, Params, PodI128};

    pub fn market(mark: f64, index: f64, mark_ts: i64) -> Market {
        let mut m = Market::zeroed();
        m.discriminator = DISC_MARKET;
        m.params = Params::DEFAULT;
        m.mark_ln = PodI128::new(tradingtime_math::from_f64(mark.ln()));
        m.index_ln = PodI128::new(tradingtime_math::from_f64(index.ln()));
        m.mark_updated_ts = mark_ts;
        m.index_updated_ts = mark_ts;
        m
    }

    pub fn trader(owner: [u8; 32], cash: i64, q: i64, entry_secs: f64) -> Trader {
        let mut t = Trader::zeroed();
        t.discriminator = DISC_TRADER;
        t.owner = owner;
        t.cash = cash;
        t.q = q;
        t.cost = PodI128::new(q as i128 * tradingtime_math::from_f64(entry_secs.ln()));
        t
    }

    pub fn operator(owner: [u8; 32], trader: [u8; 32], fair_secs: f64, slot: u64) -> Operator {
        let mut o = Operator::zeroed();
        o.discriminator = DISC_OPERATOR;
        o.owner = owner;
        o.trader = trader;
        o.fair_ln = PodI128::new(tradingtime_math::from_f64(fair_secs.ln()));
        o.half_spread = PodI128::new(ONE / 1000);
        o.impact_per_usdc = PodI128::new(ONE / 1_000_000);
        o.inventory_limit = 20_000_000_000;
        o.updated_slot = slot;
        o
    }

    pub fn trade_ev(trader: [u8; 32], side: u8, size: u64, secs: f64, ts: i64) -> TradeEvent {
        TradeEvent {
            trader,
            operator: [9; 32],
            side,
            kind: TRADE_SWAP,
            _pad: [0; 6],
            size,
            ln_price: PodI128::new(tradingtime_math::from_f64(secs.ln())),
            fee: size / 2000,
            slot: 50,
            ts,
        }
    }

    pub fn order_ev(owner: [u8; 32], id: u64, kind: u8, filled: u64, ts: i64) -> OrderEvent {
        OrderEvent {
            owner,
            order_id: id,
            kind,
            side: 0,
            _pad: [0; 6],
            size: 1_000_000,
            filled,
            limit_ln: PodI128::new(tradingtime_math::from_f64(3600f64.ln())),
            ts,
        }
    }

    #[test]
    fn accounts_round_trip_and_older_slot_is_ignored() {
        let mut s = Store::open_in_memory().unwrap();
        let m = market(3600.0, 3000.0, 100);
        assert_eq!(s.upsert_account(&[1; 32], 10, bytemuck::bytes_of(&m)).unwrap(), Some(Changed::Market));
        assert_eq!(s.market().unwrap(), Some(m));
        let older = market(1.0, 1.0, 0);
        assert_eq!(s.upsert_account(&[1; 32], 5, bytemuck::bytes_of(&older)).unwrap(), None);
        assert_eq!(s.market().unwrap(), Some(m));
        assert_eq!(s.slot(), 10);

        let t = trader([7; 32], 5, 6, 3600.0);
        assert_eq!(
            s.upsert_account(&[2; 32], 11, bytemuck::bytes_of(&t)).unwrap(),
            Some(Changed::Trader { owner: b58(&[7; 32]) })
        );
        assert_eq!(s.trader_by_owner(&b58(&[7; 32])).unwrap(), Some(([2; 32], t)));
        assert_eq!(s.pda_owners().unwrap().get(&b58(&[2; 32])), Some(&b58(&[7; 32])));
        let o = operator([8; 32], [2; 32], 3600.0, 3);
        s.upsert_account(&[3; 32], 11, bytemuck::bytes_of(&o)).unwrap();
        assert_eq!(s.operators().unwrap(), vec![([3; 32], o)]);
        assert_eq!(s.upsert_account(&[4; 32], 11, &[0u8; 10]).unwrap(), None);
        assert_eq!(s.mark_points(0).unwrap().len(), 1);
    }

    #[test]
    fn events_are_idempotent_and_queryable() {
        let mut s = Store::open_in_memory().unwrap();
        let owner = [7; 32];
        let pda = [2; 32];
        s.upsert_account(&pda, 1, bytemuck::bytes_of(&trader(owner, 0, 0, 1.0))).unwrap();
        let e = Event::Trade(trade_ev(owner, 0, 2_000_000, 3600.0, 1000));
        assert!(s.insert_event("sigA", 0, 50, &e).unwrap());
        assert!(!s.insert_event("sigA", 0, 50, &e).unwrap());
        // An event naming the PDA is attributed to the same owner.
        assert!(s.insert_event("sigB", 0, 51, &Event::Trade(trade_ev(pda, 1, 1_000_000, 1800.0, 1010))).unwrap());
        let fills = s.fills_for(&b58(&owner), 10).unwrap();
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].signature, "sigB");
        assert_eq!(fills[1].size, 2_000_000);
        assert_eq!(s.fills_for(&b58(&owner), 1).unwrap().len(), 1);
        assert!(s.fill("sigA", 0).unwrap().is_some());
        assert_eq!(s.trade_sizes(1005).unwrap(), vec![(1010, 1_000_000)]);

        let cash = CashEvent { owner, kind: CASH_FUNDING, _pad: [0; 7], amount: -42, ts: 1100 };
        s.insert_event("sigC", 0, 52, &Event::Cash(cash)).unwrap();
        s.insert_event("sigC", 1, 52, &Event::Cash(CashEvent { kind: CASH_DEPOSIT, ..cash })).unwrap();
        assert_eq!(s.funding_for(&b58(&owner), 10).unwrap(), vec![(1100, -42)]);

        let idx = IndexEvent {
            index_ln: PodI128::new(5 * ONE),
            var_ewma: PodI128::new(1),
            pyth_price: 1,
            pyth_expo: -8,
            _pad: 0,
            publish_time: 1,
            ts: 900,
        };
        s.insert_event("sigD", 0, 53, &Event::Index(idx)).unwrap();
        s.insert_event("sigE", 0, 54, &Event::Index(IndexEvent { ts: 950, ..idx })).unwrap();
        let pts = s.index_points(1).unwrap();
        assert_eq!(pts, vec![IndexPoint { ts: 950, index_ln: 5 * ONE }]);
        let f = FundingEvent { funding_index: PodI128::new(1), mark_ln: PodI128::new(ONE), index_ln: PodI128::new(ONE), ts: 960 };
        s.insert_event("sigF", 0, 55, &Event::Funding(f)).unwrap();
        assert_eq!(s.mark_near(965).unwrap(), Some(1.0));
        assert_eq!(s.mark_near(0).unwrap(), Some(1.0));
        assert_eq!(s.count("funding_events").unwrap(), 1);
        let l = LiquidationEvent {
            trader: owner,
            liquidator: [5; 32],
            size: 1,
            bankruptcy_ln: PodI128::new(0),
            penalty: 1,
            insurance_used: 0,
            ts: 1,
        };
        s.insert_event("sigG", 0, 56, &Event::Liquidation(l)).unwrap();
        assert_eq!(s.count("liquidations").unwrap(), 1);
    }

    #[test]
    fn order_lifecycle_from_ix_events_and_accounts() {
        let mut s = Store::open_in_memory().unwrap();
        let owner = [7; 32];
        s.record_order_placement(&owner, 1, 5000, 77).unwrap();
        s.insert_event("p1", 0, 77, &Event::Order(order_ev(owner, 1, ORDER_PLACED, 0, 100))).unwrap();
        s.insert_event("p2", 0, 78, &Event::Order(order_ev(owner, 2, ORDER_PLACED, 0, 101))).unwrap();
        let open = s.open_orders().unwrap();
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].expiry_ts, Some(5000));
        assert_eq!(open[0].created_slot, Some(77));
        s.insert_event("p3", 0, 79, &Event::Order(order_ev(owner, 1, ORDER_PARTIAL, 400_000, 102))).unwrap();
        s.insert_event("p4", 0, 80, &Event::Order(order_ev(owner, 2, ORDER_CANCELLED, 0, 103))).unwrap();
        let mine = s.orders_for(&b58(&owner)).unwrap();
        assert_eq!(mine[0].filled, 400_000);
        assert_eq!(mine[0].status, "open");
        assert_eq!(mine[1].status, "cancelled");
        // A late account snapshot of a cancelled order does not reopen it.
        let mut lo = LimitOrder::zeroed();
        lo.discriminator = DISC_ORDER;
        lo.owner = owner;
        lo.order_id = 2;
        lo.size = 1_000_000;
        lo.expiry_ts = 6000;
        lo.created_slot = 78;
        s.upsert_account(&[6; 32], 78, bytemuck::bytes_of(&lo)).unwrap();
        assert_eq!(s.open_orders().unwrap().len(), 1);
        s.insert_event("p5", 0, 81, &Event::Order(order_ev(owner, 1, ORDER_FILLED, 1_000_000, 104))).unwrap();
        assert!(s.open_orders().unwrap().is_empty());
        assert_eq!(s.orders_for(&b58(&owner)).unwrap()[1].expiry_ts, Some(6000));
    }

    #[test]
    fn opens_on_disk_with_wal() {
        let dir = std::env::temp_dir().join(format!("tt-indexer-{}", std::process::id()));
        let path = dir.join("sub/indexer.sqlite");
        let mut s = Store::open(&path).unwrap();
        s.insert_event("x", 0, 1, &Event::Trade(trade_ev([1; 32], 0, 1, 2.0, 3))).unwrap();
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.count("fills").unwrap(), 1);
        let mode: String = s.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        let _ = std::fs::remove_dir_all(dir);
    }
}
