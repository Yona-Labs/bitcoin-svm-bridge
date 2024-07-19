use anchor_client::anchor_lang::prelude::AccountMeta;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{Keypair, Signature};
use anchor_client::Program;
use bitcoin::hashes::Hash;
use bitcoin::hex::DisplayHex;
use bitcoin::{Block, BlockHash};
use bitcoincore_rpc::{Client as BitcoinRpcClient, RpcApi};
use btc_relay::accounts::{Deposit, Initialize, SubmitBlockHeaders};
use btc_relay::instruction::{
    Deposit as DepositInstruction, Initialize as InitializeInstruction,
    SubmitBlockHeaders as SubmitBlockHeadersInstruction,
};
use btc_relay::structs::{BlockHeader, CommittedBlockHeader};
use log::{debug, info};
use std::sync::Arc;

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

pub(crate) fn init_deposit(program: &Program<Arc<Keypair>>, amount: u64) {
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
    program: &Program<Arc<Keypair>>,
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

pub(crate) fn submit_block(
    program: &Program<Arc<Keypair>>,
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
