mod mint;

use std::collections::HashMap;

use asset_contract::{AssetContract, InputAsset, PurchaseBurnSwap};
use bitcoin::{
    hashes::{sha256, Hash},
    key::Secp256k1,
    opcodes,
    script::Instruction,
    secp256k1::{schnorr::Signature, Message},
    Address, Transaction, XOnlyPublicKey,
};
use database::{
    DatabaseError, ASSET_CONTRACT_DATA_PREFIX, ASSET_LIST_PREFIX, MESSAGE_PREFIX,
    TRANSACTION_TO_BLOCK_TX_PREFIX,
};
use flaw::Flaw;
use message::{CallType, ContractType, OpReturnMessage, TxType};

use super::*;

#[derive(Deserialize, Serialize, Clone, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub struct AssetContractData {
    pub minted_supply: u128,
    pub burned_supply: u128,
}

#[derive(Deserialize, Serialize, Clone, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub struct AssetList {
    pub list: HashMap<String, u32>,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct MessageDataOutcome {
    pub message: Option<OpReturnMessage>,
    pub flaw: Option<Flaw>,
}
pub struct MintResult {
    pub out_value: u128,
    pub txout: u32,
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
                            if let Some(purchase) = asset_contract.distribution_schemes.purchase {
                                if let InputAsset::GlittrAsset(block_tx_tuple) =
                                    purchase.input_asset
                                {
                                    if let Ok(tx_type) =
                                        self.get_message_txtype(block_tx_tuple).await
                                    {
                                        match tx_type {
                                            TxType::ContractCreation { .. } => None,
                                            _ => Some(Flaw::ReferencingFlawedBlockTx),
                                        };
                                    }
                                }
                            }
                        }
                        None
                    }
                    TxType::ContractCall {
                        contract,
                        call_type,
                    } => match call_type {
                        CallType::Mint(mint_option) => {
                            self.mint(tx, block_tx, &contract, &mint_option).await
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
            .put(MESSAGE_PREFIX, block_tx.to_string().as_str(), outcome);

        self.database.lock().await.put(
            TRANSACTION_TO_BLOCK_TX_PREFIX,
            tx.compute_txid().to_string().as_str(),
            block_tx.to_tuple(),
        );

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
            Err(Flaw::ReferencingFlawedBlockTx)
        } else {
            Ok(outcome.message.unwrap().tx_type)
        }
    }

    async fn get_asset_list(&self, outpoint: &Outpoint) -> Result<AssetList, Flaw> {
        let result: Result<AssetList, DatabaseError> = self
            .database
            .lock()
            .await
            .get(ASSET_LIST_PREFIX, &outpoint.to_string());

        match result {
            Ok(data) => Ok(data),
            Err(DatabaseError::NotFound) => Ok(AssetList::default()),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    async fn set_asset_list(&self, outpoint: &Outpoint, asset_list: &AssetList) {
        self.database
            .lock()
            .await
            .put(ASSET_LIST_PREFIX, &outpoint.to_string(), asset_list);
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

    async fn get_asset_contract_data(
        &self,
        contract_id: &BlockTxTuple,
    ) -> Result<AssetContractData, Flaw> {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let data: Result<AssetContractData, DatabaseError> = self
            .database
            .lock()
            .await
            .get(ASSET_CONTRACT_DATA_PREFIX, &contract_key);

        match data {
            Ok(data) => Ok(data),
            Err(DatabaseError::NotFound) => Ok(AssetContractData::default()),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    async fn set_asset_contract_data(
        &self,
        contract_id: &BlockTxTuple,
        asset_contract_data: &AssetContractData,
    ) {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        self.database.lock().await.put(
            ASSET_CONTRACT_DATA_PREFIX,
            &contract_key,
            asset_contract_data,
        );
    }
}
