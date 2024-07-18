use crate::merkle::Proof;
use anchor_client::anchor_lang::prelude::AccountMeta;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{Keypair, Signature};
use anchor_client::Program;
use bitcoin::hashes::Hash;
use bitcoin::hex::DisplayHex;
use bitcoin::{Block, BlockHash, Txid};
use bitcoincore_rpc::{Auth, Client as BitcoinRpcClient, RpcApi};
use btc_relay::accounts::{Deposit, Initialize, SubmitBlockHeaders, VerifyTransaction};
use btc_relay::instruction::{
    Deposit as DepositInstruction, Initialize as InitializeInstruction,
    SubmitBlockHeaders as SubmitBlockHeadersInstruction, VerifySmallTx as VerifySmallTxInstruction,
};
use btc_relay::structs::{BlockHeader, CommittedBlockHeader};
use log::{debug, info, warn};
use std::rc::Rc;

pub(crate) fn reconstruct_commited_header(
    bitcoind_client: &BitcoinRpcClient,
    hash: &BlockHash,
    height: u32,
    last_diff_adjustment: u32,
) -> CommittedBlockHeader {
    let header = bitcoind_client.get_block_header(hash).unwrap();
    debug!("Got header {header:?}");

    let mut prev_block_timestamps = [0; 10];
    for i in 0..10 {
        let prev_block_hash = bitcoind_client
            .get_block_hash(height as u64 - i as u64 - 1)
            .unwrap();
        let block = bitcoind_client.get_block(&prev_block_hash).unwrap();
        prev_block_timestamps[9 - i] = block.header.time;
    }

    CommittedBlockHeader {
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
    }
}

pub(crate) fn init_deposit(program: &Program<Rc<Keypair>>, amount: u64) {
    let (deposit_account, _) = Pubkey::find_program_address(&[b"solana_deposit"], &program.id());

    let res = program
        .request()
        .accounts(Deposit {
            user: program.payer(),
            deposit_account,
            system_program: anchor_client::solana_sdk::system_program::ID,
        })
        .args(DepositInstruction { amount })
        .send()
        .unwrap();

    info!("Deposit tx sig {res}");
}

#[derive(Debug)]
pub enum InitError {}

pub fn init_program(
    program: &Program<Rc<Keypair>>,
    block: Block,
    block_height: u32,
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

    let block_hash = yona_block_header.get_block_hash().unwrap();

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
            prev_block_timestamps: [yona_block_header.timestamp; 10],
        })
        .send()
        .unwrap();

    info!(
        "Submitted block {}, tx sig {res}",
        block_hash.to_lower_hex_string()
    );

    Ok(res)
}

pub(crate) fn relay_tx(
    program: &Program<Rc<Keypair>>,
    main_state: Pubkey,
    btc_client: &BitcoinRpcClient,
    tx_id: Txid,
    last_diff_adjustment: u32,
) {
    let transaction = btc_client.get_raw_transaction_info(&tx_id, None).unwrap();
    let hash = match transaction.blockhash {
        Some(hash) => hash,
        _ => {
            warn!("Transaction {tx_id} is not included to block yet");
            return;
        }
    };

    let block_info = btc_client.get_block_info(&hash).unwrap();

    let commited_header = reconstruct_commited_header(
        &btc_client,
        &hash,
        block_info.height as u32,
        last_diff_adjustment,
    );
    let tx_pos = block_info
        .tx
        .iter()
        .position(|in_block| *in_block == tx_id)
        .unwrap();
    let proof = Proof::create(&block_info.tx, tx_pos);

    let (deposit_account, _) = Pubkey::find_program_address(&[b"solana_deposit"], &program.id());

    let relay_yona_tx = program
        .request()
        .accounts(VerifyTransaction {
            signer: program.payer(),
            main_state,
            deposit_account,
            // Pubkey::from_str("CgxQmREYVuwyPzHcH19iBQDtPjcHEWuzfRgWrtzepHLs").unwrap()
            mint_receiver: program.payer(),
        })
        .args(VerifySmallTxInstruction {
            tx_bytes: transaction.hex,
            confirmations: 1,
            tx_index: tx_pos as u32,
            commited_header,
            reversed_merkle_proof: proof.to_reversed_vec(),
        })
        .send()
        .unwrap();

    info!("Relayed bitcoin tx {} to Yona: {relay_yona_tx}", tx_id);
}

pub(crate) fn submit_block(
    program: &Program<Rc<Keypair>>,
    main_state: Pubkey,
    block: Block,
    height: u32,
    commited_header: CommittedBlockHeader,
) -> Signature {
    let yona_block_header = BlockHeader {
        version: block.header.version.to_consensus() as u32,
        reversed_prev_blockhash: block.header.prev_blockhash.to_byte_array(),
        merkle_root: block.header.merkle_root.to_byte_array(),
        timestamp: block.header.time,
        nbits: block.header.bits.to_consensus(),
        nonce: block.header.nonce,
    };

    let mut block_hash = yona_block_header.get_block_hash().unwrap();
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
        .unwrap();

    block_hash.reverse();
    info!(
        "Submitted block header. Hash {}, height {height}, Yona tx {res}",
        block_hash.to_lower_hex_string()
    );

    res
}
