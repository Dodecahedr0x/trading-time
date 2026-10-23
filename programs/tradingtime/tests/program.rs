mod common;

use common::*;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use tradingtime_interface::{events::*, *};
use tradingtime_math::{ONE, position};

fn decoded<T: bytemuck::Pod>(meta: &litesvm::types::TransactionMetadata, tag: u8) -> Vec<T> {
    events(meta).into_iter().filter(|b| b[0] == tag).map(|b| bytemuck::pod_read_unaligned(&b[1..])).collect()
}

fn market_with_ops() -> (Env, User, User) {
    let mut e = Env::new();
    e.init_market();
    let a = e.operator_with_curve(20_000 * USDC, 3600.0, 1e-5, 20_000 * USDC);
    let b = e.operator_with_curve(20_000 * USDC, 3700.0, 1e-5, 20_000 * USDC);
    (e, a, b)
}

fn no_limit_long() -> i128 {
    ln_secs(1e9)
}

fn no_limit_short() -> i128 {
    ln_secs(1.0)
}

fn equity(e: &Env, owner: &Pubkey) -> i64 {
    let m = e.market();
    let t = e.trader(owner);
    position::equity(&t.position(), m.effective_mark(e.ts)).unwrap()
}

// ---- scenarios (shared by the per-instruction tests and the CU report) ----

fn scenario_initialize() {
    let mut e = Env::new();
    let admin = e.admin.insecure_clone();
    let mut bad = Params::DEFAULT;
    bad.max_leverage_bps = 60_000;
    let max = Params::DEFAULT.index_max_secs;
    e.fails(ix::initialize_market(&admin.pubkey(), bad, max), &[&admin], TtError::ParamOutOfBounds);
    for secs in [0, Params::DEFAULT.index_min_secs - 1, max + 1] {
        e.fails(ix::initialize_market(&admin.pubkey(), Params::DEFAULT, secs), &[&admin], TtError::ParamOutOfBounds);
    }
    let mut wrong_mint = ix::initialize_market(&admin.pubkey(), Params::DEFAULT, max);
    wrong_mint.1.accounts[3].pubkey = Pubkey::new_unique();
    e.fails(wrong_mint, &[&admin], TtError::InvalidAccount);
    let mut wrong_market = ix::initialize_market(&admin.pubkey(), Params::DEFAULT, max);
    wrong_market.1.accounts[1].pubkey = Pubkey::new_unique();
    e.fails(wrong_market, &[&admin], TtError::InvalidPda);

    e.ok(ix::initialize_market(&admin.pubkey(), Params::DEFAULT, 5_400), &[&admin]);
    let m = e.market();
    assert_eq!(m.discriminator, DISC_MARKET);
    assert_eq!(m.admin, admin.pubkey().to_bytes());
    assert_eq!(m.params, Params::DEFAULT);
    assert_eq!(m.vault, pda::vault().to_bytes());
    let p = Params::DEFAULT;
    let var0 = m.var_ewma.get();
    let want = tradingtime_math::index::index_ln(var0, p.calib.get(), p.band_bps, p.index_min_secs, p.index_max_secs).unwrap();
    assert!(var0 > 0);
    assert!(((m.index_ln.get() - want) as f64 / want as f64).abs() < 1e-6);
    assert!((to_f(m.index_ln.get()).exp() - 5_400.0).abs() < 1.0, "{}", to_f(m.index_ln.get()).exp());
    let vault = e.svm.get_account(&pda::vault()).unwrap();
    assert_eq!(vault.owner, token_program());
    assert_eq!(&vault.data[32..64], pda::market().as_ref());
    e.fails(ix::initialize_market(&admin.pubkey(), Params::DEFAULT, max), &[&admin], TtError::AlreadyInitialized);
}

fn scenario_admin() {
    let mut e = Env::new();
    e.init_market();
    let admin = e.admin.insecure_clone();
    let mallory = e.user(0);
    let mut p = Params::DEFAULT;
    p.taker_fee_bps = 10;
    e.fails(ix::update_params(&mallory.key(), p), &[&mallory.kp], TtError::Unauthorized);
    let mut bad = p;
    bad.maintenance_bps = 50;
    e.fails(ix::update_params(&admin.pubkey(), bad), &[&admin], TtError::ParamOutOfBounds);
    e.ok(ix::update_params(&admin.pubkey(), p), &[&admin]);
    assert_eq!(e.market().params.taker_fee_bps, 10);

    e.fails(ix::set_paused(&mallory.key(), true), &[&mallory.kp], TtError::Unauthorized);
    e.ok(ix::set_paused(&admin.pubkey(), true), &[&admin]);
    assert_eq!(e.market().paused, 1);
    e.ok(ix::set_paused(&admin.pubkey(), false), &[&admin]);
    assert_eq!(e.market().paused, 0);
}

