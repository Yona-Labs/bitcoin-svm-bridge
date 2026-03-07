use crate::merkle::Proof;
use anchor_client::anchor_lang::{InstructionData, ToAccountMetas};
use anchor_client::anchor_lang::prelude::{AccountDeserialize, AccountMeta};
use anchor_client::solana_sdk::compute_budget::ComputeBudgetInstruction;
use anchor_client::solana_sdk::instruction::Instruction;
use anchor_client::solana_sdk::message::{Message, VersionedMessage};
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{Keypair,
     Signature
    };
use anchor_client::ClientError as AnchorClientError;
use anchor_client::Program;
use anchor_client::solana_sdk::transaction::VersionedTransaction;
use anchor_spl::associated_token::get_associated_token_address;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
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
use btc_relay::structs::{BlockHeader, CommittedBlockHeader, TxProofHeader};
use log::{debug, info};
use serde::Serialize;
use std::fmt;
use std::sync::Arc;

const MAX_RAW_TX: usize = 1232;
const MAX_B64_TX: usize = 1644;

pub(crate) fn reconstruct_commited_header(
    bitcoind_client: &BitcoinRpcClient,
    hash: &BlockHash,
    height: u32,
    last_diff_adjustment: u32,
) -> Result<CommittedBlockHeader, BtcRpcError> {
    let header = bitcoind_client.get_block_header(hash)?;
    debug!("Got header {header:?}");

    let mut prev_block_timestamps = [0; 10];
    for i in 0..10 {
        let prev_block_hash = bitcoind_client.get_block_hash(height as u64 - i as u64 - 1)?;
        let hdr = bitcoind_client.get_block_header(&prev_block_hash)?;
        prev_block_timestamps[9 - i] = hdr.time;
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


// chainwork из bitcoind: big-endian bytes
fn chainwork_bytes_to_u256_be(chainwork: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    if chainwork.is_empty() {
        return out;
    }
    let n = chainwork.len().min(32);
    out[32 - n..].copy_from_slice(&chainwork[chainwork.len() - n..]);
    out
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
        let hdr = bitcoind_client.get_block_header(&prev_block_hash)?;
        prev_block_timestamps[9 - i] = hdr.time;
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

    let block_info = tokio::task::block_in_place(|| bitcoind_client.get_block_info(&block.block_hash()))?;
    let chain_work = chainwork_bytes_to_u256_be(&block_info.chainwork);

    info!("chainwork as lossy string = {}", String::from_utf8_lossy(&block_info.chainwork));
    info!("chainwork bytes head = {:?}", &block_info.chainwork.get(..16));

    let last_adj_height = block_height - (block_height % 2016);
    let last_adj_hash = tokio::task::block_in_place(|| bitcoind_client.get_block_hash(last_adj_height as u64))?;
    let last_adj_hdr = tokio::task::block_in_place(|| bitcoind_client.get_block_header(&last_adj_hash))?;
    let last_diff_adjustment = last_adj_hdr.time;



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
            chain_work,
            last_diff_adjustment,
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
    TxIsNotIncludedToBlockBuffer
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

fn estimate_vtx_sizes(payer: Pubkey, ixs: &[Instruction]) -> (usize, usize, usize) {
    let msg = Message::new(ixs, Some(&payer));
    let vmsg = VersionedMessage::Legacy(msg);

    let sig_count = vmsg.header().num_required_signatures as usize;
    let vtx = VersionedTransaction {
        signatures: vec![Signature::default(); sig_count],
        message: vmsg,
    };

    let raw = bincode::serialize(&vtx).expect("serialize vtx");
    let raw_len = raw.len();
    let b64_len = BASE64_STANDARD.encode(&raw).len();

    let keys = match &vtx.message {
        VersionedMessage::Legacy(m) => m.account_keys.len(),
        VersionedMessage::V0(m) => m.account_keys.len(),
    };

    (raw_len, b64_len, keys)
}

fn fits_tx(payer: Pubkey, ixs: &[Instruction]) -> bool {
    let (raw_len, b64_len, _) = estimate_vtx_sizes(payer, ixs);
    raw_len <= MAX_RAW_TX && b64_len <= MAX_B64_TX
}

fn max_store_chunk_size(
    program_id: Pubkey,
    payer: Pubkey,
    tx_account: Pubkey,
    tx_id: [u8; 32],
    max_try: usize,
) -> usize {
    let mut lo = 0usize;
    let mut hi = max_try;

    while lo < hi {
        let mid = (lo + hi + 1) / 2;

        let store_ix = Instruction {
            program_id,
            accounts: StoreTxBytes { signer: payer, tx_account }.to_account_metas(None),
            data: StoreTxBytesInstruction {
                tx_id,
                bytes: vec![0u8; mid],
            }
            .data(),
        };

        if fits_tx(payer, &[store_ix]) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    lo
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
        None => return Err(RelayTxError::TxIsNotIncludedToBlock),
    };

    let client_clone = bitcoind_client.clone();
    let block_info = tokio::task::spawn_blocking(move || client_clone.get_block_info(&block_hash))
        .await
        .expect("no panic")?;

    let tx_height = block_info.height as u32;
    if tx_height > main_state_data.block_height {
        return Err(RelayTxError::TxIsNotIncludedToBlock);
    }
    if tx_height < main_state_data.start_height {
        return Err(RelayTxError::TxIsNotIncludedToBlockBuffer);
    }

    let commit_hash = main_state_data.get_commitment(tx_height);

    let merkle_root = {
        block_info.merkleroot.to_byte_array()
    };

    let proof_header = TxProofHeader {
        blockheight: tx_height,
        merkle_root,
        commit_hash,
    };

    let tx_pos = block_info
        .tx
        .iter()
        .position(|in_block| *in_block == tx_id)
        .ok_or(RelayTxError::CouldNotFindTxidInBlock)?;

    let reversed_merkle_proof = Proof::create(&block_info.tx, tx_pos).to_reversed_vec();
    let tx_bytes_len = transaction.hex.len();
    let proof_len = reversed_merkle_proof.len();
    let proof_bytes = 32 * proof_len;

    info!(
        "[RELAY_TX INPUT] tx_bytes={} proof_len={} proof_bytes={} sum(tx+proof)={}",
        tx_bytes_len,
        proof_len,
        proof_bytes,
        tx_bytes_len + proof_bytes
    );

    let tx_id = transaction.txid.to_byte_array();
    let (tx_account, _) = Pubkey::find_program_address(&[tx_id.as_slice()], &program.id());

    let cu_ix = ComputeBudgetInstruction::set_compute_unit_limit(500_000);

    let verify_ix = Instruction {
        program_id: program.id(),
        accounts: VerifyTransaction {
            signer: program.payer(),
            main_state,
            tx_account,
            wbtc_receiver_sol,
            wbtc_mint,
            wbtc_receiver,
            associated_token_program: anchor_spl::associated_token::ID,
            token_program: anchor_spl::token::ID,
            system_program: anchor_client::solana_sdk::system_program::ID,
        }
        .to_account_metas(None),
        data: VerifySmallTxInstruction {
            tx_id,
            tx_bytes: transaction.hex.clone(),
            confirmations: 1,
            tx_index: tx_pos as u32,
            reversed_merkle_proof: reversed_merkle_proof.clone(),
            proof_header,
        }
        .data(),
    };

    let (small_raw, small_b64, small_keys) = estimate_vtx_sizes(program.payer(), &[cu_ix.clone(), verify_ix.clone()]);
    info!(
        "[SIZE CHECK] small raw={} (max {}) b64={} (max {}) keys={}",
        small_raw, MAX_RAW_TX, small_b64, MAX_B64_TX, small_keys
    );

    let small_fits = small_raw <= MAX_RAW_TX && small_b64 <= MAX_B64_TX;

    if small_fits {
        // small-tx flow
        let sig = program
            .request()
            .instruction(cu_ix)
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
                reversed_merkle_proof,
                proof_header,
            })
            .send()
            .await?;

        Ok(sig)
    } else {
        // big-tx flow
        let init_ix = Instruction {
            program_id: program.id(),
            accounts: InitBigTxVerify {
                signer: program.payer(),
                tx_account,
                system_program: anchor_client::solana_sdk::system_program::ID,
                main_state,
            }
            .to_account_metas(None),
            data: InitBigTxVerifyInstruction {
                tx_id,
                tx_size: transaction.hex.len() as u64,
                confirmations: 1,
                tx_index: tx_pos as u32,
                reversed_merkle_proof: reversed_merkle_proof.clone(),
                proof_header,
            }
            .data(),
        };

        let init_fits_with_cu = fits_tx(program.payer(), &[cu_ix.clone(), init_ix.clone()]);
        if !init_fits_with_cu {
            let (r, b, k) = estimate_vtx_sizes(program.payer(), &[cu_ix.clone(), init_ix.clone()]);
            return Err(RelayTxError::Anchor(AnchorClientError::from(std::io::Error::other(
                format!("big-init too large even with CU: raw={r} b64={b} keys={k}"),
            ))));
        }

        program
            .request()
            .instruction(cu_ix.clone())
            .accounts(InitBigTxVerify {
                signer: program.payer(),
                tx_account,
                system_program: anchor_client::solana_sdk::system_program::ID,
                main_state,
            })
            .args(InitBigTxVerifyInstruction {
                tx_id,
                tx_size: transaction.hex.len() as u64,
                confirmations: 1,
                tx_index: tx_pos as u32,
                reversed_merkle_proof,
                proof_header,
            })
            .send()
            .await?;

        let chunk_size = max_store_chunk_size(program.id(), program.payer(), tx_account, tx_id,800);
        if chunk_size == 0 {
            return Err(RelayTxError::Anchor(AnchorClientError::from(std::io::Error::new(
                std::io::ErrorKind::Other,
                "could not find any StoreTxBytes chunk size that fits into tx limit",
            ))));
        }
        info!("[BIG FLOW] using chunk_size={}", chunk_size);

        for chunk in transaction.hex.chunks(chunk_size) {
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

        let sig = program
            .request()
            .instruction(cu_ix)
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

        Ok(sig)
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