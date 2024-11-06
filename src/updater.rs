mod mint;

use std::collections::HashMap;

use asset_contract::{AssetContract, InputAsset, PurchaseBurnSwap, VestingPlan};
use bitcoin::{
    hashes::{sha256, Hash},
    key::Secp256k1,
    opcodes,
    script::Instruction,
    secp256k1::{schnorr::Signature, Message},
    Address, Transaction, TxOut, XOnlyPublicKey,
};
use database::{
    DatabaseError, ASSET_CONTRACT_DATA_PREFIX, ASSET_LIST_PREFIX, MESSAGE_PREFIX,
    TRANSACTION_TO_BLOCK_TX_PREFIX, VESTING_CONTRACT_DATA_PREFIX,
};
use flaw::Flaw;
use message::{CallType, ContractType, OpReturnMessage, TxTypeTransfer};

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
    pub list: HashMap<BlockTxString, u128>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MessageDataOutcome {
    pub message: Option<OpReturnMessage>,
    pub flaw: Option<Flaw>,
}
pub struct PBSMintResult {
    pub out_value: u128,
    pub txout: u32,
}

#[derive(Deserialize, Serialize, Clone, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub struct VestingContractData {
    pub claimed_allocations: HashMap<String, u128>,
}

pub struct Updater {
    pub database: Arc<Mutex<Database>>,
    is_read_only: bool,

    unallocated_asset_list: AssetList,
    allocated_asset_list: HashMap<u32, AssetList>,
}

impl Updater {
    pub async fn new(database: Arc<Mutex<Database>>, is_read_only: bool) -> Self {
        Updater {
            database,
            is_read_only,

            unallocated_asset_list: AssetList::default(),
            allocated_asset_list: HashMap::new(),
        }
    }

    pub async fn unallocate_asset(&mut self, tx: &Transaction) -> Result<(), Box<dyn Error>> {
        for tx_input in tx.input.iter() {
            let outpoint = &Outpoint {
                txid: tx_input.previous_output.txid.to_string(),
                vout: tx_input.previous_output.vout,
            };

            if let Ok(asset_list) = self.get_asset_list(outpoint).await {
                for asset in asset_list.list.iter() {
                    let previous_amount =
                        self.unallocated_asset_list.list.get(asset.0).unwrap_or(&0);
                    self.unallocated_asset_list.list.insert(
                        asset.0.to_string(),
                        previous_amount.saturating_add(*asset.1),
                    );
                }
            }

            // TODO: Implement a backup mechanism to recover when downtime occurs
            self.delete_asset(outpoint).await;
        }

        Ok(())
    }

    pub async fn allocate_new_asset(
        &mut self,
        vout: u32,
        contract_id: &BlockTxTuple,
        amount: u128,
    ) {
        let block_tx = BlockTx::from_tuple(*contract_id);

        let asset = self
            .allocated_asset_list
            .entry(vout)
            .or_insert_with(AssetList::default);

        let previous_amount = asset.list.entry(block_tx.to_str()).or_insert(0);
        *previous_amount = previous_amount.saturating_add(amount);
    }

    pub async fn move_allocation(
        &mut self,
        vout: u32,
        contract_id: &BlockTxTuple,
        max_amount: u128,
    ) {
        let block_tx = BlockTx::from_tuple(*contract_id);
        let Some(asset) = self
            .unallocated_asset_list
            .list
            .get_mut(&block_tx.to_string())
        else {
            return;
        };

        let amount = max_amount.min(*asset);
        if amount == 0 {
            return;
        }

        *asset = asset.saturating_sub(amount);
        if *asset == 0 {
            self.unallocated_asset_list
                .list
                .remove(&block_tx.to_string());
        }

        self.allocate_new_asset(vout, contract_id, amount).await;
    }

    pub async fn commit_asset(&mut self, tx: &Transaction) -> Result<(), Box<dyn Error>> {
        // move unallocated to first non op_return index (fallback)
        let list = self.unallocated_asset_list.list.clone();
        for asset in list.iter() {
            let block_tx = BlockTx::from_str(asset.0);

            if let Some(vout) = self.first_non_op_return_index(tx) {
                self.move_allocation(vout, &block_tx.to_tuple(), *asset.1)
                    .await;
            } else {
                log::info!("No non op_return index found, unallocated asset is lost");
            }
        }

        let txid = tx.compute_txid().to_string();
        for asset in self.allocated_asset_list.iter() {
            let outpoint = &Outpoint {
                txid: txid.clone(),
                vout: *asset.0,
            };

            self.set_asset_list(outpoint, asset.1).await;
        }

        // reset asset list
        self.unallocated_asset_list = AssetList::default();
        self.allocated_asset_list = HashMap::new();

        Ok(())
    }

    fn is_op_return_index(&self, output: &TxOut) -> bool {
        let mut instructions = output.script_pubkey.instructions();
        if instructions.next() == Some(Ok(Instruction::Op(opcodes::all::OP_RETURN))) {
            return true;
        }

        return false;
    }

