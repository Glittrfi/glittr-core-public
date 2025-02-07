mod burn;
mod collateralized;
mod mint;
mod updater_shared;

use api::MintType;
use collateralized::CollateralizedAssetData;
pub use updater_shared::*;
mod spec;

use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use bitcoin::{
    hashes::{sha256, Hash},
    key::Secp256k1,
    opcodes,
    script::Instruction,
    secp256k1::{schnorr::Signature, Message},
    Address, OutPoint, Transaction, TxOut, XOnlyPublicKey,
};
use database::{
    DatabaseError, ASSET_CONTRACT_DATA_PREFIX, ASSET_LIST_PREFIX, COLLATERALIZED_CONTRACT_DATA,
    INDEXER_LAST_BLOCK_PREFIX, MESSAGE_PREFIX, STATE_KEYS_PREFIX, TICKER_TO_BLOCK_TX_PREFIX,
    TRANSACTION_TO_BLOCK_TX_PREFIX, VESTING_CONTRACT_DATA_PREFIX,
};
use flaw::Flaw;
use message::{CallType, ContractType, OpReturnMessage, TxTypeTransfer};
use mint_only_asset::MintOnlyAssetContract;
use transaction_shared::{InputAsset, PurchaseBurnSwap, VestingPlan};

use super::*;

#[derive(Deserialize, Serialize, Clone, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub struct AssetContractData {
    pub minted_supply: u128,
    pub minted_supply_by_freemint: u128,
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

#[derive(Serialize, Deserialize, Debug, Clone, Default, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct CollateralAccounts {
    pub collateral_accounts: HashMap<BlockTxString, CollateralAccount>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, Eq, Hash, PartialEq)]
pub struct CollateralAccount {
    pub collateral_amounts: Vec<(BlockTxTuple, u128)>,
    // TODO: remove total_collateral_amount
    pub total_collateral_amount: u128,
    pub ltv: Fraction, // ltv = total_amount_used / total_collateral_amount (in lending amount)
    pub amount_outstanding: u128,
    pub share_amount: u128, // TODO: implement
}

// TODO: statekey should be general, could accept dynamic value for the key value
#[derive(Serialize, Deserialize, Default, Eq, PartialEq, Debug)]
pub struct StateKeys {
    pub contract_ids: HashSet<BlockTxTuple>,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct SpecContractOwned {
    pub specs: HashSet<BlockTxTuple>,
}
#[cfg(feature = "helper-api")]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct UTXOBalances {
    pub txid: String,
    pub vout: u32,
    pub assets: HashMap<BlockTxString, U128>,
}

#[cfg(feature = "helper-api")]
#[derive(Deserialize, Serialize, Clone, Default, Debug)]
pub struct AddressAssetList {
    pub summarized: HashMap<BlockTxString, U128>,
    pub utxos: Vec<UTXOBalances>,
}

#[derive(Default)]
pub struct Allocation {
    asset_list: AssetList,
    spec_owned: SpecContractOwned,
    state_keys: StateKeys,
    collateral_accounts: CollateralAccounts,

    helper_outpoint_collateral_accounts: HashMap<CollateralAccount, OutPoint>,
}

pub struct Updater {
    pub database: Arc<Mutex<Database>>,
    is_read_only: bool,

    unallocated_inputs: Allocation,
    allocated_outputs: HashMap<u32, Allocation>,
}

impl Updater {
    impl_ops_for_outpoint_data!(AssetList);

    pub async fn new(database: Arc<Mutex<Database>>, is_read_only: bool) -> Self {
        Updater {
            database,
            is_read_only,

            unallocated_inputs: Allocation::default(),
            allocated_outputs: HashMap::new(),
        }
    }

