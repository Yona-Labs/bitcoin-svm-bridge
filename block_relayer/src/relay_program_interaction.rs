use crate::merkle::Proof;
use anchor_client::anchor_lang::prelude::{AccountDeserialize, AccountMeta};
use anchor_client::solana_sdk::compute_budget::ComputeBudgetInstruction;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{Keypair, Signature};
use anchor_client::ClientError as AnchorClientError;
use anchor_client::Program;
use anchor_spl::associated_token::get_associated_token_address;
use bitcoin::hashes::Hash;
use bitcoin::hex::DisplayHex;
use bitcoin::{Block, BlockHash, Txid};
use bitcoincore_rpc::{Client as BitcoinRpcClient, Error as BtcRpcError, RpcApi};
use btc_relay::accounts::{
    BridgeWithdraw, FinalizeTx, InitBigTxVerify, InitWbtcMeta, Initialize, StoreTxBytes,
    SubmitBlockHeaders, SubmitShortForkHeaders, VerifyTransaction,
};
use btc_relay::config::WBTC_MINT_SEED;
use btc_relay::instruction::{
    BridgeWithdraw as BridgeWithdrawInstruction, FinalizeTxProcessing,
    InitBigTxVerify as InitBigTxVerifyInstruction, InitWbtcMeta as InitWbtcMetaIx,
    Initialize as InitializeInstruction, StoreTxBytes as StoreTxBytesInstruction,
    SubmitBlockHeaders as SubmitBlockHeadersInstruction,
    SubmitShortForkHeaders as SubmitShortForkHeadersIx, VerifySmallTx as VerifySmallTxInstruction,
};
use btc_relay::state::{DepositTxState as ProgramDepositTxState, MainState, TxState};
use btc_relay::structs::{BlockHeader, CommittedBlockHeader};
use log::{debug, info};
use serde::Serialize;
use std::fmt;
use std::sync::Arc;

/// Calculate the difficulty adjustment boundary height for a given block height.
/// Difficulty adjustment happens every 2016 blocks.
/// Returns the height of the most recent difficulty adjustment block (inclusive).
pub(crate) fn calculate_diff_adjustment_boundary(height: u32) -> u32 {
    const DIFF_ADJUSTMENT_INTERVAL: u32 = 2016;
    height - (height % DIFF_ADJUSTMENT_INTERVAL)
}

pub(crate) fn reconstruct_commited_header(
    bitcoind_client: &BitcoinRpcClient,
    hash: &BlockHash,
    height: u32,
) -> Result<CommittedBlockHeader, BtcRpcError> {
    let header = bitcoind_client.get_block_header(hash)?;
    debug!("Got header {header:?}");

    // Calculate the correct last_diff_adjustment for this block height
    // Difficulty adjustment happens every 2016 blocks
    let boundary_height = calculate_diff_adjustment_boundary(height);
    
    let last_diff_adjustment = if boundary_height == height {
        // This block is itself a difficulty adjustment block
        header.time
    } else {
        // Get the timestamp of the most recent difficulty adjustment block
        let boundary_hash = bitcoind_client.get_block_hash(boundary_height as u64)?;
        let boundary_block = bitcoind_client.get_block(&boundary_hash)?;
        boundary_block.header.time
    };

    let mut prev_block_timestamps = [0; 10];
    for i in 0..10 {
        let prev_block_hash = bitcoind_client.get_block_hash(height as u64 - i as u64 - 1)?;
        let block = bitcoind_client.get_block(&prev_block_hash)?;
        prev_block_timestamps[9 - i] = block.header.time;
    }

    Ok(CommittedBlockHeader {
        chain_work: [0; 32],
        header: BlockHeader {
            version: header.version.to_consensus() as u32,
            reversed_prev_blockhash: header.prev_blockhash.to_byte_array(),
            merkle_root: header.merkle_root.to_byte_array(),
            timestamp: header.time,
            nbits: header.bits.to_consensus(),
            nonce: header.nonce,
        },
        last_diff_adjustment,
        blockheight: height,
        prev_block_timestamps,
    })
}

