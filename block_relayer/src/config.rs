use serde::Deserialize;
use std::fs;
use std::io;
use toml;

#[derive(Deserialize, Debug)]
pub struct RelayConfig {
    pub bitcoind_url: String,
    pub bitcoin_cookie_file: String,
    pub yona_http: String,
    pub yona_ws: String,
    pub yona_keipair: String,
}

#[derive(Deserialize, Debug)]
struct Config {
    relay: RelayConfig,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Toml(toml::de::Error),
}

impl From<io::Error> for ConfigError {
    fn from(error: io::Error) -> Self {
        ConfigError::Io(error)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(error: toml::de::Error) -> Self {
        ConfigError::Toml(error)
    }
}

pub fn read_config() -> Result<RelayConfig, ConfigError> {
    // Read the contents of the TOML file
    let config_contents = fs::read_to_string("./config.toml")?;

    // Parse the TOML string into our Config struct
    let config: Config = toml::from_str(&config_contents)?;

    Ok(config.relay)
}
