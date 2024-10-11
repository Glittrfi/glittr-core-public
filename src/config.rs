use config::Config;
use lazy_static::lazy_static;
use serde::Deserialize;

#[derive(serde::Deserialize, Debug)]
pub struct Settings {
    pub btc_rpc_url: String,
    pub btc_rpc_username: String,
    pub btc_rpc_password: String,
    pub rocks_db_path: String,
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