fn scenario_trader_cash() {
    let mut e = Env::new();
    e.init_market();
    let u = e.user(5_000 * USDC);
    e.ok(ix::create_trader(&u.key()), &[&u.kp]);
    let t = e.trader(&u.key());
    assert_eq!((t.discriminator, t.owner, t.cash), (DISC_TRADER, u.key().to_bytes(), 0));
    e.fails(ix::create_trader(&u.key()), &[&u.kp], TtError::AlreadyInitialized);

    let meta = e.ok(ix::deposit(&u.key(), &u.usdc, 3_000 * USDC), &[&u.kp]);
    let ev: Vec<CashEvent> = decoded(&meta, TAG_CASH);
    assert_eq!((ev[0].kind, ev[0].amount), (CASH_DEPOSIT, 3_000 * USDC as i64));
    assert_eq!(e.trader(&u.key()).cash, 3_000 * USDC as i64);
    assert_eq!(e.token_amount(&pda::vault()), 3_000 * USDC);
    assert_eq!(e.market().total_cash, 3_000 * USDC);

    let other = e.user(0);
    let mut wrong_vault = ix::deposit(&u.key(), &u.usdc, USDC);
    wrong_vault.1.accounts[4].pubkey = other.usdc;
    e.fails(wrong_vault, &[&u.kp], TtError::InvalidAccount);
    let mut not_owner = ix::deposit(&other.key(), &other.usdc, USDC);
    not_owner.1.accounts[1].pubkey = pda::trader(&u.key());
    e.fails(not_owner, &[&other.kp], TtError::Unauthorized);

    e.fails(ix::withdraw(&u.key(), &u.usdc, 4_000 * USDC, false), &[&u.kp], TtError::InsufficientCollateral);
    let meta = e.ok(ix::withdraw(&u.key(), &u.usdc, 1_000 * USDC, false), &[&u.kp]);
    let ev: Vec<CashEvent> = decoded(&meta, TAG_CASH);
    assert_eq!(ev[0].kind, CASH_WITHDRAW);
    assert_eq!(e.trader(&u.key()).cash, 2_000 * USDC as i64);
    assert_eq!(e.token_amount(&u.usdc), 3_000 * USDC);
    assert_eq!(e.market().total_cash, 2_000 * USDC);
}

fn scenario_operator_lifecycle() {
    let mut e = Env::new();
    e.init_market();
    let poor = e.funded_trader(5_000 * USDC, 1_000 * USDC);
    e.fails(ix::register_operator(&poor.key(), &poor.usdc), &[&poor.kp], TtError::InsufficientCollateral);

    let o = e.funded_trader(12_000 * USDC, 1_000 * USDC);
    let meta = e.ok(ix::register_operator(&o.key(), &o.usdc), &[&o.kp]);
    let ev: Vec<OperatorEvent> = decoded(&meta, TAG_OPERATOR);
    assert_eq!((ev[0].kind, ev[0].bond), (OPERATOR_REGISTERED, 1_000 * USDC));
    let op = e.operator(&o.key());
    assert_eq!((op.discriminator, op.bond, op.status), (DISC_OPERATOR, 1_000 * USDC, OperatorStatus::Active as u8));
    assert_eq!(op.trader, pda::trader(&o.key()).to_bytes());
    assert_eq!(e.trader(&o.key()).is_operator, 1);
    assert_eq!(e.token_amount(&o.usdc), 0);
    assert_eq!(e.market().operator_count, 1);
    e.fails(ix::register_operator(&o.key(), &o.usdc), &[&o.kp], TtError::AlreadyInitialized);

    e.quote(&o, 3600.0, 1e-5, 10_000 * USDC);
    let op = e.operator(&o.key());
    assert_eq!(op.updated_slot, e.slot);
    assert_eq!(op.inventory_limit, 10_000 * USDC);
    let mallory = e.user(0);
    let mut stolen = ix::update_curve(&mallory.key(), ln_secs(60.0), 0, 0, 1);
    stolen.1.accounts[1].pubkey = pda::operator(&o.key());
    e.fails(stolen, &[&mallory.kp], TtError::Unauthorized);
    e.fails(ix::update_curve(&o.key(), ln_secs(60.0), -1, 0, 1), &[&o.kp], TtError::ParamOutOfBounds);

    e.fails(ix::withdraw(&o.key(), &o.usdc, USDC, true), &[&o.kp], TtError::OperatorActive);
    e.fails(ix::complete_unbond(&o.key(), &o.usdc), &[&o.kp], TtError::OperatorActive);
    let meta = e.ok(ix::request_unbond(&o.key()), &[&o.kp]);
    assert_eq!(decoded::<OperatorEvent>(&meta, TAG_OPERATOR)[0].kind, OPERATOR_UNBOND_REQUESTED);
    e.fails(ix::request_unbond(&o.key()), &[&o.kp], TtError::OperatorNotActive);
    e.fails(ix::register_operator(&o.key(), &o.usdc), &[&o.kp], TtError::AlreadyInitialized);
    e.fails(ix::update_curve(&o.key(), ln_secs(60.0), 0, 0, 1), &[&o.kp], TtError::OperatorNotActive);
    e.fails(ix::complete_unbond(&o.key(), &o.usdc), &[&o.kp], TtError::UnbondNotReady);
    e.fails(ix::withdraw(&o.key(), &o.usdc, USDC, true), &[&o.kp], TtError::OperatorActive);
    e.advance(10, 86_400);
    let meta = e.ok(ix::complete_unbond(&o.key(), &o.usdc), &[&o.kp]);
    assert_eq!(decoded::<OperatorEvent>(&meta, TAG_OPERATOR)[0].kind, OPERATOR_CLOSED);
    assert_eq!(e.operator(&o.key()).status, OperatorStatus::Closed as u8);
    assert_eq!(e.token_amount(&o.usdc), 1_000 * USDC);
    assert_eq!(e.market().operator_count, 0);
    e.fails(ix::complete_unbond(&o.key(), &o.usdc), &[&o.kp], TtError::OperatorNotActive);
    e.ok(ix::withdraw(&o.key(), &o.usdc, 12_000 * USDC, true), &[&o.kp]);
    assert_eq!(e.token_amount(&o.usdc), 13_000 * USDC);

    // A Closed operator re-registers with a fresh bond and the same capital checks.
    e.ok(ix::deposit(&o.key(), &o.usdc, 9_000 * USDC), &[&o.kp]);
    e.fails(ix::register_operator(&o.key(), &o.usdc), &[&o.kp], TtError::InsufficientCollateral);
    e.ok(ix::deposit(&o.key(), &o.usdc, 2_000 * USDC), &[&o.kp]);
    let meta = e.ok(ix::register_operator(&o.key(), &o.usdc), &[&o.kp]);
    assert_eq!(decoded::<OperatorEvent>(&meta, TAG_OPERATOR)[0].kind, OPERATOR_REGISTERED);
    let op = e.operator(&o.key());
    assert_eq!((op.bond, op.status, op.unbond_requested_ts), (1_000 * USDC, OperatorStatus::Active as u8, 0));
    assert_eq!((op.fair_ln.get(), op.inventory_limit, op.updated_slot), (0, 0, 0));
    assert_eq!(e.token_amount(&o.usdc), 1_000 * USDC);
    assert_eq!(e.market().operator_count, 1);
    e.fails(ix::register_operator(&o.key(), &o.usdc), &[&o.kp], TtError::AlreadyInitialized);
    e.advance(1, 1);
    e.quote(&o, 3600.0, 1e-5, 10_000 * USDC);
    let t = e.funded_trader(1_000 * USDC, 0);
    let meta = e.ok(ix::swap(&t.key(), Side::Long, 100 * USDC, 100 * USDC, no_limit_long(), &[o.key()]), &[&t.kp]);
    assert_eq!(decoded::<TradeEvent>(&meta, TAG_TRADE)[0].operator, o.key().to_bytes());
}

