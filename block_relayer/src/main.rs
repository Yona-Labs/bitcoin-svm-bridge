mod merkle;

use std::rc::Rc;
use std::str::FromStr;
use std::time::Duration;
use std::{env, thread};

use anchor_client::anchor_lang::prelude::*;
use anchor_client::solana_sdk::signature::{read_keypair_file, Keypair};
use anchor_client::solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Signature, Signer},
};
use anchor_client::{Client as AnchorClient, Cluster, Program};
use anchor_client::anchor_lang::solana_program::example_mocks::solana_address_lookup_table_program::state;
use anchor_client::solana_sdk::native_token::LAMPORTS_PER_SOL;
use base64::prelude::*;
use bitcoin::hex::DisplayHex;
use bitcoin::{Address, Network, PublicKey, Txid};
use bitcoin::hashes::Hash;
use bitcoincore_rpc::bitcoin::blockdata::block::Block;
use bitcoincore_rpc::bitcoin::hashes::Hash as BitcoinRpcHash;
use bitcoincore_rpc::{Auth, Client as BitcoinRpcClient, RpcApi};
use bitcoincore_rpc::bitcoin::BlockHash;
use log::{debug, info, error, warn};
use solana_transaction_status::UiTransactionEncoding;

use crate::merkle::Proof;
use btc_relay::accounts::{Deposit, Initialize, SubmitBlockHeaders, VerifyTransaction};
use btc_relay::events::StoreHeader;
use btc_relay::instruction::{
    Deposit as DepositInstruction, Initialize as InitializeInstruction,
    SubmitBlockHeaders as SubmitBlockHeadersInstruction, VerifySmallTx as VerifySmallTxInstruction,
};
use btc_relay::program::BtcRelay;
use btc_relay::state::MainState;
use btc_relay::structs::{BlockHeader, CommittedBlockHeader};
use btc_relay::utils::{bridge_deposit_script, BITCOIN_DEPOSIT_PUBKEY};

const START_SUBMIT_FROM_TX: &str =
    "HGmcAboCdFVvPqqebE8KRR288bmooHEed9KMtkEe2cy4fKuySHqQ5nz2LAkwWVH65miUJ7HdvgRFvaADGmW3fdZ";

const SOLANA_DEPOSIT_PUBKEY: &str = "5Xy6zEA64yENXm9Zz5xDmTdB8t9cQpNaD3ZwNLBeiSc5";

const FIRST_BRIDGE_TX_ID: &str = "a17e0a4375868aef5bbd602be151889f23c292ee03039aa353b61ca8c717458e";

