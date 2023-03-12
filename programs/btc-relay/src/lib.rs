use anchor_lang::{
    prelude::*,
    solana_program::clock,
    solana_program::hash
};

pub fn now_ts() -> Result<u32> {
    Ok(clock::Clock::get()?.unix_timestamp.try_into().unwrap())
}

declare_id!("8DMFpUfCk8KPkNLtE25XHuCSsT1GqYxuLdGzu59QK3Rt");

static MAX_DIFFICULTY: [u8; 32] = [
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0xFF as u8,
    0xFF as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8
];

static UNROUNDED_MAX_TARGET: [u8; 32] = [
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0x00 as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8,
    0xFF as u8
];

const PRUNING_FACTOR_U32: u32 = 250;
const PRUNING_FACTOR: usize = PRUNING_FACTOR_U32 as usize;

static DIFF_ADJUSTMENT_INTERVAL: u32 = 2016;
static TARGET_TIMESPAN: u32 = 14 * 24 * 60 * 60; // 2 weeks
static TARGET_TIMESPAN_DIV_4: u32 = TARGET_TIMESPAN / 4;
static TARGET_TIMESPAN_MUL_4: u32 = TARGET_TIMESPAN * 4;
static MAX_FUTURE_BLOCKTIME: u32 = 4 * 60 * 60;

pub mod utils {
    use super::*;

    pub fn target_to_nbits(target: [u8; 32]) -> u32 {

        let mut n_size: u32 = 0;
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
            n_compact = n_compact >> 8;
            n_size += 1;
        }
    
        n_compact = n_compact & 0x00FFFFFF |
                    (n_size<<24) & 0xFF000000;
    