pub enum InitError {
    Anchor(AnchorClientError),
    Bitcoin(BtcRpcError),
}

impl From<AnchorClientError> for InitError {
    fn from(error: AnchorClientError) -> Self {
        InitError::Anchor(error)
    }
}

impl From<BtcRpcError> for InitError {
    fn from(error: BtcRpcError) -> Self {
        InitError::Bitcoin(error)
    }
}

pub async fn init_program(
    program: &Program<Arc<Keypair>>,
    bitcoind_client: &BitcoinRpcClient,
    block: Block,
    block_height: u32,
    deposit_pubkey_hash: [u8; 20],
) -> Result<Signature, InitError> {
    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &program.id());

    let yona_block_header = BlockHeader {
        version: block.header.version.to_consensus() as u32,
        reversed_prev_blockhash: block.header.prev_blockhash.to_byte_array(),
        merkle_root: block.header.merkle_root.to_byte_array(),
        timestamp: block.header.time,
        nbits: block.header.bits.to_consensus(),
        nonce: block.header.nonce,
    };

    let block_hash = yona_block_header
        .get_block_hash()
        .map_err(AnchorClientError::from)?;

    let mut prev_block_timestamps = [0; 10];
    for i in 0..10 {
        let prev_block_hash = tokio::task::block_in_place(|| {
            bitcoind_client.get_block_hash(block_height as u64 - i as u64 - 1)
        })?;
        let block = tokio::task::block_in_place(|| bitcoind_client.get_block(&prev_block_hash))?;
        prev_block_timestamps[9 - i] = block.header.time;
    }

    let (header_topic, _) =
        Pubkey::find_program_address(&[b"header", block_hash.as_slice()], &program.id());

    let (wbtc_mint, _) = Pubkey::find_program_address(&[WBTC_MINT_SEED], &program.id());

    let (wbtc_metadata, _) = Pubkey::find_program_address(
        &[
            b"metadata".as_slice(),
            anchor_spl::metadata::ID.as_ref(),
            wbtc_mint.as_ref(),
        ],
        &anchor_spl::metadata::ID,
    );

    let res = program
        .request()
        .accounts(Initialize {
            signer: program.payer(),
            main_state,
            wbtc_mint,
            header_topic,
            system_program: anchor_client::solana_sdk::system_program::ID,
            token_program: anchor_spl::token::ID,
            rent: anchor_client::solana_sdk::rent::sysvar::ID,
        })
        .args(InitializeInstruction {
            data: yona_block_header,
            block_height,
            chain_work: [0; 32],
            last_diff_adjustment: yona_block_header.timestamp,
            prev_block_timestamps,
            deposit_pubkey_hash,
        })
        .send()
        .await?;

    info!(
        "Submitted block {}, tx sig {res}",
        block_hash.to_lower_hex_string()
    );

    let meta_res = program
        .request()
        .accounts(InitWbtcMeta {
            signer: program.payer(),
            main_state,
            wbtc_mint,
            wbtc_metadata,
            token_metadata_program: anchor_spl::metadata::ID,
            system_program: anchor_client::solana_sdk::system_program::ID,
            token_program: anchor_spl::token::ID,
            rent: anchor_client::solana_sdk::rent::sysvar::ID,
        })
        .args(InitWbtcMetaIx {})
        .send()
        .await?;

    info!("Initialized wBTC metadata, tx sig {meta_res}");

    Ok(res)
}