fn relay_blocks_from_full_node() {
    let bitcoin_pubkey = PublicKey::from_str(BITCOIN_DEPOSIT_PUBKEY).unwrap();

    let solana_address = Pubkey::from_str(SOLANA_DEPOSIT_PUBKEY).unwrap();

    let script = bridge_deposit_script(
        solana_address.to_bytes(),
        bitcoin_pubkey.pubkey_hash().to_byte_array(),
    );

    info!("{script:?}");

    let deposit_address = Address::p2wsh(script.as_script(), Network::Regtest);
    println!("Deposit address {deposit_address}");

    let bitcoind_client = BitcoinRpcClient::new(
        "http://localhost:19001",
        Auth::UserPass("test".into(), "test".into()),
    )
    .unwrap();

    let yona_client = get_yona_client();

    let tip = bitcoind_client.get_chain_tips().unwrap().remove(0);
    debug!("Current bitcoin tip {tip:?}");

    let last_block = bitcoind_client.get_block(&tip.hash).unwrap();
    debug!("Bitcoin last block {last_block:?}");

    let relay_program = BtcRelay::id();
    let program = yona_client.program(relay_program).unwrap();

    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);

    if env::var("INIT_PROGRAM").is_ok() {
        init_program(&program, main_state, last_block, tip.height as u32);
    }

    let mut last_submit_tx = Signature::from_str(START_SUBMIT_FROM_TX).unwrap();
    if env::var("RELAY_BLOCKS").is_ok() {
        loop {
            // Notes on using get_signature_status_with_commitment_and_history instead of
            // get_signature_status_with_commitment https://solana.stackexchange.com/a/326
            if let Ok(Some(Ok(_))) = program
                .rpc()
                .get_signature_status_with_commitment_and_history(
                    &last_submit_tx,
                    CommitmentConfig::finalized(),
                    true,
                )
            {
                break;
            }

            let stored_header = match program
                .rpc()
                .get_transaction(&last_submit_tx, UiTransactionEncoding::Binary)
            {
                Ok(tx) => {
                    let messages: Option<Vec<String>> =
                        tx.transaction.meta.unwrap().log_messages.into();
                    let parsed_base64 = BASE64_STANDARD
                        .decode(messages.unwrap()[2].strip_prefix("Program data: ").unwrap())
                        .unwrap();
                    StoreHeader::try_from_slice(&parsed_base64[8..]).unwrap()
                }
                Err(e) => {
                    error!("Got error {e} on get_transaction({last_submit_tx})");
                    let raw_account = program.rpc().get_account(&main_state).unwrap();
                    info!("Data len {}", raw_account.data.len());
                    info!("Main state space {}", MainState::space());
                    info!("Main state size {}", std::mem::size_of::<MainState>());

                    let main_state_data =
                        MainState::try_deserialize_unchecked(&mut &raw_account.data[..8128])
                            .unwrap();

                    let max_block = 3659 + 100;
                    if main_state_data.block_height >= max_block {
                        info!("Not relaying blocks over {max_block} temporary");
                        break;
                    }

                    let mut block_hash = main_state_data.tip_block_hash;
                    let commited_header = reconstruct_commited_header(
                        &bitcoind_client,
                        &BlockHash::from_byte_array(block_hash),
                        main_state_data.block_height,
                        main_state_data.last_diff_adjustment,
                    );
                    block_hash.reverse();
                    info!(
                        "Last stored block hash {} and height {}",
                        block_hash.to_lower_hex_string(),
                        main_state_data.block_height
                    );

                    StoreHeader {
                        block_hash,
                        commit_hash: main_state_data.tip_commit_hash,
                        header: commited_header,
                    }
                }
            };

            let last_submitted_height = stored_header.header.blockheight;
            let new_height = last_submitted_height + 1;

            let block_hash_to_submit = bitcoind_client.get_block_hash(new_height as u64).unwrap();
            let block_to_submit = bitcoind_client.get_block(&block_hash_to_submit).unwrap();

            last_submit_tx =
                submit_block(&program, main_state, block_to_submit, stored_header.header);
        }
    }

    if env::var("INIT_DEPOSIT").is_ok() {
        init_deposit(&program, 100 * LAMPORTS_PER_SOL);
    }

    if env::var("RELAY_TX").is_ok() {
        let raw_account = program.rpc().get_account(&main_state).unwrap();
        info!("Data len {}", raw_account.data.len());
        info!("Main state space {}", MainState::space());
        info!("Main state size {}", std::mem::size_of::<MainState>());

        let main_state_data =
            MainState::try_deserialize_unchecked(&mut &raw_account.data[..8128]).unwrap();

        let tx_id = Txid::from_str(FIRST_BRIDGE_TX_ID).unwrap();
        relay_tx(
            &program,
            main_state,
            &bitcoind_client,
            tx_id,
            main_state_data.last_diff_adjustment,
        );
    }
}

fn init_deposit(program: &Program<Rc<Keypair>>, amount: u64) {
    let (deposit_account, _) = Pubkey::find_program_address(&[b"solana_deposit"], &program.id());

    let res = program
        .request()
        .accounts(Deposit {
            user: program.payer(),
            deposit_account,
            system_program: anchor_client::solana_sdk::system_program::ID,
        })
        .args(DepositInstruction { amount })
        .send()
        .unwrap();

    info!("Deposit tx sig {res}");
}