        return n_compact;
    
    }
    
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
    
        return target;
    }

    pub fn add_in_place(arr: &mut [u8; 32], add: [u8; 32]) {
        let mut remainder: u16 = 0;

        for i in 0..32 {
            let pos = 31-i;
            
            let val = ((arr[pos] as u16) + (add[pos] as u16)) + remainder;
            
            let byte = val & 0xFF;
            remainder = val >> 8;

            arr[pos] = byte as u8;
        }
    }

    pub fn mul_in_place(arr: &mut [u8; 32], multiplicator: u32) {
        let casted_mul: u64 = multiplicator as u64;
        let mut remainder: u64 = 0;
    
        for i in 0..32 {
            let pos = 31-i;
    
            let val = ((arr[pos] as u64)*casted_mul) + remainder;
    
            let byte = val & 0xFF;
            remainder = val >> 8;
    
            arr[pos] = byte as u8;
        }
    }

    pub fn div_in_place(arr: &mut [u8; 32], divisor: u32) {
        let casted_div: u64 = divisor as u64;
        let mut remainder: u64 = 0;

        for i in 0..32 {
            let val: u64 = (arr[i] as u64) + remainder;
            let result = val / casted_div;

            remainder = (val % casted_div)<<8;

            arr[i] = result as u8;
        }
    }

    pub fn gte_arr(arr1: [u8; 32], arr2: [u8; 32]) -> bool {
        for i in 0..32 {
            if arr1[i]>arr2[i] {return true};
            if arr1[i]<arr2[i] {return false};
        }
        return true;
    }

    pub fn lte_arr(arr1: [u8; 32], arr2: [u8; 32]) -> bool {
        return gte_arr(arr2, arr1);
    }

    pub fn gt_arr(arr1: [u8; 32], arr2: [u8; 32]) -> bool {
        for i in 0..32 {
            if arr1[i]>arr2[i] {return true};
            if arr1[i]<arr2[i] {return false};
        }
        return false;
    }

    pub fn lt_arr(arr1: [u8; 32], arr2: [u8; 32]) -> bool {
        return gt_arr(arr2, arr1);
    }

    pub fn get_difficulty(target: [u8; 32]) -> [u8; 32] {

        let mut start = 0;
        for i in 0..32 {
            if target[i]>0 {
                start = i;
                break
            }
        }
    
        let shift = 32 - start - 3;
    
        let mut num: u32 = 0;
        for i in 0..3 {
            num |= (target[start+i] as u32) << ((2-i)*8);
        }
    
        let mut arr: [u8; 32] = MAX_DIFFICULTY;
    
        div_in_place(&mut arr, num);
    
        let mut result: [u8; 32] = [0;32];
    
        for i in 0..(32-shift) {
            result[i+shift] = arr[i];
        }
    
        return result;
    
    }

    pub fn compute_new_nbits(prev_time: u32, start_time: u32, prev_target: &mut [u8; 32]) -> u32 {

        let mut time_span = prev_time - start_time;

        if time_span < TARGET_TIMESPAN_DIV_4 {
            time_span = TARGET_TIMESPAN_DIV_4;
        }

        if time_span > TARGET_TIMESPAN_MUL_4 {
            time_span = TARGET_TIMESPAN_MUL_4;
        }

        mul_in_place(prev_target, time_span);
        div_in_place(prev_target, TARGET_TIMESPAN);

        if gt_arr(*prev_target, UNROUNDED_MAX_TARGET) {
            return target_to_nbits(UNROUNDED_MAX_TARGET);
        }

        return target_to_nbits(*prev_target);
    }

    pub fn should_diff_adjust(block_height: u32) -> bool {
        return block_height % DIFF_ADJUSTMENT_INTERVAL == 0;
    }

    pub fn has_correct_difficulty_target(prev_committed_header: CommittedBlockHeader, current_nbits: u32) -> bool {
        let prev_nbits = prev_committed_header.header.nbits;
        let mut prev_target = nbits_to_target(prev_nbits);

        if should_diff_adjust(prev_committed_header.blockheight+1) {
            let prev_time = prev_committed_header.header.timestamp;
            let start_time = prev_committed_header.last_diff_adjustment;
            msg!("Prev target: {:x?}", prev_target);
            let new_nbits = compute_new_nbits(prev_time, start_time, &mut prev_target);
            msg!("New computed nbits: {:x?}", new_nbits);
            msg!("New target: {:x?}", prev_target);
            return current_nbits == new_nbits;
        } else {
            return current_nbits == prev_nbits;
        }
    }

    pub fn is_larger_than_median(arr: [u32; 10], additional: u32, curr_timestamp: u32) -> bool {
        let mut amt = 0;
        for i in 0..10 {
            if curr_timestamp>arr[i] {
                amt += 1;
            }
        }
        if curr_timestamp>additional {
            amt += 1;
        }

        return amt>5;
    }

    pub fn verify_header(header: &BlockHeader, last_commited_header: &mut CommittedBlockHeader, remaining_account: &AccountInfo, _signer: &Signer, program_id: &Pubkey) -> Result<[u8; 32]> {
        
        //Correct difficulty target
        // !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
        // !!!!!!!!!!!!!!  DISABLE FOR TESTNET  !!!!!!!!!!!
        // !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
        require!(
            utils::has_correct_difficulty_target(*last_commited_header, header.nbits),
            RelayErrorCode::ErrDiffTarget
        );
        
        //Set last_diff_adjustment if should be adjusted
        let timestamp = header.timestamp;
        if utils::should_diff_adjust(last_commited_header.blockheight+1) {
            last_commited_header.last_diff_adjustment = timestamp;
        }
        
        //Check if valid topic was specified in remaining accounts
        let last_block_hash = header.get_block_hash()?;
        let (block_header_topic, _block_header_bump) = Pubkey::find_program_address(&[b"header", &last_block_hash], &program_id);
        require!(
            block_header_topic == *remaining_account.key,
            RelayErrorCode::InvalidHeaderTopic
        );

        //Check block's PoW
        let mut block_hash = last_block_hash;
        block_hash.reverse();
        let target = utils::nbits_to_target(header.nbits);
        require!(
            utils::lte_arr(block_hash, target),
            RelayErrorCode::ErrPowToolow
        );

        let prev_block_timestamp = last_commited_header.header.timestamp;

        //Verify timestamp is larger than median of last 11 block timestamps
        require!(
            utils::is_larger_than_median(last_commited_header.prev_block_timestamps, prev_block_timestamp, timestamp),
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
        utils::add_in_place(&mut last_commited_header.chain_work, utils::get_difficulty(target));

        Ok(last_block_hash)
    }

    pub fn compute_merkle(reversed_txid: &[u8; 32], _tx_index: u32, reversed_merkle_proof: &Vec<[u8; 32]>) -> [u8; 32] {
        if reversed_merkle_proof.len()==0 {
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
            tx_index = tx_index >> 1;
        }

        return current_hash;
    }

}

