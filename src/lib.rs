use api::run_api;
use config::CONFIG;
use serde::{Deserialize, Serialize};
use std::{error::Error, sync::Arc, thread};
use store::database::Database;
use tokio::sync::Mutex;

mod api;
mod config;
mod indexer;
mod store;
mod transaction;
mod types;

pub use types::*;

#[tokio::main]
pub async fn run() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let database = Arc::new(Mutex::new(Database::new()));
    let database_indexer = Arc::clone(&database);

    tokio::spawn(async {
        let mut current_indexer =
            indexer::Indexer::new(database_indexer).await.expect("New indexer");
        current_indexer.run_indexer().await.expect("Run indexer");
    });

    run_api(database).await?;

    log::info!("here");

    Ok(())
}