fn init_program(
    program: &Program<Rc<Keypair>>,
    main_state: Pubkey,
    block: Block,
    block_height: u32,
) {
    let yona_block_header = BlockHeader {
        version: block.header.version.to_consensus() as u32,
        reversed_prev_blockhash: block.header.prev_blockhash.to_byte_array(),
        merkle_root: block.header.merkle_root.to_byte_array(),
        timestamp: block.header.time,
        nbits: block.header.bits.to_consensus(),
        nonce: block.header.nonce,
    };

    let block_hash = yona_block_header.get_block_hash().unwrap();

    let (header_topic, _) =
        Pubkey::find_program_address(&[b"header", block_hash.as_slice()], &program.id());

    let res = program
        .request()
        .accounts(Initialize {
            signer: program.payer(),
            main_state,
            header_topic,
            system_program: anchor_client::solana_sdk::system_program::ID,
        })
        .args(InitializeInstruction {
            data: yona_block_header,
            block_height,
            chain_work: [0; 32],
            last_diff_adjustment: yona_block_header.timestamp,
            prev_block_timestamps: [yona_block_header.timestamp; 10],
        })
        .send()
        .unwrap();

    info!(
        "Submitted block {}, tx sig {res}",
        block_hash.to_lower_hex_string()
    );
}

fn get_yona_client() -> AnchorClient<Rc<Keypair>> {
    // Connect to the Yona devnet
    let rpc_url = "http://devnet-rpc.yona.network:8899".to_string();
    let ws_url = "ws://devnet-rpc.yona.network:8900".to_string();

    let mut keypair_path = env::home_dir().unwrap();
    keypair_path.push(".config/solana/id.json");
    // Set up sender and recipient keypairs
    let sender = read_keypair_file(keypair_path).unwrap();

    let signer = Rc::new(sender);
    let cluster = Cluster::Custom(rpc_url, ws_url);
    AnchorClient::new_with_options(cluster, signer, CommitmentConfig::confirmed())
}

fn submit_block(
    program: &Program<Rc<Keypair>>,
    main_state: Pubkey,
    block: Block,
    commited_header: CommittedBlockHeader,
) -> Signature {
    let yona_block_header = BlockHeader {
        version: block.header.version.to_consensus() as u32,
        reversed_prev_blockhash: block.header.prev_blockhash.to_byte_array(),
        merkle_root: block.header.merkle_root.to_byte_array(),
        timestamp: block.header.time,
        nbits: block.header.bits.to_consensus(),
        nonce: block.header.nonce,
    };

    let mut block_hash = yona_block_header.get_block_hash().unwrap();
    let (header_topic, _) =
        Pubkey::find_program_address(&[b"header", block_hash.as_slice()], &program.id());

    let header_account = AccountMeta::new(header_topic, false);

    let res = program
        .request()
        .accounts(SubmitBlockHeaders {
            signer: program.payer(),
            main_state,
        })
        .accounts(vec![header_account])
        .args(SubmitBlockHeadersInstruction {
            data: vec![yona_block_header],
            commited_header,
        })
        .send()
        .unwrap();

    block_hash.reverse();
    info!(
        "Submitted block header {} tx {res}",
        block_hash.to_lower_hex_string()
    );

    res
}

fn reconstruct_commited_header(
    bitcoind_client: &BitcoinRpcClient,
    hash: &BlockHash,
    height: u32,
    last_diff_adjustment: u32,
) -> CommittedBlockHeader {
    let header = bitcoind_client.get_block_header(hash).unwrap();
    debug!("Got header {header:?}");

    let mut prev_block_timestamps = [0; 10];
    for i in 0..10 {
        let prev_block_hash = bitcoind_client
            .get_block_hash(height as u64 - i as u64 - 1)
            .unwrap();
        let block = bitcoind_client.get_block(&prev_block_hash).unwrap();
        prev_block_timestamps[9 - i] = block.header.time;
    }

    CommittedBlockHeader {
        chain_work: [0; 32],
        header: BlockHeader {
            version: header.version.to_consensus() as u32,
            reversed_prev_blockhash: header.prev_blockhash.to_byte_array(),
            merkle_root: header.merkle_root.to_byte_array(),
            timestamp: header.time,
            nbits: header.bits.to_consensus(),
            nonce: header.nonce,
        },
        last_diff_adjustment,
        blockheight: height,
        prev_block_timestamps,
    }
}

