pub mod config;
mod merkle;
pub mod relay_program_interaction;

use crate::config::RelayConfig;
use crate::relay_program_interaction::*;
use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use anchor_client::anchor_lang::{AccountDeserialize, Id};
use anchor_client::solana_sdk::commitment_config::CommitmentConfig;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{read_keypair_file, Keypair, Signature};
use anchor_client::{Client as AnchorClient, ClientError as AnchorClientError, Cluster, Program};
use bitcoin::hashes::Hash;
use bitcoin::hex::DisplayHex;
use bitcoin::{Address, BlockHash, Network, PublicKey, Txid};
use bitcoincore_rpc::{Client as BitcoinRpcClient, Error as BtcError, RpcApi};
use btc_relay::events::StoreHeader;
use btc_relay::program::BtcRelay;
use btc_relay::state::MainState;
use btc_relay::utils::{bridge_deposit_script, BITCOIN_DEPOSIT_PUBKEY};
use log::{debug, error, info};
use serde::Deserialize;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::{env, error, thread};
use tokio::task::spawn_blocking;

pub fn get_yona_client(
    config: &RelayConfig,
) -> Result<AnchorClient<Arc<Keypair>>, Box<dyn error::Error>> {
    let mut keypair_path = env::home_dir().expect("to get the home dir");
    keypair_path.push(&config.yona_keipair);
    // Set up sender and recipient keypairs
    let sender = read_keypair_file(keypair_path)?;

    let signer = Arc::new(sender);
    let cluster = Cluster::Custom(config.yona_http.clone(), config.yona_ws.clone());
    Ok(AnchorClient::new_with_options(
        cluster,
        signer,
        CommitmentConfig::confirmed(),
    ))
}

pub fn relay_blocks_from_full_node(config: RelayConfig, wait_for_new_block: u64) {
    let yona_client = get_yona_client(&config).expect("Couldn't create Yona client");

    let bitcoind_client = BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into())
        .expect("Couldn't create Bitcoin client");

    let relay_program = BtcRelay::id();
    let program = yona_client
        .program(relay_program)
        .expect("Couldn't create relay program instance");

    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);

    loop {
        let raw_account = match program.rpc().get_account(&main_state) {
            Ok(acc) => acc,
            Err(e) => {
                error!("Error {e} on get_account(main_state)");
                thread::sleep(Duration::from_secs(10));
                continue;
            }
        };

        // TODO there seems to be an allocation of 8 unneeded bytes, which makes deserialization fail
        let main_state_data =
            match MainState::try_deserialize_unchecked(&mut &raw_account.data[..8128]) {
                Ok(data) => data,
                Err(e) => {
                    error!("Error {e} on main_state deserialization attempt");
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            };

        let mut block_hash = main_state_data.tip_block_hash;
        let commited_header = match reconstruct_commited_header(
            &bitcoind_client,
            &BlockHash::from_byte_array(block_hash),
            main_state_data.block_height,
            main_state_data.last_diff_adjustment,
        ) {
            Ok(header) => header,
            Err(e) => {
                error!("Error {e} on reconstruct_commited_header");
                thread::sleep(Duration::from_secs(10));
                continue;
            }
        };
        block_hash.reverse();

        info!(
            "Last stored block hash {} and height {}",
            block_hash.to_lower_hex_string(),
            main_state_data.block_height
        );

        let stored_header = StoreHeader {
            block_hash,
            commit_hash: main_state_data.tip_commit_hash,
            header: commited_header,
        };

        let last_submitted_height = stored_header.header.blockheight;

        let best_block_hash = match bitcoind_client.get_best_block_hash() {
            Ok(hash) => hash,
            Err(e) => {
                error!("Error {e} on Bitcoin's get_best_block_hash");
                thread::sleep(Duration::from_secs(10));
                continue;
            }
        };

        let best_block_height = match bitcoind_client.get_block_info(&best_block_hash) {
            Ok(info) => info.height as u32,
            Err(e) => {
                error!("Error {e} on Bitcoin's get_block_info({best_block_hash:02x})");
                thread::sleep(Duration::from_secs(10));
                continue;
            }
        };

        if last_submitted_height >= best_block_height {
            info!("Latest BTC block {best_block_height} is already submitted to Yona. Waiting for a new one.");
            thread::sleep(Duration::from_secs(wait_for_new_block));
            continue;
        }

        let new_height = last_submitted_height + 1;

        let block_hash_to_submit = match bitcoind_client.get_block_hash(new_height as u64) {
            Ok(hash) => hash,
            Err(e) => {
                error!("Error {e} on Bitcoin's get_block_hash({new_height})");
                thread::sleep(Duration::from_secs(10));
                continue;
            }
        };

        let block_to_submit = match bitcoind_client.get_block(&block_hash_to_submit) {
            Ok(block) => block,
            Err(e) => {
                error!("Error {e} on Bitcoin's get_block({block_hash_to_submit:02x})");
                thread::sleep(Duration::from_secs(10));
                continue;
            }
        };

        if let Err(e) = submit_block(
            &program,
            main_state,
            block_to_submit,
            new_height,
            stored_header.header,
        ) {
            error!("Error {e} on block submit attempt");
            thread::sleep(Duration::from_secs(10));
            continue;
        }
    }
}

#[derive(Debug)]
pub enum InitProgramError {
    Anchor(AnchorClientError),
    Bitcoin(BtcError),
    CouldNotInitYonaClient(Box<dyn error::Error>),
}

impl From<AnchorClientError> for InitProgramError {
    fn from(error: AnchorClientError) -> Self {
        InitProgramError::Anchor(error)
    }
}

