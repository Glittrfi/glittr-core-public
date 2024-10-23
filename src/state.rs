use bitcoin::Transaction;
use database::{MESSAGE_PREFIX, TRANSACTION_TO_BLOCK_TX_PREFIX};
use flaw::Flaw;
use message::{CallType, OpReturnMessage, TxType};

use super::*;

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct StateData {
    message: Option<OpReturnMessage>,
    flaw: Option<Flaw>,
}

pub struct State {
    pub database: Arc<Mutex<Database>>,
}

impl State {
    pub async fn new(database: Arc<Mutex<Database>>) -> Self {
        State { database }
    }

    // run modules here
    pub async fn index(
        &mut self,
        block_height: u64,
        tx_index: u32,
        tx: &Transaction,
        message_result: Result<OpReturnMessage, Flaw>,
    ) -> Result<(), Box<dyn Error>> {
        let mut state_data = StateData {
            message: None,
            flaw: None,
        };

        let block_tx = &BlockTx {
            block: block_height,
            tx: tx_index,
        };

        if let Ok(message) = message_result {
            state_data.message = Some(message.clone());
            if let Some(flaw) = message.validate() {
                state_data.flaw = Some(flaw);
            } else {
                state_data.flaw = match message.tx_type {
                    TxType::Transfer {
                        asset: _,
                        n_outputs: _,
                        amounts: _,
                    } => {
                        log::info!("Process transfer");
                        None
                    }
                    TxType::ContractCreation { contract_type: _ } => {
                        log::info!("Process contract creation");
                        None
                    }
                    TxType::ContractCall {
                        contract,
                        call_type,
                    } => match call_type {
                        CallType::Mint => {
                            self.mint(tx, block_tx, contract).await;
                            None
                        }
                        CallType::Burn => {
                            log::info!("Process call type burn");
                            None
                        }
                        CallType::Swap => {
                            log::info!("Process call type swap");
                            None
                        }
                    },
                }
            }
        } else {
            state_data.flaw = Some(message_result.unwrap_err());
        }

        self.database.lock().await.put(
            MESSAGE_PREFIX,
            block_tx.to_string().as_str(),
            state_data,
        )?;

        self.database.lock().await.put(
            TRANSACTION_TO_BLOCK_TX_PREFIX,
            tx.compute_txid().to_string().as_str(),
            block_tx.to_tuple(),
        )?;

        Ok(())
    }

    // TODO:
    // - add pointer to mint, specify wich output index for the mint receiver
    // - current default index is first non op_return index
    async fn mint(&mut self, _: &Transaction, _: &BlockTx, _: BlockTxTuple) {}
}
