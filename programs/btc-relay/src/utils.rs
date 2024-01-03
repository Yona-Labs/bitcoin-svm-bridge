//Utilities for blockheader verification
use anchor_lang::{
    prelude::*,
    solana_program::clock,
    solana_program::hash
};
use crate::errors::*;
use crate::structs::*;
use crate::arrayutils;

//Returns current timestamp read from Solana's on-chain clock
pub fn now_ts() -> Result<u32> {
    Ok(clock::Clock::get()?.unix_timestamp.try_into().unwrap())
}

//https://en.bitcoin.it/wiki/Difficulty#How_is_difficulty_calculated.3F_What_is_the_difference_between_bdiff_and_pdiff.3F
const MAX_DIFFICULTY: [u8; 32] = [
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0xFF_u8,
    0xFF_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8
];

//https://en.bitcoin.it/wiki/Target#What_is_the_maximum_target.3F
const UNROUNDED_MAX_TARGET: [u8; 32] = [
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0x00_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8,
    0xFF_u8
];

//Bitcoin constants
const DIFF_ADJUSTMENT_INTERVAL: u32 = 2016;
const TARGET_TIMESPAN: u32 = 14 * 24 * 60 * 60; // 2 weeks

//Pre-calculated multiples for target timespan
const TARGET_TIMESPAN_DIV_4: u32 = TARGET_TIMESPAN / 4;
const TARGET_TIMESPAN_MUL_4: u32 = TARGET_TIMESPAN * 4;

//Maximum positive difference between bitcoin block's timestamp and Solana's on-chain clock
//Nodes in bitcoin network generally reject any block with timestamp more than 2 hours in the future
//As we are dealing with another blockchain here,
// with the possibility of the Solana's on-chain clock being skewed, we chose double the value - 4 hours
const MAX_FUTURE_BLOCKTIME: u32 = 4 * 60 * 60;

//Compresses difficulty target to nBits
//Description: https://btcinformation.org/en/developer-reference#target-nbits
pub fn target_to_nbits(target: [u8; 32]) -> u32 {

    let mut n_size: u32 = 0;
    #[allow(clippy::needless_range_loop)]
    for i in 0..32 {
        if target[i]>0 {
            n_size = (32-i) as u32;
            break;
        }
    }

    let mut n_compact: u32 = 0;

    for i in 0..3 {
        let pos = (32-n_size+i) as usize;
        if pos<32 {
            n_compact |= (target[pos] as u32) << ((2-i)*8);
        }
    }
    
    if (n_compact & 0x00800000) > 0 {
        n_compact >>= 8;
        n_size += 1;
    }

    n_compact = n_compact & 0x00FFFFFF |
                (n_size<<24) & 0xFF000000;

    n_compact

}

//Calculates difficulty target from nBits
//Description: https://btcinformation.org/en/developer-reference#target-nbits
pub fn nbits_to_target(nbits: u32) -> [u8; 32] {
    let mut target: [u8; 32] = [0; 32];

    let n_size = (nbits>>24) & 0xFF;

    let n_word: [u8; 3] = [
        ((nbits >> 16) & 0x7F) as u8,
        ((nbits >> 8) & 0xFF) as u8,
        ((nbits) & 0xFF) as u8,
    ];

    let start = (32-n_size) as usize;
    for i in 0..3 {
        if start+i<32 {
            target[start+i] = n_word[i];
        }
    }

    target
}

