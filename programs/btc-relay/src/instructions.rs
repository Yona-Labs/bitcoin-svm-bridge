use anchor_lang::prelude::*;

use crate::structs::*;
use crate::state::*;

#[derive(Accounts)]
#[instruction(
    data: BlockHeader
)]
pub struct Initialize<'info> {
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

    /// CHECK: This is only used for indexing purposes
    #[account(
        seeds = [b"header".as_ref(), data.get_block_hash()?.as_ref()],
        bump
    )]
    pub header_topic: AccountInfo<'info>,

    pub system_program: Program<'info, System>
}

#[derive(Accounts)]
pub struct SubmitBlockHeaders<'info> {
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
    fork_id: u64
)]
pub struct SubmitForkHeaders<'info> {
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

    pub system_program: Program<'info, System>
}

#[derive(Accounts)]
#[instruction(
    fork_id: u64
)]
pub struct CloseForkAccount<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"fork".as_ref(), fork_id.to_le_bytes().as_ref(), signer.key.to_bytes().as_ref()],
        bump,
        close = signer
    )]
    pub fork_state: AccountLoader<'info, ForkState>,

    pub system_program: Program<'info, System>
}

#[derive(Accounts)]
pub struct VerifyTransaction<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    #[cfg(not(feature = "mocked"))]
    #[account(
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>
}

#[derive(Accounts)]
pub struct BlockHeight<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,


    #[cfg(not(feature = "mocked"))]
    #[account(
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>
}
