use solana_hash::Hash;
use solana_instruction::Instruction;
use solana_message::Message;
use solana_pubkey::Pubkey;

/// Legacy tx wire bytes with zeroed signature slots, ready for a wallet to sign.
pub fn unsigned_tx_bytes(ixs: &[Instruction], payer: &Pubkey, blockhash: [u8; 32]) -> Vec<u8> {
    let msg = Message::new_with_blockhash(ixs, Some(payer), &Hash::new_from_array(blockhash));
    let n = msg.header.num_required_signatures as usize;
    let mut out = Vec::new();
    push_shortvec(&mut out, n);
    out.resize(out.len() + 64 * n, 0);
    out.extend_from_slice(&msg.serialize());
    out
}

fn push_shortvec(out: &mut Vec<u8>, mut v: usize) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::ix;

    #[test]
    fn single_signer_layout() {
        let payer = Pubkey::new_from_array([7; 32]);
        let ixs = [ix::create_trader(&payer), ix::deposit(&payer, &Pubkey::new_from_array([8; 32]), 5)];
        let bh = [3u8; 32];
        let bytes = unsigned_tx_bytes(&ixs, &payer, bh);
        assert_eq!(bytes[0], 1);
        assert!(bytes[1..65].iter().all(|b| *b == 0));
        let msg = Message::new_with_blockhash(&ixs, Some(&payer), &Hash::new_from_array(bh));
        assert_eq!(&bytes[65..], &msg.serialize()[..]);
        assert_eq!(msg.account_keys[0], payer);
        // header(3) + shortvec(n keys) + key 0
        assert_eq!(&bytes[65 + 4..65 + 36], payer.as_ref());
    }

    #[test]
    fn two_signers() {
        let payer = Pubkey::new_from_array([7; 32]);
        let other = Pubkey::new_from_array([9; 32]);
        let ixs = [ix::create_trader(&payer), ix::create_trader(&other)];
        let bytes = unsigned_tx_bytes(&ixs, &payer, [0; 32]);
        assert_eq!(bytes[0], 2);
        assert!(bytes[1..129].iter().all(|b| *b == 0));
        assert_eq!(bytes[129], 2);
    }
}
