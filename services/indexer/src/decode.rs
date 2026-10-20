use base64::Engine;
use bytemuck::Pod;
use carbon_core::account::{AccountDecoder, DecodedAccount};
use carbon_core::instruction::InstructionDecoder;
use tradingtime_interface::events::*;
use tradingtime_interface::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TtInstruction {
    InitializeMarket(Params),
    UpdateParams(Params),
    SetPaused(u8),
    CreateTrader,
    Deposit(u64),
    Withdraw(u64),
    RegisterOperator,
    UpdateCurve(UpdateCurveData),
    RequestUnbond,
    CompleteUnbond,
    Swap(SwapData),
    PlaceLimitOrder(PlaceLimitOrderData),
    CancelLimitOrder,
    ExecuteLimitOrder(CountData),
    CrankIndex,
    CrankFunding,
    CrankMark(CountData),
    Liquidate(LiquidateData),
    WithdrawTreasury(u64),
}

pub fn decode_ix_data(data: &[u8]) -> Option<TtInstruction> {
    use TtInstruction as T;
    let tag = Ix::from_u8(*data.first()?)?;
    Some(match tag {
        Ix::InitializeMarket => T::InitializeMarket(read_payload::<InitializeMarketData>(data)?.params),
        Ix::UpdateParams => T::UpdateParams(read_payload::<UpdateParamsData>(data)?.params),
        Ix::SetPaused => T::SetPaused(read_payload::<SetPausedData>(data)?.paused),
        Ix::CreateTrader => T::CreateTrader,
        Ix::Deposit => T::Deposit(read_payload::<AmountData>(data)?.amount),
        Ix::Withdraw => T::Withdraw(read_payload::<AmountData>(data)?.amount),
        Ix::RegisterOperator => T::RegisterOperator,
        Ix::UpdateCurve => T::UpdateCurve(read_payload(data)?),
        Ix::RequestUnbond => T::RequestUnbond,
        Ix::CompleteUnbond => T::CompleteUnbond,
        Ix::Swap => T::Swap(read_payload(data)?),
        Ix::PlaceLimitOrder => T::PlaceLimitOrder(read_payload(data)?),
        Ix::CancelLimitOrder => T::CancelLimitOrder,
        Ix::ExecuteLimitOrder => T::ExecuteLimitOrder(read_payload(data)?),
        Ix::CrankIndex => T::CrankIndex,
        Ix::CrankFunding => T::CrankFunding,
        Ix::CrankMark => T::CrankMark(read_payload(data)?),
        Ix::Liquidate => T::Liquidate(read_payload(data)?),
        Ix::WithdrawTreasury => T::WithdrawTreasury(read_payload::<AmountData>(data)?.amount),
    })
}

pub struct TtInstructionDecoder;

impl<'a> InstructionDecoder<'a> for TtInstructionDecoder {
    type InstructionType = TtInstruction;

