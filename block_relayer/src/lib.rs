pub mod bridge_db;
pub mod config;
mod merkle;
pub mod metrics;
pub mod relay_program_interaction;
pub mod relay_transactions;

use crate::bridge_db::{
    insert_solana_transaction, set_transaction_processed, solana_transaction_processed, Utxo,
    WithdrawStatus, WithdrawTransactionInfo, UTXO_STATUS_CONFIRMED, UTXO_STATUS_PENDING_CHANGE,
};
use crate::config::RelayConfig;
use crate::metrics::{
    inc_event, observe_confirmations, set_block_height, start_reconcile_timer,
    update_pending_metrics, MetricChain, MetricEventStatus, MetricFlow, MetricReason,
};
use crate::relay_program_interaction::*;
use anchor_client::anchor_lang::{AccountDeserialize, AnchorDeserialize, Discriminator, Id};
use anchor_client::solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config;
use anchor_client::solana_client::rpc_config::RpcTransactionConfig;
use anchor_client::solana_sdk::commitment_config::CommitmentConfig;
use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{read_keypair_file, Keypair, Signature};
use anchor_client::{
    solana_client, Client as AnchorClient, ClientError as AnchorClientError, Cluster,
};
use base64::Engine;
use bitcoin::absolute::LockTime;
use bitcoin::consensus::serialize;
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
use bitcoincore_rpc::jsonrpc::minreq_http::MinreqHttpTransport;
use bitcoincore_rpc::{Client as BitcoinRpcClient, Error as BtcError, RpcApi};
use btc_relay::events::{DepositTxVerified, StoreHeader, Withdrawal};
use btc_relay::program::BtcRelay;
use btc_relay::state::MainState;
use btc_relay::utils::bridge_deposit_script;
use btc_relay::utils::{compute_new_nbits, nbits_to_target};
use log::{debug, error, info, warn};
use solana_transaction_status::option_serializer::OptionSerializer;
use sqlx::SqlitePool;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::{env, error};

const MAX_REORG_RECOVERY_DEPTH: u32 = 249;

fn safe_bitcoin_height(best_block_height: u32, required_confirmations: u32) -> u32 {
    best_block_height.saturating_sub(required_confirmations.saturating_sub(1))
}

fn last_diff_adjustment_height(block_height: u32) -> u32 {
    block_height - (block_height % 2016)
}

