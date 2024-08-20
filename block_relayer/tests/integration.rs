use anchor_client::anchor_lang::prelude::Pubkey;
use anchor_client::anchor_lang::Key;
use anchor_client::solana_sdk::native_token::LAMPORTS_PER_SOL;
use bitcoin::hashes::hash160::Hash as Hash160;
use bitcoin::hashes::Hash;
use bitcoin::hex::FromHex;
use bitcoin::{Address, Amount, Network};
use bitcoincore_rpc::{Client as BitcoinRpcClient, RpcApi};
use block_relayer_lib::config::{BitcoinAuth, RelayConfig};
use block_relayer_lib::relay_program_interaction::{bridge_withdraw, relay_tx};
use block_relayer_lib::{
    get_yona_client, relay_blocks_from_full_node, run_deposit, run_init_program,
};
use bollard::container::RemoveContainerOptions;
use bollard::Docker;
use btc_relay::events::{DepositTxVerified, Withdrawal};
use btc_relay::utils::{bridge_deposit_script, BITCOIN_DEPOSIT_PUBKEY};
use once_cell::sync::Lazy;
use std::env;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ContainerRequest, GenericImage, ImageExt};
use tokio::runtime::Runtime;

const ESPLORA_CONTAINER: &str = "esplora_for_bridge_tests";

struct TestCtx {
    docker: Docker,
    esplora_container: ContainerAsync<GenericImage>,
    anchor_localnet_handle: Mutex<Child>,
    current_dir: PathBuf,
    relay_config: RelayConfig,
}

static TEST_RUNTIME: Lazy<Runtime> =
    Lazy::new(|| Runtime::new().expect("Tokio runtime to be created"));