    fn decode_instruction(&self, ix: &'a solana_instruction::Instruction) -> Option<TtInstruction> {
        if ix.program_id.to_bytes() != PROGRAM_ID {
            return None;
        }
        decode_ix_data(&ix.data)
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TtAccount {
    Market(Market),
    Trader(Trader),
    Operator(Operator),
    Order(LimitOrder),
}

fn read_pod<T: Pod>(data: &[u8]) -> Option<T> {
    bytemuck::try_pod_read_unaligned(data.get(..core::mem::size_of::<T>())?).ok()
}

pub fn decode_account_data(data: &[u8]) -> Option<TtAccount> {
    let disc = u64::from_le_bytes(data.get(..8)?.try_into().ok()?);
    Some(match disc {
        DISC_MARKET => TtAccount::Market(read_pod(data)?),
        DISC_TRADER => TtAccount::Trader(read_pod(data)?),
        DISC_OPERATOR => TtAccount::Operator(read_pod(data)?),
        DISC_ORDER => TtAccount::Order(read_pod(data)?),
        _ => return None,
    })
}

pub struct TtAccountDecoder;

impl<'a> AccountDecoder<'a> for TtAccountDecoder {
    type AccountType = TtAccount;

    fn decode_account(&self, account: &'a solana_account::Account) -> Option<DecodedAccount<TtAccount>> {
        if account.owner.to_bytes() != PROGRAM_ID {
            return None;
        }
        Some(DecodedAccount {
            lamports: account.lamports,
            data: decode_account_data(&account.data)?,
            owner: account.owner,
            executable: account.executable,
            rent_epoch: account.rent_epoch,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Trade(TradeEvent),
    Index(IndexEvent),
    Funding(FundingEvent),
    Liquidation(LiquidationEvent),
    Order(OrderEvent),
    Operator(OperatorEvent),
    Cash(CashEvent),
}

fn exact<T: Pod>(body: &[u8]) -> Option<T> {
    if body.len() != core::mem::size_of::<T>() {
        return None;
    }
    bytemuck::try_pod_read_unaligned(body).ok()
}

pub fn decode_event(bytes: &[u8]) -> Option<Event> {
    let (tag, body) = bytes.split_first()?;
    Some(match *tag {
        TAG_TRADE => Event::Trade(exact(body)?),
        TAG_INDEX => Event::Index(exact(body)?),
        TAG_FUNDING => Event::Funding(exact(body)?),
        TAG_LIQUIDATION => Event::Liquidation(exact(body)?),
        TAG_ORDER => Event::Order(exact(body)?),
        TAG_OPERATOR => Event::Operator(exact(body)?),
        TAG_CASH => Event::Cash(exact(body)?),
        _ => return None,
    })
}

pub fn decode_log_line(line: &str) -> Option<Event> {
    let b64 = line.strip_prefix("Program data: ")?;
    // sol_log_data with one slice logs a single base64 field.
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    decode_event(&bytes)
}

/// Events emitted by `program_id` itself (not by programs it invokes), in log order.
pub fn program_events(logs: &[String], program_id: &str) -> Vec<Event> {
    let mut stack: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for line in logs {
        let mut parts = line.split_whitespace();
        if let (Some("Program"), Some(id), Some(word)) = (parts.next(), parts.next(), parts.next()) {
            if word == "invoke" {
                stack.push(id);
                continue;
            }
            if (word == "success" || word == "failed:") && stack.last() == Some(&id) {
                stack.pop();
                continue;
            }
        }
        if stack.last() == Some(&program_id)
            && let Some(ev) = decode_log_line(line)
        {
            out.push(ev);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;

    pub fn log_line<T: Pod>(tag: u8, ev: &T) -> String {
        let mut buf = vec![0u8; 1 + core::mem::size_of::<T>()];
        events::encode(tag, ev, &mut buf);
        format!("Program data: {}", base64::engine::general_purpose::STANDARD.encode(buf))
    }

    fn trade() -> TradeEvent {
        TradeEvent {
            trader: [1; 32],
            operator: [2; 32],
            side: 0,
            kind: TRADE_SWAP,
            _pad: [0; 6],
            size: 5_000_000,
            ln_price: PodI128::new(7 * ONE),
            fee: 2500,
            slot: 9,
            ts: 1_700_000_000,
        }
    }

    #[test]
    fn decodes_swap_and_place_order_ix() {
        let d = SwapData { side: 1, n_ops: 2, _pad: [0; 6], size: 10, min_fill: 3, limit_ln: PodI128::new(-4) };
        let mut buf = [0u8; 64];
        let n = ix_data(Ix::Swap, &d, &mut buf);
        assert_eq!(decode_ix_data(&buf[..n]), Some(TtInstruction::Swap(d)));
        assert_eq!(decode_ix_data(&buf[..n - 1]), None);

        let p = PlaceLimitOrderData { side: 0, _pad: [0; 7], order_id: 42, size: 9, expiry_ts: 77, limit_ln: PodI128::new(ONE) };
        let n = ix_data(Ix::PlaceLimitOrder, &p, &mut buf);
        let ix = solana_instruction::Instruction {
            program_id: Pubkey::new_from_array(PROGRAM_ID),
            accounts: vec![],
            data: buf[..n].to_vec(),
        };
        assert_eq!(TtInstructionDecoder.decode_instruction(&ix), Some(TtInstruction::PlaceLimitOrder(p)));
        let other = solana_instruction::Instruction { program_id: Pubkey::new_from_array([9; 32]), ..ix };
        assert_eq!(TtInstructionDecoder.decode_instruction(&other), None);

        assert_eq!(decode_ix_data(&[Ix::CrankFunding as u8]), Some(TtInstruction::CrankFunding));
        assert_eq!(decode_ix_data(&[4, 1, 0, 0, 0, 0, 0, 0, 0]), Some(TtInstruction::Deposit(1)));
        assert_eq!(decode_ix_data(&[99]), None);
        assert_eq!(decode_ix_data(&[]), None);
    }

    #[test]
    fn decodes_accounts_by_discriminator_and_owner() {
        let mut t: Trader = bytemuck::Zeroable::zeroed();
        t.discriminator = DISC_TRADER;
        t.owner = [3; 32];
        t.q = -5;
        let acct = solana_account::Account {
            lamports: 1,
            data: bytemuck::bytes_of(&t).to_vec(),
            owner: Pubkey::new_from_array(PROGRAM_ID),
            executable: false,
            rent_epoch: 0,
        };
        let d = TtAccountDecoder.decode_account(&acct).unwrap();
        assert_eq!(d.data, TtAccount::Trader(t));

        let mut m: Market = bytemuck::Zeroable::zeroed();
        m.discriminator = DISC_MARKET;
        assert!(matches!(decode_account_data(bytemuck::bytes_of(&m)), Some(TtAccount::Market(_))));
        let mut o: Operator = bytemuck::Zeroable::zeroed();
        o.discriminator = DISC_OPERATOR;
        assert!(matches!(decode_account_data(bytemuck::bytes_of(&o)), Some(TtAccount::Operator(_))));
        let mut lo: LimitOrder = bytemuck::Zeroable::zeroed();
        lo.discriminator = DISC_ORDER;
        assert!(matches!(decode_account_data(bytemuck::bytes_of(&lo)), Some(TtAccount::Order(_))));

        assert_eq!(decode_account_data(&bytemuck::bytes_of(&t)[..50]), None);
        assert_eq!(decode_account_data(&[0u8; 200]), None);
        let foreign = solana_account::Account { owner: Pubkey::new_from_array([0; 32]), ..acct };
        assert!(TtAccountDecoder.decode_account(&foreign).is_none());
    }

    #[test]
    fn decodes_events_from_logs_only_for_our_program() {
        let me = Pubkey::new_from_array(PROGRAM_ID).to_string();
        let token = Pubkey::new_from_array(TOKEN_PROGRAM).to_string();
        let cash = CashEvent { owner: [4; 32], kind: CASH_DEPOSIT, _pad: [0; 7], amount: 100, ts: 5 };
        let logs = vec![
            format!("Program {me} invoke [1]"),
            "Program log: hello".to_string(),
            format!("Program {token} invoke [2]"),
            log_line(TAG_TRADE, &trade()),
            format!("Program {token} success"),
            log_line(TAG_TRADE, &trade()),
            log_line(TAG_CASH, &cash),
            "Program data: !!!".to_string(),
            format!("Program {me} consumed 100 of 200000 compute units"),
            format!("Program {me} success"),
            log_line(TAG_CASH, &cash),
        ];
        let evs = program_events(&logs, &me);
        assert_eq!(evs, vec![Event::Trade(trade()), Event::Cash(cash)]);
        assert_eq!(decode_log_line(&log_line(TAG_TRADE, &trade())), Some(Event::Trade(trade())));
        assert_eq!(decode_event(&[TAG_TRADE, 1, 2]), None);
    }
}
