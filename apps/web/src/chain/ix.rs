//! Mirrors `tradingtime_client::{pda, ix}` for the instructions the web app sends.
use bytemuck::Pod;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use tradingtime_interface::{self as interface, Fx, Ix, PodI128, Side};

const ATA_PROGRAM: Pubkey = Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

pub fn program_id() -> Pubkey {
    Pubkey::new_from_array(interface::PROGRAM_ID)
}

pub fn usdc_mint() -> Pubkey {
    Pubkey::new_from_array(interface::USDC_MINT)
}

fn token_program() -> Pubkey {
    Pubkey::new_from_array(interface::TOKEN_PROGRAM)
}

fn system_program() -> Pubkey {
    Pubkey::new_from_array(interface::SYSTEM_PROGRAM)
}

pub mod pda {
    use super::*;
    use interface::{SEED_MARKET, SEED_OPERATOR, SEED_ORDER, SEED_TRADER, SEED_VAULT};

    pub fn market() -> (Pubkey, u8) {
        Pubkey::find_program_address(&[SEED_MARKET], &program_id())
    }
    pub fn vault() -> (Pubkey, u8) {
        Pubkey::find_program_address(&[SEED_VAULT], &program_id())
    }
    pub fn trader(owner: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[SEED_TRADER, owner.as_ref()], &program_id())
    }
    pub fn operator(owner: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[SEED_OPERATOR, owner.as_ref()], &program_id())
    }
    pub fn order(owner: &Pubkey, order_id: u64) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[SEED_ORDER, owner.as_ref(), &order_id.to_le_bytes()], &program_id())
    }
}

pub fn associated_token_address(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[owner.as_ref(), &interface::TOKEN_PROGRAM, mint.as_ref()], &ATA_PROGRAM).0
}

fn data<T: Pod>(tag: Ix, payload: &T) -> Vec<u8> {
    let mut buf = vec![0u8; 1 + core::mem::size_of::<T>()];
    interface::ix_data(tag, payload, &mut buf);
    buf
}

fn build(accounts: Vec<AccountMeta>, data: Vec<u8>) -> Instruction {
    Instruction { program_id: program_id(), accounts, data }
}

pub fn create_trader(owner: &Pubkey) -> Instruction {
    build(
        vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new(pda::trader(owner).0, false),
            AccountMeta::new_readonly(pda::market().0, false),
            AccountMeta::new_readonly(system_program(), false),
        ],
        vec![Ix::CreateTrader as u8],
    )
}

pub fn deposit(owner: &Pubkey, owner_usdc: &Pubkey, amount: u64) -> Instruction {
    build(
        vec![
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new(pda::trader(owner).0, false),
            AccountMeta::new(pda::market().0, false),
            AccountMeta::new(*owner_usdc, false),
            AccountMeta::new(pda::vault().0, false),
            AccountMeta::new_readonly(usdc_mint(), false),
            AccountMeta::new_readonly(token_program(), false),
        ],
        data(Ix::Deposit, &interface::AmountData { amount }),
    )
}

/// `operators` are operator owner wallets; each adds the pair [operator PDA, its trader PDA].
pub fn swap(owner: &Pubkey, side: Side, size: u64, min_fill: u64, limit_ln: Fx, operators: &[Pubkey]) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new_readonly(*owner, true),
        AccountMeta::new(pda::trader(owner).0, false),
        AccountMeta::new(pda::market().0, false),
    ];
    for o in operators {
        accounts.push(AccountMeta::new(pda::operator(o).0, false));
        accounts.push(AccountMeta::new(pda::trader(o).0, false));
    }
    let payload = interface::SwapData {
        side: side as u8,
        n_ops: operators.len() as u8,
        _pad: [0; 6],
        size,
        min_fill,
        limit_ln: PodI128::new(limit_ln),
    };
    build(accounts, data(Ix::Swap, &payload))
}

pub fn place_limit_order(owner: &Pubkey, order_id: u64, side: Side, size: u64, limit_ln: Fx, expiry_ts: i64) -> Instruction {
    let payload = interface::PlaceLimitOrderData {
        side: side as u8,
        _pad: [0; 7],
        order_id,
        size,
        expiry_ts,
        limit_ln: PodI128::new(limit_ln),
    };
    build(
        vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new(pda::trader(owner).0, false),
            AccountMeta::new(pda::order(owner, order_id).0, false),
            AccountMeta::new(pda::market().0, false),
            AccountMeta::new_readonly(system_program(), false),
        ],
        data(Ix::PlaceLimitOrder, &payload),
    )
}