impl From<BtcError> for InitProgramError {
    fn from(error: BtcError) -> Self {
        InitProgramError::Bitcoin(error)
    }
}

impl From<InitError> for InitProgramError {
    fn from(err: InitError) -> Self {
        match err {
            InitError::Anchor(e) => InitProgramError::Anchor(e),
            InitError::Bitcoin(e) => InitProgramError::Bitcoin(e),
        }
    }
}

/// Initializes BTC relay program using the current Bitcoin tip (latest block)
pub fn run_init_program(config: RelayConfig) -> Result<Signature, InitProgramError> {
    let yona_client = get_yona_client(&config).map_err(InitProgramError::CouldNotInitYonaClient)?;

    let bitcoind_client = BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into())?;

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program)?;

    let tip = bitcoind_client.get_chain_tips()?.remove(0);
    debug!("Current bitcoin tip {tip:?}");

    let last_block = bitcoind_client.get_block(&tip.hash)?;
    debug!("Bitcoin last block {last_block:?}");

    Ok(init_program(
        &program,
        &bitcoind_client,
        last_block,
        tip.height as u32,
    )?)
}

#[derive(Debug)]
pub enum DepositError {
    Anchor(AnchorClientError),
    CouldNotInitYonaClient(Box<dyn error::Error>),
}

impl From<AnchorClientError> for DepositError {
    fn from(error: AnchorClientError) -> Self {
        DepositError::Anchor(error)
    }
}

pub fn run_deposit(config: RelayConfig, amount: u64) -> Result<Signature, DepositError> {
    let yona_client = get_yona_client(&config).map_err(DepositError::CouldNotInitYonaClient)?;

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program)?;

    Ok(init_deposit(&program, amount)?)
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

async fn relay_tx_web_api(
    data: web::Data<RelayTransactionsState>,
    req: web::Json<RelayTxRequest>,
) -> impl Responder {
    let tx_id = match Txid::from_str(&req.tx_id) {
        Ok(tx_id) => tx_id,
        Err(_) => return HttpResponse::BadRequest().json("tx_id is not valid"),
    };
    let mint_receiver = match Pubkey::from_str(&req.yona_address) {
        Ok(pubkey) => pubkey,
        Err(_) => return HttpResponse::BadRequest().json("yona_address is not valid"),
    };

    let relay_tx_res = spawn_blocking(move || {
        relay_tx(
            &data.relay_program,
            data.main_state,
            &data.bitcoin_rpc_client,
            tx_id,
            mint_receiver,
        )
    })
    .await
    .expect("relay_tx to not panic");

    match relay_tx_res {
        Ok(sig) => HttpResponse::Ok().json(format!("{sig}")),
        Err(e) => {
            error!("{e:?}");
            HttpResponse::InternalServerError().json("Failed to relay bitcoin tx")
        }
    }
}

#[derive(Deserialize)]
struct TxStateRequest {
    tx_id: String,
}

async fn get_state_tx_web_api(
    data: web::Data<RelayTransactionsState>,
    req: web::Query<TxStateRequest>,
) -> impl Responder {
    let tx_id = match Txid::from_str(&req.tx_id) {
        Ok(tx_id) => tx_id,
        Err(_) => return HttpResponse::BadRequest().json("tx_id is not valid"),
    };

    let deposit_tx_state_res = spawn_blocking(move || deposit_tx_state(&data.relay_program, tx_id))
        .await
        .expect("deposit_tx_state to not panic");

    match deposit_tx_state_res {
        Ok(state) => HttpResponse::Ok().json(format!("{state}")),
        Err(e) => {
            error!("{e:?}");
            HttpResponse::InternalServerError().json("Failed to get deposit tx state")
        }
    }
}

#[derive(Deserialize)]
struct GetDepositAddrReq {
    yona_address: String,
}

async fn get_deposit_address(req: web::Query<GetDepositAddrReq>) -> impl Responder {
    let yona_address = match Pubkey::from_str(&req.yona_address) {
        Ok(pubkey) => pubkey,
        Err(_) => return HttpResponse::BadRequest().json("yona_address is not valid"),
    };

    let bitcoin_pubkey =
        PublicKey::from_str(BITCOIN_DEPOSIT_PUBKEY).expect("Valid bitcoin public key");

    let script = bridge_deposit_script(
        yona_address.to_bytes(),
        bitcoin_pubkey.pubkey_hash().to_byte_array(),
    );

    let deposit_address = Address::p2wsh(script.as_script(), Network::Regtest);

    HttpResponse::Ok().json(deposit_address.to_string())
}

pub async fn relay_transactions(config: RelayConfig) {
    let yona_client = get_yona_client(&config).expect("Couldn't create Yona client");

    let bitcoin_rpc_client =
        BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into())
            .expect("Couldn't create Bitcoin client");

    let relay_program = BtcRelay::id();
    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);
    let relay_program = yona_client
        .program(relay_program)
        .expect("Couldn't create relay program instance");

    let app_state = web::Data::new(RelayTransactionsState {
        relay_program,
        bitcoin_rpc_client,
        main_state,
    });

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .app_data(app_state.clone())
            .route("/relay_tx", web::post().to(relay_tx_web_api))
            .route("/get_deposit_address", web::get().to(get_deposit_address))
            .route("/get_tx_state", web::get().to(get_state_tx_web_api))
    })
    .bind("0.0.0.0:8199")
    .expect("Couldn't bind to 0.0.0.0:8199")
    .run()
    .await
    .expect("HTTP server hasn't gracefully stop");
}