fn scenario_swap() {
    let (mut e, a, b) = market_with_ops();
    let ops = [a.key(), b.key()];
    let t = e.funded_trader(10_000 * USDC, 0);
    let meta = e.ok(ix::swap(&t.key(), Side::Long, 5_000 * USDC, 5_000 * USDC, no_limit_long(), &ops), &[&t.kp]);
    let fills: Vec<TradeEvent> = decoded(&meta, TAG_TRADE);
    assert_eq!(fills.len(), 2, "routes across both curves");
    assert_eq!(fills.iter().map(|f| f.size).sum::<u64>(), 5_000 * USDC);
    let fee = 5_000 * USDC * Params::DEFAULT.taker_fee_bps as u64 / 10_000;
    assert_eq!(fills.iter().map(|f| f.fee).sum::<u64>(), fee);
    assert!(fills.iter().all(|f| f.kind == TRADE_SWAP && f.trader == t.key().to_bytes()));
    let tr = e.trader(&t.key());
    let (qa, qb) = (e.trader(&a.key()).q, e.trader(&b.key()).q);
    assert_eq!(tr.q, 5_000 * USDC as i64);
    assert_eq!(qa + qb, -tr.q);
    assert!(qa < 0 && qb < 0);
    assert_eq!(tr.cash, (10_000 * USDC - fee) as i64);
    let m = e.market();
    // Insurance is below its target (5% of open interest), so the whole fee funds it.
    assert_eq!((m.insurance, m.treasury, m.long_q, m.short_q), (fee, 0, 5_000 * USDC, 5_000 * USDC));
    assert_eq!(m.mark_updated_ts, e.ts);
    let avg = to_f(tr.cost.get()) / tr.q as f64;
    assert!(avg > 3600f64.ln() && avg < 3700f64.ln() + 0.1, "avg ln price {avg}");

    let meta = e.ok(ix::swap(&t.key(), Side::Short, 5_000 * USDC, 0, no_limit_short(), &ops), &[&t.kp]);
    assert!(!decoded::<TradeEvent>(&meta, TAG_TRADE).is_empty());
    let tr = e.trader(&t.key());
    assert_eq!((tr.q, tr.cost.get()), (0, 0));
    // Fair values differ, so the round trip arbitrages A against B; only fees are certain.
    let m = e.market();
    assert_eq!(m.insurance + m.treasury, 2 * fee);
    assert!(m.insurance <= fee.max(m.long_q * Params::DEFAULT.insurance_target_bps as u64 / 10_000));
    let m = e.market();
    let qa = e.trader(&a.key()).q;
    assert_eq!(qa + e.trader(&b.key()).q, 0);
    assert_eq!((m.long_q, m.short_q), (qa.unsigned_abs(), qa.unsigned_abs()));
}

