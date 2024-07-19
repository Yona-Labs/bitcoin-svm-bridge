pub mod config;
mod merkle;
mod relay_program_interaction;

use crate::config::RelayConfig;
use crate::merkle::Proof;
use crate::relay_program_interaction::{
    init_deposit, init_program, reconstruct_commited_header, submit_block, InitError,
};
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use anchor_client::anchor_lang::{AccountDeserialize, AnchorDeserialize, Id};
use anchor_client::solana_sdk::commitment_config::CommitmentConfig;
use anchor_client::solana_sdk::native_token::LAMPORTS_PER_SOL;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{read_keypair_file, Keypair, Signature};
use anchor_client::{Client as AnchorClient, Cluster, Program};
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use bitcoin::hashes::Hash;
use bitcoin::hex::DisplayHex;
use bitcoin::{Address, BlockHash, Network, PublicKey, Txid};
use bitcoincore_rpc::{Client as BitcoinRpcClient, RpcApi};
use btc_relay::accounts::VerifyTransaction;
use btc_relay::events::StoreHeader;
use btc_relay::instruction::VerifySmallTx as VerifySmallTxInstruction;
use btc_relay::program::BtcRelay;
use btc_relay::state::MainState;
use btc_relay::utils::{bridge_deposit_script, BITCOIN_DEPOSIT_PUBKEY};
use log::{debug, error, info};
use serde::Deserialize;
use solana_transaction_status::UiTransactionEncoding;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::{env, thread};
use tokio::task::spawn_blocking;

const START_SUBMIT_FROM_TX: &str =
    "HGmcAboCdFVvPqqebE8KRR288bmooHEed9KMtkEe2cy4fKuySHqQ5nz2LAkwWVH65miUJ7HdvgRFvaADGmW3fdZ";

fn get_yona_client(config: &RelayConfig) -> AnchorClient<Arc<Keypair>> {
    let mut keypair_path = env::home_dir().unwrap();
    keypair_path.push(&config.yona_keipair);
    // Set up sender and recipient keypairs
    let sender = read_keypair_file(keypair_path).unwrap();

    let signer = Arc::new(sender);
    let cluster = Cluster::Custom(config.yona_http.clone(), config.yona_ws.clone());
    AnchorClient::new_with_options(cluster, signer, CommitmentConfig::confirmed())
}

pub fn relay_blocks_from_full_node(config: RelayConfig) {
    let yona_client = get_yona_client(&config);

    let bitcoind_client =
        BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into()).unwrap();

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program).unwrap();

    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);

    let mut last_submit_tx = Signature::from_str(START_SUBMIT_FROM_TX).unwrap();
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
                    MainState::try_deserialize_unchecked(&mut &raw_account.data[..8128]).unwrap();

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

    /*
    if env::var("INIT_DEPOSIT").is_ok() {
        init_deposit(&program, 100 * LAMPORTS_PER_SOL);
    }

     */
}

/// Initializes BTC relay program using the current Bitcoin tip (latest block)
pub fn run_init_program(config: RelayConfig) -> Result<Signature, InitError> {
    let yona_client = get_yona_client(&config);

    let bitcoind_client =
        BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into()).unwrap();

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program).unwrap();

    let tip = bitcoind_client.get_chain_tips().unwrap().remove(0);
    debug!("Current bitcoin tip {tip:?}");

    let last_block = bitcoind_client.get_block(&tip.hash).unwrap();
    debug!("Bitcoin last block {last_block:?}");

    init_program(&program, last_block, tip.height as u32)
}

struct RelayTransactionsState {
    relay_program: Program<Arc<Keypair>>,
    bitcoin_rpc_client: BitcoinRpcClient,
    main_state: Pubkey,
}

#[derive(Deserialize)]
struct RelayTxRequest {
    tx_id: String,
    yona_address: String,
}

