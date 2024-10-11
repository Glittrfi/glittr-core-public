use config::CONFIG;
use std::{error::Error, thread};
use store::database::Database;

mod config;
mod indexer;
mod modules;
mod store;

pub fn run() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let handle = thread::spawn(|| {
        let mut current_indexer = indexer::Indexer::new().expect("New indexer");
        current_indexer.run_indexer().expect("Run indexer");
    });

    handle.join().unwrap();

    Ok(())
}
