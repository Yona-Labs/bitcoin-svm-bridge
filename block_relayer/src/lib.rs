pub mod bridge_db;
pub mod config;
mod merkle;
pub mod relay_program_interaction;

use crate::bridge_db::{
    insert_solana_transaction, set_transaction_processed, solana_transaction_processed,
};
use crate::config::RelayConfig;
use crate::relay_program_interaction::*;
use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use anchor_client::anchor_lang::{AccountDeserialize, AnchorDeserialize, Discriminator, Id};
use anchor_client::solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config;
use anchor_client::solana_client::rpc_config::RpcTransactionConfig;
use anchor_client::solana_sdk::commitment_config::CommitmentConfig;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{read_keypair_file, Keypair, Signature};
use anchor_client::{Client as AnchorClient, ClientError as AnchorClientError, Cluster, Program};
use base64::Engine;
use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::hex::DisplayHex;
use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::{All, Message};
use bitcoin::sighash::SighashCache;
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, BlockHash, EcdsaSighashType, KnownHrp, Network, OutPoint, PrivateKey,
    PublicKey, Script, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};
use bitcoincore_rpc::{Client as BitcoinRpcClient, Error as BtcError, RpcApi};
use bridge_db::Utxo;
use btc_relay::events::{DepositTxVerified, StoreHeader, Withdrawal};
use btc_relay::program::BtcRelay;
use btc_relay::state::MainState;
use btc_relay::utils::bridge_deposit_script;
use futures::future::join_all;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use solana_transaction_status::option_serializer::OptionSerializer;
use sqlx::SqlitePool;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::{env, error, thread};
use tokio::runtime::Runtime;
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