fn scenario_swap_errors() {
    let (mut e, a, b) = market_with_ops();
    let ops = [a.key(), b.key()];
    let t = e.funded_trader(10_000 * USDC, 0);
    let admin = e.admin.insecure_clone();
    e.fails(ix::swap(&t.key(), Side::Long, 1_000 * USDC, 1, ln_secs(3000.0), &ops), &[&t.kp], TtError::MinFillNotMet);
    e.fails(ix::swap(&t.key(), Side::Long, 1_000 * USDC, 2_000 * USDC, no_limit_long(), &ops), &[&t.kp], TtError::MinFillNotMet);
    e.fails(ix::swap(&t.key(), Side::Long, 15_000 * USDC, 0, no_limit_long(), &ops), &[&t.kp], TtError::InsufficientMargin);
    let nine: Vec<Pubkey> = (0..9).map(|_| Pubkey::new_unique()).collect();
    e.fails(ix::swap(&t.key(), Side::Long, USDC, 0, no_limit_long(), &nine), &[&t.kp], TtError::TooManyAccounts);
    e.fails(ix::swap(&a.key(), Side::Long, USDC, 0, no_limit_long(), &ops), &[&a.kp], TtError::InvalidAccount);
    let mut mismatched = ix::swap(&t.key(), Side::Long, USDC, 0, no_limit_long(), &ops);
    mismatched.1.accounts[4].pubkey = pda::trader(&b.key());
    mismatched.1.accounts[6].pubkey = pda::trader(&a.key());
    e.fails(mismatched, &[&t.kp], TtError::InvalidAccount);

    e.ok(ix::swap(&t.key(), Side::Long, 2_000 * USDC, 0, no_limit_long(), &ops), &[&t.kp]);
    e.ok(ix::set_paused(&admin.pubkey(), true), &[&admin]);
    e.fails(ix::swap(&t.key(), Side::Long, 100 * USDC, 0, no_limit_long(), &ops), &[&t.kp], TtError::Paused);
    e.ok(ix::swap(&t.key(), Side::Short, 500 * USDC, 0, no_limit_short(), &ops), &[&t.kp]);
    e.ok(ix::set_paused(&admin.pubkey(), false), &[&admin]);

    e.advance(11, 5);
    e.fails(ix::swap(&t.key(), Side::Long, 100 * USDC, 0, no_limit_long(), &ops), &[&t.kp], TtError::NoFreshCurves);
}

fn scenario_limit_orders() {
    let (mut e, a, b) = market_with_ops();
    let ops = [a.key(), b.key()];
    let t = e.funded_trader(10_000 * USDC, 0);
    let k = e.funded_trader(0, 0);
    let ts = e.ts;
    e.fails(ix::place_limit_order(&t.key(), 1, Side::Long, 1_000 * USDC, ln_secs(3000.0), ts), &[&t.kp], TtError::OrderExpired);
    e.fails(
        ix::place_limit_order(&t.key(), 1, Side::Long, 20_000 * USDC, ln_secs(3000.0), ts + 3600),
        &[&t.kp],
        TtError::InsufficientMargin,
    );
    let meta = e.ok(ix::place_limit_order(&t.key(), 1, Side::Long, 1_000 * USDC, ln_secs(3000.0), ts + 3600), &[&t.kp]);
    let ev: Vec<OrderEvent> = decoded(&meta, TAG_ORDER);
    assert_eq!((ev[0].kind, ev[0].order_id, ev[0].size), (ORDER_PLACED, 1, 1_000 * USDC));
    let tr = e.trader(&t.key());
    assert_eq!((tr.reserved_long, tr.reserved_short, tr.open_orders), (1_000 * USDC, 0, 1));
    let o: LimitOrder = e.acct(&pda::order(&t.key(), 1));
    assert_eq!((o.discriminator, o.size, o.reserved, o.created_slot), (DISC_ORDER, 1_000 * USDC, 1_000 * USDC, e.slot));

    e.fails(ix::execute_limit_order(&k.key(), &t.key(), 1, &ops), &[&k.kp], TtError::LimitNotReached);
    e.fails(ix::execute_limit_order(&t.key(), &t.key(), 1, &ops), &[&t.kp], TtError::InvalidAccount);

    e.advance(1, 1);
    e.quote(&a, 2900.0, 1e-5, 20_000 * USDC);
    e.quote(&b, 2950.0, 1e-5, 20_000 * USDC);
    let meta = e.ok(ix::execute_limit_order(&k.key(), &t.key(), 1, &ops), &[&k.kp]);
    let ev: Vec<OrderEvent> = decoded(&meta, TAG_ORDER);
    assert_eq!((ev[0].kind, ev[0].filled), (ORDER_FILLED, 1_000 * USDC));
    let fills: Vec<TradeEvent> = decoded(&meta, TAG_TRADE);
    assert!(fills.iter().all(|f| f.kind == TRADE_LIMIT && to_f(f.ln_price.get()) <= 3000f64.ln()));
    assert!(e.svm.get_account(&pda::order(&t.key(), 1)).is_none_or(|a| a.lamports == 0));
    let tr = e.trader(&t.key());
    assert_eq!((tr.q, tr.reserved_long, tr.open_orders), (1_000 * USDC as i64, 0, 0));
    assert_eq!(e.trader(&k.key()).cash, 100_000, "keeper earns 1bp");

    e.ok(ix::place_limit_order(&t.key(), 2, Side::Short, 500 * USDC, ln_secs(10.0), ts + 3600), &[&t.kp]);
    let rent_before = e.svm.get_balance(&t.key()).unwrap();
    let meta = e.ok(ix::cancel_limit_order(&t.key(), 2), &[&t.kp]);
    assert_eq!(decoded::<OrderEvent>(&meta, TAG_ORDER)[0].kind, ORDER_CANCELLED);
    assert!(e.svm.get_balance(&t.key()).unwrap() > rent_before);
    let tr = e.trader(&t.key());
    assert_eq!((tr.reserved_long, tr.reserved_short, tr.open_orders), (0, 0, 0));
    let mallory = e.funded_trader(0, 0);
    e.ok(ix::place_limit_order(&t.key(), 3, Side::Long, 100 * USDC, ln_secs(1e6), e.ts + 10), &[&t.kp]);
    let mut theft = ix::cancel_limit_order(&mallory.key(), 3);
    theft.1.accounts[2].pubkey = pda::order(&t.key(), 3);
    e.fails(theft, &[&mallory.kp], TtError::Unauthorized);
    e.fails(ix::cancel_expired_order(&mallory.key(), &t.key(), 3), &[&mallory.kp], TtError::Unauthorized);
    e.advance(1, 20);
    e.fails(ix::execute_limit_order(&k.key(), &t.key(), 3, &ops), &[&k.kp], TtError::OrderExpired);
    let mut wrong_payee = ix::cancel_expired_order(&mallory.key(), &t.key(), 3);
    wrong_payee.1.accounts[3].pubkey = mallory.key();
    e.fails(wrong_payee, &[&mallory.kp], TtError::InvalidAccount);
    let (owner_before, stranger_before) = (e.svm.get_balance(&t.key()).unwrap(), e.svm.get_balance(&mallory.key()).unwrap());
    let rent = e.svm.get_balance(&pda::order(&t.key(), 3)).unwrap();
    let meta = e.ok(ix::cancel_expired_order(&mallory.key(), &t.key(), 3), &[&mallory.kp]);
    assert_eq!(decoded::<OrderEvent>(&meta, TAG_ORDER)[0].kind, ORDER_CANCELLED);
    assert_eq!(e.svm.get_balance(&t.key()).unwrap(), owner_before + rent);
    assert!(e.svm.get_balance(&mallory.key()).unwrap() < stranger_before, "stranger only pays the fee");
    let tr = e.trader(&t.key());
    assert_eq!((tr.reserved_long, tr.reserved_short, tr.open_orders), (0, 0, 0));
}