fn last_diff_adjustment_for_height(
    bitcoind_client: &BitcoinRpcClient,
    block_height: u32,
) -> Result<u32, BtcError> {
    let adjustment_height = last_diff_adjustment_height(block_height);
    let adjustment_hash = bitcoind_client.get_block_hash(adjustment_height as u64)?;
    let adjustment_header = bitcoind_client.get_block_header(&adjustment_hash)?;
    Ok(adjustment_header.time)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderRelayAction {
    WaitForSafeHeight { safe_height: u32 },
    AheadOfSafeHeight { safe_height: u32 },
    SubmitNext { new_height: u32 },
    RecoverFork { fork_height: u32, safe_height: u32 },
}

fn decide_header_relay_action(
    relay_tip_height: u32,
    best_block_height: u32,
    required_confirmations: u32,
    tip_matches_chain: bool,
    common_ancestor_height: Option<u32>,
) -> HeaderRelayAction {
    let safe_height = safe_bitcoin_height(best_block_height, required_confirmations);
    if tip_matches_chain && relay_tip_height == safe_height {
        return HeaderRelayAction::WaitForSafeHeight { safe_height };
    }
    if tip_matches_chain && relay_tip_height > safe_height {
        return HeaderRelayAction::AheadOfSafeHeight { safe_height };
    }

    let new_height = relay_tip_height + 1;
    if tip_matches_chain {
        return HeaderRelayAction::SubmitNext { new_height };
    }

    let fork_height = common_ancestor_height
        .map(|height| height + 1)
        .unwrap_or(new_height);

    HeaderRelayAction::RecoverFork {
        fork_height,
        safe_height,
    }
}

fn find_common_ancestor(
    bitcoind_client: &BitcoinRpcClient,
    relay_tip_hash: BlockHash,
    relay_tip_height: u32,
) -> Result<Option<(u32, BlockHash)>, BtcError> {
    let mut relay_hash = relay_tip_hash;
    let mut relay_height = relay_tip_height;

    for _ in 0..=MAX_REORG_RECOVERY_DEPTH {
        let bitcoin_hash_at_height = bitcoind_client.get_block_hash(relay_height as u64)?;
        if bitcoin_hash_at_height == relay_hash {
            return Ok(Some((relay_height, relay_hash)));
        }

        if relay_height == 0 {
            break;
        }

        let relay_header = bitcoind_client.get_block_header(&relay_hash)?;
        relay_hash = relay_header.prev_blockhash;
        relay_height -= 1;
    }

    Ok(None)
}

fn tip_matches_bitcoin_chain(
    bitcoind_client: &BitcoinRpcClient,
    relay_tip_hash: BlockHash,
    relay_tip_height: u32,
) -> Result<bool, BtcError> {
    let bitcoin_hash_at_height = bitcoind_client.get_block_hash(relay_tip_height as u64)?;
    Ok(bitcoin_hash_at_height == relay_tip_hash)
}

fn can_check_tip_consistency(relay_tip_height: u32, bitcoin_best_height: u32) -> bool {
    bitcoin_best_height >= relay_tip_height
}

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

    let transport = MinreqHttpTransport::builder()
        .url(&config.bitcoind_url)
        .map_err(|e| BtcError::JsonRpc(e.into()))
        .unwrap()
        .build();

    let bitcoind_client =
        BitcoinRpcClient::from_jsonrpc(jsonrpc::Client::with_transport(transport));
    // .expect("Couldn't create Bitcoin client");

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
        let mut commited_header = match tokio::task::block_in_place(|| {
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

        commited_header.chain_work = main_state_data.chain_work;

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

        set_block_height(MetricChain::BitcoinBest, best_block_height);
        set_block_height(MetricChain::YonaRelayTip, last_submitted_height);

        let relay_tip_hash = BlockHash::from_byte_array(main_state_data.tip_block_hash);
        if !can_check_tip_consistency(last_submitted_height, best_block_height) {
            warn!(
                "Bitcoin node best height {} is behind relay tip {}. Waiting for backend to catch up.",
                best_block_height,
                last_submitted_height
            );
            tokio::time::sleep(Duration::from_secs(wait_for_new_block)).await;
            continue;
        }
        let tip_matches_chain = match tokio::task::block_in_place(|| {
            tip_matches_bitcoin_chain(&bitcoind_client, relay_tip_hash, last_submitted_height)
        }) {
            Ok(matches) => matches,
            Err(e) => {
                error!(
                    "Error {e} on Bitcoin tip consistency check at height {}",
                    last_submitted_height
                );
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        let header_action = if tip_matches_chain {
            decide_header_relay_action(
                last_submitted_height,
                best_block_height,
                config.btc_header_confirmations,
                true,
                None,
            )
        } else {
            let common_ancestor = match tokio::task::block_in_place(|| {
                find_common_ancestor(&bitcoind_client, relay_tip_hash, last_submitted_height)
            }) {
                Ok(Some(ancestor)) => ancestor,
                Ok(None) => {
                    error!(
                        "Could not find a common ancestor within {} blocks of relay tip {} at height {}. Manual recovery is required.",
                        MAX_REORG_RECOVERY_DEPTH,
                        relay_tip_hash,
                        last_submitted_height
                    );
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
                Err(e) => {
                    error!("Error {e} while searching for a common ancestor");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };

            decide_header_relay_action(
                last_submitted_height,
                best_block_height,
                config.btc_header_confirmations,
                false,
                Some(common_ancestor.0),
            )
        };

        let new_height = last_submitted_height + 1;

        match header_action {
            HeaderRelayAction::WaitForSafeHeight { safe_height } => {
                info!(
                    "Yona relay tip {} is caught up to safe BTC height {} (best {}, header confirmations {}). Waiting for a new safe block.",
                    last_submitted_height,
                    safe_height,
                    best_block_height,
                    config.btc_header_confirmations
                );
                tokio::time::sleep(Duration::from_secs(wait_for_new_block)).await;
                continue;
            }
            HeaderRelayAction::AheadOfSafeHeight { safe_height } => {
                warn!(
                    "Yona relay tip {} is ahead of safe BTC height {} (best {}, header confirmations {}). Waiting for Bitcoin to catch up before applying delay policy.",
                    last_submitted_height,
                    safe_height,
                    best_block_height,
                    config.btc_header_confirmations
                );
                tokio::time::sleep(Duration::from_secs(wait_for_new_block)).await;
                continue;
            }
            HeaderRelayAction::SubmitNext { .. } => {}
            HeaderRelayAction::RecoverFork {
                fork_height,
                safe_height,
            } => {
                if fork_height > safe_height {
                    warn!(
                        "Relay tip mismatch at height {} but fork height {} is above safe BTC height {}. Waiting for confirmations.",
                        last_submitted_height, fork_height, safe_height
                    );
                    tokio::time::sleep(Duration::from_secs(wait_for_new_block)).await;
                    continue;
                }
                warn!(
                    "Relay tip mismatch at height {}: Yona tip {} is no longer canonical. Starting fork recovery at height {}.",
                    last_submitted_height,
                    relay_tip_hash,
                    fork_height
                );

                let common_ancestor = match tokio::task::block_in_place(|| {
                    find_common_ancestor(&bitcoind_client, relay_tip_hash, last_submitted_height)
                }) {
                    Ok(Some(ancestor)) => ancestor,
                    Ok(None) => {
                        error!(
                        "Could not find a common ancestor within {} blocks of relay tip {} at height {}. Manual recovery is required.",
                        MAX_REORG_RECOVERY_DEPTH,
                        relay_tip_hash,
                        last_submitted_height
                    );
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                    Err(e) => {
                        error!("Error {e} while searching for a common ancestor");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                let fork_block_hash = match tokio::task::block_in_place(|| {
                    bitcoind_client.get_block_hash(fork_height as u64)
                }) {
                    Ok(hash) => hash,
                    Err(e) => {
                        error!(
                        "Error {e} on Bitcoin's get_block_hash({fork_height}) during fork recovery"
                    );
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                let fork_block = match tokio::task::block_in_place(|| {
                    bitcoind_client.get_block(&fork_block_hash)
                }) {
                    Ok(block) => block,
                    Err(e) => {
                        error!("Error {e} on Bitcoin's get_block({fork_block_hash:02x}) during fork recovery");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                let ancestor_last_diff_adjustment = match tokio::task::block_in_place(|| {
                    last_diff_adjustment_for_height(&bitcoind_client, common_ancestor.0)
                }) {
                    Ok(timestamp) => timestamp,
                    Err(e) => {
                        error!(
                            "Error {e} while loading last_diff_adjustment for common ancestor height {} during fork recovery",
                            common_ancestor.0
                        );
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                let prev_committed_header = match tokio::task::block_in_place(|| {
                    reconstruct_commited_header(
                        &bitcoind_client,
                        &common_ancestor.1,
                        common_ancestor.0,
                        ancestor_last_diff_adjustment,
                    )
                }) {
                    Ok(header) => header,
                    Err(e) => {
                        error!("Error {e} while reconstructing the common ancestor header during fork recovery");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };
                let ancestor_chain_work = match tokio::task::block_in_place(|| {
                    bitcoind_client.get_block_info(&common_ancestor.1)
                }) {
                    Ok(info) => chainwork_bytes_to_u256_be(&info.chainwork),
                    Err(e) => {
                        error!(
                            "Error {e} while loading chainwork for common ancestor height {} during fork recovery",
                            common_ancestor.0
                        );
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };
                let mut prev_committed_header = prev_committed_header;
                prev_committed_header.chain_work = ancestor_chain_work;

                match submit_block_fork(
                    &program,
                    main_state,
                    fork_block,
                    fork_height,
                    prev_committed_header,
                )
                .await
                {
                    Ok(_) => {
                        info!(
                        "Recovered reorg by submitting fork block at height {} from common ancestor height {}",
                        fork_height, common_ancestor.0
                    );
                    }
                    Err(e) => {
                        error!("Error {e:?} on fork recovery submit attempt");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                    }
                }
                continue;
            }
        }

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

        if block_to_submit.header.prev_blockhash != relay_tip_hash {
            error!(
                "Bitcoin height {} no longer builds on relay tip {} after consistency check. Retrying.",
                new_height, relay_tip_hash
            );
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        }

        if new_height % 2016 == 0 {
            let prev_hash = tokio::task::block_in_place(|| {
                bitcoind_client.get_block_hash((new_height - 1) as u64)
            })
            .expect("Bad retarget");
            let start_hash = tokio::task::block_in_place(|| {
                bitcoind_client.get_block_hash((new_height - 2016) as u64)
            })
            .expect("Bad retarget");

            let prev_hdr =
                tokio::task::block_in_place(|| bitcoind_client.get_block_header(&prev_hash))
                    .expect("Bad retarget");
            let start_hdr =
                tokio::task::block_in_place(|| bitcoind_client.get_block_header(&start_hash))
                    .expect("Bad retarget");

            if main_state_data.last_diff_adjustment != start_hdr.time {
                error!("[RETARGET] BAD STATE: main_state.last_diff_adjustment={} but BTC start_time(H-2016)={}. You initialized incorrectly; retarget may fail.",
                    main_state_data.last_diff_adjustment,
                    start_hdr.time
                );
            }

            let mut prev_target = nbits_to_target(prev_hdr.bits.to_consensus());
            let expected_nbits = compute_new_nbits(prev_hdr.time, start_hdr.time, &mut prev_target);

            let actual_nbits = block_to_submit.header.bits.to_consensus();

            info!(
                "[RETARGET CHECK] height={} expected={:08x} actual={:08x} start_time={} prev_time={}",
                new_height, expected_nbits, actual_nbits, start_hdr.time, prev_hdr.time
            );

            if expected_nbits != actual_nbits {
                error!(
                    "[RETARGET CHECK] MISMATCH at height {}: expected {:08x} got {:08x}. NOT submitting.",
                    new_height, expected_nbits, actual_nbits
                );
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        }

        if let Err(e) = submit_block(
            &program,
            main_state,
            block_to_submit,
            new_height,
            stored_header.header,
        )
        .await
        {
            inc_event(
                MetricFlow::HeaderRelay,
                MetricEventStatus::Error,
                MetricReason::SubmitFailed,
            );
            error!("Error {e:?} on block submit attempt");
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        }
        inc_event(
            MetricFlow::HeaderRelay,
            MetricEventStatus::Ok,
            MetricReason::None,
        );
    }
}

#[derive(Debug)]
pub enum BlockRelayerError {
    AnchorClient(AnchorClientError),
    AnchorLang(anchor_client::anchor_lang::error::Error),
    SolanaClient(solana_client::client_error::ClientError),
    Bitcoin(BtcError),
    CouldNotInitYonaClient(Box<dyn error::Error>),
}

impl From<AnchorClientError> for BlockRelayerError {
    fn from(error: AnchorClientError) -> Self {
        BlockRelayerError::AnchorClient(error)
    }
}

impl From<anchor_client::anchor_lang::error::Error> for BlockRelayerError {
    fn from(error: anchor_client::anchor_lang::error::Error) -> Self {
        BlockRelayerError::AnchorLang(error)
    }
}

impl From<solana_client::client_error::ClientError> for BlockRelayerError {
    fn from(error: solana_client::client_error::ClientError) -> Self {
        BlockRelayerError::SolanaClient(error)
    }
}

impl From<BtcError> for BlockRelayerError {
    fn from(error: BtcError) -> Self {
        BlockRelayerError::Bitcoin(error)
    }
}

impl From<InitError> for BlockRelayerError {
    fn from(err: InitError) -> Self {
        match err {
            InitError::Anchor(e) => BlockRelayerError::AnchorClient(e),
            InitError::Bitcoin(e) => BlockRelayerError::Bitcoin(e),
        }
    }
}

/// Initializes BTC relay program using the current Bitcoin tip (latest block)
pub async fn run_init_program(
    config: RelayConfig,
    deposit_pubkey_hash: [u8; 20],
    init_height: Option<u32>,
) -> Result<Signature, BlockRelayerError> {
    let yona_client =
        get_yona_client(&config).map_err(BlockRelayerError::CouldNotInitYonaClient)?;

    let transport = MinreqHttpTransport::builder()
        .url(&config.bitcoind_url)
        .map_err(|e| BtcError::JsonRpc(e.into()))
        .unwrap()
        .build();

    let bitcoind_client =
        BitcoinRpcClient::from_jsonrpc(jsonrpc::Client::with_transport(transport));

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program)?;

    let (block_height, block_hash) = if let Some(h) = init_height {
        let bh = tokio::task::block_in_place(|| bitcoind_client.get_block_hash(h as u64))?;
        (h, bh)
    } else {
        let best = tokio::task::block_in_place(|| bitcoind_client.get_best_block_hash())?;
        let info = tokio::task::block_in_place(|| bitcoind_client.get_block_info(&best))?;
        (info.height as u32, best)
    };

    debug!("Init using BTC block height={block_height}, hash={block_hash:02x}");
    let last_block = tokio::task::block_in_place(|| bitcoind_client.get_block(&block_hash))?;

    Ok(init_program(
        &program,
        &bitcoind_client,
        last_block,
        block_height,
        deposit_pubkey_hash,
    )
    .await?)
}

pub async fn run_submit_block_fork(
    config: RelayConfig,
    block_number: u64,
) -> Result<Signature, BlockRelayerError> {
    let yona_client =
        get_yona_client(&config).map_err(BlockRelayerError::CouldNotInitYonaClient)?;

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program)?;

    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);

    let raw_account = program.rpc().get_account(&main_state).await?;

    // TODO there seems to be an allocation of 8 unneeded bytes, which makes deserialization fail
    let main_state_data = MainState::try_deserialize_unchecked(&mut &raw_account.data[..8160])?;

    let transport = MinreqHttpTransport::builder()
        .url(&config.bitcoind_url)
        .map_err(|e| BtcError::JsonRpc(e.into()))
        .unwrap()
        .build();

    let bitcoind_client =
        BitcoinRpcClient::from_jsonrpc(jsonrpc::Client::with_transport(transport));

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program)?;

    let block_hash = tokio::task::block_in_place(|| bitcoind_client.get_block_hash(block_number))?;
    let block = tokio::task::block_in_place(|| bitcoind_client.get_block(&block_hash))?;

    let prev_commited_header = tokio::task::block_in_place(|| {
        reconstruct_commited_header(
            &bitcoind_client,
            &block.header.prev_blockhash,
            block_number as u32 - 1,
            main_state_data.last_diff_adjustment,
        )
    })?;

    Ok(submit_block_fork(
        &program,
        main_state,
        block,
        block_number as u32,
        prev_commited_header,
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

async fn reconcile_pending_withdrawals(
    pool: &SqlitePool,
    bitcoin_rpc_client: &BitcoinRpcClient,
    required_confirmations: u32,
) {
    let _timer = start_reconcile_timer(MetricFlow::Withdraw);
    let pending_withdrawals = match WithdrawTransactionInfo::get_non_finalized(pool).await {
        Ok(withdrawals) => withdrawals,
        Err(e) => {
            inc_event(
                MetricFlow::WithdrawReconcile,
                MetricEventStatus::Error,
                MetricReason::LoadPendingFailed,
            );
            error!("Error {e:?} on getting pending withdrawals");
            return;
        }
    };

    for withdrawal in pending_withdrawals {
        let txid = withdrawal.bitcoin_tx_id;

        match bitcoin_rpc_client.get_raw_transaction_info(&txid, None) {
            Ok(info) => {
                let confirmations = info.confirmations.unwrap_or(0);
                observe_confirmations(MetricFlow::WithdrawReconcile, confirmations);
                if confirmations >= required_confirmations {
                    if let Err(e) = finalize_withdrawal_in_db(pool, &withdrawal).await {
                        inc_event(
                            MetricFlow::WithdrawReconcile,
                            MetricEventStatus::Error,
                            MetricReason::FinalizeDbFailed,
                        );
                        error!("Error {e:?} on finalizing confirmed withdrawal");
                    } else {
                        inc_event(
                            MetricFlow::WithdrawReconcile,
                            MetricEventStatus::Confirmed,
                            MetricReason::None,
                        );
                    }
                } else {
                    inc_event(
                        MetricFlow::WithdrawReconcile,
                        MetricEventStatus::Pending,
                        MetricReason::AwaitingConfirmations,
                    );
                }
            }
            Err(_) => {
                let Some(raw_tx) = withdrawal.raw_tx.as_ref() else {
                    inc_event(
                        MetricFlow::WithdrawReconcile,
                        MetricEventStatus::Error,
                        MetricReason::MissingRawTx,
                    );
                    error!(
                        "Pending withdrawal {} has no raw tx bytes",
                        withdrawal.solana_tx_signature
                    );
                    continue;
                };

                match bitcoin_rpc_client.send_raw_transaction(
                    &bitcoin::consensus::deserialize::<Transaction>(raw_tx)
                        .expect("valid serialized bitcoin tx"),
                ) {
                    Ok(rebroadcast_txid) => {
                        inc_event(
                            MetricFlow::WithdrawReconcile,
                            MetricEventStatus::Rebroadcast,
                            MetricReason::TxNotInBlock,
                        );
                        if rebroadcast_txid != txid {
                            inc_event(
                                MetricFlow::WithdrawReconcile,
                                MetricEventStatus::Error,
                                MetricReason::RebroadcastTxidMismatch,
                            );
                            error!(
                                "Rebroadcast txid mismatch: expected {}, got {}",
                                txid, rebroadcast_txid
                            );
                        }
                    }
                    Err(e) => {
                        inc_event(
                            MetricFlow::WithdrawReconcile,
                            MetricEventStatus::Error,
                            MetricReason::RebroadcastFailed,
                        );
                        error!("Error {e:?} on rebroadcasting pending withdrawal {txid}");
                    }
                }
            }
        }
    }
}

async fn finalize_withdrawal_in_db(
    pool: &SqlitePool,
    withdrawal: &WithdrawTransactionInfo,
) -> Result<(), sqlx::Error> {
    let txid_bytes = withdrawal.bitcoin_tx_id.to_byte_array();

    Utxo::promote_pending_change(pool, &txid_bytes).await?;

    let spent_inputs = Utxo::get_by_spending_txid(pool, &txid_bytes).await?;
    for utxo in spent_inputs {
        Utxo::delete_utxo(pool, &utxo.txid, utxo.vout).await?;
    }

    WithdrawTransactionInfo::set_status(
        pool,
        &withdrawal.solana_tx_signature,
        WithdrawStatus::Confirmed,
    )
    .await
}

pub async fn process_bridge_events(
    config: RelayConfig,
    pool: SqlitePool,
    bridge_privkey: PrivateKey,
    bridge_pubkey: PublicKey,
    secp_context: Secp256k1<All>,
) {
    let yona_client = get_yona_client(&config).expect("Couldn't create Yona client");

    let transport = MinreqHttpTransport::builder()
        .url(&config.bitcoind_url)
        .map_err(|e| BtcError::JsonRpc(e.into()))
        .unwrap()
        .build();

    let bitcoin_rpc_client =
        BitcoinRpcClient::from_jsonrpc(jsonrpc::Client::with_transport(transport));

    let program = yona_client
        .program(btc_relay::id())
        .expect("Couldn't create relay program instance");

    loop {
        reconcile_pending_withdrawals(
            &pool,
            &bitcoin_rpc_client,
            config.btc_withdraw_confirmations,
        )
        .await;
        update_pending_metrics(&pool).await;

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
                    info!(
                        "[BRIDGE EVENT] DEPOSIT sol_sig={} btc_txid={}",
                        signature,
                        Txid::from_byte_array(event.tx_id)
                    );
                    let bitcoin_tx = bitcoin_rpc_client
                        .get_raw_transaction(&Txid::from_byte_array(event.tx_id), None)
                        .expect("get_raw_transaction");

                    let deposit_script = bridge_deposit_script(
                        event.wbtc_receiver_sol.to_bytes(),
                        event.deposit_pubkey_hash,
                    );
                    let expected_script_pubkey =
                        Address::p2wsh(deposit_script.as_script(), Network::Bitcoin)
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
                                status: UTXO_STATUS_CONFIRMED.to_string(),
                                spent_by_txid: None,
                            };
                            if let Err(e) = utxo.insert(&pool).await {
                                inc_event(
                                    MetricFlow::WithdrawBroadcast,
                                    MetricEventStatus::Error,
                                    MetricReason::DepositUtxoInsertFailed,
                                );
                                error!("Error on UTXO insertion {e:?}");
                            }
                        }
                    }
                } else if bytes.starts_with(&Withdrawal::DISCRIMINATOR) {
                    let event = Withdrawal::try_from_slice(&bytes[8..]).unwrap();
                    info!("Got withdrawal event {event:?}");
                    info!(
                        "[BRIDGE EVENT] WITHDRAW sol_sig={} btc_addr={} gross_sats={}",
                        signature, event.bitcoin_address, event.amount
                    );
                    if event.amount < 1546 {
                        inc_event(
                            MetricFlow::WithdrawBroadcast,
                            MetricEventStatus::Rejected,
                            MetricReason::AmountTooSmall,
                        );
                        error!(
                            "Skip withdrawal: amount {} too small (min gross is 1546 sats)",
                            event.amount
                        );
                        continue;
                    }
                    let available_utxos = match Utxo::get_spendable_utxos(&pool).await {
                        Ok(utxos) => utxos,
                        Err(e) => {
                            inc_event(
                                MetricFlow::WithdrawBroadcast,
                                MetricEventStatus::Error,
                                MetricReason::LoadUtxosFailed,
                            );
                            error!("Error {e:?} on getting utxos");
                            return;
                        }
                    };
                    let address = Address::from_str(&event.bitcoin_address)
                        .unwrap()
                        .require_network(Network::Bitcoin)
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

                    if collected_amount < event.amount + 546 {
                        inc_event(
                            MetricFlow::WithdrawBroadcast,
                            MetricEventStatus::Rejected,
                            MetricReason::InsufficientUtxos,
                        );
                        error!(
                            "Skip withdrawal: insufficient UTXOs, collected={}, need_at_least={}",
                            collected_amount,
                            event.amount + 546
                        );
                        continue;
                    }

                    let change = collected_amount - event.amount;

                    let bridge_script_pubkey = Address::p2wpkh(
                        &bridge_pubkey
                            .try_into()
                            .expect("bridge_pubkey is compressed"),
                        KnownHrp::Mainnet,
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
                            inc_event(
                                MetricFlow::WithdrawBroadcast,
                                MetricEventStatus::Ok,
                                MetricReason::None,
                            );
                            info!("Processed bridge withdrawal, Bitcoin tx id {}", id);
                            let raw_tx = serialize(&tx);

                            WithdrawTransactionInfo::add_new(&pool, &signature, &id, &raw_tx)
                                .await
                                .expect("WithdrawTransactionInfo::add_new success");

                            for input in tx.input.iter() {
                                Utxo::mark_spent_pending(
                                    &pool,
                                    &input.previous_output.txid.to_byte_array(),
                                    input.previous_output.vout,
                                    &id.to_byte_array(),
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
                                status: UTXO_STATUS_PENDING_CHANGE.to_string(),
                                spent_by_txid: None,
                            };

                            if let Err(e) = utxo.insert(&pool).await {
                                inc_event(
                                    MetricFlow::WithdrawBroadcast,
                                    MetricEventStatus::Error,
                                    MetricReason::ChangeUtxoInsertFailed,
                                );
                                error!("Error on UTXO insertion {e:?}");
                            }
                        }
                        Err(e) => {
                            inc_event(
                                MetricFlow::WithdrawBroadcast,
                                MetricEventStatus::Error,
                                MetricReason::BroadcastFailed,
                            );
                            error!("Error {e:?} on broadcasting Bitcoin tx")
                        }
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

#[cfg(test)]
mod tests {
    use super::decide_header_relay_action;
    use super::finalize_withdrawal_in_db;
    use super::can_check_tip_consistency;
    use super::last_diff_adjustment_height;
    use super::reconcile_pending_withdrawals;
    use super::safe_bitcoin_height;
    use super::HeaderRelayAction;
    use crate::bridge_db::{
        init_test_pool, Utxo, WithdrawStatus, WithdrawTransactionInfo, UTXO_STATUS_CONFIRMED,
        UTXO_STATUS_PENDING_CHANGE, UTXO_STATUS_SPENT_PENDING,
    };
    use anchor_client::solana_sdk::signature::Signature;
    use bitcoin::amount::Amount;
    use bitcoin::hashes::Hash;
    use bitcoin::hex::FromHex;
    use bitcoin::Network;
    use bitcoin::Txid;
    use bitcoincore_rpc::jsonrpc::minreq_http::MinreqHttpTransport;
    use bitcoincore_rpc::{Client as BitcoinRpcClient, RpcApi};
    use std::env;
    use std::str::FromStr;

    fn txid_from_hex(hex: &str) -> Txid {
        Txid::from_str(hex).expect("valid txid")
    }

    fn rpc_client(url: &str, user: &str, password: &str) -> BitcoinRpcClient {
        let transport = MinreqHttpTransport::builder()
            .url(url)
            .map_err(|e| bitcoincore_rpc::Error::JsonRpc(e.into()))
            .unwrap()
            .basic_auth(user.to_string(), Some(password.to_string()))
            .build();
        BitcoinRpcClient::from_jsonrpc(jsonrpc::Client::with_transport(transport))
    }

    fn e2e_rpc_env(default_wallet: &str) -> (String, String, String, String) {
        let base_url = env::var("BITCOIN_E2E_RPC_URL")
            .expect("BITCOIN_E2E_RPC_URL must point to a running regtest RPC");
        let rpc_user = env::var("BITCOIN_E2E_RPC_USER").unwrap_or_else(|_| "test".to_string());
        let rpc_password =
            env::var("BITCOIN_E2E_RPC_PASSWORD").unwrap_or_else(|_| "test".to_string());
        let wallet_name =
            env::var("BITCOIN_E2E_WALLET").unwrap_or_else(|_| default_wallet.to_string());

        (base_url, rpc_user, rpc_password, wallet_name)
    }

    fn ensure_wallet_loaded(root_client: &BitcoinRpcClient, wallet_name: &str) {
        if !root_client
            .list_wallets()
            .unwrap()
            .iter()
            .any(|wallet| wallet == wallet_name)
        {
            root_client
                .create_wallet(wallet_name, None, None, None, None)
                .unwrap();
        }
    }

    #[test]
    fn test_safe_bitcoin_height_applies_header_delay() {
        assert_eq!(safe_bitcoin_height(100, 6), 95);
        assert_eq!(safe_bitcoin_height(6, 6), 1);
        assert_eq!(safe_bitcoin_height(3, 6), 0);
        assert_eq!(safe_bitcoin_height(100, 0), 100);
    }

    #[test]
    fn test_last_diff_adjustment_height_uses_ancestor_epoch_boundary() {
        assert_eq!(last_diff_adjustment_height(0), 0);
        assert_eq!(last_diff_adjustment_height(1), 0);
        assert_eq!(last_diff_adjustment_height(2015), 0);
        assert_eq!(last_diff_adjustment_height(2016), 2016);
        assert_eq!(last_diff_adjustment_height(4031), 2016);
        assert_eq!(last_diff_adjustment_height(4032), 4032);
    }

    #[test]
    fn test_can_check_tip_consistency_only_when_bitcoin_backend_has_the_height() {
        assert!(can_check_tip_consistency(95, 95));
        assert!(can_check_tip_consistency(95, 100));
        assert!(!can_check_tip_consistency(100, 95));
    }

    #[test]
    fn test_decide_header_relay_action_waits_when_tip_is_at_safe_height() {
        let action = decide_header_relay_action(95, 100, 6, true, None);

        assert_eq!(
            action,
            HeaderRelayAction::WaitForSafeHeight { safe_height: 95 }
        );
    }

    #[test]
    fn test_decide_header_relay_action_marks_ahead_of_safe_tip_as_non_compliant() {
        let action = decide_header_relay_action(100, 100, 6, true, None);

        assert_eq!(
            action,
            HeaderRelayAction::AheadOfSafeHeight { safe_height: 95 }
        );
    }

    #[test]
    fn test_decide_header_relay_action_submits_next_when_parent_matches_and_safe() {
        let action = decide_header_relay_action(94, 100, 6, true, None);

        assert_eq!(action, HeaderRelayAction::SubmitNext { new_height: 95 });
    }

    #[test]
    fn test_decide_header_relay_action_recovers_fork_once_fork_block_is_safe() {
        let action = decide_header_relay_action(94, 100, 6, false, Some(93));

        assert_eq!(
            action,
            HeaderRelayAction::RecoverFork {
                fork_height: 94,
                safe_height: 95
            }
        );
    }

    #[test]
    fn test_decide_header_relay_action_recovers_reorg_at_safe_boundary_without_exceeding_it() {
        let action = decide_header_relay_action(95, 101, 6, false, Some(95));

        assert_eq!(
            action,
            HeaderRelayAction::RecoverFork {
                fork_height: 96,
                safe_height: 96
            }
        );
    }

    #[test]
    fn test_decide_header_relay_action_recovers_from_deeper_reorg_without_exceeding_safe_height() {
        let action = decide_header_relay_action(94, 100, 6, false, Some(90));

        assert_eq!(
            action,
            HeaderRelayAction::RecoverFork {
                fork_height: 91,
                safe_height: 95
            }
        );
    }

    #[test]
    fn test_decide_header_relay_action_recovers_when_tip_is_orphaned_at_safe_height() {
        let action = decide_header_relay_action(95, 100, 6, false, Some(94));

        assert_eq!(
            action,
            HeaderRelayAction::RecoverFork {
                fork_height: 95,
                safe_height: 95
            }
        );
    }

    #[test]
    fn test_decide_header_relay_action_recovers_when_ahead_of_safe_tip_is_orphaned() {
        let action = decide_header_relay_action(100, 100, 6, false, Some(98));

        assert_eq!(
            action,
            HeaderRelayAction::RecoverFork {
                fork_height: 99,
                safe_height: 95
            }
        );
    }

    #[test]
    fn test_decide_header_relay_action_identifies_recovery_above_safe_height() {
        let action = decide_header_relay_action(100, 100, 6, false, Some(99));

        assert_eq!(
            action,
            HeaderRelayAction::RecoverFork {
                fork_height: 100,
                safe_height: 95
            }
        );
    }

    #[tokio::test]
    async fn test_finalize_withdrawal_in_db_promotes_change_and_removes_spent_inputs() {
        let pool = init_test_pool().await;
        let spending_txid =
            txid_from_hex("1111111111111111111111111111111111111111111111111111111111111111");
        let input_txid =
            txid_from_hex("2222222222222222222222222222222222222222222222222222222222222222");
        let sol_sig = Signature::new_unique();

        let spent_input = Utxo {
            txid: input_txid.to_byte_array(),
            vout: 0,
            amount: 50_000,
            script_pubkey: vec![1, 2, 3],
            yona_address: String::new(),
            bridge_pubkey: vec![],
            redeem_script: vec![],
            status: UTXO_STATUS_SPENT_PENDING.to_string(),
            spent_by_txid: Some(spending_txid.to_byte_array()),
        };
        spent_input.insert(&pool).await.unwrap();

        let pending_change = Utxo {
            txid: spending_txid.to_byte_array(),
            vout: 1,
            amount: 49_000,
            script_pubkey: vec![4, 5, 6],
            yona_address: String::new(),
            bridge_pubkey: vec![],
            redeem_script: vec![],
            status: UTXO_STATUS_PENDING_CHANGE.to_string(),
            spent_by_txid: None,
        };
        pending_change.insert(&pool).await.unwrap();

        WithdrawTransactionInfo::add_new(&pool, &sol_sig, &spending_txid, &[7, 8, 9])
            .await
            .unwrap();
        let withdrawal = WithdrawTransactionInfo::get_by_solana_signature(&pool, &sol_sig)
            .await
            .unwrap()
            .unwrap();

        finalize_withdrawal_in_db(&pool, &withdrawal).await.unwrap();

        assert!(Utxo::get_utxo(&pool, &input_txid.to_byte_array(), 0)
            .await
            .unwrap()
            .is_none());

        let confirmed_change = Utxo::get_utxo(&pool, &spending_txid.to_byte_array(), 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(confirmed_change.status, UTXO_STATUS_CONFIRMED);

        let stored = WithdrawTransactionInfo::get_by_solana_signature(&pool, &sol_sig)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, WithdrawStatus::Confirmed);
    }

    #[tokio::test]
    async fn test_spendable_utxos_exclude_pending_and_spent_rows() {
        let pool = init_test_pool().await;

        let confirmed = Utxo {
            txid: [1; 32],
            vout: 0,
            amount: 10_000,
            script_pubkey: vec![1],
            yona_address: String::new(),
            bridge_pubkey: vec![],
            redeem_script: vec![],
            status: UTXO_STATUS_CONFIRMED.to_string(),
            spent_by_txid: None,
        };
        confirmed.insert(&pool).await.unwrap();

        let pending = Utxo {
            txid: [2; 32],
            vout: 0,
            amount: 20_000,
            script_pubkey: vec![2],
            yona_address: String::new(),
            bridge_pubkey: vec![],
            redeem_script: vec![],
            status: UTXO_STATUS_PENDING_CHANGE.to_string(),
            spent_by_txid: None,
        };
        pending.insert(&pool).await.unwrap();

        let spent = Utxo {
            txid: [3; 32],
            vout: 0,
            amount: 30_000,
            script_pubkey: vec![3],
            yona_address: String::new(),
            bridge_pubkey: vec![],
            redeem_script: vec![],
            status: UTXO_STATUS_SPENT_PENDING.to_string(),
            spent_by_txid: Some([9; 32]),
        };
        spent.insert(&pool).await.unwrap();

        let spendable = Utxo::get_spendable_utxos(&pool).await.unwrap();
        assert_eq!(spendable.len(), 1);
        assert_eq!(spendable[0].txid, [1; 32]);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires a running bitcoin regtest rpc"]
    async fn test_regtest_withdrawal_reorg_recovery_without_yona() {
        let (base_url, rpc_user, rpc_password, wallet_name) =
            e2e_rpc_env("bridge-test-withdraw-reorg");

        let root_client = rpc_client(&base_url, &rpc_user, &rpc_password);
        root_client.version().expect("bitcoind rpc ready");

        ensure_wallet_loaded(&root_client, &wallet_name);

        let wallet_client = rpc_client(
            &format!("{base_url}/wallet/{wallet_name}"),
            &rpc_user,
            &rpc_password,
        );

        let mining_address = wallet_client
            .get_new_address(None, None)
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();

        root_client
            .generate_to_address(101, &mining_address)
            .unwrap();

        let payout_address = wallet_client
            .get_new_address(None, None)
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();

        let withdrawal_txid = wallet_client
            .send_to_address(
                &payout_address,
                Amount::from_sat(100_000),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let raw_tx_hex = root_client
            .get_raw_transaction_hex(&withdrawal_txid, None)
            .unwrap();
        let raw_tx = Vec::<u8>::from_hex(&raw_tx_hex).unwrap();

        let pool = init_test_pool().await;
        let sol_sig = Signature::new_unique();

        let spent_input = Utxo {
            txid: [4; 32],
            vout: 0,
            amount: 150_000,
            script_pubkey: vec![1, 2, 3],
            yona_address: String::new(),
            bridge_pubkey: vec![],
            redeem_script: vec![],
            status: UTXO_STATUS_SPENT_PENDING.to_string(),
            spent_by_txid: Some(withdrawal_txid.to_byte_array()),
        };
        spent_input.insert(&pool).await.unwrap();

        let pending_change = Utxo {
            txid: withdrawal_txid.to_byte_array(),
            vout: 1,
            amount: 49_000,
            script_pubkey: vec![4, 5, 6],
            yona_address: String::new(),
            bridge_pubkey: vec![],
            redeem_script: vec![],
            status: UTXO_STATUS_PENDING_CHANGE.to_string(),
            spent_by_txid: None,
        };
        pending_change.insert(&pool).await.unwrap();

        WithdrawTransactionInfo::add_new(&pool, &sol_sig, &withdrawal_txid, &raw_tx)
            .await
            .unwrap();

        reconcile_pending_withdrawals(&pool, &root_client, 2).await;
        let before_mining = WithdrawTransactionInfo::get_by_solana_signature(&pool, &sol_sig)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before_mining.status, WithdrawStatus::Broadcasted);

        let stale_block = root_client.generate_to_address(1, &mining_address).unwrap()[0];
        reconcile_pending_withdrawals(&pool, &root_client, 2).await;
        let after_one_conf = WithdrawTransactionInfo::get_by_solana_signature(&pool, &sol_sig)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_one_conf.status, WithdrawStatus::Broadcasted);

        root_client.invalidate_block(&stale_block).unwrap();
        reconcile_pending_withdrawals(&pool, &root_client, 2).await;

        let after_reorg = WithdrawTransactionInfo::get_by_solana_signature(&pool, &sol_sig)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_reorg.status, WithdrawStatus::Broadcasted);
        assert_eq!(
            Utxo::get_utxo(&pool, &[4; 32], 0)
                .await
                .unwrap()
                .unwrap()
                .status,
            UTXO_STATUS_SPENT_PENDING
        );
        assert_eq!(
            Utxo::get_utxo(&pool, &withdrawal_txid.to_byte_array(), 1)
                .await
                .unwrap()
                .unwrap()
                .status,
            UTXO_STATUS_PENDING_CHANGE
        );

        // After the reorg the relayer must not finalize the withdrawal.
        // The raw tx stays journaled, inputs remain reserved, and change is not promoted.
        // Finalization on canonical confirmations is covered separately by the deterministic
        // DB-level test `test_finalize_withdrawal_in_db_promotes_change_and_removes_spent_inputs`.
        let raw_tx_still_present = root_client
            .get_raw_transaction_info(&withdrawal_txid, None)
            .is_ok();
        if !raw_tx_still_present {
            root_client.send_raw_transaction(&raw_tx).unwrap();
        }

        let still_pending = WithdrawTransactionInfo::get_by_solana_signature(&pool, &sol_sig)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still_pending.status, WithdrawStatus::Broadcasted);
        assert_eq!(
            Utxo::get_utxo(&pool, &[4; 32], 0)
                .await
                .unwrap()
                .unwrap()
                .status,
            UTXO_STATUS_SPENT_PENDING
        );
        assert_eq!(
            Utxo::get_utxo(&pool, &withdrawal_txid.to_byte_array(), 1)
                .await
                .unwrap()
                .unwrap()
                .status,
            UTXO_STATUS_PENDING_CHANGE
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires a running bitcoin regtest rpc"]
    async fn test_regtest_deposit_confirmation_depth_with_multiple_txs_in_one_block() {
        let (base_url, rpc_user, rpc_password, wallet_name) =
            e2e_rpc_env("bridge-test-deposit-depth");

        let root_client = rpc_client(&base_url, &rpc_user, &rpc_password);
        root_client.version().expect("bitcoind rpc ready");
        ensure_wallet_loaded(&root_client, &wallet_name);

        let wallet_client = rpc_client(
            &format!("{base_url}/wallet/{wallet_name}"),
            &rpc_user,
            &rpc_password,
        );

        let mining_address = wallet_client
            .get_new_address(None, None)
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();
        root_client
            .generate_to_address(101, &mining_address)
            .unwrap();

        let deposit_addr_1 = wallet_client
            .get_new_address(None, None)
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();
        let deposit_addr_2 = wallet_client
            .get_new_address(None, None)
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();

        let txid_1 = wallet_client
            .send_to_address(
                &deposit_addr_1,
                Amount::from_sat(50_000),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let txid_2 = wallet_client
            .send_to_address(
                &deposit_addr_2,
                Amount::from_sat(70_000),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        root_client.generate_to_address(1, &mining_address).unwrap();

        let tx_1_after_first_block = root_client.get_raw_transaction_info(&txid_1, None).unwrap();
        let tx_2_after_first_block = root_client.get_raw_transaction_info(&txid_2, None).unwrap();

        assert_eq!(tx_1_after_first_block.confirmations.unwrap_or(0), 1);
        assert_eq!(tx_2_after_first_block.confirmations.unwrap_or(0), 1);
        assert_eq!(
            tx_1_after_first_block.blockhash,
            tx_2_after_first_block.blockhash
        );
        assert!(tx_1_after_first_block.confirmations.unwrap_or(0) < 6);
        assert!(tx_2_after_first_block.confirmations.unwrap_or(0) < 6);

        root_client.generate_to_address(5, &mining_address).unwrap();

        let tx_1_after_six_blocks = root_client.get_raw_transaction_info(&txid_1, None).unwrap();
        let tx_2_after_six_blocks = root_client.get_raw_transaction_info(&txid_2, None).unwrap();

        assert!(tx_1_after_six_blocks.confirmations.unwrap_or(0) >= 6);
        assert!(tx_2_after_six_blocks.confirmations.unwrap_or(0) >= 6);
    }
}
