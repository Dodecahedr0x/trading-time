#![allow(dead_code)]

use std::{cell::RefCell, collections::BTreeMap};

use bytemuck::Pod;
use litesvm::{LiteSVM, types::TransactionMetadata};
use solana_account::Account;
use solana_clock::Clock;
use solana_instruction::{AccountMeta, Instruction, error::InstructionError};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;
use tradingtime_interface::{self as tti, *};
use tradingtime_math::{Fx, ONE, from_f64};

pub fn pid() -> Pubkey {
    Pubkey::new_from_array(PROGRAM_ID)
}
pub fn usdc_mint() -> Pubkey {
    Pubkey::new_from_array(USDC_MINT)
}
pub fn feed() -> Pubkey {
    Pubkey::new_from_array(PYTH_SOL_USD_FEED)
}
pub fn token_program() -> Pubkey {
    Pubkey::new_from_array(tti::TOKEN_PROGRAM)
}
pub fn system_program() -> Pubkey {
    Pubkey::new_from_array([0; 32])
}

pub mod pda {
    use super::*;
    fn find(seeds: &[&[u8]]) -> Pubkey {
        Pubkey::find_program_address(seeds, &pid()).0
    }
    pub fn market() -> Pubkey {
        find(&[SEED_MARKET])
    }
    pub fn vault() -> Pubkey {
        find(&[SEED_VAULT])
    }
    pub fn trader(o: &Pubkey) -> Pubkey {
        find(&[SEED_TRADER, o.as_ref()])
    }
    pub fn operator(o: &Pubkey) -> Pubkey {
        find(&[SEED_OPERATOR, o.as_ref()])
    }
    pub fn order(o: &Pubkey, id: u64) -> Pubkey {
        find(&[SEED_ORDER, o.as_ref(), &id.to_le_bytes()])
    }
}

/// Fx ln of seconds.
pub fn ln_secs(s: f64) -> Fx {
    from_f64(s.ln())
}

pub const USDC: u64 = 1_000_000;

fn data<T: Pod>(tag: Ix, p: &T) -> Vec<u8> {
    let mut buf = vec![0u8; 1 + std::mem::size_of::<T>()];
    ix_data(tag, p, &mut buf);
    buf
}

fn w(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, false)
}
fn r(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, false)
}
fn sw(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, true)
}
fn s(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, true)
}

fn ix(tag: Ix, d: Vec<u8>, accounts: Vec<AccountMeta>) -> (Ix, Instruction) {
    let _ = tag;
    (tag, Instruction { program_id: pid(), accounts, data: d })
}

fn tag_only(tag: Ix) -> Vec<u8> {
    vec![tag as u8]
}

pub fn pairs(ops: &[Pubkey]) -> Vec<AccountMeta> {
    ops.iter().flat_map(|o| [w(pda::operator(o)), w(pda::trader(o))]).collect()
}

pub mod ix {
    use super::*;
    pub type Ixn = (Ix, Instruction);

