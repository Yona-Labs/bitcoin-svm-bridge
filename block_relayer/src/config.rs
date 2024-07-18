use bitcoincore_rpc::Auth;
use serde::Deserialize;
use std::fs;
use std::io;
use toml;

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
pub enum BitcoinAuth {
    Cookie { path: String },
    UserPass { user: String, password: String },
}

impl From<BitcoinAuth> for Auth {
    fn from(value: BitcoinAuth) -> Self {
        match value {
            BitcoinAuth::Cookie { path } => Auth::CookieFile(path.into()),
            BitcoinAuth::UserPass { user, password } => Auth::UserPass(user, password),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct RelayConfig {
    pub bitcoind_url: String,
    pub bitcoin_auth: BitcoinAuth,
    pub yona_http: String,
    pub yona_ws: String,
    pub yona_keipair: String,
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
    Ok(toml::from_str(&config_contents)?)
}