#[program]
pub mod btc_relay {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        data: BlockHeader,
        block_height: u32,
        chain_work: [u8; 32],
        last_diff_adjustment: u32,
        prev_block_timestamps: [u32; 10]
    ) -> Result<()> {
        
        let main_state = &mut ctx.accounts.main_state.load_init()?;

        main_state.last_diff_adjustment = last_diff_adjustment;
        main_state.block_height = block_height;
        main_state.chain_work = chain_work;

        main_state.fork_counter = 0;

        let commited_header = CommittedBlockHeader {
            chain_work: chain_work,

            header: data,
        
            last_diff_adjustment: last_diff_adjustment,
            blockheight: block_height,
        
            prev_block_timestamps: prev_block_timestamps
        };

        let hash_result = commited_header.get_commit_hash()?;
        let block_hash = data.get_block_hash()?;

        // let mut vec: Vec<[u8; 32]> = Vec::new();
        // vec.push(hash_result);
        main_state.block_commitments[0] = hash_result;
        
        main_state.start_height = block_height;
        main_state.total_blocks = 1;

        main_state.tip_block_hash = block_hash;
        main_state.tip_commit_hash = hash_result;

        emit!(StoreHeader {
            block_hash: block_hash,
            commit_hash: hash_result,
            header: commited_header
        });

        Ok(())
    }

    pub fn submit_block_headers(ctx: Context<SubmitBlockHeaders>, data: Vec<BlockHeader>, commited_header: CommittedBlockHeader) -> Result<()> {
        require!(
            data.len() > 0,
            RelayErrorCode::NoHeaders
        );

        //Verify commited header was indeed committed
        let commit_hash = commited_header.get_commit_hash()?;

        let main_state = &mut ctx.accounts.main_state.load_mut()?;

        require!(
            commit_hash == main_state.get_commitment(main_state.block_height),
            RelayErrorCode::PrevBlockCommitment
        );

        let mut last_commited_header = commited_header;
        let mut last_block_hash: [u8; 32] = commited_header.header.get_block_hash()?;
        let mut block_height = main_state.block_height;

        let mut block_commit_hash: [u8; 32] = [0; 32];

        let mut block_cnt = 0;
        for header in data.iter() {
            //Prev block hash matches
            require!(
                last_block_hash == header.reversed_prev_blockhash,
                RelayErrorCode::PrevBlock
            );

            block_height+=1;

            last_block_hash = utils::verify_header(header, &mut last_commited_header, &ctx.remaining_accounts[block_cnt], &ctx.accounts.signer, ctx.program_id)?;
            
            //Compute commit hash
            block_commit_hash = last_commited_header.get_commit_hash()?;

            //Store and emit
            main_state.store_block_commitment(block_height, block_commit_hash);
            emit!(StoreHeader {
                block_hash: last_block_hash,
                commit_hash: block_commit_hash,
                header: last_commited_header
            });

            block_cnt += 1;
        }

        //Update globals
        main_state.last_diff_adjustment = last_commited_header.last_diff_adjustment;
        main_state.block_height = block_height;
        main_state.chain_work = last_commited_header.chain_work;
        main_state.tip_commit_hash = block_commit_hash;
        main_state.tip_block_hash = last_block_hash;

        Ok(())
    }

    pub fn submit_short_fork_headers(ctx: Context<SubmitShortForkHeaders>, data: Vec<BlockHeader>, commited_header: CommittedBlockHeader) -> Result<()> {
        require!(
            data.len() > 0,
            RelayErrorCode::NoHeaders
        );

        //Verify commited header was indeed committed
        let commit_hash = commited_header.get_commit_hash()?;

        let main_state = &mut ctx.accounts.main_state.load_mut()?;

        require!(
            commit_hash == main_state.get_commitment(commited_header.blockheight),
            RelayErrorCode::PrevBlockCommitment
        );

        let fork_id = main_state.fork_counter;
        let mut last_commited_header = commited_header;
        let mut last_block_hash: [u8; 32] = commited_header.header.get_block_hash()?;
        let mut block_height = commited_header.blockheight;

        let mut block_commit_hash: [u8; 32] = [0; 32];

        let mut block_cnt = 0;
        for header in data.iter() {
            //Prev block hash matches
            require!(
                last_block_hash == header.reversed_prev_blockhash,
                RelayErrorCode::PrevBlock
            );

            block_height+=1;

            last_block_hash = utils::verify_header(header, &mut last_commited_header, &ctx.remaining_accounts[block_cnt], &ctx.accounts.signer, ctx.program_id)?;
            
            //Compute commit hash
            block_commit_hash = last_commited_header.get_commit_hash()?;

            //Store and emit
            main_state.store_block_commitment(block_height, block_commit_hash);
            emit!(StoreFork {
                fork_id: fork_id,
                block_hash: last_block_hash,
                commit_hash: block_commit_hash,
                header: last_commited_header
            });

            block_cnt += 1;
        }

        require!(
            utils::gt_arr(last_commited_header.chain_work, main_state.chain_work),
            RelayErrorCode::ForkTooShort
        );

        //Update globals
        main_state.last_diff_adjustment = last_commited_header.last_diff_adjustment;
        main_state.block_height = block_height;
        main_state.chain_work = last_commited_header.chain_work;
        main_state.tip_commit_hash = block_commit_hash;
        main_state.tip_block_hash = last_block_hash;
        main_state.fork_counter = fork_id+1;

        Ok(())
    }

    pub fn submit_fork_headers(ctx: Context<SubmitForkHeaders>, data: Vec<BlockHeader>, commited_header: CommittedBlockHeader, fork_id: u64, init: bool) -> Result<()> {

        let mut close = false;

        {
            require!(
                data.len() > 0,
                RelayErrorCode::NoHeaders
            );

            let load_res;
            if init {
                load_res = ctx.accounts.fork_state.load_init();
            } else {
                load_res = ctx.accounts.fork_state.load_mut();
            }

            let fork_state = &mut load_res?;

            require!(
                init == (fork_state.initialized==0),
                RelayErrorCode::ErrInit
            );

            let main_state = &mut ctx.accounts.main_state.load_mut()?;

            let commit_hash = commited_header.get_commit_hash()?;

            let mut block_height = commited_header.blockheight;

            if fork_state.initialized==0 {
                
                require!(
                    main_state.fork_counter == fork_id,
                    RelayErrorCode::NoHeaders
                );

                main_state.fork_counter = fork_id+1;

                //Verify commited header was indeed committed
                require!(
                    commit_hash == main_state.get_commitment(commited_header.blockheight),
                    RelayErrorCode::PrevBlockCommitment
                );

                fork_state.initialized = 1;
                fork_state.start_height = block_height;
            } else {
                require!(
                    commit_hash == fork_state.tip_commit_hash,
                    RelayErrorCode::PrevBlockCommitment
                );
            }

            let mut last_commited_header = commited_header;
            let mut last_block_hash: [u8; 32] = commited_header.header.get_block_hash()?;

            let mut block_commit_hash: [u8; 32] = [0; 32];

            let mut block_cnt = 0;
            for header in data.iter() {
                //Prev block hash matches
                require!(
                    last_block_hash == header.reversed_prev_blockhash,
                    RelayErrorCode::PrevBlock
                );

                block_height+=1;

                last_block_hash = utils::verify_header(header, &mut last_commited_header, &ctx.remaining_accounts[block_cnt], &ctx.accounts.signer, ctx.program_id)?;
                
                //Compute commit hash
                block_commit_hash = last_commited_header.get_commit_hash()?;

                //Store and emit
                fork_state.store_block_commitment(block_commit_hash);
                emit!(StoreFork {
                    fork_id: fork_id,
                    block_hash: last_block_hash,
                    commit_hash: block_commit_hash,
                    header: last_commited_header
                });

                block_cnt += 1;
            }

            if utils::gt_arr(last_commited_header.chain_work, main_state.chain_work) {
                //Successful fork

                msg!("Successful fork...");

                let start_height = fork_state.start_height;
                for i in 0..fork_state.length {
                    main_state.store_block_commitment(start_height+1+i, fork_state.block_commitments[i as usize]);
                }

                msg!("Commitments stored...");

                main_state.last_diff_adjustment = last_commited_header.last_diff_adjustment;
                main_state.block_height = block_height;
                main_state.chain_work = last_commited_header.chain_work;
                main_state.tip_commit_hash = block_commit_hash;
                main_state.tip_block_hash = last_block_hash;

                msg!("Main state updated");

                //Close the account
                close = true;

                emit!(ChainReorg {
                    fork_id: fork_id,
                    start_height: start_height,
                    tip_block_hash: last_block_hash,
                    tip_commit_hash: block_commit_hash
                });
            } else {
                //Fork still needs to be appended
                fork_state.tip_block_hash = last_block_hash;
                fork_state.tip_commit_hash = block_commit_hash;
            }
        }

        if close {
            ctx.accounts.fork_state.close(ctx.accounts.signer.to_account_info())?;
            msg!("Account closed");
        }

        Ok(())
    }

    pub fn close_fork_account(_ctx: Context<CloseForkAccount>, _fork_id: u64) -> Result<()> {
        Ok(())
    }

    pub fn verify_transaction(ctx: Context<VerifyTransaction>, reversed_txid: [u8; 32], confirmations: u32, tx_index: u32, reversed_merkle_proof: Vec<[u8; 32]>, commited_header: CommittedBlockHeader) -> Result<()> {
        let block_height = commited_header.blockheight;

        let main_state = &mut ctx.accounts.main_state.load_mut()?;

        require!(
            main_state.block_height - block_height + 1 >= confirmations,
            RelayErrorCode::BlockConfirmations
        );

        let commit_hash = commited_header.get_commit_hash()?;
        require!(
            commit_hash == main_state.get_commitment(block_height),
            RelayErrorCode::PrevBlockCommitment
        );

        let computed_merkle = utils::compute_merkle(&reversed_txid, tx_index, &reversed_merkle_proof);

        require!(
            computed_merkle == commited_header.header.merkle_root,
            RelayErrorCode::MerkleRoot
        );

        Ok(())
    }

    pub fn block_height(ctx: Context<BlockHeight>, value: u32, operation: u32) -> Result<()> {
        let main_state = &mut ctx.accounts.main_state.load_mut()?;
        let block_height = main_state.block_height;

        let result = match operation {
            0 => block_height < value,
            1 => block_height <= value,
            2 => block_height > value,
            3 => block_height >= value,
            4 => block_height == value,
            _ => false
        };

        require!(
            result,
            RelayErrorCode::InvalidBlockheight
        );

        Ok(())
    }

}

