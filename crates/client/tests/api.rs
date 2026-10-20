use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use tradingtime_client::interface::{self as tti, Ix, Params, PodI128, Side};
use tradingtime_client::{CurveQuote, accounts, events, ix, pda, program_id, units};

fn pk(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn w(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, false)
}
fn ws(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, true)
}
fn r(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, false)
}
fn rs(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, true)
}
fn market() -> Pubkey {
    pda::market().0
}
fn vault() -> Pubkey {
    pda::vault().0
}
fn trader(o: Pubkey) -> Pubkey {
    pda::trader(&o).0
}
fn operator(o: Pubkey) -> Pubkey {
    pda::operator(&o).0
}
fn usdc() -> Pubkey {
    Pubkey::new_from_array(tti::USDC_MINT)
}
fn token() -> Pubkey {
    Pubkey::new_from_array(tti::TOKEN_PROGRAM)
}
fn system() -> Pubkey {
    Pubkey::new_from_array(tti::SYSTEM_PROGRAM)
}
fn data<T: bytemuck::Pod>(tag: Ix, p: &T) -> Vec<u8> {
    let mut v = vec![tag as u8];
    v.extend_from_slice(bytemuck::bytes_of(p));
    v
}
fn check(i: Instruction, metas: Vec<AccountMeta>, d: Vec<u8>) {
    assert_eq!(i.program_id, program_id());
    assert_eq!(i.accounts, metas);
    assert_eq!(i.data, d);
}

#[test]
fn pdas_match_seeds() {
    let pid = program_id();
    assert_eq!(pid.to_bytes(), tti::PROGRAM_ID);
    assert_eq!(pid.to_string(), "7thnMqBXPE3QaNnDjFaEVhcMK8UBiwRaRELtZSDBkz4h");
    assert_eq!(pda::market(), Pubkey::find_program_address(&[b"market"], &pid));
    assert_eq!(pda::vault(), Pubkey::find_program_address(&[b"vault"], &pid));
    let o = pk(7);
    assert_eq!(pda::trader(&o), Pubkey::find_program_address(&[b"trader", o.as_ref()], &pid));
    assert_eq!(pda::operator(&o), Pubkey::find_program_address(&[b"operator", o.as_ref()], &pid));
    assert_eq!(
        pda::order(&o, 42),
        Pubkey::find_program_address(&[b"order", o.as_ref(), &42u64.to_le_bytes()], &pid)
    );
}

#[test]
fn admin_builders() {
    let (a, m, f) = (pk(1), pk(2), pk(3));
    let p = Params::DEFAULT;
    check(
        ix::initialize_market(&a, &m, &f, p, 5_400),
        vec![ws(a), w(market()), w(vault()), r(m), r(f), r(system()), r(token())],
        data(Ix::InitializeMarket, &tti::InitializeMarketData { params: p, initial_index_secs: 5_400, _pad0: 0 }),
    );
    check(ix::update_params(&a, p), vec![rs(a), w(market())], data(Ix::UpdateParams, &tti::UpdateParamsData { params: p }));
    check(ix::set_paused(&a, true), vec![rs(a), w(market())], vec![Ix::SetPaused as u8, 1]);
    check(ix::set_paused(&a, false), vec![rs(a), w(market())], vec![Ix::SetPaused as u8, 0]);
    let au = pk(4);
    check(
        ix::withdraw_treasury(&a, &au, 9),
        vec![rs(a), w(market()), w(au), w(vault()), r(usdc()), r(token())],
        data(Ix::WithdrawTreasury, &tti::AmountData { amount: 9 }),
    );
}

#[test]
fn trader_builders() {
    let (o, ou) = (pk(5), pk(6));
    check(ix::create_trader(&o), vec![ws(o), w(trader(o)), r(market()), r(system())], vec![Ix::CreateTrader as u8]);
    let dep = vec![rs(o), w(trader(o)), w(market()), w(ou), w(vault()), r(usdc()), r(token())];
    check(ix::deposit(&o, &ou, 100), dep.clone(), data(Ix::Deposit, &tti::AmountData { amount: 100 }));
    check(ix::withdraw(&o, &ou, 50, false), dep.clone(), data(Ix::Withdraw, &tti::AmountData { amount: 50 }));
    let mut wo = dep;
    wo.push(r(operator(o)));
    check(ix::withdraw(&o, &ou, 50, true), wo, data(Ix::Withdraw, &tti::AmountData { amount: 50 }));
}