    pub fn initialize_market(admin: &Pubkey, params: Params, initial_index_secs: u32) -> Ixn {
        ix(
            Ix::InitializeMarket,
            data(Ix::InitializeMarket, &InitializeMarketData { params, initial_index_secs, _pad0: 0 }),
            vec![
                sw(*admin),
                w(pda::market()),
                w(pda::vault()),
                r(usdc_mint()),
                r(feed()),
                r(system_program()),
                r(token_program()),
            ],
        )
    }
    pub fn update_params(admin: &Pubkey, params: Params) -> Ixn {
        ix(Ix::UpdateParams, data(Ix::UpdateParams, &UpdateParamsData { params }), vec![s(*admin), w(pda::market())])
    }
    pub fn set_paused(admin: &Pubkey, paused: bool) -> Ixn {
        ix(Ix::SetPaused, data(Ix::SetPaused, &SetPausedData { paused: paused as u8 }), vec![s(*admin), w(pda::market())])
    }
    pub fn create_trader(owner: &Pubkey) -> Ixn {
        ix(
            Ix::CreateTrader,
            tag_only(Ix::CreateTrader),
            vec![sw(*owner), w(pda::trader(owner)), r(pda::market()), r(system_program())],
        )
    }
    fn cash_accounts(owner: &Pubkey, owner_usdc: &Pubkey) -> Vec<AccountMeta> {
        vec![
            s(*owner),
            w(pda::trader(owner)),
            w(pda::market()),
            w(*owner_usdc),
            w(pda::vault()),
            r(usdc_mint()),
            r(token_program()),
        ]
    }
    pub fn deposit(owner: &Pubkey, owner_usdc: &Pubkey, amount: u64) -> Ixn {
        ix(Ix::Deposit, data(Ix::Deposit, &AmountData { amount }), cash_accounts(owner, owner_usdc))
    }
    pub fn withdraw(owner: &Pubkey, owner_usdc: &Pubkey, amount: u64, is_operator: bool) -> Ixn {
        let mut a = cash_accounts(owner, owner_usdc);
        if is_operator {
            a.push(r(pda::operator(owner)));
        }
        ix(Ix::Withdraw, data(Ix::Withdraw, &AmountData { amount }), a)
    }
    pub fn register_operator(owner: &Pubkey, owner_usdc: &Pubkey) -> Ixn {
        ix(
            Ix::RegisterOperator,
            tag_only(Ix::RegisterOperator),
            vec![
                sw(*owner),
                w(pda::trader(owner)),
                w(pda::operator(owner)),
                w(pda::market()),
                w(*owner_usdc),
                w(pda::vault()),
                r(usdc_mint()),
                r(token_program()),
                r(system_program()),
            ],
        )
    }
    pub fn update_curve(owner: &Pubkey, fair_ln: Fx, half_spread: Fx, impact_per_usdc: Fx, inventory_limit: u64) -> Ixn {
        let d = UpdateCurveData {
            fair_ln: PodI128::new(fair_ln),
            half_spread: PodI128::new(half_spread),
            impact_per_usdc: PodI128::new(impact_per_usdc),
            inventory_limit,
        };
        ix(Ix::UpdateCurve, data(Ix::UpdateCurve, &d), vec![s(*owner), w(pda::operator(owner))])
    }
    pub fn request_unbond(owner: &Pubkey) -> Ixn {
        ix(Ix::RequestUnbond, tag_only(Ix::RequestUnbond), vec![s(*owner), w(pda::operator(owner))])
    }
    pub fn complete_unbond(owner: &Pubkey, owner_usdc: &Pubkey) -> Ixn {
        ix(
            Ix::CompleteUnbond,
            tag_only(Ix::CompleteUnbond),
            vec![
                s(*owner),
                w(pda::operator(owner)),
                w(pda::trader(owner)),
                w(pda::market()),
                w(*owner_usdc),
                w(pda::vault()),
                r(usdc_mint()),
                r(token_program()),
            ],
        )
    }
    pub fn swap(owner: &Pubkey, side: Side, size: u64, min_fill: u64, limit_ln: Fx, ops: &[Pubkey]) -> Ixn {
        let d = SwapData {
            side: side as u8,
            n_ops: ops.len() as u8,
            _pad: [0; 6],
            size,
            min_fill,
            limit_ln: PodI128::new(limit_ln),
        };
        let mut a = vec![s(*owner), w(pda::trader(owner)), w(pda::market())];
        a.extend(pairs(ops));
        ix(Ix::Swap, data(Ix::Swap, &d), a)
    }
    pub fn place_limit_order(owner: &Pubkey, order_id: u64, side: Side, size: u64, limit_ln: Fx, expiry_ts: i64) -> Ixn {
        let d = PlaceLimitOrderData {
            side: side as u8,
            _pad: [0; 7],
            order_id,
            size,
            expiry_ts,
            limit_ln: PodI128::new(limit_ln),
        };
        ix(
            Ix::PlaceLimitOrder,
            data(Ix::PlaceLimitOrder, &d),
            vec![sw(*owner), w(pda::trader(owner)), w(pda::order(owner, order_id)), w(pda::market()), r(system_program())],
        )
    }
    pub fn cancel_limit_order(owner: &Pubkey, order_id: u64) -> Ixn {
        ix(
            Ix::CancelLimitOrder,
            tag_only(Ix::CancelLimitOrder),
            vec![sw(*owner), w(pda::trader(owner)), w(pda::order(owner, order_id))],
        )
    }
    /// Any signer closes an expired order; rent goes back to `order_owner`.
    pub fn cancel_expired_order(signer: &Pubkey, order_owner: &Pubkey, order_id: u64) -> Ixn {
        ix(
            Ix::CancelLimitOrder,
            tag_only(Ix::CancelLimitOrder),
            vec![s(*signer), w(pda::trader(order_owner)), w(pda::order(order_owner, order_id)), w(*order_owner)],
        )
    }
    pub fn execute_limit_order(keeper: &Pubkey, order_owner: &Pubkey, order_id: u64, ops: &[Pubkey]) -> Ixn {
        let d = CountData { n_ops: ops.len() as u8, _pad: [0; 7] };
        let mut a = vec![
            s(*keeper),
            w(pda::trader(keeper)),
            w(pda::order(order_owner, order_id)),
            w(*order_owner),
            w(pda::trader(order_owner)),
            w(pda::market()),
        ];
        a.extend(pairs(ops));
        ix(Ix::ExecuteLimitOrder, data(Ix::ExecuteLimitOrder, &d), a)
    }
    pub fn crank_index() -> Ixn {
        ix(Ix::CrankIndex, tag_only(Ix::CrankIndex), vec![w(pda::market()), r(feed())])
    }
    pub fn crank_funding() -> Ixn {
        ix(Ix::CrankFunding, tag_only(Ix::CrankFunding), vec![w(pda::market())])
    }
    pub fn crank_mark(ops: &[Pubkey]) -> Ixn {
        let d = CountData { n_ops: ops.len() as u8, _pad: [0; 7] };
        let mut a = vec![w(pda::market())];
        a.extend(pairs(ops));
        ix(Ix::CrankMark, data(Ix::CrankMark, &d), a)
    }
    pub fn liquidate(liquidator: &Pubkey, target: &Pubkey, ops: &[Pubkey], adl: &[Pubkey]) -> Ixn {
        let d = LiquidateData { n_ops: ops.len() as u8, n_adl: adl.len() as u8, _pad: [0; 6] };
        let mut a = vec![s(*liquidator), w(pda::trader(target)), w(pda::market())];
        a.extend(pairs(ops));
        a.extend(adl.iter().map(|o| w(pda::trader(o))));
        ix(Ix::Liquidate, data(Ix::Liquidate, &d), a)
    }
    pub fn withdraw_treasury(admin: &Pubkey, admin_usdc: &Pubkey, amount: u64) -> Ixn {
        ix(
            Ix::WithdrawTreasury,
            data(Ix::WithdrawTreasury, &AmountData { amount }),
            vec![s(*admin), w(pda::market()), w(*admin_usdc), w(pda::vault()), r(usdc_mint()), r(token_program())],
        )
    }
}

