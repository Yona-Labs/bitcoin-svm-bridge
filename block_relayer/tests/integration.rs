use anchor_client::anchor_lang::prelude::Pubkey;
use anchor_client::solana_sdk::native_token::LAMPORTS_PER_SOL;
use block_relayer_lib::config::{BitcoinAuth, RelayConfig};
use block_relayer_lib::{get_yona_client, run_deposit, run_init_program};
use bollard::container::RemoveContainerOptions;
use bollard::Docker;
use once_cell::sync::Lazy;
use std::env;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread::sleep;
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
        .with_mapped_port(50001, 50001.tcp())
        .with_mapped_port(8094, 80.tcp())
        .with_mapped_port(18443, 18443.tcp())
        .with_mount(Mount::bind_mount(host_mount_path, "/data"));

    let esplora_container = TEST_RUNTIME
        .block_on(container_req.start())
        .expect("Esplora container to be started");

    // give everything some additional time to initialize
    sleep(Duration::from_secs(10));

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
        run_deposit(relay_config.clone(), 100 * LAMPORTS_PER_SOL).expect("run_deposit");
    println!("Deposit result {}", init_result);

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
    let deposit_balance = program
        .rpc()
        .get_balance(&deposit_account)
        .expect("deposit account get_balance");
    // should be slightly larger because deposit account is rent-exempt
    assert!(deposit_balance > 100 * LAMPORTS_PER_SOL);
}
