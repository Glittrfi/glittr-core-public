#![allow(irrefutable_let_patterns)]

use config::CONFIG;
use serde::{Deserialize, Serialize};
use std::{env, error::Error, sync::Arc};
use store::database::Database;
use tokio::sync::Mutex;

mod api;
mod borsh_serde;
mod config;
mod constants;
mod flaw;
mod indexer;
mod macros;
mod store;
mod transaction;
mod types;
mod updater;
pub mod varuint;
pub mod varuint_dyn;
mod compression;

#[cfg(feature = "helper-api")]
mod helper_api;

pub use api::*;
pub use flaw::*;
pub use indexer::*;
pub use store::*;
pub use transaction::*;
pub use types::*;
pub use updater::*;

#[tokio::main]
pub async fn run() -> Result<(), Box<dyn Error>> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info")
    }
    env_logger::init();

    let database = Arc::new(Mutex::new(Database::new(CONFIG.rocks_db_path.clone())));
    let database_indexer = Arc::clone(&database);

    let indexer_handle = tokio::spawn(async {
        let mut current_indexer = Indexer::new(
            database_indexer,
            CONFIG.btc_rpc_url.clone(),
            CONFIG.btc_rpc_username.clone(),
            CONFIG.btc_rpc_password.clone(),
        )
        .await
        .expect("New indexer");
        current_indexer.run_indexer().await.expect("Run indexer");
    });

    let api_handle = tokio::spawn(async { run_api(database).await });

    indexer_handle.await?;
    let _ = api_handle.await?;

    Ok(())
}
