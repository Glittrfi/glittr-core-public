#![allow(irrefutable_let_patterns)]

use config::CONFIG;
use serde::{Deserialize, Serialize};
use std::{env, error::Error, process::exit, sync::Arc};
use store::database::Database;
use tokio::sync::Mutex;

mod api;
mod borsh_serde;
mod compression;
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
pub mod varint;
pub mod az_base26;

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

    log::info!("Glittr core starting");

    // Validate bitcoin network
    match CONFIG.bitcoin_network.as_str() {
        "mainnet" => {}
        "testnet" => {}
        "signet" => {}
        "regtest" => {}
        notexist => {
            panic!("Invalid bitcoin network in setting: {notexist}");
        }
    }

    let database = Arc::new(Mutex::new(Database::new(CONFIG.rocks_db_path.clone())));
    let database_indexer = Arc::clone(&database);

    // Add Ctrl+C handling
    let shutdown_signal = Arc::new(Mutex::new(false));
    let shutdown_signal_indexer = Arc::clone(&shutdown_signal);

    ctrlc::set_handler(move || {
        if *shutdown_signal.blocking_lock() {
            log::warn!("Exiting");
            exit(1);
        }
        log::warn!("Ctrl+C pressed, waiting for the process to gracefully exit. Ctrl+C again to force exit");
        *shutdown_signal.blocking_lock() = true;
    })
    .expect("Error setting Ctrl+C handler");

    let indexer_handle = tokio::spawn(async {
        let mut current_indexer = Indexer::new(
            database_indexer,
            CONFIG.btc_rpc_url.clone(),
            CONFIG.btc_rpc_username.clone(),
            CONFIG.btc_rpc_password.clone(),
        )
        .await
        .expect("New indexer");

        let indexer_runner = current_indexer.run_indexer(shutdown_signal_indexer).await;
        match indexer_runner {
            Ok(_) => {}
            Err(error) => {
                if error.to_string().contains("401") {
                    log::error!("Bitcoin RPC username or password incorrect.");
                } else if error.to_string().contains("Connection refused") {
                    log::error!("Connection to Bitcoin RPC URL failed.");
                }
                panic!("Error message: {error:?}");
            }
        }
    });

    let api_handle = tokio::spawn(async {
        run_api(database)
            .await
            .expect("Run API")
    });

    tokio::select! {
        result = indexer_handle => {
            log::warn!("Indexer exited {:?}", result);
        }, 
        result = api_handle => {
            log::warn!("API exited prematurely {:?}", result);
        }
    }

    Ok(())
}
