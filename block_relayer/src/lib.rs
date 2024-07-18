pub mod config;
mod merkle;
mod relay_program_interaction;

use crate::config::RelayConfig;
use crate::relay_program_interaction::{
    init_deposit, init_program, reconstruct_commited_header, relay_tx, submit_block,
};
use anchor_client::anchor_lang::{AccountDeserialize, AnchorDeserialize, Id};
use anchor_client::solana_sdk::commitment_config::CommitmentConfig;
use anchor_client::solana_sdk::native_token::LAMPORTS_PER_SOL;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{read_keypair_file, Keypair, Signature};
use anchor_client::{Client as AnchorClient, Cluster};
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use bitcoin::hashes::Hash;
use bitcoin::hex::DisplayHex;
use bitcoin::{Address, BlockHash, Network, PublicKey, Txid};
use bitcoincore_rpc::{Client as BitcoinRpcClient, RpcApi};
use btc_relay::events::StoreHeader;
use btc_relay::program::BtcRelay;
use btc_relay::state::MainState;
use btc_relay::utils::{bridge_deposit_script, BITCOIN_DEPOSIT_PUBKEY};
use log::{debug, error, info};
use solana_transaction_status::UiTransactionEncoding;
use std::rc::Rc;
use std::str::FromStr;
use std::time::Duration;
use std::{env, thread};

const START_SUBMIT_FROM_TX: &str =
    "HGmcAboCdFVvPqqebE8KRR288bmooHEed9KMtkEe2cy4fKuySHqQ5nz2LAkwWVH65miUJ7HdvgRFvaADGmW3fdZ";

const SOLANA_DEPOSIT_PUBKEY: &str = "5Xy6zEA64yENXm9Zz5xDmTdB8t9cQpNaD3ZwNLBeiSc5";

fn get_yona_client(config: &RelayConfig) -> AnchorClient<Rc<Keypair>> {
    let mut keypair_path = env::home_dir().unwrap();
    keypair_path.push(&config.yona_keipair);
    // Set up sender and recipient keypairs
    let sender = read_keypair_file(keypair_path).unwrap();

    let signer = Rc::new(sender);
    let cluster = Cluster::Custom(config.yona_http.clone(), config.yona_ws.clone());
    AnchorClient::new_with_options(cluster, signer, CommitmentConfig::confirmed())
}

pub fn relay_blocks_from_full_node(config: RelayConfig) {
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

    let bitcoind_client =
        BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into()).unwrap();

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