//Calculates difficulty from difficulty target
//https://en.bitcoin.it/wiki/Difficulty#How_is_difficulty_calculated.3F_What_is_the_difference_between_bdiff_and_pdiff.3F
// difficulty = MAX_DIFFICULTY/target
pub fn get_difficulty(target: [u8; 32]) -> [u8; 32] {

    //Find leading byte (first non-zero byte)
    let mut start = 0;
    
    #[allow(clippy::needless_range_loop)]
    for i in 0..32 {
        if target[i]>0 {
            start = i;
            break
        }
    }

    let shift = 32 - start - 3;

    //Target calculated from nBits will only ever have 3 bytes set,
    // preceeded and followed by zeroes 00 00 00 ... A1 D0 21 ... 00 00
    //We extract these first 3 bytes here to a u32,
    // essentially doing a floor division on the target:
    // num = target//(2^shift)
    let mut num: u32 = 0;
    for i in 0..3 {
        num |= (target[start+i] as u32) << ((2-i)*8);
    }

    //Do the division
    // arr = MAX_DIFFICULTY//num
    let mut arr: [u8; 32] = MAX_DIFFICULTY;
    arrayutils::div_in_place(&mut arr, num);

    let mut result: [u8; 32] = [0;32];

    //Shift the result back:
    // result = arr//(2^shift)
    #[allow(clippy::manual_memcpy)]
    for i in 0..(32-shift) {
        result[i+shift] = arr[i];
    }

    //Result
    // result = (MAX_DIFFICULTY // (target // (2^shift))) // (2^shift)
    // result = ((MAX_DIFFICULTY * (2^shift)) // target) // (2^shift)
    // result = (MAX_DIFFICULTY * (2^shift)) // (target * (2^shift))
    // result = MAX_DIFFICULTY // target
    result

}

//Difficulty retargetting algorithm
//https://minerdaily.com/2021/how-are-bitcoins-difficulty-and-hash-rate-calculated/#Difficulty_Adjustments
// new_difficulty_target = prev_difficulty_target * (timespan / target_timespan)
pub fn compute_new_nbits(prev_time: u32, start_time: u32, prev_target: &mut [u8; 32]) -> u32 {

    let mut time_span = prev_time - start_time;

    //Difficulty increase/decrease multiples are clamped between 0.25 (-75%) and 4 (+300%)
    if time_span < TARGET_TIMESPAN_DIV_4 {
        time_span = TARGET_TIMESPAN_DIV_4;
    }
    if time_span > TARGET_TIMESPAN_MUL_4 {
        time_span = TARGET_TIMESPAN_MUL_4;
    }

    arrayutils::mul_in_place(prev_target, time_span);
    arrayutils::div_in_place(prev_target, TARGET_TIMESPAN);

    //Check if the target isn't past maximum allowed target (lowest possible mining difficulty)
    //https://en.bitcoin.it/wiki/Target#What_is_the_maximum_target.3F
    if arrayutils::gt_arr(*prev_target, UNROUNDED_MAX_TARGET) {
        return target_to_nbits(UNROUNDED_MAX_TARGET);
    }

    target_to_nbits(*prev_target)
}

pub fn should_diff_adjust(block_height: u32) -> bool {
    block_height % DIFF_ADJUSTMENT_INTERVAL == 0
}

//Checks difficulty target (nBits) specified in the block,
// handles difficulty adjustmens happening every DIFF_ADJUSTMENT_INTERVAL blocks
pub fn has_correct_difficulty_target(prev_committed_header: CommittedBlockHeader, current_nbits: u32) -> bool {
    let prev_nbits = prev_committed_header.header.nbits;

    if should_diff_adjust(prev_committed_header.blockheight+1) {
        let mut prev_target = nbits_to_target(prev_nbits);
        let prev_time = prev_committed_header.header.timestamp;
        let start_time = prev_committed_header.last_diff_adjustment;
        msg!("Prev target: {:x?}", prev_target);
        let new_nbits = compute_new_nbits(prev_time, start_time, &mut prev_target);
        msg!("New computed nbits: {:x?}", new_nbits);
        msg!("New target: {:x?}", prev_target);
        current_nbits == new_nbits
    } else {
        current_nbits == prev_nbits
    }
}

//Checks if the timestamp is larger than median of the past block's timestamps (specified in arr and one additional value)
pub fn is_larger_than_median(arr: [u32; 10], additional: u32, curr_timestamp: u32) -> bool {
    let mut amt = 0;
    
    #[allow(clippy::needless_range_loop)]
    for i in 0..10 {
        if curr_timestamp>arr[i] {
            amt += 1;
        }
    }
    if curr_timestamp>additional {
        amt += 1;
    }

    amt>5
}