#[test]
fn operator_builders() {
    let (o, ou) = (pk(8), pk(9));
    check(
        ix::register_operator(&o, &ou),
        vec![ws(o), w(trader(o)), w(operator(o)), w(market()), w(ou), w(vault()), r(usdc()), r(token()), r(system())],
        vec![Ix::RegisterOperator as u8],
    );
    let q = CurveQuote { fair_ln: 11, half_spread: -2, impact_per_usdc: 3, inventory_limit: 4 };
    check(
        ix::update_curve(&o, q),
        vec![rs(o), w(operator(o))],
        data(
            Ix::UpdateCurve,
            &tti::UpdateCurveData {
                fair_ln: PodI128::new(11),
                half_spread: PodI128::new(-2),
                impact_per_usdc: PodI128::new(3),
                inventory_limit: 4,
            },
        ),
    );
    check(ix::request_unbond(&o), vec![rs(o), w(operator(o))], vec![Ix::RequestUnbond as u8]);
    check(
        ix::complete_unbond(&o, &ou),
        vec![rs(o), w(operator(o)), w(trader(o)), w(market()), w(ou), w(vault()), r(usdc()), r(token())],
        vec![Ix::CompleteUnbond as u8],
    );
}

fn pairs(ops: &[Pubkey]) -> Vec<AccountMeta> {
    ops.iter().flat_map(|o| [w(operator(*o)), w(trader(*o))]).collect()
}

#[test]
fn swap_and_orders() {
    let o = pk(10);
    let ops = [pk(11), pk(12)];
    let mut metas = vec![rs(o), w(trader(o)), w(market())];
    metas.extend(pairs(&ops));
    check(
        ix::swap(&o, Side::Short, 1000, 900, -77, &ops),
        metas,
        data(
            Ix::Swap,
            &tti::SwapData { side: 1, n_ops: 2, _pad: [0; 6], size: 1000, min_fill: 900, limit_ln: PodI128::new(-77) },
        ),
    );
    let ord = pda::order(&o, 3).0;
    check(
        ix::place_limit_order(&o, 3, Side::Long, 500, 99, 1234),
        vec![ws(o), w(trader(o)), w(ord), w(market()), r(system())],
        data(
            Ix::PlaceLimitOrder,
            &tti::PlaceLimitOrderData { side: 0, _pad: [0; 7], order_id: 3, size: 500, expiry_ts: 1234, limit_ln: PodI128::new(99) },
        ),
    );
    check(ix::cancel_limit_order(&o, 3), vec![ws(o), w(trader(o)), w(ord)], vec![Ix::CancelLimitOrder as u8]);
    let k = pk(13);
    check(ix::cancel_expired_order(&k, &o, 3), vec![rs(k), w(trader(o)), w(ord), w(o)], vec![Ix::CancelLimitOrder as u8]);
    let mut metas = vec![rs(k), w(trader(k)), w(ord), w(o), w(trader(o)), w(market())];
    metas.extend(pairs(&ops));
    check(
        ix::execute_limit_order(&k, &o, 3, &ops),
        metas,
        data(Ix::ExecuteLimitOrder, &tti::CountData { n_ops: 2, _pad: [0; 7] }),
    );
}

#[test]
fn cranks_and_liquidate() {
    let f = pk(14);
    check(ix::crank_index(&f), vec![w(market()), r(f)], vec![Ix::CrankIndex as u8]);
    check(ix::crank_funding(), vec![w(market())], vec![Ix::CrankFunding as u8]);
    let ops = [pk(15)];
    let mut metas = vec![w(market())];
    metas.extend(pairs(&ops));
    check(ix::crank_mark(&ops), metas, data(Ix::CrankMark, &tti::CountData { n_ops: 1, _pad: [0; 7] }));
    let (l, t) = (pk(16), pk(17));
    let adl = [pk(18), pk(19), pk(20)];
    let mut metas = vec![rs(l), w(trader(t)), w(market())];
    metas.extend(pairs(&ops));
    metas.extend(adl.iter().map(|a| w(trader(*a))));
    check(
        ix::liquidate(&l, &t, &ops, &adl),
        metas,
        data(Ix::Liquidate, &tti::LiquidateData { n_ops: 1, n_adl: 3, _pad: [0; 6] }),
    );
}

#[test]
fn account_decoders() {
    use bytemuck::Zeroable;
    let mut m = tti::Market::zeroed();
    m.discriminator = tti::DISC_MARKET;
    m.insurance = 77;
    let bytes = bytemuck::bytes_of(&m).to_vec();
    assert_eq!(accounts::decode_market(&bytes).unwrap(), &m);
    assert_eq!(accounts::read_market(&bytes).unwrap(), m);
    assert_eq!(accounts::decode_trader(&bytes), Err(accounts::DecodeError::WrongDiscriminator));
    assert_eq!(accounts::decode_market(&bytes[..10]), Err(accounts::DecodeError::TooShort));

    let mut t = tti::Trader::zeroed();
    t.discriminator = tti::DISC_TRADER;
    t.cash = -5;
    assert_eq!(accounts::decode_trader(bytemuck::bytes_of(&t)).unwrap(), &t);
    let mut o = tti::Operator::zeroed();
    o.discriminator = tti::DISC_OPERATOR;
    o.bond = 3;
    assert_eq!(accounts::decode_operator(bytemuck::bytes_of(&o)).unwrap(), &o);
    let mut l = tti::LimitOrder::zeroed();
    l.discriminator = tti::DISC_ORDER;
    l.size = 8;
    assert_eq!(accounts::decode_order(bytemuck::bytes_of(&l)).unwrap(), &l);
    // unaligned buffer still decodes by value
    let mut buf = vec![0u8; 1];
    buf.extend_from_slice(bytemuck::bytes_of(&l));
    assert_eq!(accounts::read_order(&buf[1..]).unwrap(), l);
}

