use anchor_lang::prelude::*;

use crate::state::*;
use crate::structs::*;

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

    pub system_program: Program<'info, System>,
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
    pub main_state: AccountLoader<'info, MainState>,
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
    pub main_state: AccountLoader<'info, MainState>,
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

    pub system_program: Program<'info, System>,
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
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct VerifyTransaction<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>,
    #[account(mut, seeds = [b"solana_deposit".as_ref()], bump)]
    pub deposit_account: AccountLoader<'info, DepositState>,
    #[account(mut)]
    pub mint_receiver: SystemAccount<'info>,
}

#[derive(Accounts)]
pub struct BlockHeight<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    /// The user account initiating the deposit.
    #[account(mut)]
    pub user: Signer<'info>,
    /// The program's account to receive the deposit. This should be a derived PDA (Program Derived Address).
    #[account(init, seeds = [b"solana_deposit".as_ref()], bump, payer = user, space = 8 + 1)]
    pub deposit_account: AccountLoader<'info, DepositState>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(
    tx_id: [u8; 32],
    tx_size: u64
)]
pub struct InitBigTxVerify<'info> {
    /// The user account initiating the verification.
    #[account(mut)]
    pub user: Signer<'info>,
    /// The program's account used to store transaction's data. This should be a derived PDA (Program Derived Address).
    #[account(init, seeds = [tx_id.as_slice()], bump, payer = user, space = 8 + 4 + tx_size as usize)]
    pub tx_account: Account<'info, BigTxState>,
    pub system_program: Program<'info, System>,
    #[account(
        seeds = [b"state".as_ref()],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>,
}

#[derive(Accounts)]
#[instruction(
    tx_id: [u8; 32]
)]
pub struct StoreTxBytes<'info> {
    /// The user account initiating the verification.
    #[account(mut)]
    pub user: Signer<'info>,
    /// The program's account used to store transaction's data. This should be a derived PDA (Program Derived Address).
    #[account(mut, seeds = [tx_id.as_slice()], bump)]
    pub tx_account: Account<'info, BigTxState>,
}

#[derive(Accounts)]
#[instruction(
    tx_id: [u8; 32]
)]
pub struct FinalizeTx<'info> {
    /// The user account initiating the verification.
    #[account(mut)]
    pub user: Signer<'info>,
    /// The program's account used to store transaction's data. This should be a derived PDA (Program Derived Address).
    #[account(mut, seeds = [tx_id.as_slice()], bump)]
    pub tx_account: Account<'info, BigTxState>,
    #[account(mut, seeds = [b"solana_deposit".as_ref()], bump)]
    pub deposit_account: AccountLoader<'info, DepositState>,
    #[account(mut)]
    pub mint_receiver: SystemAccount<'info>,
}