#[derive(Accounts)]
#[instruction(
    data: BlockHeader,
    block_height: u32,
    chain_work: [u8; 32],
    last_diff_adjustment: u32,
    prev_block_timestamps: [u32; 10]
)]
pub struct Initialize<'info> {
    /// CHECK: This is not dangerous because we don't read or write from this account
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        init,
        seeds = [b"state".as_ref()],
        bump,
        payer = signer,
        space = MainState::space()
    )]
    pub main_state: AccountLoader<'info, MainState>,

    /// CHECK: This is not dangerous because we don't read or write from this account
    #[account(
        seeds = [b"header".as_ref(), data.get_block_hash()?.as_ref()],
        bump
    )]
    pub header_topic: AccountInfo<'info>,

    /// CHECK: This is not dangerous because we don't read or write from this account
    pub system_program: Program<'info, System>
}

#[derive(Accounts)]
pub struct SubmitBlockHeaders<'info> {
    /// CHECK: This is not dangerous because we don't read or write from this account
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>
}

#[derive(Accounts)]
pub struct SubmitShortForkHeaders<'info> {
    /// CHECK: This is not dangerous because we don't read or write from this account
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>
}

#[derive(Accounts)]
#[instruction(
    headers: Vec<BlockHeader>,
    commited_header: CommittedBlockHeader,
    fork_id: u64,
    init: bool
)]
pub struct SubmitForkHeaders<'info> {
    /// CHECK: This is not dangerous because we don't read or write from this account
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>,

    #[account(
        init_if_needed,
        seeds = [b"fork".as_ref(), fork_id.to_le_bytes().as_ref(), signer.key.to_bytes().as_ref()],
        bump,
        payer = signer,
        space = ForkState::space()
    )]
    pub fork_state: AccountLoader<'info, ForkState>,

    /// CHECK: This is not dangerous because we don't read or write from this account
    pub system_program: Program<'info, System>
}

