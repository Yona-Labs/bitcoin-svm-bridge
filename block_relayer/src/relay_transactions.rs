use crate::bridge_db::{
    WithdrawTransactionInfo,
};
use crate::config::RelayConfig;
use crate::relay_program_interaction::*;
use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use anchor_client::anchor_lang::Id;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{Keypair, Signature};
use anchor_client::Program;
use bitcoin::{Address, Network, Txid};
use bitcoincore_rpc::Client as BitcoinRpcClient;
use bitcoincore_rpc::Error as BtcError;

use btc_relay::program::BtcRelay;
use btc_relay::utils::bridge_deposit_script;
use futures::future::join_all;
use jsonrpc::minreq_http::MinreqHttpTransport;
use log::{error};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::sync::Arc;

use crate::get_yona_client;

pub struct RelayTransactionsState {
    pub relay_program: Program<Arc<Keypair>>,
    pub bitcoin_rpc_client: Arc<BitcoinRpcClient>,
    pub sqlite_pool: SqlitePool,
    pub deposit_pubkey_hash: [u8; 20],
    pub main_state: Pubkey,
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

    let relay_tx_res = relay_tx(
        &data.relay_program,
        data.main_state,
        data.bitcoin_rpc_client.clone(),
        tx_id,
        mint_receiver,
    )
    .await;

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

#[derive(Serialize)]
struct DepositTxStateResult {
    tx_id: String,
    status: DepositTxState,
}

async fn get_tx_state_web_api(
    data: web::Data<RelayTransactionsState>,
    req: web::Query<TxStateRequest>,
) -> impl Responder {
    let tx_id = match Txid::from_str(&req.tx_id) {
        Ok(tx_id) => tx_id,
        Err(_) => return HttpResponse::BadRequest().json("tx_id is not valid"),
    };

    let deposit_tx_state_res = deposit_tx_state(&data.relay_program, tx_id).await;

    match deposit_tx_state_res {
        Ok(state) => HttpResponse::Ok().json(DepositTxStateResult {
            tx_id: req.into_inner().tx_id,
            status: state,
        }),
        Err(e) => {
            error!("{e:?}");
            HttpResponse::InternalServerError().json("Failed to get deposit tx state")
        }
    }
}

#[derive(Deserialize)]
struct TxStatesRequest {
    tx_ids: Vec<String>,
}

async fn get_tx_states_web_api(
    data: web::Data<RelayTransactionsState>,
    req: actix_web_lab::extract::Query<TxStatesRequest>,
) -> impl Responder {
    let tx_ids: Vec<_> = match req.tx_ids.iter().map(|id| Txid::from_str(id)).collect() {
        Ok(tx_ids) => tx_ids,
        Err(_) => return HttpResponse::BadRequest().json("tx_id is not valid"),
    };

    let deposit_tx_state_fut = tx_ids
        .into_iter()
        .map(|tx_id| deposit_tx_state(&data.relay_program, tx_id));

    let deposit_tx_state_results: Result<Vec<_>, _> = join_all(deposit_tx_state_fut)
        .await
        .into_iter()
        .zip(req.into_inner().tx_ids.into_iter())
        .map(|(res, tx_id)| res.map(|status| DepositTxStateResult { tx_id, status }))
        .collect();

    match deposit_tx_state_results {
        Ok(states) => HttpResponse::Ok().json(states),
        Err(e) => {
            error!("{e:?}");
            HttpResponse::InternalServerError().json("Failed to get deposit tx states")
        }
    }
}

#[derive(Deserialize)]
struct GetDepositAddrReq {
    yona_address: String,
}

#[derive(Deserialize)]
struct GetWithdrawInfoReq {
    signature: String,
}

#[derive(Serialize)]
struct WithdrawInfoResponse {
    bitcoin_tx_id: String,
}

async fn get_withdraw_info_web_api(
    data: web::Data<RelayTransactionsState>,
    req: web::Query<GetWithdrawInfoReq>,
) -> impl Responder {
    let signature = match Signature::from_str(&req.signature) {
        Ok(sig) => sig,
        Err(_) => return HttpResponse::BadRequest().json("signature is not valid"),
    };

    let withdraw_info_res = WithdrawTransactionInfo::get_by_solana_signature(&data.sqlite_pool, &signature).await;

    match withdraw_info_res {
        Ok(Some(withdraw_info)) => HttpResponse::Ok().json(WithdrawInfoResponse {
            bitcoin_tx_id: withdraw_info.bitcoin_tx_id.to_string(),
        }),
        Ok(None) => HttpResponse::NotFound().json("Withdraw transaction not found"),
        Err(e) => {
            error!("{e:?}");
            HttpResponse::InternalServerError().json("Failed to get withdraw transaction info")
        }
    }
}

async fn get_deposit_address(
    data: web::Data<RelayTransactionsState>,
    req: web::Query<GetDepositAddrReq>,
) -> impl Responder {
    let yona_address = match Pubkey::from_str(&req.yona_address) {
        Ok(pubkey) => pubkey,
        Err(_) => return HttpResponse::BadRequest().json("yona_address is not valid"),
    };

    let script = bridge_deposit_script(yona_address.to_bytes(), data.deposit_pubkey_hash);

    let deposit_address = Address::p2wsh(script.as_script(), Network::Bitcoin);

    HttpResponse::Ok().json(deposit_address.to_string())
}

pub async fn relay_transactions(
    config: RelayConfig,
    deposit_pubkey_hash: [u8; 20],
    sqlite_pool: SqlitePool,
) {
    let yona_client = get_yona_client(&config).expect("Couldn't create Yona client");

    let transport = MinreqHttpTransport::builder()
    .url(&config.bitcoind_url)
    .map_err(|e| BtcError::JsonRpc(e.into())).unwrap().build();

    let bitcoin_rpc_client = BitcoinRpcClient::from_jsonrpc(jsonrpc::Client::with_transport(transport));

    let relay_program = BtcRelay::id();
    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);
    let relay_program = yona_client
        .program(relay_program)
        .expect("Couldn't create relay program instance");

    let app_state = web::Data::new(RelayTransactionsState {
        relay_program,
        bitcoin_rpc_client: Arc::new(bitcoin_rpc_client),
        main_state,
        deposit_pubkey_hash,
        sqlite_pool,
    });

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .app_data(app_state.clone())
            .route("/relay_tx", web::post().to(relay_tx_web_api))
            .route("/get_deposit_address", web::get().to(get_deposit_address))
            .route("/get_tx_state", web::get().to(get_tx_state_web_api))
            .route("/get_tx_states", web::get().to(get_tx_states_web_api))
            .route("/get_withdraw_info", web::get().to(get_withdraw_info_web_api))
    })
    .bind("0.0.0.0:8199")
    .expect("Couldn't bind to 0.0.0.0:8199")
    .run()
    .await
    .expect("HTTP server hasn't gracefully stop");
}