pub(crate) async fn submit_block(
    program: &Program<Arc<Keypair>>,
    main_state: Pubkey,
    block: Block,
    height: u32,
    commited_header: CommittedBlockHeader,
) -> Result<Signature, AnchorClientError> {
    let yona_block_header = BlockHeader {
        version: block.header.version.to_consensus() as u32,
        reversed_prev_blockhash: block.header.prev_blockhash.to_byte_array(),
        merkle_root: block.header.merkle_root.to_byte_array(),
        timestamp: block.header.time,
        nbits: block.header.bits.to_consensus(),
        nonce: block.header.nonce,
    };

    let mut block_hash = yona_block_header.get_block_hash()?;
    let (header_topic, _) =
        Pubkey::find_program_address(&[b"header", block_hash.as_slice()], &program.id());

    let header_account = AccountMeta::new(header_topic, false);

    let res = program
        .request()
        .accounts(SubmitBlockHeaders {
            signer: program.payer(),
            main_state,
        })
        .accounts(vec![header_account])
        .args(SubmitBlockHeadersInstruction {
            data: vec![yona_block_header],
            commited_header,
        })
        .send()
        .await?;

    block_hash.reverse();
    info!(
        "Submitted block header. Hash {}, height {height}, Yona tx {res}",
        block_hash.to_lower_hex_string()
    );

    Ok(res)
}

pub(crate) async fn submit_block_fork(
    program: &Program<Arc<Keypair>>,
    main_state: Pubkey,
    block: Block,
    height: u32,
    commited_header: CommittedBlockHeader,
) -> Result<Signature, AnchorClientError> {
    let yona_block_header = BlockHeader {
        version: block.header.version.to_consensus() as u32,
        reversed_prev_blockhash: block.header.prev_blockhash.to_byte_array(),
        merkle_root: block.header.merkle_root.to_byte_array(),
        timestamp: block.header.time,
        nbits: block.header.bits.to_consensus(),
        nonce: block.header.nonce,
    };

    let mut block_hash = yona_block_header.get_block_hash()?;
    let (header_topic, _) =
        Pubkey::find_program_address(&[b"header", block_hash.as_slice()], &program.id());

    let header_account = AccountMeta::new(header_topic, false);

    let res = program
        .request()
        .accounts(SubmitShortForkHeaders {
            signer: program.payer(),
            main_state,
        })
        .accounts(vec![header_account])
        .args(SubmitShortForkHeadersIx {
            data: vec![yona_block_header],
            commited_header,
        })
        .send()
        .await?;

    block_hash.reverse();
    info!(
        "Submitted block header fork. Hash {}, height {height}, Yona tx {res}",
        block_hash.to_lower_hex_string()
    );

    Ok(res)
}

#[derive(Debug)]
pub enum RelayTxError {
    Anchor(AnchorClientError),
    BitcoinRpc(BtcRpcError),
    TxIsNotIncludedToBlock,
    CouldNotFindTxidInBlock,
}

impl From<AnchorClientError> for RelayTxError {
    fn from(error: AnchorClientError) -> Self {
        RelayTxError::Anchor(error)
    }
}

impl From<BtcRpcError> for RelayTxError {
    fn from(error: BtcRpcError) -> Self {
        RelayTxError::BitcoinRpc(error)
    }
}