fn relay_tx(
    program: &Program<Rc<Keypair>>,
    main_state: Pubkey,
    btc_client: &BitcoinRpcClient,
    tx_id: Txid,
    last_diff_adjustment: u32,
) {
    let transaction = btc_client.get_transaction(&tx_id, None).unwrap();
    println!("{transaction:?}");
    let (hash, height) = match (transaction.info.blockhash, transaction.info.blockheight) {
        (Some(hash), Some(height)) => (hash, height),
        _ => {
            warn!("Transaction {tx_id} is not included to block yet");
            return;
        }
    };
    let commited_header =
        reconstruct_commited_header(&btc_client, &hash, height, last_diff_adjustment);
    let block_info = btc_client.get_block_info(&hash).unwrap();
    let tx_pos = block_info
        .tx
        .iter()
        .position(|in_block| *in_block == tx_id)
        .unwrap();
    let proof = Proof::create(&block_info.tx, tx_pos);

    let (deposit_account, _) = Pubkey::find_program_address(&[b"solana_deposit"], &program.id());

    let relay_yona_tx = program
        .request()
        .accounts(VerifyTransaction {
            signer: program.payer(),
            main_state,
            deposit_account,
            mint_receiver: program.payer(),
        })
        .args(VerifySmallTxInstruction {
            tx_bytes: transaction.hex,
            confirmations: 1,
            tx_index: tx_pos as u32,
            commited_header,
            reversed_merkle_proof: proof.to_reversed_vec(),
        })
        .send()
        .unwrap();

    info!("Relayed {relay_yona_tx}");
}

