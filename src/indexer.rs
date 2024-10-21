use super::*;

use bitcoincore_rpc::{Auth, Client, RpcApi};
use std::{error::Error, time::Duration};
use store::database::{INDEXER_LAST_BLOCK_PREFIX, MESSAGE_PREFIX, TRANSACTION_TO_BLOCK_TX_PREFIX};
use transaction::message::OpReturnMessage;

pub struct Indexer {
    rpc: Client,
    database: Arc<Mutex<Database>>,
    last_indexed_block: u64,
}

impl Indexer {
    pub async fn new(database: Arc<Mutex<Database>>) -> Result<Self, Box<dyn Error>> {
        log::info!("Indexer start");
        let rpc = Client::new(
            CONFIG.btc_rpc_url.as_str(),
            Auth::UserPass(
                CONFIG.btc_rpc_username.clone(),
                CONFIG.btc_rpc_password.clone(),
            ),
        )?;

        let last_indexed_block: u64 = database
            .lock()
            .await
            .get(INDEXER_LAST_BLOCK_PREFIX, "")
            .unwrap_or(0);

        Ok(Indexer {
            last_indexed_block, // Todo: change this to rocksdb
            database,
            rpc,
        })
    }

    pub async fn run_indexer(&mut self) -> Result<(), Box<dyn Error>> {
        log::info!("Indexer start");
        loop {
            let best_block_height = self.rpc.get_block_count()?;

            while self.last_indexed_block < best_block_height {
                let block_height = self.last_indexed_block + 1;
                let block_hash = self.rpc.get_block_hash(block_height)?;
                let block = self.rpc.get_block(&block_hash)?;

                log::info!("Indexing block {}: {}", block_height, block_hash);

                for (pos, tx) in block.txdata.iter().enumerate() {
                    // run modules here
                    let blocktx = BlockTx {
                        block: block_height,
                        tx: pos as u32,
                    };
                    let message = OpReturnMessage::parse_tx(tx).ok();
                    if let Some(message) = message {
                        if message.validate() {
                            self.database.lock().await.put(
                                MESSAGE_PREFIX,
                                blocktx.to_string().as_str(),
                                message,
                            )?;

                            self.database.lock().await.put(
                                TRANSACTION_TO_BLOCK_TX_PREFIX,
                                tx.compute_txid().to_string().as_str(),
                                blocktx.to_tuple(),
                            )?;
                        }
                    }
                }

                self.last_indexed_block += 1;
            }

            self.database.lock().await.put(
                INDEXER_LAST_BLOCK_PREFIX,
                "",
                self.last_indexed_block,
            )?;

            thread::sleep(Duration::from_secs(10));
        }
    }
}