    fn first_non_op_return_index(&self, tx: &Transaction) -> Option<u32> {
        for (i, output) in tx.output.iter().enumerate() {
            if !self.is_op_return_index(output) {
                return Some(i as u32);
            };
        }

        return None;
    }

    // run modules here
    pub async fn index(
        &mut self,
        block_height: u64,
        tx_index: u32,
        tx: &Transaction,
        message_result: Result<OpReturnMessage, Flaw>,
    ) -> Result<MessageDataOutcome, Box<dyn Error>> {
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
                outcome.flaw = Some(flaw)
            }

            if let Some(transfer) = message.transfer {
                if outcome.flaw.is_none() {
                    outcome.flaw = self.transfers(tx, transfer.transfers).await;
                }
            }

            if let Some(contract_creation) = message.contract_creation {
                if let ContractType::Asset(asset_contract) = contract_creation.contract_type {
                    if let Some(purchase) = asset_contract.distribution_schemes.purchase {
                        if let InputAsset::GlittrAsset(block_tx_tuple) = purchase.input_asset {
                            let message = self.get_message(&block_tx_tuple).await;

                            if let Some(message) = message.ok() {
                                if message.contract_creation.is_none() && outcome.flaw.is_none() {
                                    outcome.flaw = Some(Flaw::ReferencingFlawedBlockTx)
                                }
                            } else {
                                if outcome.flaw.is_none() {
                                    outcome.flaw = Some(Flaw::ReferencingFlawedBlockTx);
                                }
                            }
                        }
                    }
                }
            }

            if let Some(contract_call) = message.contract_call {
                match contract_call.call_type {
                    CallType::Mint(mint_option) => {
                        if outcome.flaw.is_none() {
                            outcome.flaw = self
                                .mint(tx, block_tx, &contract_call.contract, &mint_option)
                                .await;
                        }
                    }
                    CallType::Burn => {
                        log::info!("Process call type burn");
                    }
                    CallType::Swap => {
                        log::info!("Process call type swap");
                    }
                }
            }
        } else {
            outcome.flaw = Some(message_result.unwrap_err());
        }

        if !self.is_read_only {
            log::info!(
                "# Outcome {:?}, {:?} at {}",
                outcome.flaw,
                outcome.message,
                block_tx
            );
            self.database.lock().await.put(
                MESSAGE_PREFIX,
                block_tx.to_string().as_str(),
                outcome.clone(),
            );

            self.database.lock().await.put(
                TRANSACTION_TO_BLOCK_TX_PREFIX,
                tx.compute_txid().to_string().as_str(),
                block_tx.to_tuple(),
            );
        }

        Ok(outcome)
    }

    pub async fn transfers(
        &mut self,
        tx: &Transaction,
        transfers: Vec<TxTypeTransfer>,
    ) -> Option<Flaw> {
        let mut overflow_i = Vec::new();

        for (i, transfer) in transfers.iter().enumerate() {
            if transfer.output >= tx.output.len() as u32 {
                overflow_i.push(i as u32);
                continue;
            }
            self.move_allocation(transfer.output, &transfer.asset, transfer.amount.0)
                .await;
        }

        if overflow_i.len() > 0 {
            return Some(Flaw::OutputOverflow(overflow_i));
        }

        None
    }

    async fn delete_asset(&self, outpoint: &Outpoint) {
        if !self.is_read_only {
            self.database
                .lock()
                .await
                .delete(ASSET_LIST_PREFIX, &outpoint.to_string());
        }
    }

    pub async fn get_asset_list(&self, outpoint: &Outpoint) -> Result<AssetList, Flaw> {
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
        if !self.is_read_only {
            self.database
                .lock()
                .await
                .put(ASSET_LIST_PREFIX, &outpoint.to_string(), asset_list);
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

    pub async fn get_asset_contract_data(
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
        if !self.is_read_only {
            let contract_key = BlockTx::from_tuple(*contract_id).to_string();
            self.database.lock().await.put(
                ASSET_CONTRACT_DATA_PREFIX,
                &contract_key,
                asset_contract_data,
            );
        }
    }

    pub async fn get_vesting_contract_data(
        &self,
        contract_id: &BlockTxTuple,
    ) -> Result<VestingContractData, Flaw> {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let data: Result<VestingContractData, DatabaseError> = self
            .database
            .lock()
            .await
            .get(VESTING_CONTRACT_DATA_PREFIX, &contract_key);

        match data {
            Ok(data) => Ok(data),
            Err(DatabaseError::NotFound) => Ok(VestingContractData::default()),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    async fn set_vesting_contract_data(
        &self,
        contract_id: &BlockTxTuple,
        vesting_contract_data: &VestingContractData,
    ) {
        if !self.is_read_only {
            let contract_key = BlockTx::from_tuple(*contract_id).to_string();
            self.database.lock().await.put(
                VESTING_CONTRACT_DATA_PREFIX,
                &contract_key,
                vesting_contract_data,
            );
        }
    }
}
