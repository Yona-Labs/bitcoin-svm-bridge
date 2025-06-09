use bitcoin::hashes::hash160::Hash as Hash160;
use bitcoin::hashes::Hash;
use bitcoin::hex::FromHex;
use block_relayer_lib::config::read_config;
use block_relayer_lib::{relay_blocks_from_full_node, relay_transactions, run_init_program};
use clap::{Parser, Subcommand};
use tokio::runtime::Runtime;

#[derive(Subcommand)]
enum RelayerCommand {
    InitProgram { deposit_pubkey: String },
    RelayBlocks,
    RelayTransactions { deposit_pubkey: String },
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

    match cli.command {
        RelayerCommand::InitProgram { deposit_pubkey } => {
            let bridge_pubkey: [u8; 33] =
                FromHex::from_hex(&deposit_pubkey).expect("Failed to decode pubkey");
            let pubkey_hash = Hash160::hash(&bridge_pubkey);

            let result = run_init_program(config, pubkey_hash.to_byte_array())
                .expect("Relay program initialization failed");
            println!("Initialization tx signature {}", result);
        }
        RelayerCommand::RelayBlocks => relay_blocks_from_full_node(config, 30),
        RelayerCommand::RelayTransactions { deposit_pubkey } => {
            let bridge_pubkey: [u8; 33] =
                FromHex::from_hex(&deposit_pubkey).expect("Failed to decode pubkey");
            let pubkey_hash = Hash160::hash(&bridge_pubkey);

            let runtime = Runtime::new().expect("tokio runtime to be created");
            runtime.block_on(relay_transactions(config, pubkey_hash.to_byte_array()));
        }
    }
}
