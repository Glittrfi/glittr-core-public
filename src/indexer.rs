use super::*;

use bitcoincore_rpc::{Auth, Client, RpcApi};
use constants::first_glittr_height;
use std::{error::Error, time::Duration};
use store::database::{INDEXER_LAST_BLOCK_PREFIX, MESSAGE_PREFIX, TRANSACTION_TO_BLOCK_TX_PREFIX};
use transaction::message::OpReturnMessage;

pub struct Indexer {
    rpc: Client,
    database: Arc<Mutex<Database>>,
    last_indexed_block: Option<u64>,
}

impl Indexer {
    pub async fn new(database: Arc<Mutex<Database>>) -> Result<Self, Box<dyn Error>> {
        log::info!("Indexer initiated");
        let rpc = Client::new(
            CONFIG.btc_rpc_url.as_str(),
            Auth::UserPass(
                CONFIG.btc_rpc_username.clone(),
                CONFIG.btc_rpc_password.clone(),
            ),
        )?;

        let mut last_indexed_block: Option<u64> = database.lock().await.get(INDEXER_LAST_BLOCK_PREFIX, "").ok();
        if last_indexed_block.is_none() && first_glittr_height() > 0 {
            last_indexed_block = Some(first_glittr_height() - 1)
        }

        Ok(Indexer {
            last_indexed_block,
            database,
            rpc,
        })
    }

    pub async fn run_indexer(&mut self) -> Result<(), Box<dyn Error>> {
        log::info!("Indexer start");
        loop {
            let current_block_tip = self.rpc.get_block_count()?;

            let first_block_height = first_glittr_height();
            if current_block_tip < first_block_height {
                thread::sleep(Duration::from_secs(10));
                continue;
            }

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

                self.last_indexed_block = Some(block_height);
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