pub async fn relay_blocks_from_full_node(config: RelayConfig, wait_for_new_block: u64) {
    let yona_client = get_yona_client(&config).expect("Couldn't create Yona client");

    let bitcoind_client = BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into())
        .expect("Couldn't create Bitcoin client");

    let relay_program = BtcRelay::id();
    let program = yona_client
        .program(relay_program)
        .expect("Couldn't create relay program instance");

    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);

    loop {
        let raw_account = match program.rpc().get_account(&main_state).await {
            Ok(acc) => acc,
            Err(e) => {
                error!("Error {e} on get_account(main_state)");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        // TODO there seems to be an allocation of 8 unneeded bytes, which makes deserialization fail
        let main_state_data =
            match MainState::try_deserialize_unchecked(&mut &raw_account.data[..8160]) {
                Ok(data) => data,
                Err(e) => {
                    error!("Error {e} on main_state deserialization attempt");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };

        let mut block_hash = main_state_data.tip_block_hash;
        let commited_header = match tokio::task::block_in_place(|| {
            reconstruct_commited_header(
                &bitcoind_client,
                &BlockHash::from_byte_array(block_hash),
                main_state_data.block_height,
                main_state_data.last_diff_adjustment,
            )
        }) {
            Ok(header) => header,
            Err(e) => {
                error!("Error {e} on reconstruct_commited_header");
                tokio::time::sleep(Duration::from_secs(10)).await;
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

        let best_block_hash =
            match tokio::task::block_in_place(|| bitcoind_client.get_best_block_hash()) {
                Ok(hash) => hash,
                Err(e) => {
                    error!("Error {e} on Bitcoin's get_best_block_hash");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };

        let best_block_height = match tokio::task::block_in_place(|| {
            bitcoind_client.get_block_info(&best_block_hash)
        }) {
            Ok(info) => info.height as u32,
            Err(e) => {
                error!("Error {e} on Bitcoin's get_block_info({best_block_hash:02x})");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        if last_submitted_height >= best_block_height {
            info!("Latest BTC block {best_block_height} is already submitted to Yona. Waiting for a new one.");
            tokio::time::sleep(Duration::from_secs(wait_for_new_block)).await;
            continue;
        }

        let new_height = last_submitted_height + 1;

        let block_hash_to_submit =
            match tokio::task::block_in_place(|| bitcoind_client.get_block_hash(new_height as u64))
            {
                Ok(hash) => hash,
                Err(e) => {
                    error!("Error {e} on Bitcoin's get_block_hash({new_height})");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };

        let block_to_submit = match tokio::task::block_in_place(|| {
            bitcoind_client.get_block(&block_hash_to_submit)
        }) {
            Ok(block) => block,
            Err(e) => {
                error!("Error {e} on Bitcoin's get_block({block_hash_to_submit:02x})");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        if let Err(e) = submit_block(
            &program,
            main_state,
            block_to_submit,
            new_height,
            stored_header.header,
        )
        .await
        {
            error!("Error {e:?} on block submit attempt");
            tokio::time::sleep(Duration::from_secs(10)).await;
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
pub async fn run_init_program(
    config: RelayConfig,
    deposit_pubkey_hash: [u8; 20],
) -> Result<Signature, InitProgramError> {
    let yona_client = get_yona_client(&config).map_err(InitProgramError::CouldNotInitYonaClient)?;

    let bitcoind_client = BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into())?;

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program)?;

    let tip = tokio::task::block_in_place(|| bitcoind_client.get_chain_tips())?.remove(0);
    debug!("Current bitcoin tip {tip:?}");

    let last_block = tokio::task::block_in_place(|| bitcoind_client.get_block(&tip.hash))?;
    debug!("Bitcoin last block {last_block:?}");

    Ok(init_program(
        &program,
        &bitcoind_client,
        last_block,
        tip.height as u32,
        deposit_pubkey_hash,
    )
    .await?)
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

struct RelayTransactionsState {
    relay_program: Program<Arc<Keypair>>,
    bitcoin_rpc_client: Arc<BitcoinRpcClient>,
    deposit_pubkey_hash: [u8; 20],
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

async fn get_deposit_address(
    data: web::Data<RelayTransactionsState>,
    req: web::Query<GetDepositAddrReq>,
) -> impl Responder {
    let yona_address = match Pubkey::from_str(&req.yona_address) {
        Ok(pubkey) => pubkey,
        Err(_) => return HttpResponse::BadRequest().json("yona_address is not valid"),
    };

    let script = bridge_deposit_script(yona_address.to_bytes(), data.deposit_pubkey_hash);

    let deposit_address = Address::p2wsh(script.as_script(), Network::Regtest);

    HttpResponse::Ok().json(deposit_address.to_string())
}

pub async fn relay_transactions(config: RelayConfig, deposit_pubkey_hash: [u8; 20]) {
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
        bitcoin_rpc_client: Arc::new(bitcoin_rpc_client),
        main_state,
        deposit_pubkey_hash,
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
    })
    .bind("0.0.0.0:8199")
    .expect("Couldn't bind to 0.0.0.0:8199")
    .run()
    .await
    .expect("HTTP server hasn't gracefully stop");
}

pub async fn process_bridge_events(
    config: RelayConfig,
    pool: SqlitePool,
    bridge_privkey: PrivateKey,
    bridge_pubkey: PublicKey,
    secp_context: Secp256k1<All>,
) {
    let yona_client = get_yona_client(&config).expect("Couldn't create Yona client");

    let bitcoin_rpc_client =
        BitcoinRpcClient::new(&config.bitcoind_url, config.bitcoin_auth.into())
            .expect("Couldn't create Bitcoin client");

    let program = yona_client
        .program(btc_relay::id())
        .expect("Couldn't create relay program instance");

    loop {
        let config = GetConfirmedSignaturesForAddress2Config {
            before: None,
            until: None,
            limit: Some(1000),
            commitment: Some(CommitmentConfig::confirmed()),
        };

        let transactions_history = match program
            .rpc()
            .get_signatures_for_address_with_config(&btc_relay::id(), config)
            .await
        {
            Ok(history) => history,
            Err(e) => {
                log::error!("Error getting signatures for address: {}", e);
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        for transaction in transactions_history {
            if solana_transaction_processed(&pool, &transaction.signature)
                .await
                .unwrap()
            {
                continue;
            }

            insert_solana_transaction(&pool, &transaction.signature, transaction.slot as i64)
                .await
                .unwrap();

            let signature = Signature::from_str(&transaction.signature).unwrap();
            let config = RpcTransactionConfig {
                encoding: None,
                commitment: Some(CommitmentConfig::confirmed()),
                max_supported_transaction_version: None,
            };
            let last_transaction = match program
                .rpc()
                .get_transaction_with_config(&signature, config)
                .await
            {
                Ok(tx) => tx,
                Err(e) => {
                    log::error!("Error {e} on get_transaction_with_config({signature})");
                    continue;
                }
            };

            let messages = match last_transaction.transaction.meta.unwrap().log_messages {
                OptionSerializer::Some(messages) => messages,
                _ => panic!("log_messages are not some"),
            };

            const EVENT_PREFIX: &str = "Program data: ";
            for bytes in messages
                .iter()
                .filter_map(|msg| msg.strip_prefix(EVENT_PREFIX))
                .filter_map(|maybe_base64| {
                    base64::prelude::BASE64_STANDARD.decode(maybe_base64).ok()
                })
            {
                if bytes.starts_with(&DepositTxVerified::DISCRIMINATOR) {
                    let event = DepositTxVerified::try_from_slice(&bytes[8..]).unwrap();
                    let bitcoin_tx = bitcoin_rpc_client
                        .get_raw_transaction(&Txid::from_byte_array(event.tx_id), None)
                        .expect("get_raw_transaction");

                    let deposit_script = bridge_deposit_script(
                        event.wbtc_receiver_sol.to_bytes(),
                        event.deposit_pubkey_hash,
                    );
                    let expected_script_pubkey =
                        Address::p2wsh(deposit_script.as_script(), Network::Regtest)
                            .script_pubkey();

                    for (i, out) in bitcoin_tx.output.into_iter().enumerate() {
                        if out.script_pubkey == expected_script_pubkey {
                            let utxo = Utxo {
                                txid: event.tx_id,
                                vout: i as u32,
                                amount: out.value.to_sat(),
                                script_pubkey: expected_script_pubkey.to_bytes(),
                                yona_address: event.wbtc_receiver_sol.to_string(),
                                bridge_pubkey: vec![],
                                redeem_script: deposit_script.as_bytes().into(),
                            };
                            if let Err(e) = utxo.insert(&pool).await {
                                error!("Error on UTXO insertion {e:?}");
                            }
                        }
                    }
                } else if bytes.starts_with(&Withdrawal::DISCRIMINATOR) {
                    let event = Withdrawal::try_from_slice(&bytes[8..]).unwrap();
                    info!("Got withdrawal event {event:?}");
                    let available_utxos = match Utxo::get_all_utxos(&pool).await {
                        Ok(utxos) => utxos,
                        Err(e) => {
                            error!("Error {e:?} on getting utxos");
                            return;
                        }
                    };
                    let address = Address::from_str(&event.bitcoin_address)
                        .unwrap()
                        .require_network(Network::Regtest)
                        .unwrap();

                    let tx_out = TxOut {
                        value: Amount::from_sat(event.amount - 1000),
                        script_pubkey: address.script_pubkey(),
                    };

                    let mut input = vec![];
                    let mut collected_amount = 0;
                    let mut inputs_utxos = vec![];

                    for utxo in available_utxos {
                        let previous_output = OutPoint {
                            txid: Txid::from_byte_array(utxo.txid),
                            vout: utxo.vout,
                        };
                        input.push(TxIn {
                            previous_output,
                            script_sig: Default::default(),
                            sequence: Sequence::MAX,
                            witness: Default::default(),
                        });

                        collected_amount += utxo.amount;
                        inputs_utxos.push(utxo);

                        // ensure that resulting change is more than dust
                        if collected_amount >= event.amount + 546 {
                            break;
                        }
                    }

                    let change = collected_amount - event.amount;

                    let bridge_script_pubkey = Address::p2wpkh(
                        &bridge_pubkey
                            .try_into()
                            .expect("bridge_pubkey is compressed"),
                        KnownHrp::Regtest,
                    )
                    .script_pubkey();
                    let change_out = TxOut {
                        value: Amount::from_sat(change),
                        script_pubkey: bridge_script_pubkey.clone(),
                    };

                    let tx = Transaction {
                        version: Version::TWO,
                        lock_time: LockTime::ZERO,
                        input,
                        output: vec![tx_out, change_out],
                    };

                    let mut sig_hash_cache = SighashCache::new(tx);
                    let mut witnesses = vec![];

                    for (i, utxo) in inputs_utxos.into_iter().enumerate() {
                        let sig_hash = if !utxo.redeem_script.is_empty() {
                            sig_hash_cache
                                .p2wsh_signature_hash(
                                    i,
                                    Script::from_bytes(&utxo.redeem_script),
                                    Amount::from_sat(utxo.amount),
                                    EcdsaSighashType::All,
                                )
                                .unwrap()
                        } else {
                            sig_hash_cache
                                .p2wpkh_signature_hash(
                                    i,
                                    bridge_script_pubkey.as_script(),
                                    Amount::from_sat(utxo.amount),
                                    EcdsaSighashType::All,
                                )
                                .unwrap()
                        };

                        let message = Message::from(sig_hash);
                        let signature = secp_context.sign_ecdsa(&message, &bridge_privkey.inner);

                        let mut sig = signature.serialize_der().to_vec();
                        sig.push(EcdsaSighashType::All as u8);

                        let mut witness = Witness::new();
                        witness.push(sig);
                        witness.push(bridge_pubkey.to_bytes());
                        if !utxo.redeem_script.is_empty() {
                            witness.push(utxo.redeem_script);
                        }
                        witnesses.push(witness);
                    }

                    let mut tx = sig_hash_cache.into_transaction();
                    for (input, witness) in tx.input.iter_mut().zip(witnesses) {
                        input.witness = witness;
                    }

                    match tokio::task::block_in_place(|| {
                        bitcoin_rpc_client.send_raw_transaction(&tx)
                    }) {
                        Ok(id) => {
                            info!("Processed bridge withdrawal, Bitcoin tx id {}", id);
                            for input in tx.input.iter() {
                                Utxo::delete_utxo(
                                    &pool,
                                    &input.previous_output.txid.to_byte_array(),
                                    input.previous_output.vout,
                                )
                                .await
                                .unwrap();
                            }

                            let utxo = Utxo {
                                txid: tx.compute_txid().to_byte_array(),
                                vout: 1,
                                amount: tx.output[1].value.to_sat(),
                                script_pubkey: tx.output[1].script_pubkey.to_bytes(),
                                yona_address: "".into(),
                                bridge_pubkey: vec![],
                                redeem_script: vec![],
                            };

                            if let Err(e) = utxo.insert(&pool).await {
                                error!("Error on UTXO insertion {e:?}");
                            }
                        }
                        Err(e) => error!("Error {e:?} on broadcasting Bitcoin tx"),
                    }
                }
            }
            set_transaction_processed(&pool, &transaction.signature)
                .await
                .unwrap();
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
