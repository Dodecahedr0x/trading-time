use solana_pubkey::Pubkey;
use tradingtime_interface::{SEED_MARKET, SEED_OPERATOR, SEED_ORDER, SEED_TRADER, SEED_VAULT};

use crate::program_id;

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
