//! Central limit order book venue. Traders take each other's sides; there is no
//! pool counterparty. Accounts are netted and log-quoted: exposure `q` (USD)
//! gains `q × Δln(price)`, so equity = cash + q·ln(mark) − cost, where
//! cost = Σ fill_size·ln(fill_price). Because every fill has a buyer and a
//! seller, Σq = 0 and Σcost = 0: unrealized PnL always nets to zero across
//! accounts, and cash only moves between accounts, fees and insurance.

use crate::pool::Side;

#[derive(Clone, Debug)]
pub struct BookConfig {
    /// Fee on taker notional, paid to the protocol.
    pub taker_fee: f64,
    pub max_leverage: f64,
    /// Maintenance margin as a fraction of |q|.
    pub maintenance: f64,
    /// Liquidation penalty as a fraction of closed notional, paid to insurance.
    pub liq_fee: f64,
    /// Funding per second per unit of ln(mark / index); longs pay when positive.
    pub funding_k: f64,
}

impl Default for BookConfig {
    fn default() -> Self {
        Self { taker_fee: 0.0005, max_leverage: 5.0, maintenance: 0.05, liq_fee: 0.01, funding_k: 1.0 / 86_400.0 }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    pub cash: f64,
    /// Signed USD exposure per unit of ln(price).
    pub q: f64,
    /// Σ signed fill size × ln(fill price).
    pub cost: f64,
}

impl Account {
    pub fn equity(&self, mark: f64) -> f64 {
        self.cash + self.q * mark.ln() - self.cost
    }

    /// Price at which equity is zero.
    pub fn bankruptcy_price(&self) -> f64 {
        ((self.cost - self.cash) / self.q).exp()
    }