async fn relay_tx(
    data: web::Data<RelayTransactionsState>,
    req: web::Json<RelayTxRequest>,
) -> impl Responder {
    let tx_id = match Txid::from_str(&req.tx_id) {
        Ok(tx_id) => tx_id,
        Err(_) => return HttpResponse::BadRequest().body("tx_id is not valid"),
    };
    let mint_receiver = match Pubkey::from_str(&req.yona_address) {
        Ok(pubkey) => pubkey,
        Err(_) => return HttpResponse::BadRequest().body("yona_address is not valid"),
    };

    let raw_account = spawn_blocking({
        let data = data.clone();
        move || data.relay_program.rpc().get_account(&data.main_state)
    })
    .await
    .unwrap()
    .unwrap();

    let main_state_data =
        MainState::try_deserialize_unchecked(&mut &raw_account.data[..8128]).unwrap();

    let transaction = match spawn_blocking({
        let data = data.clone();
        move || {
            data.bitcoin_rpc_client
                .get_raw_transaction_info(&tx_id, None)
        }
    })
    .await
    .unwrap()
    {
        Ok(tx) => tx,
        Err(e) => {
            error!("Could not get tx {} info {e}", req.tx_id);
            return HttpResponse::InternalServerError().body("Could not get tx info");
        }
    };

    let hash = match transaction.blockhash {
        Some(hash) => hash,
        _ => {
            return HttpResponse::BadRequest()
                .body(format!("Transaction {tx_id} is not included to block yet"));
        }
    };

    let block_info = spawn_blocking({
        let data = data.clone();
        move || data.bitcoin_rpc_client.get_block_info(&hash)
    })
    .await
    .unwrap()
    .unwrap();

    let commited_header = spawn_blocking({
        let data = data.clone();
        move || {
            reconstruct_commited_header(
                &data.bitcoin_rpc_client,
                &hash,
                block_info.height as u32,
                main_state_data.last_diff_adjustment,
            )
        }
    })
    .await
    .unwrap();

    let tx_pos = block_info
        .tx
        .iter()
        .position(|in_block| *in_block == tx_id)
        .unwrap();
    let proof = Proof::create(&block_info.tx, tx_pos);

    let (deposit_account, _) =
        Pubkey::find_program_address(&[b"solana_deposit"], &data.relay_program.id());

    let relay_yona_tx_res = spawn_blocking({
        let data = data.clone();
        move || {
            data.relay_program
                .request()
                .accounts(VerifyTransaction {
                    signer: data.relay_program.payer(),
                    main_state: data.main_state,
                    deposit_account,
                    // Pubkey::from_str("CgxQmREYVuwyPzHcH19iBQDtPjcHEWuzfRgWrtzepHLs").unwrap()
                    mint_receiver,
                })
                .args(VerifySmallTxInstruction {
                    tx_bytes: transaction.hex,
                    confirmations: 1,
                    tx_index: tx_pos as u32,
                    commited_header,
                    reversed_merkle_proof: proof.to_reversed_vec(),
                })
                .send()
        }
    })
    .await
    .unwrap();

    let relay_yona_tx = match relay_yona_tx_res {
        Ok(sig) => sig,
        Err(e) => {
            error!("Transaction {} relay failed {e:?}", req.tx_id);
            return HttpResponse::InternalServerError().json("Failed to relay tx to Yona program");
        }
    };

    HttpResponse::Ok().json(format!(
        "Relayed bitcoin tx {} to Yona: {relay_yona_tx}",
        tx_id
    ))
}
#[derive(Deserialize)]
struct GetDepositAddrReq {
    yona_address: String,
}

async fn get_deposit_address(req: web::Query<GetDepositAddrReq>) -> impl Responder {
    let yona_address = match Pubkey::from_str(&req.yona_address) {
        Ok(pubkey) => pubkey,
        Err(_) => return HttpResponse::BadRequest().body("yona_address is not valid"),
    };

    let bitcoin_pubkey = PublicKey::from_str(BITCOIN_DEPOSIT_PUBKEY).unwrap();

    let script = bridge_deposit_script(
        yona_address.to_bytes(),
        bitcoin_pubkey.pubkey_hash().to_byte_array(),
    );

    info!("{script:?}");

    let deposit_address = Address::p2wsh(script.as_script(), Network::Regtest);

    HttpResponse::Ok().body(deposit_address.to_string())
}

pub async fn relay_transactions(config: RelayConfig) {
    let yona_client = get_yona_client(&config);

    let bitcoin_rpc_client =
        BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into()).unwrap();

    let relay_program = BtcRelay::id();
    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);
    let relay_program = yona_client.program(relay_program).unwrap();

    let app_state = web::Data::new(RelayTransactionsState {
        relay_program,
        bitcoin_rpc_client,
        main_state,
    });

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .route("/relay_tx", web::post().to(relay_tx))
            .route("/get_deposit_address", web::get().to(get_deposit_address))
    })
    .bind("0.0.0.0:8199")
    .expect("bind to 0.0.0.0:8199")
    .run()
    .await
    .unwrap();
}
