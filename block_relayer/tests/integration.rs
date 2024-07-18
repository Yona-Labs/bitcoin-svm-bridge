use bollard::container::RemoveContainerOptions;
use std::time::Duration;
use testcontainers::core::{IntoContainerPort, Mount};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerRequest, GenericImage, ImageExt};

const ESPLORA_CONTAINER: &str = "esplora_for_bridge_tests";

#[tokio::test]
async fn it_works() {
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

    let container = ContainerRequest::from(GenericImage::new("artempikulin/esplora", "latest"))
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

    tokio::time::sleep(Duration::from_secs(1000)).await;
}