/// A65 override: open limit orders reserve only the exposure they can add beyond the position.
fn scenario_netted_limit_orders() {
    let (mut e, a, b) = market_with_ops();
    let ops = [a.key(), b.key()];
    let t = e.funded_trader(1_000 * USDC, 0);
    let k = e.funded_trader(0, 0);
    e.ok(ix::swap(&t.key(), Side::Long, 950 * USDC, 950 * USDC, no_limit_long(), &ops), &[&t.kp]);
    let q = e.trader(&t.key()).q as u64;
    let eq = equity(&e, &t.key()) as u64;
    assert!(q <= eq && eq < 2 * q, "q {q} eq {eq}");
    let exp = e.ts + 3600;

    // Closing the whole position reserves nothing extra (gross |q| + size would need 2q).
    e.ok(ix::place_limit_order(&t.key(), 1, Side::Short, q, ln_secs(10.0), exp), &[&t.kp]);
    assert_eq!(e.trader(&t.key()).reserved_short, q);
    e.ok(ix::cancel_limit_order(&t.key(), 1), &[&t.kp]);
    // A flip is margined only for the part past zero.
    e.fails(ix::place_limit_order(&t.key(), 2, Side::Short, q + eq + 1, ln_secs(10.0), exp), &[&t.kp], TtError::InsufficientMargin);
    e.ok(ix::place_limit_order(&t.key(), 2, Side::Short, q + eq, ln_secs(10.0), exp), &[&t.kp]);
    // Long orders add to the long side independently: max(|q + R_long|, |q − R_short|).
    e.ok(ix::place_limit_order(&t.key(), 3, Side::Long, eq - q, ln_secs(1e6), exp), &[&t.kp]);
    e.fails(ix::place_limit_order(&t.key(), 4, Side::Long, 1, ln_secs(1e6), exp), &[&t.kp], TtError::InsufficientMargin);
    let tr = e.trader(&t.key());
    assert_eq!((tr.reserved_long, tr.reserved_short, tr.open_orders), (eq - q, q + eq, 2));
    assert_eq!(tr.margin_need(), eq as u128);
    e.ok(ix::cancel_limit_order(&t.key(), 2), &[&t.kp]);
    e.ok(ix::cancel_limit_order(&t.key(), 3), &[&t.kp]);

    // A reducing order fills and releases its side.
    e.ok(ix::place_limit_order(&t.key(), 5, Side::Short, q, ln_secs(10.0), exp), &[&t.kp]);
    e.ok(ix::execute_limit_order(&k.key(), &t.key(), 5, &ops), &[&k.kp]);
    let tr = e.trader(&t.key());
    assert_eq!((tr.q, tr.reserved_long, tr.reserved_short, tr.open_orders), (0, 0, 0, 0));
}

