#![allow(irrefutable_let_patterns)]

use config::CONFIG;
use serde::{Deserialize, Serialize};
use std::{error::Error, sync::Arc, thread};
use store::database::Database;
use tokio::sync::Mutex;

mod api;
mod indexer;
mod store;
mod transaction;
mod types;
mod config;
mod constants;
mod updater;
mod flaw;

pub use api::*;
pub use updater::*;
pub use indexer::*;
pub use store::*;
pub use transaction::*;
pub use types::*;

#[tokio::main]
pub async fn run() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let database = Arc::new(Mutex::new(Database::new(CONFIG.rocks_db_path.clone())));
    let database_indexer = Arc::clone(&database);

    let indexer_handle = tokio::spawn(async {
        let mut current_indexer = indexer::Indexer::new(database_indexer,             CONFIG.btc_rpc_url.clone(),
        CONFIG.btc_rpc_username.clone(),
        CONFIG.btc_rpc_password.clone())
            .await
            .expect("New indexer");
        current_indexer.run_indexer().await.expect("Run indexer");
    });

    let api_handle = tokio::spawn(async { run_api(database).await });

    indexer_handle.await?;
    let _ = api_handle.await?;

    Ok(())
}
