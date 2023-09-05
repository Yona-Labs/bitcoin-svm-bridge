use anchor_lang::{
    prelude::*,
    solana_program::hash
};

//Struct representing bitcoin block header
//https://www.oreilly.com/library/view/mastering-bitcoin/9781491902639/ch07.html#block_header
#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize, Copy)]
pub struct BlockHeader {
    pub version: u32, //A version number to track software/protocol upgrades
    pub reversed_prev_blockhash: [u8; 32], //A reference to the hash of the previous (parent) block in the chain
    pub merkle_root: [u8; 32], //A hash of the root of the merkle tree of this block’s transactions
    pub timestamp: u32, //The approximate creation time of this block (seconds from Unix Epoch)
    pub nbits: u32, //The proof-of-work algorithm difficulty target for this block
    pub nonce: u32 //A counter used for the proof-of-work algorithm
}

impl BlockHeader {

    //Double sha256 of the blockheader
    pub fn get_block_hash(&self) -> Result<[u8; 32]> {
        let arr = self.try_to_vec()?;

        Ok(hash::hash(&hash::hash(&arr).to_bytes()).to_bytes())
    }

}

//Struct representing committed block header - bitcoin block header with additional data
#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize, Copy)]
pub struct CommittedBlockHeader {
    pub chain_work: [u8; 32], //Accumulated chain work at this block
    
    pub header: BlockHeader, //Bitcoin blockheader

    pub last_diff_adjustment: u32, //Timestamp of the last difficulty adjustment block, used for difficulty retargetting
    pub blockheight: u32, //Block's height

    pub prev_block_timestamps: [u32; 10] //Timestamps of the 10 previous blockheaders, used to calculate median block timestamp
}

impl CommittedBlockHeader {

    //Returns the commit hash (fingerprint) of the block header data to be saved to the ring buffer
    pub fn get_commit_hash(&self) -> Result<[u8; 32]> {
        let arr = self.try_to_vec()?;

        Ok(hash::hash(&arr).to_bytes())
    }

}