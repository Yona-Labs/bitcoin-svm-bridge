use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{guard, web, App, HttpResponse, HttpServer, Responder};
use derive_more::Display;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde::Serialize;

static AUTH_TOKEN: Lazy<String> =
    Lazy::new(|| std::env::var("AUTH_TOKEN").expect("AUTH_TOKEN env to be set"));

const BITCOIN_RPC_URL: &str = "http://127.0.0.1:18443";
const BITCOIN_RPC_USER: &str = "test";
const BITCOIN_RPC_PASS: &str = "test";

#[derive(Deserialize)]
struct FaucetRequest {
    address: String,
}

async fn request_funds(req: web::Query<FaucetRequest>) -> impl Responder {
    let address = req.address.clone();

    // Send funds via Bitcoin RPC
    match send_funds(&address).await {
        Ok(txid) => HttpResponse::Ok().body(format!("Funds sent. Transaction ID: {}", txid)),
        Err(e) => HttpResponse::InternalServerError().body(format!("Failed to send funds: {}", e)),
    }
}

async fn send_funds(address: &str) -> Result<String, SendFundsError> {
    const FAUCET_AMOUNT: f64 = 5.0;

    let client = reqwest::Client::new();
    let body = JsonRpcRequest {
        jsonrpc: "1.0",
        id: "faucet",
        method: "sendtoaddress",
        params: vec![
            serde_json::Value::String(address.to_string()),
            serde_json::Value::from(FAUCET_AMOUNT),
        ],
    };

    let resp = client
        .post(BITCOIN_RPC_URL)
        .basic_auth(BITCOIN_RPC_USER, Some(BITCOIN_RPC_PASS))
        .json(&body)
        .send()
        .await?
        .json::<JsonRpcResponse<String>>()
        .await?;

    match (resp.result, resp.error) {
        (Some(txid), None) => Ok(txid),
        (_, Some(err)) => Err(SendFundsError::Rpc(err.message)),
        _ => Err(SendFundsError::Rpc("Unknown error".into())),
    }
}

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: &'static str,
    method: &'a str,
    params: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Deserialize, Debug)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Display, Debug)]
enum SendFundsError {
    #[display(fmt = "Request failed: {}", _0)]
    Reqwest(reqwest::Error),
    #[display(fmt = "RPC error: {}", _0)]
    Rpc(String),
}

impl From<reqwest::Error> for SendFundsError {
    fn from(e: reqwest::Error) -> Self {
        SendFundsError::Reqwest(e)
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .wrap(Logger::default())
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