fn scenario_crank_index() {
    let mut e = Env::new();
    e.init_market();
    let var0 = e.market().var_ewma.get();
    let ts = e.ts;
    e.write_pyth(150_00000000, -8, ts - 5);
    let meta = e.ok(ix::crank_index(), &[&e.admin.insecure_clone()]);
    let ev: Vec<IndexEvent> = decoded(&meta, TAG_INDEX);
    assert_eq!((ev[0].pyth_price, ev[0].publish_time), (150_00000000, ts - 5));
    let m = e.market();
    assert_eq!((m.last_pyth_price, m.last_publish_time, m.var_ewma.get()), (150_00000000, ts - 5, var0));
    let payer = e.admin.insecure_clone();
    e.fails(ix::crank_index(), &[&payer], TtError::StaleOrSamePrice);

    e.advance(150, 60);
    e.write_pyth(150_30000000, -8, e.ts - 2);
    e.ok(ix::crank_index(), &[&payer]);
    let m = e.market();
    assert!(m.var_ewma.get() > 0);
    let r = (150.3f64 / 150.0).ln();
    let alpha = 1.0 - 2f64.powf(-63.0 / 604_800.0);
    let want_var = (1.0 - alpha) * to_f(var0) + alpha * r * r / 63.0;
    assert!((to_f(m.var_ewma.get()) / want_var - 1.0).abs() < 1e-3, "var {}", to_f(m.var_ewma.get()));
    let idx = to_f(m.index_ln.get());
    assert!(idx >= 60f64.ln() - 1e-9 && idx <= 604_800f64.ln() + 1e-9);
    assert_eq!(m.index_updated_ts, e.ts);

    e.advance(750, 300);
    e.write_pyth(150_00000000, -8, e.ts - 200);
    e.fails(ix::crank_index(), &[&payer], TtError::PythStale);
    let mut bad = pyth_bytes(150_00000000, -8, e.ts);
    bad[pyth::VERIFICATION_TAG] = 0;
    e.write(feed(), bad, Pubkey::new_from_array(PYTH_RECEIVER));
    e.fails(ix::crank_index(), &[&payer], TtError::PythInvalid);
    let mut wrong_feed = pyth_bytes(150_00000000, -8, e.ts);
    wrong_feed[pyth::FEED_ID] ^= 1;
    e.write(feed(), wrong_feed, Pubkey::new_from_array(PYTH_RECEIVER));
    e.fails(ix::crank_index(), &[&payer], TtError::PythInvalid);
    e.write(feed(), pyth_bytes(150_00000000, -8, e.ts), Pubkey::new_unique());
    e.fails(ix::crank_index(), &[&payer], TtError::InvalidOwner);
}

fn scenario_funding_and_mark() {
    let (mut e, a, b) = market_with_ops();
    let ops = [a.key(), b.key()];
    let payer = e.admin.insecure_clone();
    let t = e.funded_trader(10_000 * USDC, USDC);
    e.ok(ix::swap(&t.key(), Side::Long, 5_000 * USDC, 0, no_limit_long(), &ops), &[&t.kp]);

    e.advance(1, 3_600);
    let meta = e.ok(ix::crank_mark(&ops), &[&payer]);
    assert!(meta.compute_units_consumed > 0);
    let m = e.market();
    assert_eq!(m.mark_updated_ts, e.ts);
    let mark = to_f(m.mark_ln.get());
    assert!(mark > 3600f64.ln() && mark < 3700f64.ln() + 0.1, "mark {mark}");

    let meta = e.ok(ix::crank_funding(), &[&payer]);
    let ev: Vec<FundingEvent> = decoded(&meta, TAG_FUNDING);
    let m = e.market();
    let want = (m.mark_ln.get() - m.index_ln.get()) * 3_600 / 86_400;
    assert_eq!(m.funding_index.get(), want);
    assert_eq!(ev[0].funding_index.get(), want);
    assert_eq!(m.funding_updated_ts, e.ts);

    let cash0 = e.trader(&t.key()).cash;
    let meta = e.ok(ix::deposit(&t.key(), &t.usdc, USDC), &[&t.kp]);
    let cash: Vec<CashEvent> = decoded(&meta, TAG_CASH);
    let funding = cash.iter().find(|c| c.kind == CASH_FUNDING).expect("funding settled on touch");
    let owed = (5_000 * USDC) as i128 * want;
    assert_eq!(funding.amount as i128, -(owed.div_euclid(ONE) + (owed.rem_euclid(ONE) != 0) as i128));
    assert!(funding.amount > 0, "longs receive when mark < index");
    assert_eq!(e.trader(&t.key()).cash, cash0 + funding.amount + USDC as i64);

    e.fails(ix::crank_mark(&[a.key(), a.key(), b.key()]), &[&payer], TtError::InvalidAccount);
    e.advance(20, 1);
    e.fails(ix::crank_mark(&ops), &[&payer], TtError::NoFreshCurves);
}