fn main() {
    env_logger::init();
    relay_blocks_from_full_node();
    /*
    let electrum_client = ElectrumClient::new("tcp://electrum.blockstream.info:50001").unwrap();

    let tx_id_hex = "aa118f3dccd6d39b72d12e133b8e007dd17c1779eddff5714ab993e43035ff55";
    let tx_id = Txid::from_str(tx_id_hex).unwrap();
    let tx_bytes = electrum_client.transaction_get_raw(&tx_id).unwrap();

    let mut proof = electrum_client
        .transaction_get_merkle(&tx_id, 768686)
        .unwrap();
    proof.merkle.iter_mut().for_each(|data| data.reverse());

    let electrum_block_header = electrum_client.block_header(768687).unwrap();
    // println!("{electrum_block_header:?}");

    let block_header_from_electrum = BlockHeader {
        version: electrum_block_header.version.to_consensus() as u32,
        reversed_prev_blockhash: electrum_block_header.prev_blockhash.to_byte_array(),
        merkle_root: electrum_block_header.merkle_root.to_byte_array(),
        timestamp: electrum_block_header.time,
        nbits: electrum_block_header.bits.to_consensus(),
        nonce: electrum_block_header.nonce,
    };

    let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());

    let relay_program = BtcRelay::id();
    let (deposit_account, _) = Pubkey::find_program_address(&[b"solana_deposit"], &relay_program);
    // println!("Deposit account {deposit_account}");

    // Create the transfer instruction
    let instruction =
        system_instruction::transfer(&sender.pubkey(), &deposit_account, 4_000_000_000);

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

    // println!("Relay program ID {}", relay_program);

    let program = client.program(relay_program).unwrap();

    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &relay_program);

    let (header_account, _) = Pubkey::find_program_address(
        &[
            b"header",
            block_header_from_electrum
                .get_block_hash()
                .unwrap()
                .as_slice(),
        ],
        &relay_program,
    );

    let mint_receiver = env::args()
        .nth(1)
        .expect("Mint receiver address is not provided");
    let mint_receiver =
        Pubkey::from_str(&mint_receiver).expect("Provided argument is not a valid Yona address");

    println!("Attempting to verify Bitcoin transaction {tx_id_hex}");

    let res = program
        .request()
        .accounts(VerifyTransaction {
            signer: program.payer(),
            main_state,
            deposit_account,
            mint_receiver,
        })
        .args(VerifySmallTxInstruction {
            tx_bytes,
            confirmations: 1,
            tx_index: proof.pos as u32,
            commited_header,
            reversed_merkle_proof: proof.merkle,
        })
        .send()
        .unwrap();
    println!("Verify transaction signature {res}");

    let header_account = AccountMeta::new(header_account, false);

    let res = program
        .request()
        .accounts(SubmitBlockHeaders {
            signer: program.payer(),
            main_state,
        })
        .accounts(vec![header_account])
        .args(SubmitBlockHeadersInstruction {
            data: vec![block_header_from_electrum],
            commited_header: CommittedBlockHeader {
                header: block_header,
                blockheight: 768686,
                chain_work: [
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 60, 39, 152, 233,
                    4, 108, 127, 40, 200, 233, 11, 98,
                ],
                last_diff_adjustment: 1671463076,
                prev_block_timestamps: [
                    1671463076, 1671463076, 1671463076, 1671463076, 1671463076, 1671463076,
                    1671463076, 1671463076, 1671463076, 1671463076,
                ],
            },
        })
        .send()
        .unwrap();
    println!("Submit block headers tx {res}");

    let res = program
        .request()
        .accounts(Deposit {
            user: program.payer(),
            deposit_account,
            system_program: solana_sdk::system_program::ID,
        })
        .args(DepositInstruction { amount: 10_000_000 })
        .send()
        .unwrap();

    println!("{}", res);

    let (header_topic, _) = Pubkey::find_program_address(
        &[b"header", block_header.get_block_hash().unwrap().as_ref()],
        &relay_program,
    );

    let res = program
        .request()
        .accounts(Initialize {
            signer: program.payer(),
            main_state,
            system_program: solana_sdk::system_program::ID,
            header_topic,
        })
        .args(InitializeInstruction {
            data: block_header,
            block_height: 768686,
            chain_work: [
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 60, 39, 152, 233, 4,
                108, 127, 40, 200, 233, 11, 98,
            ],
            last_diff_adjustment: 1671463076,
            prev_block_timestamps: [
                1671463076, 1671463076, 1671463076, 1671463076, 1671463076, 1671463076, 1671463076,
                1671463076, 1671463076, 1671463076,
            ],
        })
        .send()
        .unwrap();

    println!("{}", res);


    let big_tx_hex = "020000000001141a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40900000017160014b8b72c5943b2cfd9de7d92f48e9fcad01d055de7feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40f000000171600140c532e8594f510da8b0d387f563934861575e844feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40700000017160014d07cba9645f2814af5ed30aef767234ddda9ac13feffffff3115e2b995e8a8f6ff045671e932f151b8df53c5f13ba7dcf03beb5cfaa1eec00000000000feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40f000000171600140c532e8594f510da8b0d387f563934861575e844feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40900000017160014b8b72c5943b2cfd9de7d92f48e9fcad01d055de7feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd405000000171600142aa38dc9e745450410da5bc9350ee8d6d12e0c70feffffff147d565149cd432db0af1a6fe7540e20f2878897127358605046b2e63702e2220100000000feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40100000017160014a748dee29baef670e3264341b2e7aebdfdb3a644feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40e00000017160014995cd732c74a3446f1ad35bafbd8d6f79828ef04feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40300000017160014004bebeb0b79c94754f5458a3ed8a293370df7fbfeffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40e00000017160014995cd732c74a3446f1ad35bafbd8d6f79828ef04feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40a00000017160014e3a8924beabbfcf3a3350368567a5c972dbbf2e7feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b4040000001716001476e4db8a1d5c4c238775df63899f71dfda0197f4feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b4060000001716001437453e98e510a863dd06be9f96d0292ba24cdcf3feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b405000000171600142aa38dc9e745450410da5bc9350ee8d6d12e0c70feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40100000017160014a748dee29baef670e3264341b2e7aebdfdb3a644feffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40800000017160014ae471e13840e2b5dcdec9c4a9b54ba11fe7b665cfeffffff71a304a66d429aedce174cc77ed638c678623961a16f3584bbc5a9ccdc1e40b40b0000001716001412d0bf7a471fde32b6ab8bea44c08806b6b9d178feffffff1a95051abf24602936c541ab0052cb3ea55ad07125289ad3f3742428d2839fd40300000017160014004bebeb0b79c94754f5458a3ed8a293370df7fbfeffffff0283a90f0000000000160014b32309634f6145184542e93869b693178a2b3068c01a1f15000000001976a9140035fb9766613a5d60c9cb7e68b7031b93f9c0ce88ac02473044022078d264c3710e473478b440d8bce655972d71dac199040df71a667f4616fc3d3e02203266223a3202045e40f317c717a81767979d7e569acabb9d6f80dbdcc3f61c50012103ad7a479ae2c1a65e5a81284dae6e9192cbee296377d5df5ed33ceb4d3845568702473044022050fed0de3d27a061542f9e85f32e178b6cab49a40f5411236cc471343a30fe0f02205a962c3bfe55b46022a0c5c594c13d8c7b58162f78dfeb9c9daa6ee3fa42c8330121030f0ce1edbf552656ea2ff791d88f84df9d68023fcf000d6a7ee8ecd5dc1e3ed202473044022012e7f7ad4d6ea9cdce4b91a84f02dd5141d60e968acc250c9ebf5f974549c7b102201c8687048907f27bbd0e48a98c320b1629de70bfbcdebefffa8c5cfbf76c9082012103d79e1063b8b5bd169ac5b2791b57e19f43160c7e553baa0b243cb666d30c19f40247304402207d9665fca9d76ed7737f309dda016d767cdd2c6aadb9bff20e2d6c9b5534b6130220225293e7650f9b8fe3c761d20ae226aa24e92f124d677141d3b24c4610fb2006012102100a0ba10ca19d9bfac2c933d473adc78043d4bfeae96ded14b3fc19466f351a02473044022074cbe0c5b4efb62d9f4b917bd528541ec3a6bf0f8d21ba2e789c34368241d99002201e5cbfb855698ec5ace5672193ce78c239ff33cc2cbf6b546af9e5531132e7cf0121030f0ce1edbf552656ea2ff791d88f84df9d68023fcf000d6a7ee8ecd5dc1e3ed202473044022034b92d6d96db9347e61e8a915ab3a37a295af6f08b3a2de0c4192828dc4804d6022067efdd8c5b8197f2b890bb46894e71f3088b64f731bd8dd3ae08b4f1918a4ade012103ad7a479ae2c1a65e5a81284dae6e9192cbee296377d5df5ed33ceb4d3845568702473044022041bd9d222072584b908b065311fa4fb84f58adbd6a47b911f11549b029e4363c022043c8bb3e485959c3531f078e460aad3c66c515d13e16303446733beea2e93a4c012102b9faff46c4afbb0c7303f47ef5d711bd8da4663bc92fe8c7295e0352cecb657502473044022057ced8a248383ca77d0e3c14616100e7343dc3f12f825188730ae5b3ed0c9c5e0220676d06cbd7bc52a2feef4cb5cbac942c8b61832438ca8b29a59beb407250dfaf01210273402ba5ce7143bdd1b4b1063378cae9cda29dcd820f339cf38b15549cb4007e024730440220078968f47a2dec93b1a4920f9a81177d8ce61a723a7fb183675bae23e750e5da02202271ded859d28a9d58bcada4566ed55b9cc590143af131917633de4668ab7cdb012102e8dedb110d0812d3a769c6f0539b3bc2164d748d386433407d503c4d6f2c44770247304402205882941c239cbfd0ba7619198894340a243803396913637f45ef2ff4a6a96791022041aa555dfc56b43f2d27ccf8691370b316f0195835e5d542e95640d8a04a34e401210393c79d23ae89f461f039b895dfd6f365ef5cc89f1a3ef030382f80c2cbd84caa024730440220272a9d37449a4f417657c0e1e5d3a87d1f02478f1fc1ddce9ab7013231ccee050220252fd6e3bdfd3f5499df7f064cdc5c1406b097d300aa734c31c97e017a341b1d0121025389a45a00c6d57e6a61258e95d6f7413f2c23274f7f82a7164ec040f7bf73ab024730440220700e5220c44e0f68931f046b52aa2d1273d86b50b7b86f2b847b98785dd0aa0102207aa0a3657e7cb1169a64f0efc15f10296f335cdadef99b62856de1b990c71c1c01210393c79d23ae89f461f039b895dfd6f365ef5cc89f1a3ef030382f80c2cbd84caa0247304402202f1e547a5a43776c855d0c37ec4cdc812cd381069337e6e843f49f9b4b9c5b8902207a565110b4af1f67300d41cf2f499a8be5424af394cba8c0cc7b373e650eeced012102e5aea5c0d5acb437f8f2bc6e8a293e56fe7f021c3b63aa01e5d681d8660af2730247304402205e2bfddb2e3b1eb6e43718e6aaa9d823ba47e295c5d381ba20817ecf37a2a2e9022068c9cc98005df5bc8f13385feed2350af854aec4b0de94ae1abaaddefe1d5fe00121030c9daf8f58ccd1733de05574964eaf2810c5d6e2375dee0f49603151cf589e1d0247304402205eb0fab222b1f5d3adf54784fda4a555c5c8acc4151908f194bd1a4fb2f6ba540220370be1435d09b6c1d94297a6128fff677336877ad30994befbacdab941d6c35701210241202dcd1c9a451c4a212f88b10c2bdf2afab7bfbd14b76c2a9078c932dfffeb02473044022003d2cb7fe423c634caeec0d3d00085351041f83f33a77596d5c104283bae70aa022031affaf6564c24d946cec27419bfb5182dcab6be1e26fcf2aa334eb7d6027160012102b9faff46c4afbb0c7303f47ef5d711bd8da4663bc92fe8c7295e0352cecb6575024730440220156f5c439dbc5ec403fcde44e27804906051d717466cf057026d4fd86be5522802204c97b2c98e06d6ea14572c91d5da9f89c4bf7d37df07badcc60b527b27e047e1012102e8dedb110d0812d3a769c6f0539b3bc2164d748d386433407d503c4d6f2c4477024730440220533759a56d6f141f00ecc3f889f5c839c340fbc015cdec5cc7595082c00e702802200e1829c99288c487cfb5af72e909e0f8d30d65c027207d1b9f59df7a728887dc012103a261fa4d379512d74d8eb14a7f797f694bbbda5ff3dcc05a08a55be7dd2b2e000247304402203fa51d8f3dd4cd19162760384036f75a15c56e432cb73a3c7de29f0a0359d48e0220679b4fa29004ebf1f04f924f7af859abc98a89a5476a57c7ad8cc9a6cdd1a437012102d78e2a75a028716ab8d9eb64833f546241fbe0ce54051fd534436135414d12c60247304402200fb88079388afdc469bc428bb7cff08cd3bf1ad9d2a0504eba6d6d23256959ef02204182acf8104ed927a6a96a8384c60a35ae45a10d04a25a3741830316014404e20121025389a45a00c6d57e6a61258e95d6f7413f2c23274f7f82a7164ec040f7bf73abadba0b00";
    let tx_bytes: Vec<u8> = bitcoin::hex::FromHex::from_hex(big_tx_hex).unwrap();

    let tx = bitcoin::Transaction::consensus_decode(&mut tx_bytes.as_slice()).unwrap();
    let tx_id = tx.txid();

    let tx_id_bytes = tx_id.to_byte_array();

    let (tx_account, _) = Pubkey::find_program_address(&[tx_id_bytes.as_ref()], &relay_program);

    let merkle_proof_hex = [
        "6e166b147ce38f1f0a87a127a24ecd26d41bded66bcb2b593bde6e689272a260",
        "6fe80557cadc2e1d63f97f65a9fcc6e74432495b431215619d2fe8a66800efa2",
        "d441810cda3fd857a7f68de1236ede49cf10cd7fa0f00a413eeb898c0e536d2b",
        "600d8fe6623382994da2ea73c966c700c6671919dc6360784af5af4509cedeba",
        "dce7b1f2c2a73f74db640f801ad6d01a524f61f7707024d0180cb6a332444adf",
        "76358016750f17dec7fbe0771c4abf07388734f09a79a2f670eebaf8a67a8abc",
        "35b7e6b41d1064f51deeeeb7a1c769f109bb0746565f993148fd517bc46b9b8c",
        "a628c02d83eaa32d54efbcfbcf1458a5c73d6924602b9b186731e48e1db8d19b",
        "37ffa85bd1d8d3e30e8006905115b7fa87f6399276627473975334a0ec53606f",
        "e17c0fd9dc3e9fd61f0036f8dd17df78cc7732533cf153fe332238442a58c2f6",
        "21fb731ff8fd7a5b7472f111e240417c646455022d59a71a9a4ba2d37022a9e8",
        "d265a5b211f0232e84660f97354799a64b7174139531688fde6b182aed83ff8f",
    ];

    let reversed_merkle_proof: Vec<_> = merkle_proof_hex
        .into_iter()
        .map(|str| {
            let mut bytes: [u8; 32] = FromHex::from_hex(str).unwrap();
            bytes.reverse();
            bytes
        })
        .collect();

    let res = program
        .request()
        .accounts(InitBigTxVerify {
            user: program.payer(),
            main_state,
            system_program: solana_sdk::system_program::ID,
            tx_account,
        })
        .args(InitBigTxVerifyInstruction {
            tx_id: tx_id_bytes,
            tx_size: tx_bytes.len() as u64,
            confirmations: 1,
            tx_index: 25,
            reversed_merkle_proof,
            commited_header: CommittedBlockHeader {
                chain_work: [
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 60, 39, 152, 233,
                    4, 108, 127, 40, 200, 233, 11, 98,
                ],
                header: block_header,
                last_diff_adjustment: 1671463076,
                blockheight: 768686,
                prev_block_timestamps: [
                    1671463076, 1671463076, 1671463076, 1671463076, 1671463076, 1671463076,
                    1671463076, 1671463076, 1671463076, 1671463076,
                ],
            },
        })
        .send()
        .unwrap();

    println!("{}", res);

    for chunk in tx_bytes.chunks(800) {
        let res = program
            .request()
            .accounts(StoreTxBytes {
                user: program.payer(),
                tx_account,
            })
            .args(StoreTxBytesInstruction {
                tx_id: tx_id_bytes,
                bytes: chunk.to_vec(),
            })
            .send()
            .unwrap();
        println!("{}", res);
    }

    let balance = program.rpc().get_balance(&deposit_account).unwrap();
    println!("Deposit address {deposit_account} balance {balance}");

    let res = program
        .request()
        .instruction(
            solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(300000),
        )
        .accounts(FinalizeTx {
            user: program.payer(),
            tx_account,
            deposit_account,
        })
        .args(FinalizeTxInstruction { tx_id: tx_id_bytes })
        .send()
        .unwrap();

    println!("{res}");
    */
}