#[derive(Accounts)]
#[instruction(
    fork_id: u64
)]
pub struct CloseForkAccount<'info> {
    /// CHECK: This is not dangerous because we don't read or write from this account
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"fork".as_ref(), fork_id.to_le_bytes().as_ref(), signer.key.to_bytes().as_ref()],
        bump,
        close = signer
    )]
    pub fork_state: AccountLoader<'info, ForkState>,

    /// CHECK: This is not dangerous because we don't read or write from this account
    pub system_program: Program<'info, System>
}

#[derive(Accounts)]
pub struct VerifyTransaction<'info> {
    /// CHECK: This is not dangerous because we don't read or write from this account
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>
}

#[derive(Accounts)]
pub struct BlockHeight<'info> {
    /// CHECK: This is not dangerous because we don't read or write from this account
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>
}

#[account(zero_copy)]
#[repr(C)]
pub struct MainState {
    start_height: u32,
    last_diff_adjustment: u32,
    block_height: u32,
    total_blocks: u32,
    
    fork_counter: u64,

    tip_commit_hash: [u8; 32],
    tip_block_hash: [u8; 32],

    chain_work: [u8; 32],
    block_commitments: [[u8; 32]; 250]
}

impl MainState {
    
    pub fn space() -> usize {
        8+8+4+4+4+32+8+4+32+32+(PRUNING_FACTOR*32)
    }

