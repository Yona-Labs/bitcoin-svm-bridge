use anchor_client::solana_sdk::commitment_config::CommitmentConfig;
use anchor_client::solana_sdk::signature::read_keypair_file;
use anchor_client::{Client as AnchorClient, Cluster};
use bitcoin::hashes::hash160::Hash as Hash160;
use bitcoin::hashes::Hash;
use bitcoin::hex::{DisplayHex, FromHex};
use bitcoin::key::{Keypair, Secp256k1};
use bitcoin::secp256k1::SecretKey;
use bitcoin::{Network, PrivateKey};
use block_relayer_lib::config::read_config;
use block_relayer_lib::relay_program_interaction::bridge_withdraw;
use block_relayer_lib::{
    process_bridge_events, relay_blocks_from_full_node, relay_transactions, run_init_program,
};
use clap::{Parser, Subcommand};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::sync::Arc;
use tokio::runtime::Runtime;

#[derive(Subcommand)]
enum RelayerCommand {
    InitProgram {
        deposit_pubkey: String,
    },
    Relay {
        bridge_privkey: String,
    },
    GenerateKey,
    BridgeWithdraw {
        bitcoin_address: String,
        amount: u64,
        yona_keypair: String,
    },
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct RelayerCli {
    #[command(subcommand)]
    command: RelayerCommand,
}

fn main() {
    env_logger::init();
    let cli = RelayerCli::parse();
    let config = read_config().expect("Could not read config file");
    let runtime = Runtime::new().expect("tokio runtime to be created");

    match cli.command {
        RelayerCommand::InitProgram { deposit_pubkey } => {
            let bridge_pubkey: [u8; 33] =
                FromHex::from_hex(&deposit_pubkey).expect("Failed to decode pubkey");
            let pubkey_hash = Hash160::hash(&bridge_pubkey);

            let result = runtime
                .block_on(run_init_program(config, pubkey_hash.to_byte_array()))
                .expect("Relay program initialization failed");
            println!("Initialization tx signature {}", result);
        }
        RelayerCommand::Relay { bridge_privkey } => {
            let key = SecretKey::from_str(&bridge_privkey).expect("Failed to decode privkey");
            let secp256k1 = Secp256k1::new();
            let private = PrivateKey::new(key, Network::Regtest);

            let bridge_pubkey = private.public_key(&secp256k1);
            let pubkey_hash = Hash160::hash(&bridge_pubkey.to_bytes());

            runtime.spawn({
                let config = config.clone();
                relay_blocks_from_full_node(config, 30)
            });

            let sqlite_pool = runtime
                .block_on(SqlitePool::connect("sqlite:./bridge.db?mode=rwc"))
                .unwrap();

            runtime
                .block_on(sqlx::migrate!("./migrations").run(&sqlite_pool))
                .expect("Can't migrate");

            runtime.block_on(process_bridge_events(
                config,
                sqlite_pool,
                private,
                bridge_pubkey,
                secp256k1,
            ));
        }
        RelayerCommand::GenerateKey => {
            let mut rng = rand::thread_rng();
            let key = SecretKey::new(&mut rng);
            println!("Secret key {}", key.secret_bytes().to_lower_hex_string());
            let secp256k1 = Secp256k1::new();
            let keypair = Keypair::from_secret_key(&secp256k1, &key);
            println!("Public key {:02x}", keypair.public_key());
        }
        RelayerCommand::BridgeWithdraw {
            bitcoin_address,
            amount,
            yona_keypair,
        } => {
            let yona_keypair = read_keypair_file(yona_keypair).expect("Failed to read keypair");
            let signer = Arc::new(yona_keypair);
            let cluster = Cluster::Custom(config.yona_http.clone(), config.yona_ws.clone());
            let client =
                AnchorClient::new_with_options(cluster, signer, CommitmentConfig::confirmed());
            let program = client
                .program(btc_relay::id())
                .expect("Couldn't create relay program instance");

            let result = runtime
                .block_on(bridge_withdraw(&program, amount, bitcoin_address))
                .expect("Successful withdrawal");
            println!("Withdraw result {result}");
        }
    }
}