    pub async fn unallocate_inputs(&mut self, tx: &Transaction) -> Result<(), Box<dyn Error>> {
        for tx_input in tx.input.iter() {
            let outpoint = &OutPoint {
                txid: tx_input.previous_output.txid,
                vout: tx_input.previous_output.vout,
            };

            // set asset_list
            if let Ok(asset_list) = self.get_asset_list(outpoint).await {
                for asset in asset_list.list.iter() {
                    let previous_amount = self
                        .unallocated_inputs
                        .asset_list
                        .list
                        .get(asset.0)
                        .unwrap_or(&0);
                    self.unallocated_inputs.asset_list.list.insert(
                        asset.0.to_string(),
                        previous_amount.saturating_add(*asset.1),
                    );
                }

                self.delete_asset_list(outpoint).await;
            }

            // set specs
            if let Ok(spec_contract_owned) = self.get_spec_contract_owned(outpoint).await {
                for contract in spec_contract_owned.specs.iter() {
                    self.unallocated_inputs.spec_owned.specs.insert(*contract);
                }

                self.delete_spec_contract_owned(outpoint).await
            }

            // set state_keys
            if let Ok(state_keys) = self.get_state_keys(outpoint).await {
                self.unallocated_inputs.state_keys.contract_ids = self
                    .unallocated_inputs
                    .state_keys
                    .contract_ids
                    .union(&state_keys.contract_ids)
                    .cloned()
                    .collect();
                self.delete_state_keys(outpoint).await
            }

            // set collateral_account
            if let Ok(collateral_accounts) = self.get_collateral_accounts(outpoint).await {
                for (contract_id, collateral_account) in collateral_accounts.collateral_accounts {
                    self.unallocated_inputs
                        .collateral_accounts
                        .collateral_accounts
                        .insert(contract_id, collateral_account.clone());

                    self.unallocated_inputs
                        .helper_outpoint_collateral_accounts
                        .insert(collateral_account, *outpoint);
                }

                self.delete_collateral_accounts(outpoint).await
            }
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

        let allocation = self.allocated_outputs.entry(vout).or_default();

        let previous_amount = allocation
            .asset_list
            .list
            .entry(block_tx.to_string())
            .or_insert(0);
        *previous_amount = previous_amount.saturating_add(amount);
    }

    pub async fn move_asset_allocation(
        &mut self,
        vout: u32,
        contract_id: &BlockTxTuple,
        max_amount: u128,
    ) {
        let block_tx = BlockTx::from_tuple(*contract_id);
        let Some(allocation) = self
            .unallocated_inputs
            .asset_list
            .list
            .get_mut(&block_tx.to_string())
        else {
            return;
        };

        let amount = max_amount.min(*allocation);
        if amount == 0 {
            return;
        }

        *allocation = allocation.saturating_sub(amount);
        if *allocation == 0 {
            self.unallocated_inputs
                .asset_list
                .list
                .remove(&block_tx.to_string());
        }

        self.allocate_new_asset(vout, contract_id, amount).await;
    }

    pub async fn commit_outputs(&mut self, tx: &Transaction) -> Result<(), Box<dyn Error>> {
        let txid = tx.compute_txid();

        if let Some(vout) = self.first_non_op_return_index(tx) {
            // asset
            // move unallocated to first non op_return index (fallback)
            let asset_list = self.unallocated_inputs.asset_list.list.clone();
            for asset in asset_list.iter() {
                let block_tx = BlockTx::from_str(asset.0)?;

                self.move_asset_allocation(vout, &block_tx.to_tuple(), *asset.1)
                    .await;
            }

            // specs
            let specs = &self.unallocated_inputs.spec_owned.specs.clone();
            for spec_contract_id in specs.iter() {
                self.move_spec_allocation(vout, spec_contract_id).await;
            }

            // state keys
            let state_keys_contract_ids = &self.unallocated_inputs.state_keys.contract_ids.clone();
            for contract_id in state_keys_contract_ids {
                self.move_state_keys_allocation(vout, contract_id).await;
            }

            // collateral accounts
            let collateral_accounts = &self
                .unallocated_inputs
                .collateral_accounts
                .collateral_accounts
                .clone();

            for (contract_id, collateral_account) in collateral_accounts {
                self.move_collateral_account_allocation(
                    vout,
                    collateral_account,
                    BlockTx::from_str(&contract_id).unwrap().to_string(),
                )
                .await;
            }
        } else {
            log::info!("No non op_return index, unallocated inputs are lost");
        }

        for allocation in self.allocated_outputs.iter() {
            let outpoint = &OutPoint {
                txid: txid.clone(),
                vout: *allocation.0,
            };

            self.set_asset_list(outpoint, &allocation.1.asset_list)
                .await;
            self.set_spec_contract_owned(outpoint, &allocation.1.spec_owned)
                .await;
            self.set_state_keys(outpoint, &allocation.1.state_keys)
                .await;

            if !&allocation
                .1
                .collateral_accounts
                .collateral_accounts
                .is_empty()
            {
                self.set_collateral_accounts(outpoint, &allocation.1.collateral_accounts)
                    .await;
            }
        }

        #[cfg(feature = "helper-api")]
        self.update_address_balance(tx, txid).await?;

        // reset asset list
        self.unallocated_inputs = Allocation::default();
        self.allocated_outputs = HashMap::new();

        Ok(())
    }

    fn is_op_return_index(&self, output: &TxOut) -> bool {
        let mut instructions = output.script_pubkey.instructions();
        if instructions.next() == Some(Ok(Instruction::Op(opcodes::all::OP_RETURN))) {
            return true;
        }

        false
    }

    fn first_non_op_return_index(&self, tx: &Transaction) -> Option<u32> {
        for (i, output) in tx.output.iter().enumerate() {
            if !self.is_op_return_index(output) {
                return Some(i as u32);
            };
        }

        None
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
        let mut ticker: Option<String> = None;

        if let Ok(message) = message_result.clone() {
            outcome.message = Some(message.clone());
            // NOTE: static validation
            if let Some(flaw) = message.validate() {
                outcome.flaw = Some(flaw)
            }

            // NOTE: dynamic validation
            if let Some(transfer) = message.transfer {
                if outcome.flaw.is_none() {
                    outcome.flaw = self.transfers(tx, transfer.transfers).await;
                }
            }

            if let Some(contract_creation) = message.contract_creation {
                // validate contract creation by spec
                if let Some(spec_contract_id) = contract_creation.spec {
                    if outcome.flaw.is_none() {
                        let spec = self.get_spec(&spec_contract_id).await;

                        if let Ok(spec) = spec {
                            outcome.flaw = Updater::validate_contract_by_spec(
                                spec,
                                &contract_creation.contract_type,
                            );
                        } else {
                            outcome.flaw = Some(Flaw::ReferencingFlawedBlockTx);
                        }
                    }
                };

                match contract_creation.contract_type {
                    ContractType::Moa(mint_only_asset_contract) => {
                        if outcome.flaw.is_none() {
                            if let Some(_ticker) = mint_only_asset_contract.ticker {
                                if let Some(ticker_flaw) =
                                    self.validate_ticker_not_exist(_ticker.clone()).await
                                {
                                    outcome.flaw = Some(ticker_flaw);
                                } else {
                                    ticker = Some(_ticker);
                                }
                            }
                        }

                        if let Some(purchase) = mint_only_asset_contract.mint_mechanism.purchase {
                            if let InputAsset::GlittrAsset(block_tx_tuple) = purchase.input_asset {
                                let message = self.get_message(&block_tx_tuple).await;

                                if let Ok(message) = message {
                                    if message.contract_creation.is_none() && outcome.flaw.is_none()
                                    {
                                        outcome.flaw = Some(Flaw::ReferencingFlawedBlockTx)
                                    }
                                } else if outcome.flaw.is_none() {
                                    outcome.flaw = Some(Flaw::ReferencingFlawedBlockTx);
                                }
                            }
                        }
                    }
                    ContractType::Mba(mint_burn_asset_contract) => {
                        if outcome.flaw.is_none() {
                            if let Some(_ticker) = mint_burn_asset_contract.ticker {
                                if let Some(ticker_flaw) =
                                    self.validate_ticker_not_exist(_ticker.clone()).await
                                {
                                    outcome.flaw = Some(ticker_flaw);
                                } else {
                                    ticker = Some(_ticker);
                                }
                            }
                        }

                        if let Some(purchase) = mint_burn_asset_contract.mint_mechanism.purchase {
                            if let InputAsset::GlittrAsset(block_tx_tuple) = purchase.input_asset {
                                let message = self.get_message(&block_tx_tuple).await;

                                if let Ok(message) = message {
                                    if message.contract_creation.is_none() && outcome.flaw.is_none()
                                    {
                                        outcome.flaw = Some(Flaw::ReferencingFlawedBlockTx)
                                    }
                                } else if outcome.flaw.is_none() {
                                    outcome.flaw = Some(Flaw::ReferencingFlawedBlockTx);
                                }
                            }
                        }

                        if let Some(collateralized) =
                            mint_burn_asset_contract.mint_mechanism.collateralized
                        {
                            for input_asset in collateralized.input_assets {
                                if let InputAsset::GlittrAsset(block_tx_tuple) = input_asset {
                                    let message = self.get_message(&block_tx_tuple).await;

                                    if let Ok(message) = message {
                                        if message.contract_creation.is_none()
                                            && outcome.flaw.is_none()
                                        {
                                            outcome.flaw = Some(Flaw::ReferencingFlawedBlockTx)
                                        }
                                    } else if outcome.flaw.is_none() {
                                        outcome.flaw = Some(Flaw::ReferencingFlawedBlockTx);
                                    }
                                }
                            }

                            match collateralized.mint_structure {
                                mint_burn_asset::MintStructure::Proportional(proportional_type) => {
                                    if let Some(pointer_to_key) =
                                        proportional_type.inital_mint_pointer_to_key
                                    {
                                        if let Some(flaw) =
                                            self.validate_pointer(pointer_to_key, tx)
                                        {
                                            outcome.flaw = Some(flaw)
                                        } else {
                                            self.allocate_new_state_key(
                                                pointer_to_key,
                                                &block_tx.to_tuple(),
                                            )
                                            .await;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    ContractType::Spec(spec_contract) => {
                        if let Some(contract_id) = spec_contract.block_tx {
                            // update the spec
                            if outcome.flaw.is_none() {
                                outcome.flaw =
                                    self.update_spec(tx, &contract_id, &spec_contract).await;
                            }
                        } else {
                            // create the spec
                            if outcome.flaw.is_none() {
                                outcome.flaw = self
                                    .create_spec(block_height, tx_index, tx, &spec_contract)
                                    .await;
                            }
                        }
                    }
                    ContractType::NFT(_nft_asset_contract) => {}
                }
            }

            if let Some(contract_call) = message.contract_call {
                let (message, contract_id) = match contract_call.contract {
                    Some(contract_id) => (self.get_message(&contract_id).await, contract_id),
                    None => (message_result, block_tx.to_tuple()),
                };

                match contract_call.call_type {
                    CallType::Mint(mint_option) => {
                        if outcome.flaw.is_none() {
                            outcome.flaw = self
                                .mint(tx, block_tx, &contract_id, &mint_option, message)
                                .await;
                        }
                    }
                    CallType::Burn(burn_option) => {
                        if outcome.flaw.is_none() {
                            outcome.flaw = self
                                .burn(tx, block_tx, &contract_id, &burn_option, message)
                                .await;
                        }
                    }
                    CallType::Swap(swap_option) => {
                        if outcome.flaw.is_none() {
                            outcome.flaw = self
                                .process_swap(tx, block_tx, &contract_id, &swap_option, message)
                                .await;
                        }
                    }
                    CallType::OpenAccount(open_account_option) => {
                        if outcome.flaw.is_none() {
                            outcome.flaw = self
                                .process_open_account(
                                    tx,
                                    block_tx,
                                    &contract_id,
                                    &open_account_option,
                                    message,
                                )
                                .await;
                        }
                    }
                    CallType::CloseAccount(close_account_option) => {
                        if outcome.flaw.is_none() {
                            outcome.flaw = self
                                .process_close_account(
                                    tx,
                                    block_tx,
                                    &contract_id,
                                    &close_account_option,
                                )
                                .await;
                        }
                    }
                    CallType::UpdateNft(_update_nft_option) => {
                        // TODO update nft
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

            if let Some(ticker) = ticker {
                self.database.lock().await.put(
                    TICKER_TO_BLOCK_TX_PREFIX,
                    ticker.as_str(),
                    block_tx.to_tuple(),
                );
            }
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
            self.move_asset_allocation(transfer.output, &transfer.asset, transfer.amount.0)
                .await;
        }

        if !overflow_i.is_empty() {
            return Some(Flaw::OutputOverflow(overflow_i));
        }

        None
    }

    pub async fn get_contract_block_tx_by_ticker(
        &self,
        ticker: String,
    ) -> Result<BlockTxTuple, Flaw> {
        let block_tx: Result<BlockTxTuple, DatabaseError> = self
            .database
            .lock()
            .await
            .get(TICKER_TO_BLOCK_TX_PREFIX, &ticker);

        match block_tx {
            Ok(block_tx) => {
                return Ok(block_tx);
            }
            Err(DatabaseError::NotFound) => Err(Flaw::TickerNotFound),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    pub async fn get_contract_info_by_block_tx(
        &self,
        block_tx: BlockTxTuple,
    ) -> Result<Option<ContractInfo>, Flaw> {
        let message = self.get_message(&block_tx).await?;
        let asset_data = self.get_asset_contract_data(&block_tx).await?;

        match message.contract_creation {
            Some(contract_creation) => match contract_creation.contract_type {
                ContractType::Moa(moa) => {
                    return Ok(Some(ContractInfo {
                        ticker: moa.ticker,
                        supply_cap: moa.supply_cap,
                        divisibility: Some(moa.divisibility),
                        total_supply: U128(asset_data.minted_supply - asset_data.burned_supply),
                        r#type: Some(MintType {
                            preallocated: if moa.mint_mechanism.preallocated.is_some() {
                                Some(true)
                            } else {
                                None
                            },
                            free_mint: if moa.mint_mechanism.free_mint.is_some() {
                                Some(true)
                            } else {
                                None
                            },
                            purchase_or_burn: if moa.mint_mechanism.purchase.is_some() {
                                Some(true)
                            } else {
                                None
                            },
                            collateralized: None,
                        }),
                        asset_image: None,
                    }));
                }
                ContractType::Mba(mba) => Ok(Some(ContractInfo {
                    ticker: mba.ticker,
                    supply_cap: mba.supply_cap,
                    divisibility: Some(mba.divisibility),
                    total_supply: U128(asset_data.minted_supply - asset_data.burned_supply),
                    r#type: Some(MintType {
                        preallocated: if mba.mint_mechanism.preallocated.is_some() {
                            Some(true)
                        } else {
                            None
                        },
                        free_mint: if mba.mint_mechanism.free_mint.is_some() {
                            Some(true)
                        } else {
                            None
                        },
                        purchase_or_burn: if mba.mint_mechanism.purchase.is_some() {
                            Some(true)
                        } else {
                            None
                        },
                        collateralized: if let Some(collateralized) =
                            mba.mint_mechanism.collateralized
                        {
                            let mut simple_assets: Vec<InputAssetSimple> = Vec::new();

                            for asset in collateralized.input_assets {
                                match asset {
                                    InputAsset::GlittrAsset(glittr_asset) => {
                                        let block_tx = BlockTx::from_tuple(glittr_asset);

                                        let future = Box::pin(
                                            self.get_contract_info_by_block_tx(block_tx.to_tuple()),
                                        );
                                        let asset_contract_info = future.await?.unwrap();

                                        simple_assets.push(InputAssetSimple {
                                            contract_id: block_tx.to_string(),
                                            ticker: asset_contract_info.ticker,
                                            divisibility: asset_contract_info.divisibility.unwrap(),
                                        })
                                    }
                                    _ => {}
                                }
                            }

                            if simple_assets.is_empty() {
                                None
                            } else {
                                Some(CollateralizedSimple {
                                    assets: simple_assets,
                                })
                            }
                        } else {
                            None
                        },
                    }),
                    asset_image: None,
                })),
                ContractType::Spec(_) => Ok(None),
                ContractType::NFT(nft)=> Ok(Some(ContractInfo {
                    ticker: None,
                    supply_cap: None,
                    divisibility: None,
                    total_supply: U128(asset_data.minted_supply - asset_data.burned_supply),
                    r#type: None,
                    asset_image: Some(nft.asset_image),
                })),
            },
            None => Ok(None),
        }
    }

    pub async fn get_message(&self, contract_id: &BlockTxTuple) -> Result<OpReturnMessage, Flaw> {
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

    async fn set_message(&self, contract_id: &BlockTxTuple, message: &OpReturnMessage) {
        let outcome = MessageDataOutcome {
            message: Some(message.clone()),
            flaw: None,
        };

        if !self.is_read_only {
            let contract_key = BlockTx::from_tuple(*contract_id).to_string();
            self.database
                .lock()
                .await
                .put(MESSAGE_PREFIX, &contract_key, outcome);
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

    pub async fn get_collateralized_contract_data(
        &self,
        contract_id: &BlockTxTuple,
    ) -> Result<CollateralizedAssetData, Flaw> {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let data: Result<CollateralizedAssetData, DatabaseError> = self
            .database
            .lock()
            .await
            .get(COLLATERALIZED_CONTRACT_DATA, &contract_key);

        match data {
            Ok(data) => Ok(data),
            Err(DatabaseError::NotFound) => Ok(CollateralizedAssetData::default()),
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

    // TODO: separate the updater helper-api into dedicated file.
    #[cfg(feature = "helper-api")]
    pub async fn get_address_balance(&self, address: String) -> Result<AddressAssetList, Flaw> {
        use database::ADDRESS_ASSET_LIST_PREFIX;

        let address_asset_list: Result<AddressAssetList, DatabaseError> = self
            .database
            .lock()
            .await
            .get(ADDRESS_ASSET_LIST_PREFIX, &address);

        match address_asset_list {
            Ok(asset_list) => Ok(asset_list),
            Err(DatabaseError::NotFound) => Err(Flaw::NotFound),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    #[cfg(feature = "helper-api")]
    async fn get_address_from_outpoint(&self, outpoint: &OutPoint) -> Result<String, Flaw> {
        use database::OUTPOINT_TO_ADDRESS;

        let address: Result<String, DatabaseError> = self
            .database
            .lock()
            .await
            .get(OUTPOINT_TO_ADDRESS, &outpoint.to_string());

        match address {
            Ok(address) => Ok(address),
            Err(DatabaseError::NotFound) => Err(Flaw::NotFound),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    #[cfg(feature = "helper-api")]
    async fn update_address_balance(
        &self,
        tx: &Transaction,
        txid: bitcoin::Txid,
    ) -> Result<(), Box<dyn Error>> {
        use crate::config::get_bitcoin_network;
        use database::{ADDRESS_ASSET_LIST_PREFIX, OUTPOINT_TO_ADDRESS};

        // Process outputs
        for (vout, output) in tx.output.iter().enumerate() {
            if self.is_op_return_index(output) {
                continue;
            }

            if let Ok(address) = Address::from_script(&output.script_pubkey, get_bitcoin_network())
            {
                let mut address_asset_list = self
                    .get_address_balance(address.to_string())
                    .await
                    .unwrap_or_default();

                // Get the allocation for this output
                if let Some(allocation) = self.allocated_outputs.get(&(vout as u32)) {
                    // Update the UTXO list
                    let outpoint = OutPoint {
                        txid,
                        vout: vout as u32,
                    };
                    let mut utxo_assets = HashMap::new();

                    for (asset_id, amount) in &allocation.asset_list.list {
                        utxo_assets.insert(asset_id.clone(), U128(*amount));

                        // Update summarized balances
                        let current_amount: &mut U128 = address_asset_list
                            .summarized
                            .entry(asset_id.clone())
                            .or_insert(U128(0));
                        current_amount.0 = current_amount.0.saturating_add(*amount);
                    }

                    if !utxo_assets.is_empty() {
                        address_asset_list.utxos.push(UTXOBalances {
                            txid: outpoint.txid.to_string(),
                            vout: outpoint.vout,
                            assets: utxo_assets,
                        });
                    }

                    // Save updated address asset list
                    if !self.is_read_only {
                        self.database.lock().await.put(
                            ADDRESS_ASSET_LIST_PREFIX,
                            &address.to_string(),
                            &address_asset_list,
                        );

                        self.database.lock().await.put(
                            OUTPOINT_TO_ADDRESS,
                            &outpoint.to_string(),
                            address.to_string(),
                        )
                    }
                }
            }
        }

        // Process inputs (subtract from balances)
        for input in tx.input.iter() {
            let outpoint = &input.previous_output;

            // Get the previous output's address and asset list
            if let Ok(address) = self.get_address_from_outpoint(outpoint).await {
                let mut address_asset_list = self
                    .get_address_balance(address.to_string())
                    .await
                    .unwrap_or_default();

                // Remove the spent UTXO
                address_asset_list.utxos.retain(|utxo_balances| {
                    !(utxo_balances.txid == outpoint.txid.to_string()
                        && utxo_balances.vout == outpoint.vout)
                });

                // Recalculate summarized balances
                address_asset_list.summarized.clear();
                for utxo_balances in &address_asset_list.utxos {
                    for (asset_id, amount) in &utxo_balances.assets {
                        let current_amount = address_asset_list
                            .summarized
                            .entry(asset_id.clone())
                            .or_insert(U128(0));
                        current_amount.0 = current_amount.0.saturating_add(amount.0);
                    }
                }

                // Save updated address asset list
                if !self.is_read_only {
                    self.database.lock().await.put(
                        ADDRESS_ASSET_LIST_PREFIX,
                        &address.to_string(),
                        &address_asset_list,
                    );

                    self.database
                        .lock()
                        .await
                        .delete(OUTPOINT_TO_ADDRESS, &outpoint.to_string())
                }
            }
        }

        Ok(())
    }

    pub async fn get_last_indexed_block(&self) -> Option<u64> {
        let last_indexed_block: Option<u64> = self
            .database
            .lock()
            .await
            .get(INDEXER_LAST_BLOCK_PREFIX, "")
            .ok();

        return last_indexed_block;
    }
}
