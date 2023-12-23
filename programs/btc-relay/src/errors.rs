use anchor_lang::prelude::*;

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
    InvalidBlockheight,
    #[msg("Fork has invalid ID")]
    InvalidForkId,
    #[msg("Didn't pass enough remaining accounts!")]
    InvalidRemainingAccounts
}
