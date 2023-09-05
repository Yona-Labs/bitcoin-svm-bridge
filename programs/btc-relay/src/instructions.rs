use anchor_lang::prelude::*;

use crate::structs::*;
use crate::state::*;

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
