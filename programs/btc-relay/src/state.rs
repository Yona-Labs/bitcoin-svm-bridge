use anchor_lang::prelude::*;

//How many block commitments should be kept in main state PDA's ring buffer
const PRUNING_FACTOR_U32: u32 = 250;
const PRUNING_FACTOR: usize = PRUNING_FACTOR_U32 as usize;

#[account(zero_copy)]
#[repr(C)]
pub struct MainState {
    pub start_height: u32, //Start blockheight of the current ring buffer
    pub last_diff_adjustment: u32, //Timestamp of the last difficulty adjustment block
    pub block_height: u32, //Current blockheight
    pub total_blocks: u32, //Total number of blocks validated
    
    pub fork_counter: u64, //Used for indexing fork PDA's

    pub tip_commit_hash: [u8; 32], //Blockheader data commitment hash for the latest block - blockchain tip
    pub tip_block_hash: [u8; 32], //Blockhash of the latest block - blockchain tip

    pub chain_work: [u8; 32], //Accumulated work of the chain
    pub block_commitments: [[u8; 32]; 250], //Ring buffer storing block data commitments (sha256 hashes of CommittedBlockHeader data)
}

impl MainState {
    
    pub fn space() -> usize {
        8+8+4+4+4+32+8+4+32+32+(PRUNING_FACTOR*32)
    }

    //Get's the position on the ring buffer corresponding to the block_height,
    // returns 0 or PRUNING_FACTOR in edge cases
    pub fn get_position(&self, block_height: u32) -> usize {
        if self.start_height<=block_height {
            let pos = block_height-self.start_height;
            if pos>=PRUNING_FACTOR_U32 {
                return 0;
            }
            pos as usize
        } else {
            let pos = self.start_height-block_height;
            if pos>=PRUNING_FACTOR_U32 {
                return PRUNING_FACTOR;
            }
            (PRUNING_FACTOR_U32-pos) as usize
        }
    }

    //Get's the commitment for a block_height,
    // returning empty array [0; 32] in edge cases
    pub fn get_commitment(&self, block_height: u32) -> [u8; 32] {
        //Check block_height more than than tip
        if block_height>self.block_height {
            return [0; 32];
        }
        //Check block_height out of bounds for the ring buffer
        if block_height<=self.block_height-PRUNING_FACTOR_U32 {
            return [0; 32];
        }
        let pos = self.get_position(block_height);
        if pos==PRUNING_FACTOR {
            return [0; 32];
        }
        self.block_commitments[pos]
    }

    //Stores the block commitment for the specified block_height in a ring buffer
    // returns false in case that block commitment would fall out of bounds for
    // ring buffer (more than PRUNING_FACTOR blocks in the past)
    pub fn store_block_commitment(&mut self, block_height: u32, block_commitment: [u8; 32]) -> bool {
        let position = self.get_position(block_height);
        if position==PRUNING_FACTOR {
            return false;
        }
        self.block_commitments[position] =  block_commitment;
        if position==0 {
            self.start_height = block_height;
        }
        self.total_blocks += 1;
        true
    }

}

#[account(zero_copy)]
#[repr(C)]
pub struct ForkState {
    pub initialized: u32, //1 - initialized, 0 - not yet initialized (boolean messes up the padding for zero_copy, so u32 is used instead)
    pub start_height: u32, //Blockheight of last block that is also in main chain
    pub length: u32, //Current length of the fork

    pub tip_commit_hash: [u8; 32], //Blockheader data commitment hash for the latest block - fork tip
    pub tip_block_hash: [u8; 32], //Blockhash of the latest block - fork tip

    pub block_commitments: [[u8; 32]; 250], //Buffer storing block data commitments (sha256 hashes of CommittedBlockHeader data)
}

impl ForkState {
    
    pub fn space() -> usize {
        8+4+4+4+32+32+(PRUNING_FACTOR*32)
    }
    
    //Stores block commitment in the array, no ring buffer is implemented here,
    // limiting the maximum fork length to PRUNING_FACTOR blocks
    pub fn store_block_commitment(&mut self, block_commitment: [u8; 32]) {
        self.block_commitments[self.length as usize] = block_commitment;
        self.length += 1;
    }

}
