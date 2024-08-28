use crate::merkle::Proof;
use anchor_client::anchor_lang::prelude::{AccountDeserialize, AccountMeta};
use anchor_client::solana_sdk::compute_budget::ComputeBudgetInstruction;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{Keypair, Signature};
use anchor_client::ClientError as AnchorClientError;
use anchor_client::Program;
use bitcoin::hashes::Hash;
use bitcoin::hex::DisplayHex;
use bitcoin::{Block, BlockHash, Txid};
use bitcoincore_rpc::{Client as BitcoinRpcClient, Error as BtcRpcError, RpcApi};
use btc_relay::accounts::{
    BridgeWithdraw, Deposit, FinalizeTx, InitBigTxVerify, Initialize, StoreTxBytes,
    SubmitBlockHeaders, VerifyTransaction,
};
use btc_relay::instruction::{
    BridgeWithdraw as BridgeWithdrawInstruction, Deposit as DepositInstruction,
    FinalizeTxProcessing, InitBigTxVerify as InitBigTxVerifyInstruction,
    Initialize as InitializeInstruction, StoreTxBytes as StoreTxBytesInstruction,
    SubmitBlockHeaders as SubmitBlockHeadersInstruction, VerifySmallTx as VerifySmallTxInstruction,
};
use btc_relay::state::{DepositTxState as ProgramDepositTxState, MainState, TxState};
use btc_relay::structs::{BlockHeader, CommittedBlockHeader};
use log::{debug, info};
use std::fmt;
use std::sync::Arc;

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

pub(crate) fn init_deposit(
    program: &Program<Arc<Keypair>>,
    amount: u64,
) -> Result<Signature, AnchorClientError> {
    let (deposit_account, _) = Pubkey::find_program_address(&[b"solana_deposit"], &program.id());

    program
        .request()
        .accounts(Deposit {
            signer: program.payer(),
            deposit_account,
            system_program: anchor_client::solana_sdk::system_program::ID,
        })
        .args(DepositInstruction { amount })
        .send()
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

pub fn init_program(
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
        let prev_block_hash = bitcoind_client.get_block_hash(block_height as u64 - i as u64 - 1)?;
        let block = bitcoind_client.get_block(&prev_block_hash)?;
        prev_block_timestamps[9 - i] = block.header.time;
    }

    let (header_topic, _) =
        Pubkey::find_program_address(&[b"header", block_hash.as_slice()], &program.id());

    let res = program
        .request()
        .accounts(Initialize {
            signer: program.payer(),
            main_state,
            header_topic,
            system_program: anchor_client::solana_sdk::system_program::ID,
        })
        .args(InitializeInstruction {
            data: yona_block_header,
            block_height,
            chain_work: [0; 32],
            last_diff_adjustment: yona_block_header.timestamp,
            prev_block_timestamps,
            deposit_pubkey_hash,
        })
        .send()?;

    info!(
        "Submitted block {}, tx sig {res}",
        block_hash.to_lower_hex_string()
    );

    Ok(res)
}

pub(crate) fn submit_block(
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
        .send()?;

    block_hash.reverse();
    info!(
        "Submitted block header. Hash {}, height {height}, Yona tx {res}",
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

pub fn relay_tx(
    program: &Program<Arc<Keypair>>,
    main_state: Pubkey,
    bitcoind_client: &BitcoinRpcClient,
    tx_id: Txid,
    mint_receiver: Pubkey,
) -> Result<Signature, RelayTxError> {
    let raw_account = program
        .rpc()
        .get_account(&main_state)
        .map_err(AnchorClientError::from)?;
    let main_state_data = MainState::try_deserialize(&mut &raw_account.data[..8160])
        .map_err(AnchorClientError::from)?;

    let transaction = bitcoind_client.get_raw_transaction_info(&tx_id, None)?;

    let block_hash = match transaction.blockhash {
        Some(hash) => hash,
        _ => return Err(RelayTxError::TxIsNotIncludedToBlock),
    };

    let block_info = bitcoind_client.get_block_info(&block_hash)?;

    let commited_header = reconstruct_commited_header(
        &bitcoind_client,
        &block_hash,
        block_info.height as u32,
        main_state_data.last_diff_adjustment,
    )?;

    let tx_pos = block_info
        .tx
        .iter()
        .position(|in_block| *in_block == tx_id)
        .ok_or(RelayTxError::CouldNotFindTxidInBlock)?;
    let reversed_merkle_proof = Proof::create(&block_info.tx, tx_pos).to_reversed_vec();

    let (deposit_account, _) = Pubkey::find_program_address(&[b"solana_deposit"], &program.id());
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
            .send()?;

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
                .send()?;
        }

        let res = program
            .request()
            .instruction(ComputeBudgetInstruction::set_compute_unit_limit(500_000))
            .accounts(FinalizeTx {
                signer: program.payer(),
                tx_account,
                deposit_account,
                mint_receiver,
                main_state,
            })
            .args(FinalizeTxProcessing { tx_id })
            .send()?;

        Ok(res)
    } else {
        let res = program
            .request()
            .accounts(VerifyTransaction {
                signer: program.payer(),
                main_state,
                deposit_account,
                tx_account,
                mint_receiver,
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
            .send()?;

        Ok(res)
    }
}

pub fn bridge_withdraw(
    program: &Program<Arc<Keypair>>,
    amount: u64,
    bitcoin_address: String,
) -> Result<Signature, AnchorClientError> {
    let (deposit_account, _) = Pubkey::find_program_address(&[b"solana_deposit"], &program.id());

    let res = program
        .request()
        .accounts(BridgeWithdraw {
            signer: program.payer(),
            deposit_account,
            system_program: anchor_client::solana_sdk::system_program::ID,
        })
        .args(BridgeWithdrawInstruction {
            amount,
            bitcoin_address,
        })
        .send()?;

    Ok(res)
}

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

pub fn deposit_tx_state(
    program: &Program<Arc<Keypair>>,
    tx_id: Txid,
) -> Result<DepositTxState, AnchorClientError> {
    let (tx_account, _) =
        Pubkey::find_program_address(&[tx_id.to_byte_array().as_slice()], &program.id());

    match program.account::<ProgramDepositTxState>(tx_account) {
        Ok(state) => match state.state {
            TxState::VerificationInitialized => Ok(DepositTxState::NotRelayed),
            TxState::VerificationComplete => Ok(DepositTxState::Relayed),
        },
        Err(AnchorClientError::AccountNotFound) => Ok(DepositTxState::NotRelayed),
        Err(e) => Err(e),
    }
}