pub fn cancel_limit_order(owner: &Pubkey, order_id: u64) -> Instruction {
    build(
        vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new(pda::trader(owner).0, false),
            AccountMeta::new(pda::order(owner, order_id).0, false),
        ],
        vec![Ix::CancelLimitOrder as u8],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }
    fn meta(ix: &Instruction) -> Vec<(Pubkey, bool, bool)> {
        ix.accounts.iter().map(|m| (m.pubkey, m.is_signer, m.is_writable)).collect()
    }
    fn sys() -> Pubkey {
        Pubkey::new_from_array(interface::SYSTEM_PROGRAM)
    }

    #[test]
    fn ids_and_pdas() {
        assert_eq!(program_id().to_string(), "7thnMqBXPE3QaNnDjFaEVhcMK8UBiwRaRELtZSDBkz4h");
        assert_eq!(usdc_mint().to_string(), "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        let o = k(9);
        let p = program_id();
        assert_eq!(pda::market(), Pubkey::find_program_address(&[b"market"], &p));
        assert_eq!(pda::vault(), Pubkey::find_program_address(&[b"vault"], &p));
        assert_eq!(pda::trader(&o), Pubkey::find_program_address(&[b"trader", o.as_ref()], &p));
        assert_eq!(pda::operator(&o), Pubkey::find_program_address(&[b"operator", o.as_ref()], &p));
        assert_eq!(pda::order(&o, 7), Pubkey::find_program_address(&[b"order", o.as_ref(), &7u64.to_le_bytes()], &p));
        assert_ne!(pda::order(&o, 7).0, pda::order(&o, 8).0);
        let (m, b) = pda::market();
        assert_eq!(Pubkey::create_program_address(&[b"market", &[b]], &p).unwrap(), m);
    }

    #[test]
    fn ata_known_vector() {
        // spl-token address --owner 7thn... --token EPjF... (mainnet)
        let owner = program_id();
        assert_eq!(
            associated_token_address(&owner, &usdc_mint()).to_string(),
            "47xR15HFg5LEZfgCmPBv62PWqvjwcu1a5iL13Pw5fec9"
        );
    }

    #[test]
    fn create_trader_layout() {
        let o = k(1);
        let ix = create_trader(&o);
        assert_eq!(ix.program_id, program_id());
        assert_eq!(ix.data, vec![Ix::CreateTrader as u8]);
        assert_eq!(meta(&ix), vec![(o, true, true), (pda::trader(&o).0, false, true), (pda::market().0, false, false), (sys(), false, false)]);
    }

    #[test]
    fn deposit_layout() {
        let o = k(1);
        let u = k(2);
        let ix = deposit(&o, &u, 1_500_000);
        let mut d = vec![4u8];
        d.extend_from_slice(&1_500_000u64.to_le_bytes());
        assert_eq!(ix.data, d);
        assert_eq!(
            meta(&ix),
            vec![
                (o, true, false),
                (pda::trader(&o).0, false, true),
                (pda::market().0, false, true),
                (u, false, true),
                (pda::vault().0, false, true),
                (usdc_mint(), false, false),
                (Pubkey::new_from_array(interface::TOKEN_PROGRAM), false, false),
            ]
        );
    }

    #[test]
    fn swap_layout() {
        let o = k(1);
        let ops = [k(3), k(4)];
        let ix = swap(&o, Side::Short, 10_000_000, 9_000_000, -5, &ops);
        let mut d = vec![10u8, 1, 2, 0, 0, 0, 0, 0, 0];
        d.extend_from_slice(&10_000_000u64.to_le_bytes());
        d.extend_from_slice(&9_000_000u64.to_le_bytes());
        d.extend_from_slice(&(-5i128).to_le_bytes());
        assert_eq!(ix.data, d);
        assert_eq!(ix.data.len(), 41);
        let decoded: interface::SwapData = interface::read_payload(&ix.data).unwrap();
        assert_eq!(decoded.limit_ln, PodI128::new(-5));
        assert_eq!(
            meta(&ix),
            vec![
                (o, true, false),
                (pda::trader(&o).0, false, true),
                (pda::market().0, false, true),
                (pda::operator(&ops[0]).0, false, true),
                (pda::trader(&ops[0]).0, false, true),
                (pda::operator(&ops[1]).0, false, true),
                (pda::trader(&ops[1]).0, false, true),
            ]
        );
        assert_eq!(swap(&o, Side::Long, 1, 0, 0, &[]).data[1..3], [0, 0]);
    }

    #[test]
    fn place_limit_order_layout() {
        let o = k(1);
        let ix = place_limit_order(&o, 42, Side::Long, 5_000_000, 1 << 48, 1_700_000_000);
        let mut d = vec![11u8, 0, 0, 0, 0, 0, 0, 0, 0];
        d.extend_from_slice(&42u64.to_le_bytes());
        d.extend_from_slice(&5_000_000u64.to_le_bytes());
        d.extend_from_slice(&1_700_000_000i64.to_le_bytes());
        d.extend_from_slice(&(1i128 << 48).to_le_bytes());
        assert_eq!(ix.data, d);
        assert_eq!(
            meta(&ix),
            vec![
                (o, true, true),
                (pda::trader(&o).0, false, true),
                (pda::order(&o, 42).0, false, true),
                (pda::market().0, false, true),
                (sys(), false, false),
            ]
        );
    }

    #[test]
    fn cancel_limit_order_layout() {
        let o = k(1);
        let ix = cancel_limit_order(&o, 42);
        assert_eq!(ix.data, vec![12u8]);
        assert_eq!(meta(&ix), vec![(o, true, true), (pda::trader(&o).0, false, true), (pda::order(&o, 42).0, false, true)]);
    }
}
