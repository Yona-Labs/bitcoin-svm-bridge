use anchor_lang::prelude::*;
use crate::structs::*;

#[event]
pub struct StoreHeader {
    pub block_hash: [u8; 32],
    pub commit_hash: [u8; 32],
    pub header: CommittedBlockHeader
}

#[event]
pub struct StoreFork {
    pub fork_id: u64,
    pub block_hash: [u8; 32],
    pub commit_hash: [u8; 32],
    pub header: CommittedBlockHeader
}

#[event]
pub struct ChainReorg {
    pub fork_id: u64,
    pub start_height: u32,
    pub tip_block_hash: [u8; 32],
    pub tip_commit_hash: [u8; 32]
}