thread_local! {
    static CU: RefCell<BTreeMap<String, Vec<u64>>> = const { RefCell::new(BTreeMap::new()) };
}

/// Records CU usage for an instruction name.
fn record_cu(name: &str, cu: u64) {
    CU.with(|m| m.borrow_mut().entry(name.to_string()).or_default().push(cu));
}

pub fn cu_report() -> BTreeMap<String, Vec<u64>> {
    CU.with(|m| m.borrow().clone())
}

pub struct Env {
    pub svm: LiteSVM,
    pub admin: Keypair,
    pub admin_usdc: Pubkey,
    pub slot: u64,
    pub ts: i64,
}

pub struct User {
    pub kp: Keypair,
    pub usdc: Pubkey,
}

impl User {
    pub fn key(&self) -> Pubkey {
        self.kp.pubkey()
    }
}

pub fn program_so() -> String {
    format!("{}/../../target/deploy/tradingtime_program.so", env!("CARGO_MANIFEST_DIR"))
}

pub fn mint_bytes() -> Vec<u8> {
    let mut d = vec![0u8; 82];
    d[0] = 1; // COption::Some(mint authority)
    d[4..36].copy_from_slice(&[7u8; 32]);
    d[44] = USDC_DECIMALS;
    d[45] = 1;
    d
}

pub fn token_account_bytes(owner: &Pubkey, amount: u64) -> Vec<u8> {
    let mut d = vec![0u8; 165];
    d[0..32].copy_from_slice(&USDC_MINT);
    d[32..64].copy_from_slice(owner.as_ref());
    d[64..72].copy_from_slice(&amount.to_le_bytes());
    d[108] = 1; // AccountState::Initialized
    d
}

