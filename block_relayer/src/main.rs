use anchor_client::solana_sdk::native_token::LAMPORTS_PER_SOL;
use block_relayer_lib::config::read_config;
use block_relayer_lib::{
    relay_blocks_from_full_node, relay_transactions, run_deposit, run_init_program,
};
use clap::{Parser, Subcommand};
use tokio::runtime::Runtime;

#[derive(Subcommand)]
enum RelayerCommand {
    InitDeposit,
    InitProgram,
    RelayBlocks,
    RelayTransactions,
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
        RelayerCommand::InitDeposit => {
            let result = run_deposit(config, 10_000 * LAMPORTS_PER_SOL)
                .expect("Relay program initialization failed");
            println!("Deposit tx signature {}", result);
        }
        RelayerCommand::InitProgram => {
            let result = run_init_program(config).expect("Relay program initialization failed");
            println!("Initialization tx signature {}", result);
        }
        RelayerCommand::RelayBlocks => relay_blocks_from_full_node(config, 30),
        RelayerCommand::RelayTransactions => {
            let runtime = Runtime::new().expect("tokio runtime to be created");
            runtime.block_on(relay_transactions(config));
        }
    }
}
