use crate::structs::*;
use anchor_lang::prelude::*;

#[event]
pub struct StoreHeader {
    pub block_hash: [u8; 32],
    pub commit_hash: [u8; 32],
    pub header: CommittedBlockHeader,
}

#[event]
pub struct StoreFork {
    pub fork_id: u64,
    pub block_hash: [u8; 32],
    pub commit_hash: [u8; 32],
    pub header: CommittedBlockHeader,
}

#[event]
pub struct ChainReorg {
    pub fork_id: u64,
    pub start_height: u32,
    pub tip_block_hash: [u8; 32],
    pub tip_commit_hash: [u8; 32],
}

#[event]
pub struct Withdrawal {
    pub amount: u64,
    pub bitcoin_address: String,
}

#[event]
pub struct DepositTxVerified {
    pub tx_id: [u8; 32],
    pub yona_address: Pubkey,
    pub bitcoin_pubkey: [u8; 33],
}