pub async fn relay_tx(
    program: &Program<Arc<Keypair>>,
    main_state: Pubkey,
    bitcoind_client: Arc<BitcoinRpcClient>,
    tx_id: Txid,
    wbtc_receiver_sol: Pubkey,
) -> Result<Signature, RelayTxError> {
    let (wbtc_mint, _) = Pubkey::find_program_address(&[WBTC_MINT_SEED], &program.id());
    let wbtc_receiver = get_associated_token_address(&wbtc_receiver_sol, &wbtc_mint);

    let raw_account = program
        .rpc()
        .get_account(&main_state)
        .await
        .map_err(AnchorClientError::from)?;
    let main_state_data = MainState::try_deserialize(&mut &raw_account.data[..8160])
        .map_err(AnchorClientError::from)?;

    let client_clone = bitcoind_client.clone();
    let transaction =
        tokio::task::spawn_blocking(move || client_clone.get_raw_transaction_info(&tx_id, None))
            .await
            .expect("no panic")?;

    let block_hash = match transaction.blockhash {
        Some(hash) => hash,
        _ => return Err(RelayTxError::TxIsNotIncludedToBlock),
    };

    let client_clone = bitcoind_client.clone();
    let block_info = tokio::task::spawn_blocking(move || client_clone.get_block_info(&block_hash))
        .await
        .expect("no panic")?;

    let client_clone = bitcoind_client.clone();
    let commited_header = tokio::task::spawn_blocking(move || {
        reconstruct_commited_header(
            &client_clone,
            &block_hash,
            block_info.height as u32,
        )
    })
    .await
    .expect("no panic")?;

    let tx_pos = block_info
        .tx
        .iter()
        .position(|in_block| *in_block == tx_id)
        .ok_or(RelayTxError::CouldNotFindTxidInBlock)?;
    let reversed_merkle_proof = Proof::create(&block_info.tx, tx_pos).to_reversed_vec();

    let tx_id = transaction.txid.to_byte_array();
    let (tx_account, _) = Pubkey::find_program_address(&[tx_id.as_slice()], &program.id());

    if transaction.hex.len() + 32 * reversed_merkle_proof.len() > 800 {
        program
            .request()
            .accounts(InitBigTxVerify {
                signer: program.payer(),
                tx_account,
                system_program: anchor_client::solana_sdk::system_program::ID,
                main_state,
            })
            .args(InitBigTxVerifyInstruction {
                tx_id,
                confirmations: 1,
                tx_index: tx_pos as u32,
                commited_header,
                reversed_merkle_proof,
                tx_size: transaction.hex.len() as u64,
            })
            .send()
            .await?;

        for chunk in transaction.hex.chunks(800) {
            program
                .request()
                .accounts(StoreTxBytes {
                    signer: program.payer(),
                    tx_account,
                })
                .args(StoreTxBytesInstruction {
                    tx_id,
                    bytes: chunk.to_vec(),
                })
                .send()
                .await?;
        }

        let res = program
            .request()
            .instruction(ComputeBudgetInstruction::set_compute_unit_limit(500_000))
            .accounts(FinalizeTx {
                signer: program.payer(),
                tx_account,
                wbtc_receiver_sol,
                wbtc_mint,
                wbtc_receiver,
                associated_token_program: anchor_spl::associated_token::ID,
                main_state,
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::solana_sdk::system_program::ID,
            })
            .args(FinalizeTxProcessing { tx_id })
            .send()
            .await?;

        Ok(res)
    } else {
        let res = program
            .request()
            .accounts(VerifyTransaction {
                signer: program.payer(),
                main_state,
                tx_account,
                wbtc_receiver_sol,
                wbtc_mint,
                wbtc_receiver,
                associated_token_program: anchor_spl::associated_token::ID,
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::solana_sdk::system_program::ID,
            })
            .args(VerifySmallTxInstruction {
                tx_id,
                tx_bytes: transaction.hex,
                confirmations: 1,
                tx_index: tx_pos as u32,
                commited_header,
                reversed_merkle_proof,
            })
            .send()
            .await?;

        Ok(res)
    }
}

pub async fn bridge_withdraw(
    program: &Program<Arc<Keypair>>,
    amount: u64,
    bitcoin_address: String,
) -> Result<Signature, AnchorClientError> {
    let (wbtc_mint, _) = Pubkey::find_program_address(&[WBTC_MINT_SEED], &program.id());
    let wbtc_account = get_associated_token_address(&program.payer(), &wbtc_mint);

    let res = program
        .request()
        .accounts(BridgeWithdraw {
            signer: program.payer(),
            wbtc_mint,
            wbtc_account,
            token_program: anchor_spl::token::ID,
        })
        .args(BridgeWithdrawInstruction {
            amount,
            bitcoin_address,
        })
        .send()
        .await?;

    Ok(res)
}

#[derive(Serialize)]
pub enum DepositTxState {
    NotRelayed,
    Relayed,
}

impl fmt::Display for DepositTxState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DepositTxState::NotRelayed => write!(f, "NotRelayed"),
            DepositTxState::Relayed => write!(f, "Relayed"),
        }
    }
}

