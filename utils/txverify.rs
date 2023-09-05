/*
* Utility for verifying bitcoin transaction inclusion using btcrelay program and checking a prior executed instruction data
*/

use anchor_lang::{
    prelude::*,
    solana_program::instruction::Instruction,
    solana_program::hash
};
use std::str::FromStr;

static BTC_RELAY_ID_BASE58: &str = "8DMFpUfCk8KPkNLtE25XHuCSsT1GqYxuLdGzu59QK3Rt";
static IX_PREFIX: [u8; 8] = [
    0x9d,
    0x7e,
    0xc1,
    0x86,
    0x31,
    0x33,
    0x07,
    0x58
];

pub mod txverify {
    use super::*;

    pub fn verify_tx_ix(ix: &Instruction, reversed_tx_id: &[u8; 32], confirmations: u32) -> Result<u8> {
        let btc_relay_id: Pubkey = Pubkey::from_str(BTC_RELAY_ID_BASE58).unwrap();

        if  ix.program_id       != btc_relay_id
        {
            return Ok(10);
        }

        return Ok(check_tx_data(&ix.data, reversed_tx_id, confirmations)); // If that's not the case, check data
    }

    /// Verify serialized BtcRelay instruction data
    pub fn check_tx_data(data: &[u8], reversed_tx_id: &[u8; 32], confirmations: u32) -> u8 {
        for i in 0..8 {
            if data[i] != IX_PREFIX[i] {
                return 1;
            }
        }
        for i in 8..40 {
            if data[i] != reversed_tx_id[i-8] {
                return 2;
            }
        }

        let _confirmations = u32::from_le_bytes(data[40..44].try_into().unwrap());
        if confirmations != _confirmations {
            return 3;
        }

        return 0;
    }
}