    pub fn get_commitment(&self, block_height: u32) -> [u8; 32] {
        if block_height>self.block_height {
            return [0; 32];
        }
        if block_height<=self.block_height-PRUNING_FACTOR_U32 {
            return [0; 32];
        }
        let pos = self.get_position(block_height);
        if pos==PRUNING_FACTOR {
            return [0; 32];
        }
        return self.block_commitments[pos];
    }

    pub fn get_position(&self, block_height: u32) -> usize {
        if self.start_height<=block_height {
            let pos = block_height-self.start_height;
            if pos>=PRUNING_FACTOR_U32 {
                return 0;
            }
            return pos as usize;
        } else {
            let pos = self.start_height-block_height;
            if pos>=PRUNING_FACTOR_U32 {
                return PRUNING_FACTOR;
            }
            return pos as usize;
        }
    }

    pub fn store_block_commitment(&mut self, block_height: u32, block_commitment: [u8; 32]) -> bool {
        // if self.total_blocks<PRUNING_FACTOR_U32 {
        //     self.block_commitments.push(block_commitment);
        // } else {
            let position = self.get_position(block_height);
            if position==PRUNING_FACTOR {
                return false;
            }
            self.block_commitments[position] =  block_commitment;
            if position==0 {
                self.start_height = block_height;
            }
        // }
        self.total_blocks += 1;
        return true;
    }

}

#[account(zero_copy)]
#[repr(C)]
pub struct ForkState {
    initialized: u32,
    start_height: u32, //Blockheight of last block that is also in main chain
    length: u32,

    tip_commit_hash: [u8; 32],
    tip_block_hash: [u8; 32],

    block_commitments: [[u8; 32]; 250]
}

impl ForkState {
    
    pub fn space() -> usize {
        8+4+4+4+32+32+(PRUNING_FACTOR*32)
    }
    
    pub fn store_block_commitment(&mut self, block_commitment: [u8; 32]) -> bool {
        self.block_commitments[self.length as usize] = block_commitment;
        self.length += 1;

        return true;
    }

}

#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize, Copy)]
pub struct BlockHeader {
    version: u32,
    reversed_prev_blockhash: [u8; 32],
    merkle_root: [u8; 32],
    timestamp: u32,
    nbits: u32,
    nonce: u32
}

impl BlockHeader {

    pub fn get_block_hash(&self) -> Result<[u8; 32]> {
        let arr = self.try_to_vec()?;

        Ok(hash::hash(&hash::hash(&arr).to_bytes()).to_bytes())
    }

}

#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize, Copy)]
pub struct CommittedBlockHeader {
    chain_work: [u8; 32],
    
    header: BlockHeader,

    last_diff_adjustment: u32,
    blockheight: u32,

    prev_block_timestamps: [u32; 10]
}

impl CommittedBlockHeader {

    pub fn get_commit_hash(&self) -> Result<[u8; 32]> {
        let arr = self.try_to_vec()?;

        Ok(hash::hash(&arr).to_bytes())
    }

}

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

#[error_code]
pub enum RelayErrorCode {
    #[msg("Invalid previous block commitment.")]
    PrevBlockCommitment,
    #[msg("Invalid previous block.")]
    PrevBlock,
    #[msg("Invalid difficulty target.")]
    ErrDiffTarget,
    #[msg("PoW too low.")]
    ErrPowToolow,
    #[msg("Timestamp too low.")]
    ErrTimestampToolow,
    #[msg("Timestamp too high.")]
    ErrTimestampTooHigh,
    #[msg("Invalid header topic specified in accounts.")]
    InvalidHeaderTopic,
    #[msg("No headers supplied")]
    NoHeaders,
    #[msg("Fork too short to become main chains")]
    ForkTooShort,
    #[msg("Fork initialization error")]
    ErrInit,
    #[msg("Block doesn't have required number of confirmations")]
    BlockConfirmations,
    #[msg("Invalid merkle root")]
    MerkleRoot,
    #[msg("Blockheight doesn't match")]
    InvalidBlockheight
}