mod config;
mod merkle;

use std::rc::Rc;
use std::str::FromStr;
use std::time::Duration;
use std::{env, thread};

use anchor_client::anchor_lang::prelude::*;
use anchor_client::solana_sdk::signature::{read_keypair_file, Keypair};
use anchor_client::solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Signature, Signer},
};
use anchor_client::{Client as AnchorClient, Cluster, Program};
use anchor_client::anchor_lang::solana_program::example_mocks::solana_address_lookup_table_program::state;
use anchor_client::solana_sdk::native_token::LAMPORTS_PER_SOL;
use base64::prelude::*;
use bitcoin::hex::DisplayHex;
use bitcoin::{Address, Network, PublicKey, Txid};
use bitcoin::hashes::Hash;
use bitcoincore_rpc::bitcoin::blockdata::block::Block;
use bitcoincore_rpc::bitcoin::hashes::Hash as BitcoinRpcHash;
use bitcoincore_rpc::{Auth, Client as BitcoinRpcClient, RpcApi};
use bitcoincore_rpc::bitcoin::BlockHash;
use log::{debug, info, error, warn};
use solana_transaction_status::UiTransactionEncoding;

use crate::config::{read_config, RelayConfig};
use crate::merkle::Proof;
use btc_relay::accounts::{Deposit, Initialize, SubmitBlockHeaders, VerifyTransaction};
use btc_relay::events::StoreHeader;
use btc_relay::instruction::{
    Deposit as DepositInstruction, Initialize as InitializeInstruction,
    SubmitBlockHeaders as SubmitBlockHeadersInstruction, VerifySmallTx as VerifySmallTxInstruction,
};
use btc_relay::program::BtcRelay;
use btc_relay::state::MainState;
use btc_relay::structs::{BlockHeader, CommittedBlockHeader};
use btc_relay::utils::{bridge_deposit_script, BITCOIN_DEPOSIT_PUBKEY};

const START_SUBMIT_FROM_TX: &str =
    "HGmcAboCdFVvPqqebE8KRR288bmooHEed9KMtkEe2cy4fKuySHqQ5nz2LAkwWVH65miUJ7HdvgRFvaADGmW3fdZ";

const SOLANA_DEPOSIT_PUBKEY: &str = "5Xy6zEA64yENXm9Zz5xDmTdB8t9cQpNaD3ZwNLBeiSc5";

