use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

/// ComputeBudget111111111111111111111111111111
pub fn compute_budget_program() -> Pubkey {
    "ComputeBudget111111111111111111111111111111".parse().expect("valid id")
}

/// Swaps across several operator curves exceed the 200k default (A97).
pub const TRADE_CU_LIMIT: u32 = 1_400_000;

/// SetComputeUnitLimit: data = [2] ++ units (u32 LE), no accounts.
pub fn set_compute_unit_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction { program_id: compute_budget_program(), accounts: vec![], data }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cu_limit_bytes() {
        let ix = set_compute_unit_limit(TRADE_CU_LIMIT);
        assert_eq!(ix.data, vec![2, 0xc0, 0x5c, 0x15, 0x00]);
        assert!(ix.accounts.is_empty());
        assert_eq!(ix.program_id.to_bytes()[..4], [3, 6, 70, 111]);
    }
}