pub fn verify_header(header: &BlockHeader, last_commited_header: &mut CommittedBlockHeader, remaining_account: &AccountInfo, _signer: &Signer, program_id: &Pubkey) -> Result<[u8; 32]> {
    
    //Correct difficulty target
    //
    //Should be disabled for testnet, since if no valid block is
    // found on testnet in 20 minutes, the difficulty drops to 1
    //Implementing this functionality is beyond scope of this implementation,
    // so nBits checking is disabled for TESTNET
    #[cfg(not(feature = "bitcoin_testnet"))]
    {
        require!(
            has_correct_difficulty_target(*last_commited_header, header.nbits),
            RelayErrorCode::ErrDiffTarget
        );
    }
    
    //Set last_diff_adjustment if difficulty should be adjusted
    let timestamp = header.timestamp;
    if should_diff_adjust(last_commited_header.blockheight+1) {
        last_commited_header.last_diff_adjustment = timestamp;
    }
    
    //Check if valid topic was specified in remaining accounts
    //Each block is assigned a unique generated PDA,
    // this is used purely for indexing purposes
    let last_block_hash = header.get_block_hash()?;
    let (block_header_topic, _block_header_bump) = Pubkey::find_program_address(&[b"header", &last_block_hash], program_id);
    require!(
        block_header_topic == *remaining_account.key,
        RelayErrorCode::InvalidHeaderTopic
    );

    //Check block's PoW, it's hash has to be less than the target
    let mut block_hash = last_block_hash;
    block_hash.reverse();
    let target = nbits_to_target(header.nbits);
    require!(
        arrayutils::lte_arr(block_hash, target),
        RelayErrorCode::ErrPowToolow
    );

    let prev_block_timestamp = last_commited_header.header.timestamp;

    //Verify timestamp is larger than median of last 11 block timestamps
    require!(
        is_larger_than_median(last_commited_header.prev_block_timestamps, prev_block_timestamp, timestamp),
        RelayErrorCode::ErrTimestampToolow
    );

    let current_timestamp = now_ts()?;

    //Verify timestamp is no more than MAX_FUTURE_BLOCKTIME in the future
    require!(
        timestamp < current_timestamp+MAX_FUTURE_BLOCKTIME,
        RelayErrorCode::ErrTimestampTooHigh
    );

    //Set commited header's variables
    last_commited_header.header = *header;
    last_commited_header.blockheight += 1;
    for i in 1..10 {
        last_commited_header.prev_block_timestamps[i-1] = last_commited_header.prev_block_timestamps[i];
    }
    last_commited_header.prev_block_timestamps[9] = prev_block_timestamp;
    arrayutils::add_in_place(&mut last_commited_header.chain_work, get_difficulty(target));

    Ok(last_block_hash)
}

//Calculates merkle root based on the transaction id and merkle proof,
// reversed_ prefix is used because bitcoin uses little endian encoding
pub fn compute_merkle(reversed_txid: &[u8; 32], _tx_index: u32, reversed_merkle_proof: &Vec<[u8; 32]>) -> [u8; 32] {
    if reversed_merkle_proof.is_empty() {
        return *reversed_txid;
    }

    let mut current_hash = *reversed_txid;
    let mut tx_index = _tx_index;

    for piece in reversed_merkle_proof.iter() {
        let mut msg = Vec::with_capacity(32+32);
        if tx_index & 0x1 == 0 {
            //First pos
            msg.extend_from_slice(&current_hash);
            msg.extend_from_slice(piece);
        } else {
            //Second pos
            msg.extend_from_slice(piece);
            msg.extend_from_slice(&current_hash);
        }
        current_hash = hash::hash(&hash::hash(&msg).to_bytes()).to_bytes();
        tx_index >>= 1;
    }

    current_hash
}