fn relay_blocks_from_full_node(config: RelayConfig) {
    let bitcoin_pubkey = PublicKey::from_str(BITCOIN_DEPOSIT_PUBKEY).unwrap();

    let solana_address = Pubkey::from_str(SOLANA_DEPOSIT_PUBKEY).unwrap();

    let script = bridge_deposit_script(
        solana_address.to_bytes(),
        bitcoin_pubkey.pubkey_hash().to_byte_array(),
    );

    info!("{script:?}");

    let deposit_address = Address::p2wsh(script.as_script(), Network::Regtest);
    println!("Deposit address {deposit_address}");

    let yona_client = get_yona_client(&config);

    let bitcoind_client = BitcoinRpcClient::new(
        &config.bitcoind_url,
        Auth::CookieFile(config.bitcoin_cookie_file.into()),
    )
    .unwrap();

    let tip = bitcoind_client.get_chain_tips().unwrap().remove(0);
    debug!("Current bitcoin tip {tip:?}");

    let last_block = bitcoind_client.get_block(&tip.hash).unwrap();
    debug!("Bitcoin last block {last_block:?}");

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program).unwrap();

    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);

    if env::var("INIT_PROGRAM").is_ok() {
        init_program(&program, main_state, last_block, tip.height as u32);
    }

    let mut last_submit_tx = Signature::from_str(START_SUBMIT_FROM_TX).unwrap();
    if env::var("RELAY_BLOCKS").is_ok() {
        loop {
            let stored_header = match program
                .rpc()
                .get_transaction(&last_submit_tx, UiTransactionEncoding::Binary)
            {
                Ok(tx) => {
                    let messages: Option<Vec<String>> =
                        tx.transaction.meta.unwrap().log_messages.into();
                    let parsed_base64 = BASE64_STANDARD
                        .decode(messages.unwrap()[2].strip_prefix("Program data: ").unwrap())
                        .unwrap();
                    StoreHeader::try_from_slice(&parsed_base64[8..]).unwrap()
                }
                Err(e) => {
                    error!("Got error {e} on get_transaction({last_submit_tx})");
                    let raw_account = program.rpc().get_account(&main_state).unwrap();
                    info!("Data len {}", raw_account.data.len());
                    info!("Main state space {}", MainState::space());
                    info!("Main state size {}", std::mem::size_of::<MainState>());

                    let main_state_data =
                        MainState::try_deserialize_unchecked(&mut &raw_account.data[..8128])
                            .unwrap();

                    let mut block_hash = main_state_data.tip_block_hash;
                    let commited_header = reconstruct_commited_header(
                        &bitcoind_client,
                        &BlockHash::from_byte_array(block_hash),
                        main_state_data.block_height,
                        main_state_data.last_diff_adjustment,
                    );
                    block_hash.reverse();
                    info!(
                        "Last stored block hash {} and height {}",
                        block_hash.to_lower_hex_string(),
                        main_state_data.block_height
                    );

                    StoreHeader {
                        block_hash,
                        commit_hash: main_state_data.tip_commit_hash,
                        header: commited_header,
                    }
                }
            };

            let last_submitted_height = stored_header.header.blockheight;

            let best_block_hash = bitcoind_client.get_best_block_hash().unwrap();
            let best_block_height = bitcoind_client
                .get_block_info(&best_block_hash)
                .unwrap()
                .height as u32;
            if last_submitted_height >= best_block_height {
                info!("Latest BTC block {best_block_height} is already submitted to Yona. Waiting for a new one.");
                thread::sleep(Duration::from_secs(30));
                continue;
            }

            let new_height = last_submitted_height + 1;

            let block_hash_to_submit = bitcoind_client.get_block_hash(new_height as u64).unwrap();
            let block_to_submit = bitcoind_client.get_block(&block_hash_to_submit).unwrap();

            last_submit_tx = submit_block(
                &program,
                main_state,
                block_to_submit,
                new_height,
                stored_header.header,
            );
        }
    }

    if env::var("INIT_DEPOSIT").is_ok() {
        init_deposit(&program, 100 * LAMPORTS_PER_SOL);
    }

    if env::var("RELAY_TX").is_ok() {
        let tx_id = env::var("RELAY_TX").unwrap();
        let raw_account = program.rpc().get_account(&main_state).unwrap();

        let main_state_data =
            MainState::try_deserialize_unchecked(&mut &raw_account.data[..8128]).unwrap();

        let tx_id = Txid::from_str(&tx_id).unwrap();
        relay_tx(
            &program,
            main_state,
            &bitcoind_client,
            tx_id,
            main_state_data.last_diff_adjustment,
        );
    }
}

fn init_deposit(program: &Program<Rc<Keypair>>, amount: u64) {
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

fn init_program(
    program: &Program<Rc<Keypair>>,
    main_state: Pubkey,
    block: Block,
    block_height: u32,
) {
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
}

fn get_yona_client(config: &RelayConfig) -> AnchorClient<Rc<Keypair>> {
    let mut keypair_path = env::home_dir().unwrap();
    keypair_path.push(&config.yona_keipair);
    // Set up sender and recipient keypairs
    let sender = read_keypair_file(keypair_path).unwrap();

    let signer = Rc::new(sender);
    let cluster = Cluster::Custom(config.yona_http.clone(), config.yona_ws.clone());
    AnchorClient::new_with_options(cluster, signer, CommitmentConfig::confirmed())
}

fn submit_block(
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

fn reconstruct_commited_header(
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

fn relay_tx(
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

fn main() {
    env_logger::init();
    let config = read_config().unwrap();
    relay_blocks_from_full_node(config);
}
