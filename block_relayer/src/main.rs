use block_relayer_lib::config::read_config;
use block_relayer_lib::relay_blocks_from_full_node;

fn main() {
    env_logger::init();
    let config = read_config().unwrap();
    relay_blocks_from_full_node(config);
}
