use config::CONFIG;
use serde::{Deserialize, Serialize};
use std::{error::Error, thread};
use store::database::Database;

mod config;
mod indexer;
mod store;
mod transaction;
mod types;

pub use types::*;

pub fn run() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let handle = thread::spawn(|| {
        let mut current_indexer = indexer::Indexer::new().expect("New indexer");
        current_indexer.run_indexer().expect("Run indexer");
    });

    handle.join().unwrap();

    Ok(())
}