/// Moves both operators' fair values so the curve-median mark lands on `target`.
fn shift_mark(e: &mut Env, a: &User, b: &User, target: i128) {
    let shift = target - e.market().mark_ln.get();
    e.advance(1, 1);
    for o in [a, b] {
        let op = e.operator(&o.key());
        let i = ix::update_curve(&o.key(), op.fair_ln.get() + shift, op.half_spread.get(), op.impact_per_usdc.get(), op.inventory_limit);
        e.ok(i, &[&o.kp]);
    }
    let payer = e.admin.insecure_clone();
    e.ok(ix::crank_mark(&[a.key(), b.key()]), &[&payer]);
    assert!((e.market().mark_ln.get() - target).abs() < ONE / 1_000_000);
}

fn scenario_liquidation() {
    let (mut e, a, b) = market_with_ops();
    let ops = [a.key(), b.key()];
    let liq = e.user(0);
    let t = e.funded_trader(10_000 * USDC, 0);
    let s = e.funded_trader(10_000 * USDC, 0);
    let l2 = e.funded_trader(10_000 * USDC, 0);
    e.ok(ix::swap(&t.key(), Side::Long, 9_000 * USDC, 0, no_limit_long(), &ops), &[&t.kp]);
    e.ok(ix::swap(&s.key(), Side::Short, 3_000 * USDC, 0, no_limit_short(), &ops), &[&s.kp]);
    e.ok(ix::swap(&l2.key(), Side::Long, 1_000 * USDC, 0, no_limit_long(), &ops), &[&l2.kp]);
    e.fails(ix::liquidate(&liq.key(), &t.key(), &ops, &[]), &[&liq.kp], TtError::NotLiquidatable);
    e.fails(ix::liquidate(&liq.key(), &e.admin.pubkey(), &ops, &[]), &[&liq.kp], TtError::InvalidOwner);

    // Equity at half the maintenance requirement: liquidatable but solvent.
    let tr = e.trader(&t.key());
    let maint = position::maintenance(tr.q, 500) as i128;
    let target = (tr.cost.get() - (tr.cash as i128 - maint / 2) * ONE) / tr.q as i128;
    shift_mark(&mut e, &a, &b, target);
    assert!(equity(&e, &t.key()) < maint as i64 && equity(&e, &t.key()) > 0);
    let q0 = e.trader(&t.key()).q;
    // Taker fees from the opening swaps already sit in the insurance fund.
    let ins_before = e.market().insurance;
    assert!(ins_before > 0);
    let meta = e.ok(ix::liquidate(&liq.key(), &t.key(), &ops, &[]), &[&liq.kp]);
    let lev: Vec<LiquidationEvent> = decoded(&meta, TAG_LIQUIDATION);
    let fills: Vec<TradeEvent> = decoded(&meta, TAG_TRADE);
    assert!(fills.iter().all(|f| f.kind == TRADE_LIQUIDATION));
    let q1 = e.trader(&t.key()).q;
    assert!(q1 >= 0 && q1 < q0, "partially closed against curves: {q0} -> {q1}");
    assert_eq!(lev[0].size, (q0 - q1) as u64);
    assert_eq!(lev[0].liquidator, liq.key().to_bytes());
    assert!(lev[0].penalty > 0);
    assert_eq!(e.market().insurance, ins_before + lev[0].penalty);
    let bk = lev[0].bankruptcy_ln.get();
    assert!(fills.iter().all(|f| f.ln_price.get() >= bk - 1));
    assert!(q1 > 0, "curves alone cannot absorb at bankruptcy");

    // Push the rest underwater; insurance absorbs the deficit, ADL closes against the short.
    let tr = e.trader(&t.key());
    let deficit = (e.market().insurance / 2) as i128;
    let target = (tr.cost.get() - (tr.cash as i128 + deficit) * ONE) / tr.q as i128;
    shift_mark(&mut e, &a, &b, target);
    assert!(equity(&e, &t.key()) < 0);
    e.fails(ix::liquidate(&liq.key(), &t.key(), &[], &[l2.key()]), &[&liq.kp], TtError::AdlInvalidCounterparty);
    assert!(e.trader(&a.key()).q < 0, "operator a is an in-profit short");
    e.fails(ix::liquidate(&liq.key(), &t.key(), &[], &[a.key()]), &[&liq.kp], TtError::AdlInvalidCounterparty);
    e.fails(ix::liquidate(&liq.key(), &t.key(), &[], &[s.key(), b.key()]), &[&liq.kp], TtError::AdlInvalidCounterparty);
    let sq = e.trader(&s.key()).q;
    let ins0 = e.market().insurance;
    let meta = e.ok(ix::liquidate(&liq.key(), &t.key(), &[], &[s.key()]), &[&liq.kp]);
    let lev: Vec<LiquidationEvent> = decoded(&meta, TAG_LIQUIDATION);
    let adl: Vec<TradeEvent> = decoded(&meta, TAG_TRADE);
    assert!(lev[0].insurance_used > 0);
    assert_eq!(adl.len(), 1);
    assert_eq!((adl[0].kind, adl[0].operator), (TRADE_ADL, s.key().to_bytes()));
    let closed = q1.unsigned_abs().min(sq.unsigned_abs());
    assert_eq!(adl[0].size, closed);
    assert_eq!(e.trader(&s.key()).q, sq + closed as i64);
    assert_eq!(e.trader(&t.key()).q, q1 - closed as i64);
    assert_eq!(e.market().insurance, ins0 - lev[0].insurance_used + lev[0].penalty);
    let m = e.market();
    let total_q: i64 = [&t, &s, &l2, &a, &b].iter().map(|u| e.trader(&u.key()).q).sum();
    assert_eq!(total_q, 0);
    assert_eq!(m.long_q, m.short_q);
}