pub async fn deposit_tx_state(
    program: &Program<Arc<Keypair>>,
    tx_id: Txid,
) -> Result<DepositTxState, AnchorClientError> {
    let (tx_account, _) =
        Pubkey::find_program_address(&[tx_id.to_byte_array().as_slice()], &program.id());

    match program.account::<ProgramDepositTxState>(tx_account).await {
        Ok(state) => match state.state {
            TxState::VerificationInitialized => Ok(DepositTxState::NotRelayed),
            TxState::VerificationComplete => Ok(DepositTxState::Relayed),
        },
        Err(AnchorClientError::AccountNotFound) => Ok(DepositTxState::NotRelayed),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::calculate_diff_adjustment_boundary;

    #[test]
    fn test_calculate_diff_adjustment_boundary_at_boundary() {
        // Test blocks that are exactly at difficulty adjustment boundaries
        assert_eq!(calculate_diff_adjustment_boundary(0), 0);
        assert_eq!(calculate_diff_adjustment_boundary(2016), 2016);
        assert_eq!(calculate_diff_adjustment_boundary(4032), 4032);
        assert_eq!(calculate_diff_adjustment_boundary(6048), 6048);
    }

    #[test]
    fn test_calculate_diff_adjustment_boundary_between_boundaries() {
        // Test blocks that are between boundaries
        // Should round down to the previous boundary
        
        // Height 1-2015 should map to boundary 0
        assert_eq!(calculate_diff_adjustment_boundary(1), 0);
        assert_eq!(calculate_diff_adjustment_boundary(100), 0);
        assert_eq!(calculate_diff_adjustment_boundary(1000), 0);
        assert_eq!(calculate_diff_adjustment_boundary(2015), 0);
        
        // Height 2017-4031 should map to boundary 2016
        assert_eq!(calculate_diff_adjustment_boundary(2017), 2016);
        assert_eq!(calculate_diff_adjustment_boundary(3000), 2016);
        assert_eq!(calculate_diff_adjustment_boundary(4031), 2016);
        
        // Height 4033-6047 should map to boundary 4032
        assert_eq!(calculate_diff_adjustment_boundary(4033), 4032);
        assert_eq!(calculate_diff_adjustment_boundary(5000), 4032);
        assert_eq!(calculate_diff_adjustment_boundary(6047), 4032);
    }

    #[test]
    fn test_calculate_diff_adjustment_boundary_large_heights() {
        // Test with larger heights to ensure the logic works beyond initial boundaries
        assert_eq!(calculate_diff_adjustment_boundary(10000), 8064); // 10000 - (10000 % 2016) = 10000 - 1936 = 8064
        assert_eq!(calculate_diff_adjustment_boundary(100000), 98784); // 100000 - (100000 % 2016) = 100000 - 1216 = 98784
        assert_eq!(calculate_diff_adjustment_boundary(209664), 209664); // Exactly at a boundary (104 * 2016)
        assert_eq!(calculate_diff_adjustment_boundary(210000), 209664); // Should map to previous boundary
        assert_eq!(calculate_diff_adjustment_boundary(210001), 209664); // One past boundary
        assert_eq!(calculate_diff_adjustment_boundary(211680), 211680); // Exactly at next boundary (105 * 2016)
    }

    #[test]
    fn test_calculate_diff_adjustment_boundary_edge_cases() {
        // Test edge cases
        assert_eq!(calculate_diff_adjustment_boundary(u32::MAX), u32::MAX - (u32::MAX % 2016));
        
        // Test heights just before boundaries
        assert_eq!(calculate_diff_adjustment_boundary(2015), 0);
        assert_eq!(calculate_diff_adjustment_boundary(4031), 2016);
        
        // Test heights just after boundaries
        assert_eq!(calculate_diff_adjustment_boundary(2016), 2016);
        assert_eq!(calculate_diff_adjustment_boundary(2017), 2016);
    }

    #[test]
    fn test_calculate_diff_adjustment_boundary_produces_correct_interval() {
        // Verify that all heights in an interval map to the same boundary
        let boundary = calculate_diff_adjustment_boundary(5000);
        for height in 4032..=6047 {
            assert_eq!(
                calculate_diff_adjustment_boundary(height),
                boundary,
                "Height {} should map to boundary {}, but got {}",
                height,
                boundary,
                calculate_diff_adjustment_boundary(height)
            );
        }
    }
}
