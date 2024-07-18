use block_relayer_lib::config::{BitcoinAuth, RelayConfig};
use block_relayer_lib::{relay_blocks_from_full_node, run_init_program};
use bollard::container::RemoveContainerOptions;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerRequest, GenericImage, ImageExt};

const ESPLORA_CONTAINER: &str = "esplora_for_bridge_tests";

#[tokio::test]
async fn it_works() {
    env_logger::init();

    let bollard_client =
        bollard::Docker::connect_with_defaults().expect("Docker to be installed and running");

    let rm_options = RemoveContainerOptions {
        v: false,
        force: true,
        link: false,
    };

    if let Err(_) = bollard_client
        .remove_container(ESPLORA_CONTAINER, Some(rm_options))
        .await
    {
        // just do nothing here
    }

    let current_dir = std::env::current_dir().unwrap();

    let image = GenericImage::new("artempikulin/esplora", "latest").with_wait_for(WaitFor::Log(
        LogWaitStrategy::stderr("[notice] Bootstrapped 100% (done): Done"),
    ));

    let container = ContainerRequest::from(image)
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
        .with_mount(Mount::bind_mount(
            current_dir.join("for_tests").display().to_string(),
            "/data",
        ))
        .start()
        .await
        .expect("Esplora container to be started");

    let anchor_localnet = Command::new("anchor")
        .arg("localnet")
        .current_dir(current_dir.join("../"))
        .spawn()
        .expect("spawn anchor localnet");

    tokio::time::sleep(Duration::from_secs(5)).await;

    let relay_config = RelayConfig {
        bitcoind_url: "http://localhost:18443".into(),
        bitcoin_auth: BitcoinAuth::UserPass {
            user: "test".into(),
            password: "test".into(),
        },
        yona_http: "http://localhost:8899".into(),
        yona_ws: "ws://localhost:8900/".into(),
        yona_keipair: current_dir.join("../anchor.json").display().to_string(),
    };

    let handle = std::thread::spawn(|| run_init_program(relay_config));

    tokio::time::sleep(Duration::from_secs(10)).await;

    if handle.is_finished() {
        println!("Init result {}", handle.join().unwrap().unwrap());
    }
}
