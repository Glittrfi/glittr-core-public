use super::*;

use bitcoincore_rpc::{Auth, Client, RpcApi};
use modules::dummy_glyph::DummyGlyph;
use std::{error::Error, time::Duration};
use store::database::INDEXER_LAST_BLOCK_PREFIX;

pub struct Indexer {
    rpc: Client,
    database: Database,
    last_indexed_block: u64,
}

impl Indexer {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let rpc = Client::new(
            CONFIG.btc_rpc_url.as_str(),
            Auth::UserPass(
                CONFIG.btc_rpc_username.clone(),
                CONFIG.btc_rpc_password.clone(),
            ),
        )?;

        let database = Database::new();

        let last_indexed_block: u64 = database.get(INDEXER_LAST_BLOCK_PREFIX, "").unwrap_or(0);

        Ok(Indexer {
            last_indexed_block, // Todo: change this to rocksdb
            database,
            rpc,
        })
    }

    pub fn run_indexer(&mut self) -> Result<(), Box<dyn Error>> {
        loop {
            let best_block_height = self.rpc.get_block_count()?;

            while self.last_indexed_block < best_block_height {
                let block_height = self.last_indexed_block + 1;
                let block_hash = self.rpc.get_block_hash(block_height)?;
                let block = self.rpc.get_block(&block_hash)?;

                log::info!("Indexing block {}: {}", block_height, block_hash);

                for (pos, tx) in block.txdata.iter().enumerate() {
                    // run modules here
                    let _ = DummyGlyph::parse_tx(tx).map(|value| {
                        value.put(&mut self.database, format!("{}:{}", block_height, pos))
                    });
                }

                self.last_indexed_block += 1;
            }

            self.database
                .put(INDEXER_LAST_BLOCK_PREFIX, "", self.last_indexed_block)?;

            thread::sleep(Duration::from_secs(10));
        }
    }
}
