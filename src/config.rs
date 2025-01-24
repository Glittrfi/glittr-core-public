use bitcoin::Network;
use config::Config;
use lazy_static::lazy_static;
use serde::Deserialize;

#[derive(serde::Deserialize, Debug)]
pub struct Settings {
    pub btc_rpc_url: String,
    pub btc_rpc_username: String,
    pub btc_rpc_password: String,
    pub rocks_db_path: String,
    pub api_url: String,
    pub bitcoin_network: String, // Add this line
}

pub fn get_bitcoin_network() -> Network {
    match CONFIG.bitcoin_network.as_str() {
        "mainnet" => Network::Bitcoin,
        "testnet" => Network::Testnet,
        "signet" => Network::Signet,
        "regtest" => Network::Regtest,
        _ => Network::Regtest, // Default to Regtest if not specified
    }
}

lazy_static! {
    pub static ref CONFIG: Settings = Settings::deserialize(
        Config::builder()
            .add_source(config::File::with_name("settings"))
            .build()
            .unwrap()
    )
    .unwrap();
}