static TEST_CTX: Lazy<TestCtx> = Lazy::new(|| {
    env_logger::init();

    let docker = Docker::connect_with_defaults().expect("Docker to be installed and running");

    let rm_options = RemoveContainerOptions {
        v: false,
        force: true,
        link: false,
    };

    if let Err(_) =
        TEST_RUNTIME.block_on(docker.remove_container(ESPLORA_CONTAINER, Some(rm_options)))
    {
        // just do nothing here
    };

    let current_dir = env::current_dir().unwrap();

    let anchor_localnet_handle = Command::new("anchor")
        .arg("localnet")
        .arg("--skip-build")
        .current_dir(current_dir.join("../"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn anchor localnet");

    let image = GenericImage::new("artempikulin/esplora", "latest").with_wait_for(WaitFor::Log(
        LogWaitStrategy::stderr("Electrum RPC server running on"),
    ));

    let host_mount_path = match env::var("GITHUB_ACTIONS") {
        Ok(_) => {
            "/home/runner/work/btc-lightclient/btc-lightclient/block_relayer/for_tests".to_string()
        }
        Err(_) => current_dir.join("for_tests").display().to_string(),
    };

    let container_req = ContainerRequest::from(image)
        .with_cmd([
            "bash",
            "-c",
            "/srv/explorer/run.sh bitcoin-regtest explorer",
        ])
        .with_container_name(ESPLORA_CONTAINER)
        // Blockstream seem to not port configuration update from romanz upstream, which has a separate
        // --auth arg.
        .with_env_var("ELECTRS_ARGS", "--cookie=test:test")
        .with_env_var("WALLET", "default")
        .with_env_var("BLOCK_TIME", "1")
        .with_mapped_port(50001, 50001.tcp())
        .with_mapped_port(8094, 80.tcp())
        .with_mapped_port(18443, 18443.tcp())
        .with_mount(Mount::bind_mount(host_mount_path, "/data"));

    let esplora_container = TEST_RUNTIME
        .block_on(container_req.start())
        .expect("Esplora container to start");

    // give everything some additional time to initialize
    thread::sleep(Duration::from_secs(10));

    // init program, deposit some amount and run block relay in background
    let bitcoind_url = match env::var("GITHUB_ACTIONS") {
        Ok(_) => "http://172.17.0.1:18443".into(),
        Err(_) => "http://127.0.0.1:18443".into(),
    };

    let relay_config = RelayConfig {
        bitcoind_url,
        bitcoin_auth: BitcoinAuth::UserPass {
            user: "test".into(),
            password: "test".into(),
        },
        yona_http: "http://127.0.0.1:8899".into(),
        yona_ws: "ws://127.0.0.1:8900/".into(),
        yona_keipair: current_dir.join("../anchor.json").display().to_string(),
    };

    let init_result = run_init_program(relay_config.clone()).expect("run_init_program");
    println!("Init result {}", init_result);

    let deposit_result =
        run_deposit(relay_config.clone(), 1000 * LAMPORTS_PER_SOL).expect("run_deposit");
    println!("Deposit result {}", init_result);

    thread::spawn({
        let relay_config = relay_config.clone();
        move || relay_blocks_from_full_node(relay_config, 1)
    });

    TestCtx {
        docker,
        esplora_container,
        anchor_localnet_handle: Mutex::new(anchor_localnet_handle),
        current_dir,
        relay_config,
    }
});

#[test]
fn program_initialized() {
    let client = get_yona_client(&TEST_CTX.relay_config).expect("get_yona_client");

    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &btc_relay::id());

    let program = client.program(btc_relay::id()).expect("btc_relay program");
    // this call will work only if program is initialized
    program
        .rpc()
        .get_account(&main_state)
        .expect("get main state account");

    let (deposit_account, _) = Pubkey::find_program_address(&[b"solana_deposit"], &btc_relay::id());

    let rent_exempt = program
        .rpc()
        .get_minimum_balance_for_rent_exemption(9)
        .expect("get_minimum_balance_for_rent_exemption");

    let deposit_balance = program
        .rpc()
        .get_balance(&deposit_account)
        .expect("deposit account get_balance");

    assert_eq!(deposit_balance, 1000 * LAMPORTS_PER_SOL + rent_exempt);
}

#[test]
fn relay_deposit_transaction() {
    // send it first on Bitcoin
    let client = get_yona_client(&TEST_CTX.relay_config).expect("get_yona_client");
    let program = client.program(btc_relay::id()).expect("btc_relay program");
    let bridge_pubkey: [u8; 33] = FromHex::from_hex(BITCOIN_DEPOSIT_PUBKEY).unwrap();
    let pubkey_hash = Hash160::hash(&bridge_pubkey);

    let output_script = bridge_deposit_script(
        program.payer().key().to_bytes(),
        pubkey_hash.to_byte_array(),
    );

    let deposit_address = Address::p2wsh(output_script.as_script(), Network::Regtest);

    let bitcoin_client = BitcoinRpcClient::new(
        &TEST_CTX.relay_config.bitcoind_url,
        TEST_CTX.relay_config.bitcoin_auth.clone().into(),
    )
    .expect("init bitcoin_client");

    // this is small tx
    let deposit_tx_id = bitcoin_client
        .send_to_address(
            &deposit_address,
            Amount::ONE_BTC,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("send_to_address");
    println!("Deposit tx id {}", deposit_tx_id);

    let _deposit_sub_handle = program
        .on(|ctx, event: DepositTxVerified| {
            println!("DepositTxVerified signature {}", ctx.signature)
        })
        .expect("Subscribe to DepositTxVerified");

    // give tx some time to be mined
    thread::sleep(Duration::from_secs(5));

    let (main_state, _) = Pubkey::find_program_address(&[b"state"], &btc_relay::id());
    relay_tx(
        &program,
        main_state,
        &bitcoin_client,
        deposit_tx_id,
        program.payer().key(),
    )
    .expect("relay_tx");

    // give event some time to be processed
    thread::sleep(Duration::from_secs(5));

    let big_deposit_tx_id = bitcoin_client
        .send_to_address(
            &deposit_address,
            Amount::from_int_btc(400),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("send_to_address");
    println!("Big deposit tx id {}", big_deposit_tx_id);

    // give tx some time to be mined
    thread::sleep(Duration::from_secs(5));

    relay_tx(
        &program,
        main_state,
        &bitcoin_client,
        big_deposit_tx_id,
        program.payer().key(),
    )
    .expect("relay_tx big_deposit_tx_id");

    // give event some time to be processed
    thread::sleep(Duration::from_secs(5));
}

#[test]
fn process_withdrawal() {
    let client = get_yona_client(&TEST_CTX.relay_config).expect("get_yona_client");
    let program = client.program(btc_relay::id()).expect("btc_relay program");

    let _withdraw_sub_handle = program
        .on(|ctx, event: Withdrawal| println!("Withdrawal signature {}", ctx.signature))
        .expect("Subscribe to Withdrawal");

    let bitcoin_address = "bcrt1qm3zxtz0evpc0r5ch3az2ulx0cxce9yjkcs73cq".into();

    bridge_withdraw(&program, LAMPORTS_PER_SOL, bitcoin_address).expect("bridge_withdraw");

    // give event some time to be processed
    thread::sleep(Duration::from_secs(5));
}
