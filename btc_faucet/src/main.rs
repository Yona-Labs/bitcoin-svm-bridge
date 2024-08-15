use actix_cors::Cors;
use actix_web::{guard, web, App, HttpResponse, HttpServer, Responder};
use bitcoincore_rpc::bitcoin::address::{Address, ParseError};
use bitcoincore_rpc::bitcoin::{Amount, Network, Txid};
use bitcoincore_rpc::{Auth, Client, RpcApi};
use derive_more::Display;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::str::FromStr;

static AUTH_TOKEN: Lazy<String> =
    Lazy::new(|| std::env::var("AUTH_TOKEN").expect("AUTH_TOKEN env to be set"));

struct AppState {
    rpc_client: Client,
}

#[derive(Deserialize)]
struct FaucetRequest {
    address: String,
}

async fn request_funds(
    data: web::Data<AppState>,
    req: web::Query<FaucetRequest>,
) -> impl Responder {
    let address = &req.address;

    // Send funds via Bitcoin RPC
    match send_funds(&data.rpc_client, address) {
        Ok(txid) => HttpResponse::Ok().body(format!("Funds sent. Transaction ID: {}", txid)),
        Err(e) => HttpResponse::InternalServerError().body(format!("Failed to send funds: {}", e)),
    }
}

#[derive(Display)]
enum SendFundsError {
    Parse(ParseError),
    Rpc(bitcoincore_rpc::Error),
}

impl From<ParseError> for SendFundsError {
    fn from(e: ParseError) -> Self {
        SendFundsError::Parse(e)
    }
}

impl From<bitcoincore_rpc::Error> for SendFundsError {
    fn from(e: bitcoincore_rpc::Error) -> Self {
        SendFundsError::Rpc(e)
    }
}

fn send_funds(rpc_client: &Client, address: &str) -> Result<Txid, SendFundsError> {
    const FAUCET_AMOUNT: u64 = 5 * 100_000_000; // Amount in BTC
    let address = Address::from_str(address)?;

    Ok(rpc_client.send_to_address(
        &address.require_network(Network::Regtest)?,
        Amount::from_sat(FAUCET_AMOUNT),
        None,
        None,
        None,
        None,
        None,
        None,
    )?)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let rpc_url = "http://127.0.0.1:18443";
    let rpc_auth = Auth::UserPass("test".into(), "test".into());
    let rpc_client = Client::new(rpc_url, rpc_auth).expect("Failed to create RPC client");

    // Initialize app state
    let app_state = web::Data::new(AppState { rpc_client });

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .app_data(app_state.clone())
            .route(
                "/faucet",
                web::get()
                    .guard(guard::Header("auth_token", AUTH_TOKEN.as_str()))
                    .to(request_funds),
            )
    })
    .bind("0.0.0.0:8099")?
    .run()
    .await
}