pub fn pyth_bytes(price: i64, expo: i32, publish_time: i64) -> Vec<u8> {
    let mut d = vec![0u8; pyth::LEN];
    d[..8].copy_from_slice(&pyth::DISCRIMINATOR);
    d[pyth::VERIFICATION_TAG] = pyth::VERIFICATION_FULL;
    d[pyth::FEED_ID..pyth::FEED_ID + 32].copy_from_slice(&PYTH_SOL_USD_FEED_ID);
    d[pyth::PRICE..pyth::PRICE + 8].copy_from_slice(&price.to_le_bytes());
    d[pyth::EXPONENT..pyth::EXPONENT + 4].copy_from_slice(&expo.to_le_bytes());
    d[pyth::PUBLISH_TIME..pyth::PUBLISH_TIME + 8].copy_from_slice(&publish_time.to_le_bytes());
    d[pyth::PREV_PUBLISH_TIME..pyth::PREV_PUBLISH_TIME + 8].copy_from_slice(&(publish_time - 1).to_le_bytes());
    d
}

pub type TxErr = (Option<u32>, String);

impl Env {
    pub fn new() -> Env {
        let mut svm = LiteSVM::new();
        svm.add_program_from_file(pid(), program_so()).expect("build the program first: cargo build-sbf");
        let mut svm_env = Env { svm, admin: Keypair::new(), admin_usdc: Pubkey::new_unique(), slot: 100, ts: 1_800_000_000 };
        svm_env.write(usdc_mint(), mint_bytes(), token_program());
        let admin = svm_env.admin.pubkey();
        svm_env.svm.airdrop(&admin, 100_000_000_000).unwrap();
        let au = svm_env.admin_usdc;
        svm_env.write(au, token_account_bytes(&admin, 0), token_program());
        svm_env.set_clock(100, 1_800_000_000);
        svm_env
    }

