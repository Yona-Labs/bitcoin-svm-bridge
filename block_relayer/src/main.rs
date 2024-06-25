use std::str::FromStr;

use electrum_client::{Client, ElectrumApi};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::Transaction,
};
use solana_sdk::instruction::{AccountMeta, Instruction};

const LOCAL_DEV_PROGRAM_ID: &str = "AVAcrXmp7Y71q3uv3zkM5yM8rd9ASyBafQgcaVduFgN3";

fn main() {
    let client = Client::new("tcp://electrum.blockstream.info:50001").unwrap();
    let res = client.server_features().unwrap();
    println!("{:#?}", res);

    let subscribe = client.block_headers_subscribe().unwrap();
    println!("{:?}", subscribe);

    // Connect to the Solana devnet
    let rpc_url = "http://127.0.0.1:8899".to_string();
    let client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

    // Set up sender and recipient keypairs
    let sender = Keypair::new();

    println!("Sender pubkey {}", sender.pubkey());
    let recipient = Pubkey::from_str("C8TjwuyfWZoQHky9gvgGdhEoNUucDPE3WCtoUMDkKQb8").unwrap();

    // Amount to transfer (in lamports)
    let amount = 1_000_000; // 0.001 SOL

    // Request an airdrop for the sender to have some SOL to transfer
    let signature = client
        .request_airdrop(&sender.pubkey(), amount * 2)
        .expect("Failed to request airdrop");

    let recent_blockhash = client
        .get_latest_blockhash()
        .expect("Failed to get recent blockhash");

    let confirmed = client
        .confirm_transaction_with_spinner(
            &signature,
            &recent_blockhash,
            CommitmentConfig::confirmed(),
        )
        .expect("Failed to confirm airdrop transaction");
    println!("Confirmed {:?}", confirmed);

    // Create the transfer instruction
    let instruction = system_instruction::transfer(&sender.pubkey(), &recipient, amount);

    // Get a recent blockhash
    let recent_blockhash = client
        .get_latest_blockhash()
        .expect("Failed to get recent blockhash");

    // Create the transaction
    let transaction = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&sender.pubkey()),
        &[&sender],
        recent_blockhash,
    );

    // Send and confirm the transaction
    let signature = client
        .send_and_confirm_transaction(&transaction)
        .expect("Failed to send transaction");

    println!("Transaction successful! Signature: {}", signature);

    let program_id = Pubkey::from_str(LOCAL_DEV_PROGRAM_ID).unwrap();

    // Create the instruction data
    // Assuming the "initialize" instruction is identified by a u8 value of 0
    let instruction_data = vec![0u8];

    // Create a new keypair for the account to be initialized
    let account_to_initialize = Keypair::new();

    // Create the instruction
    let initialize_instruction = Instruction::new_with_bytes(
        program_id,
        &instruction_data,
        vec![
            AccountMeta::new(account_to_initialize.pubkey(), true),
            AccountMeta::new_readonly(sender.pubkey(), true),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        ],
    );

    // Get a recent blockhash
    let recent_blockhash = client
        .get_latest_blockhash()
        .expect("Failed to get recent blockhash");

    // Create the transaction
    let transaction = Transaction::new_signed_with_payer(
        &[initialize_instruction],
        Some(&sender.pubkey()),
        &[&sender, &account_to_initialize],
        recent_blockhash,
    );

    // Send and confirm the transaction
    match client.send_and_confirm_transaction(&transaction) {
        Ok(signature) => println!(
            "Initialize transaction successful! Signature: {}",
            signature
        ),
        Err(e) => println!("Initialize transaction failed: {:?}", e),
    }
}
