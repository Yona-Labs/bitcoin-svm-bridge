use crate::config::*;
use crate::state::*;
use crate::structs::*;

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Mint, Token, TokenAccount};

#[derive(Accounts)]
#[instruction(
    data: BlockHeader
)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(
        init,
        seeds = [STATE_SEED],
        bump,
        payer = signer,
        space = MainState::space()
    )]
    pub main_state: AccountLoader<'info, MainState>,
    #[account(
        init,
        seeds = [WBTC_MINT_SEED],
        bump,
        payer = signer,
        mint::decimals = 8,
        mint::authority = main_state,
        mint::freeze_authority = main_state
    )]
    pub wbtc_mint: Account<'info, Mint>,
    /// CHECK: This is only used for indexing purposes
    #[account(
        seeds = [b"header".as_ref(), data.get_block_hash()?.as_ref()],
        bump
    )]
    pub header_topic: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct SubmitBlockHeaders<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(
        mut,
        seeds = [STATE_SEED],
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
        seeds = [STATE_SEED],
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
        seeds = [STATE_SEED],
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
#[instruction(
    tx_id: [u8; 32]
)]
pub struct VerifyTransaction<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(
        seeds = [STATE_SEED],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>,
    /// We don't need to store transaction bytes here
    #[account(init, seeds = [tx_id.as_slice()], bump, payer = signer, space = DepositTxState::space(0))]
    pub tx_account: Account<'info, DepositTxState>,
    #[account()]
    pub wbtc_receiver_sol: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [WBTC_MINT_SEED],
        bump
    )]
    pub wbtc_mint: Account<'info, Mint>,
    #[account(
        init_if_needed,
        payer = signer,
        associated_token::mint = wbtc_mint,
        associated_token::authority = wbtc_receiver_sol
    )]
    pub wbtc_receiver: Account<'info, TokenAccount>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BlockHeight<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(
        seeds = [STATE_SEED],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>,
}

#[derive(Accounts)]
#[instruction(
    tx_id: [u8; 32],
    tx_size: u64
)]
pub struct InitBigTxVerify<'info> {
    /// The user account initiating the verification.
    #[account(mut)]
    pub signer: Signer<'info>,
    /// The program's account used to store transaction's data. This should be a derived PDA (Program Derived Address).
    #[account(init, seeds = [tx_id.as_slice()], bump, payer = signer, space = DepositTxState::space(tx_size))]
    pub tx_account: Account<'info, DepositTxState>,
    pub system_program: Program<'info, System>,
    #[account(
        seeds = [STATE_SEED],
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
    pub signer: Signer<'info>,
    /// The program's account used to store transaction's data. This should be a derived PDA (Program Derived Address).
    #[account(mut, seeds = [tx_id.as_slice()], bump)]
    pub tx_account: Account<'info, DepositTxState>,
}

#[derive(Accounts)]
#[instruction(
    tx_id: [u8; 32]
)]
pub struct FinalizeTx<'info> {
    /// The user account initiating the verification.
    #[account(mut)]
    pub signer: Signer<'info>,
    /// The program's account used to store transaction's data. This should be a derived PDA (Program Derived Address).
    #[account(mut, seeds = [tx_id.as_slice()], bump)]
    pub tx_account: Account<'info, DepositTxState>,
    #[account()]
    pub wbtc_receiver_sol: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [WBTC_MINT_SEED],
        bump
    )]
    pub wbtc_mint: Account<'info, Mint>,
    #[account(
        init_if_needed,
        payer = signer,
        associated_token::mint = wbtc_mint,
        associated_token::authority = wbtc_receiver_sol
    )]
    pub wbtc_receiver: Account<'info, TokenAccount>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Program<'info, Token>,
    #[account(
        seeds = [STATE_SEED],
        bump
    )]
    pub main_state: AccountLoader<'info, MainState>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BridgeWithdraw<'info> {
    /// The user account initiating the withdrawal.
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(
        mut,
        seeds = [WBTC_MINT_SEED],
        bump
    )]
    pub wbtc_mint: Account<'info, Mint>,
    #[account(
        mut,
        associated_token::mint = wbtc_mint,
        associated_token::authority = signer,
    )]
    pub wbtc_account: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}