    pub fn write(&mut self, key: Pubkey, data: Vec<u8>, owner: Pubkey) {
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len()).max(1);
        self.svm.set_account(key, Account { lamports, data, owner, executable: false, rent_epoch: 0 }).unwrap();
    }

    pub fn set_clock(&mut self, slot: u64, ts: i64) {
        self.slot = slot;
        self.ts = ts;
        let mut c: Clock = self.svm.get_sysvar();
        c.slot = slot;
        c.unix_timestamp = ts;
        self.svm.set_sysvar(&c);
    }

    /// Advances time by `secs` and slots by `slots`.
    pub fn advance(&mut self, slots: u64, secs: i64) {
        self.set_clock(self.slot + slots, self.ts + secs);
    }

    pub fn user(&mut self, usdc: u64) -> User {
        let kp = Keypair::new();
        self.svm.airdrop(&kp.pubkey(), 10_000_000_000).unwrap();
        let ua = Pubkey::new_unique();
        self.write(ua, token_account_bytes(&kp.pubkey(), usdc), token_program());
        User { kp, usdc: ua }
    }

    pub fn send(&mut self, i: ix::Ixn, signers: &[&Keypair]) -> Result<TransactionMetadata, TxErr> {
        self.send_with_limit(i, signers, None)
    }

    /// Sends one program instruction, optionally behind a SetComputeUnitLimit.
    pub fn send_with_limit(&mut self, i: ix::Ixn, signers: &[&Keypair], cu_limit: Option<u32>) -> Result<TransactionMetadata, TxErr> {
        self.svm.expire_blockhash();
        let (tag, instr) = i;
        let payer = signers[0];
        let mut ixs = Vec::new();
        if let Some(limit) = cu_limit {
            let mut d = vec![2u8];
            d.extend_from_slice(&limit.to_le_bytes());
            let budget = Pubkey::from_str_const("ComputeBudget111111111111111111111111111111");
            ixs.push(Instruction { program_id: budget, accounts: vec![], data: d });
        }
        ixs.push(instr);
        let msg = Message::new(&ixs, Some(&payer.pubkey()));
        let tx = Transaction::new(signers, msg, self.svm.latest_blockhash());
        match self.svm.send_transaction(tx) {
            Ok(meta) => {
                let name = if cu_limit.is_some() { format!("{tag:?}+budget") } else { format!("{tag:?}") };
                record_cu(&name, meta.compute_units_consumed);
                Ok(meta)
            }
            Err(f) => {
                let code = match f.err {
                    solana_transaction_error::TransactionError::InstructionError(_, InstructionError::Custom(c)) => Some(c),
                    _ => None,
                };
                Err((code, format!("{:?}\n{}", f.err, f.meta.logs.join("\n"))))
            }
        }
    }

    pub fn ok(&mut self, i: ix::Ixn, signers: &[&Keypair]) -> TransactionMetadata {
        match self.send(i, signers) {
            Ok(m) => m,
            Err((_, e)) => panic!("transaction failed: {e}"),
        }
    }

    pub fn fails(&mut self, i: ix::Ixn, signers: &[&Keypair], want: TtError) {
        match self.send(i, signers) {
            Ok(_) => panic!("expected {want:?}, transaction succeeded"),
            Err((code, logs)) => assert_eq!(code, Some(want as u32), "expected {want:?}: {logs}"),
        }
    }

    pub fn acct<T: Pod>(&self, k: &Pubkey) -> T {
        let a = self.svm.get_account(k).expect("account exists");
        bytemuck::pod_read_unaligned(&a.data[..std::mem::size_of::<T>()])
    }

    pub fn market(&self) -> Market {
        self.acct(&pda::market())
    }
    pub fn trader(&self, owner: &Pubkey) -> Trader {
        self.acct(&pda::trader(owner))
    }
    pub fn operator(&self, owner: &Pubkey) -> Operator {
        self.acct(&pda::operator(owner))
    }
    pub fn token_amount(&self, k: &Pubkey) -> u64 {
        let a = self.svm.get_account(k).unwrap();
        u64::from_le_bytes(a.data[64..72].try_into().unwrap())
    }

    pub fn write_pyth(&mut self, price: i64, expo: i32, publish_time: i64) {
        let d = pyth_bytes(price, expo, publish_time);
        self.write(feed(), d, Pubkey::new_from_array(PYTH_RECEIVER));
    }

    pub fn init_market(&mut self) {
        let admin = self.admin.insecure_clone();
        self.ok(ix::initialize_market(&admin.pubkey(), Params::DEFAULT, Params::DEFAULT.index_max_secs), &[&admin]);
    }

    /// Creates a funded trader with `deposit` USDC in the market and `extra` USDC left in its wallet.
    pub fn funded_trader(&mut self, deposit: u64, extra: u64) -> User {
        let u = self.user(deposit + extra);
        self.ok(ix::create_trader(&u.key()), &[&u.kp]);
        if deposit > 0 {
            self.ok(ix::deposit(&u.key(), &u.usdc, deposit), &[&u.kp]);
        }
        u
    }

    /// Registers an operator with `capital` USDC and quotes a curve at `fair_secs`.
    pub fn operator_with_curve(&mut self, capital: u64, fair_secs: f64, impact: f64, limit: u64) -> User {
        let u = self.funded_trader(capital, 1_000 * USDC);
        self.ok(ix::register_operator(&u.key(), &u.usdc), &[&u.kp]);
        self.quote(&u, fair_secs, impact, limit);
        u
    }

    pub fn quote(&mut self, u: &User, fair_secs: f64, impact: f64, limit: u64) {
        self.ok(ix::update_curve(&u.key(), ln_secs(fair_secs), from_f64(0.0002), from_f64(impact), limit), &[&u.kp]);
    }
}

pub fn fx(x: f64) -> Fx {
    from_f64(x)
}

pub fn to_f(x: Fx) -> f64 {
    x as f64 / ONE as f64
}

/// Decodes "Program data: <base64>" log lines into raw event bytes.
pub fn events(meta: &TransactionMetadata) -> Vec<Vec<u8>> {
    meta.logs
        .iter()
        .filter_map(|l| l.strip_prefix("Program data: "))
        .map(|b| b64(b.split(' ').next().unwrap()))
        .collect()
}

fn b64(s: &str) -> Vec<u8> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|c| *c != b'=') {
        buf = (buf << 6) | A.iter().position(|x| *x == c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}