    /// Extra exposure this account may add on `side` at `mark`.
    pub fn capacity(&self, side: Side, mark: f64, max_leverage: f64) -> f64 {
        (self.equity(mark).max(0.0) * max_leverage - side.sign() * self.q).max(0.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Order {
    pub owner: usize,
    pub price: f64,
    pub size: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Liquidation {
    pub owner: usize,
    pub size: f64,
    /// Part closed by auto-deleveraging at the bankruptcy price.
    pub adl: f64,
}

/// Below this |q| an account is treated as flat and its cost is realized into cash.
const DUST: f64 = 1e-6;

#[derive(Clone, Debug)]
pub struct Book {
    pub cfg: BookConfig,
    pub accounts: Vec<Account>,
    /// Best (highest) first; equal prices keep arrival order.
    bids: Vec<Order>,
    /// Best (lowest) first.
    asks: Vec<Order>,
    pub insurance: f64,
    pub fees: f64,
    pub volume: f64,
    pub adl_volume: f64,
    /// Value ADL counterparties gave up versus the mark: Σ size·|ln(mark / bankruptcy)|.
    pub adl_shortfall: f64,
}

impl Book {
    pub fn new(cfg: BookConfig, deposits: &[f64]) -> Self {
        Self {
            cfg,
            accounts: deposits.iter().map(|&cash| Account { cash, ..Default::default() }).collect(),
            bids: Vec::new(),
            asks: Vec::new(),
            insurance: 0.0,
            fees: 0.0,
            volume: 0.0,
            adl_volume: 0.0,
            adl_shortfall: 0.0,
        }
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|o| o.price)
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|o| o.price)
    }

    pub fn mid(&self) -> Option<f64> {
        Some((self.best_bid()? * self.best_ask()?).sqrt())
    }

    /// Resting size on one side within `ln_width` of the best price.
    pub fn depth(&self, side: Side, ln_width: f64) -> f64 {
        let book = if side == Side::Long { &self.bids } else { &self.asks };
        let Some(best) = book.first() else { return 0.0 };
        book.iter().take_while(|o| (o.price / best.price).ln().abs() <= ln_width).map(|o| o.size).sum()
    }

    pub fn cancel_all(&mut self, owner: usize) {
        self.bids.retain(|o| o.owner != owner);
        self.asks.retain(|o| o.owner != owner);
    }

    /// Replaces `owner`'s resting orders, trimming each side to the account's margin capacity.
    /// Quotes that would cross the opposite side are dropped rather than matched.
    pub fn replace_quotes(&mut self, owner: usize, quotes: &[(Side, f64, f64)], mark: f64) {
        self.cancel_all(owner);
        for side in [Side::Long, Side::Short] {
            let mut room = self.accounts[owner].capacity(side, mark, self.cfg.max_leverage);
            let mut mine: Vec<(f64, f64)> =
                quotes.iter().filter(|q| q.0 == side && q.2 > 0.0 && q.1 > 0.0).map(|q| (q.1, q.2)).collect();
            // Best price first so trimming drops the far levels.
            mine.sort_by(|a, b| if side == Side::Long { b.0.total_cmp(&a.0) } else { a.0.total_cmp(&b.0) });
            for (price, size) in mine {
                let crosses = match side {
                    Side::Long => self.best_ask().is_some_and(|a| price >= a),
                    Side::Short => self.best_bid().is_some_and(|b| price <= b),
                };
                let size = size.min(room);
                if crosses || size <= 0.0 {
                    continue;
                }
                room -= size;
                let book = if side == Side::Long { &mut self.bids } else { &mut self.asks };
                let at = book.partition_point(|o| if side == Side::Long { o.price >= price } else { o.price <= price });
                book.insert(at, Order { owner, price, size });
            }
        }
    }

    fn trade(&mut self, buyer: usize, seller: usize, price: f64, size: f64) {
        let lp = price.ln();
        for (who, sign) in [(buyer, 1.0), (seller, -1.0)] {
            let a = &mut self.accounts[who];
            a.q += sign * size;
            a.cost += sign * size * lp;
            if a.q.abs() < DUST {
                // Realize the remaining equity at this fill price so it survives the reset.
                a.cash += a.q * lp - a.cost;
                a.q = 0.0;
                a.cost = 0.0;
            }
        }
        self.volume += size;
    }

    /// Market order for up to `size`, never filling beyond `limit`. Skips the taker's own quotes.
    /// Returns the filled size. `fee` is charged on filled notional.
    pub fn market(&mut self, owner: usize, side: Side, size: f64, limit: f64, fee: f64) -> f64 {
        let mut left = size;
        let mut i = 0;
        while left > DUST {
            let book = if side == Side::Long { &mut self.asks } else { &mut self.bids };
            let Some(o) = book.get(i) else { break };
            let beyond = if side == Side::Long { o.price > limit } else { o.price < limit };
            if beyond {
                break;
            }
            if o.owner == owner {
                i += 1;
                continue;
            }
            let (maker, price) = (o.owner, o.price);
            let fill = left.min(o.size);
            book[i].size -= fill;
            if book[i].size <= DUST {
                book.remove(i);
            }
            left -= fill;
            if side == Side::Long {
                self.trade(owner, maker, price, fill)
            } else {
                self.trade(maker, owner, price, fill)
            }
        }
        let filled = size - left;
        let charge = fee * filled;
        self.accounts[owner].cash -= charge;
        self.fees += charge;
        filled
    }

    /// Funding for `dt` seconds: every account pays q·k·ln(mark/index)·dt. Nets to zero.
    pub fn accrue_funding(&mut self, mark: f64, index: f64, dt: f64) {
        let rate = self.cfg.funding_k * (mark / index).ln() * dt;
        for a in &mut self.accounts {
            a.cash -= a.q * rate;
        }
    }

    pub fn is_liquidatable(&self, owner: usize, mark: f64) -> bool {
        let a = &self.accounts[owner];
        a.q != 0.0 && a.equity(mark) < self.cfg.maintenance * a.q.abs()
    }

    /// Closes every under-margined account: first into the book down to its
    /// bankruptcy price, then the rest against opposite accounts pro rata (ADL)
    /// at that price, so no account ends with negative equity.
    pub fn liquidate_all(&mut self, mark: f64) -> Vec<Liquidation> {
        let owners: Vec<usize> = (0..self.accounts.len()).filter(|&o| self.is_liquidatable(o, mark)).collect();
        let mut out = Vec::new();
        for owner in owners {
            if !self.is_liquidatable(owner, mark) {
                continue; // already closed out by an earlier ADL
            }
            self.cancel_all(owner);
            let q = self.accounts[owner].q;
            let bankrupt = self.accounts[owner].bankruptcy_price();
            let side = if q > 0.0 { Side::Short } else { Side::Long };
            let filled = self.market(owner, side, q.abs(), bankrupt, 0.0);
            let rest = q.abs() - filled;
            if rest > DUST {
                self.deleverage(owner, rest, bankrupt, mark);
            }
            let a = &mut self.accounts[owner];
            let penalty = (self.cfg.liq_fee * q.abs()).min(a.cash.max(0.0));
            a.cash -= penalty;
            self.insurance += penalty;
            if a.cash < 0.0 {
                // Rounding at the bankruptcy price; insurance makes the account whole.
                self.insurance += a.cash;
                a.cash = 0.0;
            }
            out.push(Liquidation { owner, size: q.abs(), adl: rest.max(0.0) });
        }
        out
    }

    /// Closes `size` of `owner`'s position against accounts on the other side, pro rata, at `price`.
    fn deleverage(&mut self, owner: usize, size: f64, price: f64, mark: f64) {
        let sign = self.accounts[owner].q.signum();
        let others: Vec<(usize, f64)> = (0..self.accounts.len())
            .filter(|&o| o != owner && self.accounts[o].q * sign < 0.0)
            .map(|o| (o, self.accounts[o].q.abs()))
            .collect();
        let total: f64 = others.iter().map(|o| o.1).sum();
        if total <= 0.0 {
            return;
        }
        for (other, exposure) in others {
            let x = size * exposure / total;
            self.cancel_all(other);
            if sign > 0.0 {
                self.trade(other, owner, price, x)
            } else {
                self.trade(owner, other, price, x)
            }
            self.adl_volume += x;
            self.adl_shortfall += x * (mark / price).ln().abs();
        }
    }

    /// Σ(cash − cost) + fees + insurance; constant over the life of the book.
    pub fn total_value(&self) -> f64 {
        self.accounts.iter().map(|a| a.cash - a.cost).sum::<f64>() + self.fees + self.insurance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Book {
        Book::new(BookConfig { taker_fee: 0.0, ..Default::default() }, &[10_000.0, 10_000.0, 10_000.0])
    }

    #[test]
    fn quotes_sort_and_mid() {
        let mut b = book();
        b.replace_quotes(0, &[(Side::Long, 990.0, 1_000.0), (Side::Short, 1_010.0, 1_000.0)], 1_000.0);
        b.replace_quotes(1, &[(Side::Long, 995.0, 1_000.0), (Side::Short, 1_020.0, 1_000.0)], 1_000.0);
        assert_eq!((b.best_bid(), b.best_ask()), (Some(995.0), Some(1_010.0)));
        assert!((b.mid().unwrap() - (995.0f64 * 1_010.0).sqrt()).abs() < 1e-9);
        // Crossing quotes are dropped.
        b.replace_quotes(2, &[(Side::Long, 1_015.0, 1_000.0)], 1_000.0);
        assert_eq!(b.best_bid(), Some(995.0));
    }

    #[test]
    fn market_walks_levels_and_pnl_is_log_quoted() {
        let mut b = book();
        b.replace_quotes(0, &[(Side::Short, 1_000.0, 1_000.0), (Side::Short, 1_100.0, 1_000.0)], 1_000.0);
        let filled = b.market(1, Side::Long, 1_500.0, 2_000.0, 0.0);
        assert_eq!(filled, 1_500.0);
        assert_eq!(b.accounts[1].q, 1_500.0);
        assert_eq!(b.accounts[0].q, -1_500.0);
        assert_eq!(b.best_ask(), Some(1_100.0));
        // Mark doubles: buyer gains 1500·ln2 minus what it paid above 1000.
        let gain = b.accounts[1].equity(2_000.0) - 10_000.0;
        let expected = 1_500.0 * 2f64.ln() - 500.0 * (1.1f64).ln();
        assert!((gain - expected).abs() < 1e-6, "{gain} vs {expected}");
        assert!((b.accounts[0].equity(2_000.0) + b.accounts[1].equity(2_000.0) - 20_000.0).abs() < 1e-6);
    }

    #[test]
    fn limit_and_self_trade_protection() {
        let mut b = book();
        b.replace_quotes(0, &[(Side::Short, 1_000.0, 500.0)], 1_000.0);
        b.replace_quotes(1, &[(Side::Short, 1_050.0, 500.0)], 1_000.0);
        assert_eq!(b.market(0, Side::Long, 1_000.0, 1_020.0, 0.0), 0.0, "own quote skipped, next beyond limit");
        assert_eq!(b.market(2, Side::Long, 1_000.0, 1_020.0, 0.0), 500.0);
    }

    #[test]
    fn quotes_trimmed_to_margin() {
        let mut b = book();
        b.replace_quotes(0, &[(Side::Long, 990.0, 40_000.0), (Side::Long, 980.0, 40_000.0)], 1_000.0);
        assert!((b.depth(Side::Long, 1.0) - 50_000.0).abs() < 1e-6); // 10k equity × 5
    }

    #[test]
    fn funding_nets_to_zero() {
        let mut b = book();
        b.replace_quotes(0, &[(Side::Short, 1_000.0, 5_000.0)], 1_000.0);
        b.market(1, Side::Long, 5_000.0, 1_000.0, 0.0);
        let before = b.total_value();
        b.accrue_funding(1_100.0, 1_000.0, 3_600.0);
        assert!(b.accounts[1].cash < 10_000.0 && b.accounts[0].cash > 10_000.0);
        assert!((b.total_value() - before).abs() < 1e-9);
    }

    #[test]
    fn liquidation_fills_book_then_adl_without_bad_debt() {
        let mut b = Book::new(BookConfig { taker_fee: 0.0, ..Default::default() }, &[1_000.0, 50_000.0, 50_000.0]);
        let v0 = b.total_value();
        // Account 0 goes 5x long against account 1.
        b.replace_quotes(1, &[(Side::Short, 1_000.0, 5_000.0)], 1_000.0);
        b.market(0, Side::Long, 5_000.0, 1_000.0, 0.0);
        // Thin bid from account 2, far below bankruptcy (~818): only ADL can close the rest.
        b.replace_quotes(2, &[(Side::Long, 820.0, 1_000.0), (Side::Long, 500.0, 50_000.0)], 1_000.0);
        let liq = b.liquidate_all(700.0);
        assert_eq!(liq.len(), 1);
        assert!((liq[0].adl - 4_000.0).abs() < 1e-6, "{liq:?}");
        assert_eq!(b.accounts[0].q, 0.0);
        assert!(b.accounts[0].cash >= 0.0);
        assert!((b.accounts[1].q + 1_000.0).abs() < 1e-6, "ADL closed 4k of account 1's short");
        assert!(b.adl_shortfall > 0.0);
        assert!((b.total_value() - v0).abs() < 1e-6);
    }
}
