use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use bitcoincore_rpc::bitcoin::address::{Address, ParseError};
use bitcoincore_rpc::bitcoin::{Amount, Network, Txid};
use bitcoincore_rpc::{Auth, Client, RpcApi};
use chrono::{Duration, Utc};
use derive_more::Display;
use rusqlite::{Connection, Result as SqliteResult};
use serde::Deserialize;
use std::str::FromStr;
use std::sync::Mutex;

struct AppState {
    db: Mutex<Connection>,
    rpc_client: Mutex<Client>,
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

    let db = data.db.lock().unwrap();
    let rpc_client = data.rpc_client.lock().unwrap();

    // Check if the address has requested funds in the last 24 hours
    match check_last_request(&db, address) {
        Ok(true) => {
            return HttpResponse::TooManyRequests()
                .body("Address has already received funds in the last 24 hours")
        }
        Err(_) => return HttpResponse::InternalServerError().body("Database error"),
        _ => {}
    }

    // Send funds via Bitcoin RPC
    match send_funds(&rpc_client, address) {
        Ok(txid) => {
            // Record the request in the database
            if let Err(_) = record_request(&db, address) {
                return HttpResponse::InternalServerError().body("Failed to record request");
            }
            HttpResponse::Ok().body(format!("Funds sent. Transaction ID: {}", txid))
        }
        Err(e) => HttpResponse::InternalServerError().body(format!("Failed to send funds: {}", e)),
    }
}

fn check_last_request(db: &Connection, address: &str) -> SqliteResult<bool> {
    let mut stmt = db.prepare(
        "SELECT timestamp FROM requests WHERE address = ? ORDER BY timestamp DESC LIMIT 1",
    )?;
    let mut rows = stmt.query([address])?;

    if let Some(row) = rows.next()? {
        let timestamp: i64 = row.get(0)?;
        let last_request = chrono::NaiveDateTime::from_timestamp(timestamp, 0);
        let now = Utc::now().naive_utc();
        Ok(now.signed_duration_since(last_request) < Duration::hours(24))
    } else {
        Ok(false)
    }
}

fn record_request(db: &Connection, address: &str) -> SqliteResult<()> {
    db.execute(
        "INSERT INTO requests (address, timestamp) VALUES (?, ?)",
        [address, Utc::now().timestamp().to_string().as_str()],
    )?;
    Ok(())
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
    const FAUCET_AMOUNT: u64 = 10_000_000; // Amount in BTC
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
    // Initialize SQLite database
    let db = Connection::open("faucet.db").expect("Failed to open database");
    db.execute(
        "CREATE TABLE IF NOT EXISTS requests (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            address TEXT NOT NULL,
            timestamp INTEGER NOT NULL
        )",
        [],
    )
    .expect("Failed to create table");

    let rpc_url = "http://127.0.0.1:18443";
    let rpc_auth = Auth::UserPass("test".into(), "test".into());
    let rpc_client = Client::new(rpc_url, rpc_auth).expect("Failed to create RPC client");

    // Initialize app state
    let app_state = web::Data::new(AppState {
        db: Mutex::new(db),
        rpc_client: Mutex::new(rpc_client),
    });

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .app_data(app_state.clone())
            .route("/faucet", web::get().to(request_funds))
    })
    .bind("0.0.0.0:8099")?
    .run()
    .await
}
