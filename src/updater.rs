use asset_contract::{AssetContract, AssetContractFreeMint, InputAsset};
use bitcoin::Transaction;
use database::{
    DatabaseError, MESSAGE_PREFIX, MINT_DATA_PREFIX, MINT_OUTPUT_PREFIX,
    TRANSACTION_TO_BLOCK_TX_PREFIX,
};
use flaw::Flaw;
use message::{CallType, ContractType, OpReturnMessage, TxType};

use super::*;

#[derive(Deserialize, Serialize, Clone, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MintData {
    minted: u32,
    burned: u32,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct MessageDataOutcome {
    message: Option<OpReturnMessage>,
    flaw: Option<Flaw>,
}

pub struct Updater {
    pub database: Arc<Mutex<Database>>,
}

impl Updater {
    pub async fn new(database: Arc<Mutex<Database>>) -> Self {
        Updater { database }
    }

    // run modules here
    pub async fn index(
        &mut self,
        block_height: u64,
        tx_index: u32,
        tx: &Transaction,
        message_result: Result<OpReturnMessage, Flaw>,
    ) -> Result<(), Box<dyn Error>> {
        let mut outcome = MessageDataOutcome {
            message: None,
            flaw: None,
        };

        let block_tx = &BlockTx {
            block: block_height,
            tx: tx_index,
        };

        if let Ok(message) = message_result {
            outcome.message = Some(message.clone());
            if let Some(flaw) = message.validate() {
                outcome.flaw = Some(flaw);
            } else {
                outcome.flaw = match message.tx_type {
                    TxType::Transfer {
                        asset: _,
                        n_outputs: _,
                        amounts: _,
                    } => {
                        log::info!("Process transfer");
                        None
                    }
                    TxType::ContractCreation { contract_type } => {
                        log::info!("Process contract creation");
                        if let ContractType::Asset(asset_contract) = contract_type {
                            if let AssetContract::PurchaseBurnSwap { input_asset, .. } =
                                asset_contract
                            {
                                if let InputAsset::GlittrAsset(block_tx_tuple) = input_asset {
                                    if let Some(tx_type) =
                                        self.get_message_txtype(block_tx_tuple).await.ok()
                                    {
                                        match tx_type {
                                            TxType::ContractCreation { .. } => None,
                                            _ => Some(Flaw::ReferencingFlawedBlockTx),
                                        };
                                    }
                                } else if let InputAsset::Rune(_block_tx_tuple) = input_asset {
                                    // NOTE: design decision, IMO we shouldn't check if rune exist as validation
                                    // since rune is a separate meta-protocol
                                    // validating rune is exist / not here means our core must index runes
                                }
                            }
                        }
                        None
                    }
                    TxType::ContractCall {
                        contract,
                        call_type,
                    } => match call_type {
                        CallType::Mint => {
                            self.mint(tx, contract).await;
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
            outcome.flaw = Some(message_result.unwrap_err());
        }

        self.database
            .lock()
            .await
            .put(MESSAGE_PREFIX, block_tx.to_string().as_str(), outcome)?;

        self.database.lock().await.put(
            TRANSACTION_TO_BLOCK_TX_PREFIX,
            tx.compute_txid().to_string().as_str(),
            block_tx.to_tuple(),
        )?;

        Ok(())
    }

    async fn get_message_txtype(&self, block_tx: BlockTxTuple) -> Result<TxType, Flaw> {
        let outcome: MessageDataOutcome = self
            .database
            .lock()
            .await
            .get(
                MESSAGE_PREFIX,
                BlockTx {
                    block: block_tx.0,
                    tx: block_tx.1,
                }
                .to_string()
                .as_str(),
            )
            .unwrap();

        if outcome.flaw.is_some() {
            return Err(Flaw::ReferencingFlawedBlockTx);
        } else {
            return Ok(outcome.message.unwrap().tx_type);
        }
    }

    async fn get_message(&self, contract_id: &BlockTxTuple) -> Result<OpReturnMessage, Flaw> {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let outcome: Result<MessageDataOutcome, DatabaseError> = self
            .database
            .lock()
            .await
            .get(MESSAGE_PREFIX, &contract_key);

        match outcome {
            Ok(outcome) => {
                if let Some(flaw) = outcome.flaw {
                    Err(flaw)
                } else {
                    outcome.message.ok_or(Flaw::MessageInvalid)
                }
            }
            Err(DatabaseError::NotFound) => Err(Flaw::ContractNotFound),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    async fn mint(&mut self, tx: &Transaction, contract_id: BlockTxTuple) -> Option<Flaw> {
        let message = self.get_message(&contract_id).await;
        match message {
            Ok(op_return_message) => match op_return_message.tx_type {
                TxType::ContractCreation { contract_type } => match contract_type {
                    message::ContractType::Asset(asset) => match asset {
                        AssetContract::FreeMint(free_mint) => {
                            self.mint_free_mint(free_mint, tx, &contract_id).await
                        }
                        // TODO: add others asset types
                        _ => Some(Flaw::ContractNotMatch),
                    },
                },
                _ => Some(Flaw::ContractNotMatch),
            },
            Err(flaw) => Some(flaw),
        }
    }

    async fn get_mint_data(&self, contract_id: &BlockTxTuple) -> Result<MintData, Flaw> {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let result: Result<MintData, DatabaseError> = self
            .database
            .lock()
            .await
            .get(MINT_DATA_PREFIX, &contract_key);

        match result {
            Ok(data) => Ok(data),
            Err(DatabaseError::NotFound) => Ok(MintData::default()),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    async fn set_mint_output(
        &self,
        contract_id: &BlockTxTuple,
        tx_id: &str,
        n_output: u32,
    ) -> Option<Flaw> {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let key = format!("{}:{}:{}", contract_key, tx_id, n_output.to_string());
        let result = self.database.lock().await.put(MINT_OUTPUT_PREFIX, &key, 1);
        if result.is_err() {
            return Some(Flaw::WriteError);
        }
        None
    }

    async fn update_mint_data(
        &self,
        contract_id: &BlockTxTuple,
        mint_data: &MintData,
    ) -> Option<Flaw> {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let result = self
            .database
            .lock()
            .await
            .put(MINT_DATA_PREFIX, &contract_key, mint_data);
        if result.is_err() {
            return Some(Flaw::WriteError);
        }
        None
    }

    // TODO:
    // - add pointer to mint, specify wich output index for the mint receiver
    // - current default index is 0
    async fn mint_free_mint(
        &self,
        asset: AssetContractFreeMint,
        tx: &Transaction,
        contract_id: &BlockTxTuple,
    ) -> Option<Flaw> {
        let mut mint_data = match self.get_mint_data(contract_id).await {
            Ok(data) => data,
            Err(flaw) => return Some(flaw),
        };

        // check the supply
        if let Some(supply_cap) = asset.supply_cap {
            let next_supply = mint_data
                .minted
                .saturating_mul(asset.amount_per_mint)
                .saturating_add(asset.amount_per_mint);

            if next_supply > supply_cap {
                return Some(Flaw::SupplyCapExceeded);
            }
        }
        mint_data.minted = mint_data.minted.saturating_add(1);

        // set the outpoint
        let default_n_pointer = 0;
        let flaw = self
            .set_mint_output(
                contract_id,
                tx.compute_txid().to_string().as_str(),
                default_n_pointer,
            )
            .await;
        if flaw.is_some() {
            return flaw;
        }

        // update the mint data
        self.update_mint_data(contract_id, &mint_data).await
    }
}
