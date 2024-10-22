use super::*;

use bitcoincore_rpc::{Auth, Client, RpcApi};
use std::{error::Error, time::Duration};
use store::database::INDEXER_LAST_BLOCK_PREFIX;
use transaction::message::OpReturnMessage;

pub struct Indexer {
    rpc: Client,
    database: Database,
    last_indexed_block: Option<u64>,
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

        let last_indexed_block: Option<u64> = database.get(INDEXER_LAST_BLOCK_PREFIX, "");

        Ok(Indexer {
            last_indexed_block, // Todo: change this to rocksdb
            database,
            rpc,
        })
    }

    pub fn run_indexer(&mut self) -> Result<(), Box<dyn Error>> {
        loop {
            let current_block_tip = self.rpc.get_block_count()?;

            while self.last_indexed_block.is_none()
                || self.last_indexed_block.unwrap() < current_block_tip
            {
                let block_height = match self.last_indexed_block {
                    Some(value) => value + 1,
                    None => 0,
                };

                let block_hash = self.rpc.get_block_hash(block_height)?;
                let block = self.rpc.get_block(&block_hash)?;

                log::info!("Indexing block {}: {}", block_height, block_hash);
                println!("Indexing block {}: {}", block_height, block_hash);

                for (pos, tx) in block.txdata.iter().enumerate() {
                    // run modules here
                    let _ = OpReturnMessage::parse_tx(tx).map(|value| {
                        if value.validate() {
                            value
                                .put(
                                    &mut self.database,
                                    BlockTx {
                                        block: block_height,
                                        tx: pos as u32,
                                    },
                                )
                                .unwrap();
                        }
                    });
                }

                self.last_indexed_block = Some(block_height);
            }

            self.database
                .put(INDEXER_LAST_BLOCK_PREFIX, "", self.last_indexed_block.unwrap())?;

            thread::sleep(Duration::from_secs(10));
        }
    }
}