fn scenario_treasury() {
    let (mut e, a, b) = market_with_ops();
    let t = e.funded_trader(10_000 * USDC, 0);
    let admin = e.admin.insecure_clone();
    // No insurance target: every fee goes to the treasury.
    e.ok(ix::update_params(&admin.pubkey(), Params { insurance_target_bps: 0, ..Params::DEFAULT }), &[&admin]);
    e.ok(ix::swap(&t.key(), Side::Long, 4_000 * USDC, 0, no_limit_long(), &[a.key(), b.key()]), &[&t.kp]);
    let au = e.admin_usdc;
    let tre = e.market().treasury;
    assert_eq!(tre, 4_000 * USDC * Params::DEFAULT.taker_fee_bps as u64 / 10_000);
    assert_eq!(e.market().insurance, 0);
    let mallory = e.user(0);
    let mut stolen = ix::withdraw_treasury(&mallory.key(), &mallory.usdc, 1);
    stolen.1.accounts[0].pubkey = mallory.key();
    e.fails(stolen, &[&mallory.kp], TtError::Unauthorized);
    e.fails(ix::withdraw_treasury(&admin.pubkey(), &au, tre + 1), &[&admin], TtError::InsufficientCollateral);
    let meta = e.ok(ix::withdraw_treasury(&admin.pubkey(), &au, tre), &[&admin]);
    let ev: Vec<CashEvent> = decoded(&meta, TAG_CASH);
    assert_eq!((ev[0].kind, ev[0].amount), (CASH_TREASURY_WITHDRAW, tre as i64));
    assert_eq!(e.token_amount(&au), tre);
    assert_eq!(e.market().treasury, 0);
}

fn scenario_eight_operators() {
    let mut e = Env::new();
    e.init_market();
    let ops: Vec<User> = (0..8).map(|i| e.operator_with_curve(20_000 * USDC, 3600.0 + 10.0 * i as f64, 1e-5, 20_000 * USDC)).collect();
    let keys: Vec<Pubkey> = ops.iter().map(|o| o.key()).collect();
    let t = e.funded_trader(20_000 * USDC, 0);
    let i = ix::swap(&t.key(), Side::Long, 16_000 * USDC, 16_000 * USDC, no_limit_long(), &keys);
    let meta = match e.send_with_limit(i, &[&t.kp], Some(1_400_000)) {
        Ok(m) => m,
        Err((_, logs)) => panic!("8-operator swap failed: {logs}"),
    };
    assert_eq!(decoded::<TradeEvent>(&meta, TAG_TRADE).len(), 8);
    let payer = e.admin.insecure_clone();
    let liq = e.user(0);
    e.ok(ix::crank_mark(&keys), &[&payer]);
    e.fails(ix::liquidate(&liq.key(), &t.key(), &keys, &[]), &[&liq.kp], TtError::NotLiquidatable);
}

#[test]
fn swap_with_eight_operators() {
    scenario_eight_operators();
}

#[test]
fn initialize_market() {
    scenario_initialize();
}

#[test]
fn update_params_and_set_paused() {
    scenario_admin();
}

#[test]
fn create_trader_deposit_withdraw() {
    scenario_trader_cash();
}

#[test]
fn operator_register_update_unbond() {
    scenario_operator_lifecycle();
}

#[test]
fn swap_routes_across_curves() {
    scenario_swap();
}

#[test]
fn swap_error_paths() {
    scenario_swap_errors();
}

#[test]
fn limit_orders_place_cancel_execute() {
    scenario_limit_orders();
}

#[test]
fn limit_orders_net_against_position() {
    scenario_netted_limit_orders();
}

#[test]
fn crank_index_from_pyth() {
    scenario_crank_index();
}

#[test]
fn crank_mark_and_funding() {
    scenario_funding_and_mark();
}

#[test]
fn liquidate_curves_insurance_adl() {
    scenario_liquidation();
}

#[test]
fn withdraw_treasury() {
    scenario_treasury();
}

#[test]
fn compute_units_per_instruction() {
    scenario_initialize();
    scenario_admin();
    scenario_trader_cash();
    scenario_operator_lifecycle();
    scenario_swap();
    scenario_swap_errors();
    scenario_limit_orders();
    scenario_netted_limit_orders();
    scenario_crank_index();
    scenario_funding_and_mark();
    scenario_liquidation();
    scenario_treasury();
    scenario_eight_operators();
    let report = cu_report();
    println!("{:<20} {:>6} {:>8} {:>8}", "instruction", "calls", "max CU", "avg CU");
    for (name, v) in &report {
        let avg = v.iter().sum::<u64>() / v.len() as u64;
        println!("{name:<20} {:>6} {:>8} {:>8}", v.len(), v.iter().max().unwrap(), avg);
    }
    for t in 0..=18u8 {
        let name = format!("{:?}", Ix::from_u8(t).unwrap());
        let v = report.get(&name).unwrap_or_else(|| panic!("{name} never succeeded"));
        assert!(*v.iter().max().unwrap() < 200_000, "{name} exceeds the default budget");
    }
}