#[test]
fn events_round_trip() {
    use base64::Engine;
    use tti::events::*;
    let t = TradeEvent {
        trader: [1; 32],
        operator: [2; 32],
        side: 1,
        kind: TRADE_LIMIT,
        _pad: [0; 6],
        size: 5,
        ln_price: PodI128::new(-9),
        fee: 1,
        slot: 2,
        ts: 3,
    };
    let mut buf = [0u8; 256];
    let n = encode(TAG_TRADE, &t, &mut buf);
    assert_eq!(events::decode_event(&buf[..n]), Some(events::Event::Trade(t)));
    assert_eq!(events::decode_event(&buf[..n - 1]), None);
    assert_eq!(events::decode_event(&[99, 0, 0]), None);

    let c = CashEvent { owner: [3; 32], kind: CASH_WITHDRAW, _pad: [0; 7], amount: -4, ts: 9 };
    let n = encode(TAG_CASH, &c, &mut buf);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
    let line = format!("Program data: {b64} AAAA");
    assert_eq!(events::decode_log_line(&line), Some(events::Event::Cash(c)));
    assert_eq!(events::decode_log_line("Program log: hi"), None);

    let i = IndexEvent { index_ln: PodI128::new(1), var_ewma: PodI128::new(2), pyth_price: 3, pyth_expo: -8, _pad: 0, publish_time: 4, ts: 5 };
    let n = encode(TAG_INDEX, &i, &mut buf);
    assert_eq!(events::decode_event(&buf[..n]), Some(events::Event::Index(i)));
    let f = FundingEvent { funding_index: PodI128::new(1), mark_ln: PodI128::new(2), index_ln: PodI128::new(3), ts: 4 };
    let n = encode(TAG_FUNDING, &f, &mut buf);
    assert_eq!(events::decode_event(&buf[..n]), Some(events::Event::Funding(f)));
    let l = LiquidationEvent { trader: [1; 32], liquidator: [2; 32], size: 3, bankruptcy_ln: PodI128::new(4), penalty: 5, insurance_used: 6, ts: 7 };
    let n = encode(TAG_LIQUIDATION, &l, &mut buf);
    assert_eq!(events::decode_event(&buf[..n]), Some(events::Event::Liquidation(l)));
    let o = OrderEvent { owner: [1; 32], order_id: 2, kind: ORDER_FILLED, side: 0, _pad: [0; 6], size: 3, filled: 3, limit_ln: PodI128::new(5), ts: 6 };
    let n = encode(TAG_ORDER, &o, &mut buf);
    assert_eq!(events::decode_event(&buf[..n]), Some(events::Event::Order(o)));
    let op = OperatorEvent { owner: [1; 32], kind: OPERATOR_CLOSED, _pad: [0; 7], bond: 2, ts: 3 };
    let n = encode(TAG_OPERATOR, &op, &mut buf);
    assert_eq!(events::decode_event(&buf[..n]), Some(events::Event::Operator(op)));
}

#[test]
fn units() {
    let l = units::secs_to_ln(3600.0);
    assert!((units::ln_to_secs(l) - 3600.0).abs() < 1e-6);
    assert_eq!(units::secs_to_ln(1.0), 0);
    assert_eq!(units::usdc(1_500_000u64), 1.5);
    assert_eq!(units::usdc(-2_000_000i64), -2.0);
    assert_eq!(units::format_duration(5400.0), "1h 30m");
    assert_eq!(units::format_duration(2700.0), "45m");
    assert_eq!(units::format_duration(2.4 * 86_400.0), "2.4d");
    assert_eq!(units::format_duration(3600.0), "1h");
    assert_eq!(units::format_duration(42.0), "42s");
    assert_eq!(units::format_duration(150.0), "2m 30s");
    assert_eq!(units::format_duration(0.4), "0s");
    assert_eq!(units::format_duration(86_400.0), "1.0d");
}

#[test]
fn compute_unit_limit_instruction_bytes() {
    let ix = tradingtime_client::ix::set_compute_unit_limit(1_400_000);
    assert_eq!(ix.program_id.to_string(), "ComputeBudget111111111111111111111111111111");
    assert!(ix.accounts.is_empty());
    assert_eq!(ix.data, vec![2, 0xc0, 0x5c, 0x15, 0x00]);
}
