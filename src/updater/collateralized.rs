use num::integer::Roots;
use std::cmp::min;

use super::*;
use crate::updater::database::COLLATERAL_ACCOUNTS_PREFIX;
use bitcoin::{OutPoint, Transaction};
use database::COLLATERALIZED_CONTRACT_DATA;
use message::{CloseAccountOption, MintBurnOption, OpenAccountOption, SwapOption};
use mint_burn_asset::{Collateralized, MintBurnAssetContract, MintStructure, RatioModel};

#[derive(Serialize, Deserialize, Default)]
pub struct CollateralizedAssetData {
    pub amounts: HashMap<BlockTxString, u128>,
    pub total_supply: u128,
}

impl Updater {
    impl_ops_for_outpoint_data!(CollateralAccounts);
    impl_ops_for_outpoint_data!(StateKeys);

    pub async fn allocate_new_collateral_accounts(
        &mut self,
        vout: u32,
        collateral_account: &CollateralAccount,
        contract_id: BlockTxString,
    ) {
        let allocation: &mut Allocation = self.allocated_outputs.entry(vout).or_default();
        allocation
            .collateral_accounts
            .collateral_accounts
            .insert(contract_id, collateral_account.clone());
    }

    pub async fn move_collateral_account_allocation(
        &mut self,
        vout: u32,
        collateral_account: &CollateralAccount,
        contract_id: BlockTxString,
    ) {
        if self
            .unallocated_inputs
            .collateral_accounts
            .collateral_accounts
            .remove(&contract_id)
            .is_some()
        {
            self.allocate_new_collateral_accounts(vout, collateral_account, contract_id)
                .await;
        };
    }

    pub async fn allocate_new_state_key(&mut self, vout: u32, contract_id: &BlockTxTuple) {
        let allocation: &mut Allocation = self.allocated_outputs.entry(vout).or_default();
        allocation.state_keys.contract_ids.insert(*contract_id);
    }

    pub async fn move_state_keys_allocation(&mut self, vout: u32, contract_id: &BlockTxTuple) {
        if self
            .unallocated_inputs
            .state_keys
            .contract_ids
            .remove(contract_id)
        {
            self.allocate_new_state_key(vout, contract_id).await;
        };
    }

    pub async fn mint_collateralized(
        &mut self,
        mba: &MintBurnAssetContract,
        collateralized: Collateralized,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: &MintBurnOption,
    ) -> Option<Flaw> {
        let mut input_values: Vec<u128> = vec![];
        let mut total_collateralized: Vec<u128> = vec![];
        let mut out_value: u128 = 0;

        match collateralized.mint_structure {
            mint_burn_asset::MintStructure::Ratio(ratio_type) => {
                let available_amount =
                    if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[0] {
                        let burned_amount = self
                            .unallocated_inputs
                            .asset_list
                            .list
                            .remove(&BlockTx::from_tuple(asset_id).to_string())
                            .unwrap_or(0);
                        burned_amount
                    } else {
                        0
                    };

                input_values.push(available_amount);

                let process_ratio_result = self.validate_and_calculate_ratio_type(
                    &ratio_type,
                    &available_amount,
                    &mint_option,
                    &tx,
                    &block_tx,
                    false,
                );

                if let Ok(_out_value) = process_ratio_result {
                    out_value = _out_value;
                } else {
                    return process_ratio_result.err();
                }
            }
            mint_burn_asset::MintStructure::Account(account_type) => {
                let collateral_account: Option<CollateralAccount> = self
                    .unallocated_inputs
                    .collateral_accounts
                    .collateral_accounts
                    .remove(&BlockTx::from_tuple(*contract_id).to_string());

                if collateral_account.is_none() {
                    return Some(Flaw::CollateralAccountNotFound);
                }

                let mut collateral_account = collateral_account.unwrap();
                let collateral_account_outpoint: Option<OutPoint> = self
                    .unallocated_inputs
                    .helper_outpoint_collateral_accounts
                    .remove(&collateral_account);

                match account_type.ratio {
                    transaction_shared::RatioType::Fixed { ratio } => {
                        // Get collateral account available amount
                        let available_amount = collateral_account.total_collateral_amount
                            - (collateral_account.total_collateral_amount
                                .saturating_mul(collateral_account.ltv.0 as u128))
                            .saturating_div(collateral_account.ltv.1 as u128);

                        input_values.push(available_amount);

                        // Allowed amount = min(total_amount * max_ltv, available_amount)
                        let allowed_amount = min(
                            collateral_account
                                .total_collateral_amount
                                .saturating_mul(account_type.max_ltv.0 as u128)
                                .saturating_div(account_type.max_ltv.1 as u128),
                            available_amount,
                        );

                        out_value = allowed_amount
                            .saturating_mul(ratio.0 as u128)
                            .saturating_div(ratio.1 as u128);

                        collateral_account.ltv = account_type.max_ltv;
                        collateral_account.amount_outstanding = out_value;
                    }
                    transaction_shared::RatioType::Oracle { setting } => {
                        if let Some(oracle_message_signed) = &mint_option.oracle_message {
                            if let Some(expected_input_outpoint) =
                                oracle_message_signed.message.input_outpoint
                            {
                                if expected_input_outpoint != collateral_account_outpoint.unwrap() {
                                    return Some(Flaw::OracleMintFailed);
                                }

                                let oracle_validate = self.validate_oracle_message(
                                    oracle_message_signed,
                                    &setting,
                                    block_tx,
                                );

                                if oracle_validate.is_some() {
                                    return oracle_validate;
                                }

                                // LTV and outstanding always updated by the oracle
                                if let Some(ltv) = oracle_message_signed.message.ltv {
                                    if ltv > account_type.max_ltv {
                                        return Some(Flaw::MaxLtvExceeded);
                                    }
                                    collateral_account.ltv = ltv;
                                } else {
                                    return Some(Flaw::LtvMustBeUpdated);
                                }

                                if let Some(outstanding) = &oracle_message_signed.message.outstanding {
                                    collateral_account.amount_outstanding = outstanding.0;
                                } else {
                                    return Some(Flaw::OutstandingMustBeUpdated);
                                }

                                if let Some(_out_value) = &oracle_message_signed.message.out_value {
                                    out_value = _out_value.0;
                                } else {
                                    return Some(Flaw::OutValueNotFound);
                                }
                            }
                        }
                    }
                }

                if let Some(pointer_to_key) = mint_option.pointer_to_key {
                    if let Some(flaw) = self.validate_pointer(pointer_to_key, tx) {
                        return Some(flaw);
                    }

                    self.allocate_new_collateral_accounts(
                        pointer_to_key,
                        &collateral_account,
                        BlockTx::from_tuple(*contract_id).to_string(),
                    )
                    .await;
                } else {
                    return Some(Flaw::PointerKeyNotFound);
                }
            }
            mint_burn_asset::MintStructure::Proportional(proportional_type) => {
                match proportional_type.ratio_model {
                    RatioModel::ConstantProduct => {
                        let first_asset_id: BlockTx;
                        let second_asset_id: BlockTx;
                        if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[0] {
                            first_asset_id = BlockTx::from_tuple(asset_id)
                        } else {
                            return Some(Flaw::PoolNotFound);
                        }

                        if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[1] {
                            second_asset_id = BlockTx::from_tuple(asset_id)
                        } else {
                            return Some(Flaw::PoolNotFound);
                        }

                        let input_first_asset = self
                            .unallocated_inputs
                            .asset_list
                            .list
                            .remove(&first_asset_id.to_string())
                            .unwrap_or(0);

                        let input_second_asset = self
                            .unallocated_inputs
                            .asset_list
                            .list
                            .remove(&second_asset_id.to_string())
                            .unwrap_or(0);

                        let pool_key = BlockTx::from_tuple(*contract_id).to_string();
                        let pool_data: Result<CollateralizedAssetData, DatabaseError> = self
                            .database
                            .lock()
                            .await
                            .get(COLLATERALIZED_CONTRACT_DATA, &pool_key);

                        match pool_data {
                            // If pool exists, update using constant product logic
                            Ok(mut existing_pool) => {
                                // Calculate k = x * y
                                let existing_pool_amounts0 =
                                    existing_pool.amounts.get(&first_asset_id.to_string());
                                let existing_pool_amounts1 =
                                    existing_pool.amounts.get(&second_asset_id.to_string());
                                if existing_pool_amounts0.is_none() || existing_pool_amounts1.is_none() {
                                    return Some(Flaw::PoolNotFound);
                                }
                                let existing_pool_amounts0 = existing_pool_amounts0.unwrap().clone();
                                let existing_pool_amounts1 = existing_pool_amounts1.unwrap().clone();

                                let k = existing_pool_amounts0.saturating_mul(existing_pool_amounts1);

                                // Add new liquidity
                                let new_amount0 =
                                    existing_pool_amounts0.saturating_add(input_first_asset);
                                let new_amount1 =
                                    existing_pool_amounts1.saturating_add(input_second_asset);

                                // Validate that k increases proportionally
                                let new_k = new_amount0.saturating_mul(new_amount1);
                                if new_k <= k {
                                    return Some(Flaw::InvalidConstantProduct);
                                }

                                // Calculate LP tokens to mint using the constant product formula
                                let total_supply = existing_pool.total_supply;
                                let mint_amount = (input_first_asset.saturating_mul(total_supply))
                                    .saturating_div(existing_pool_amounts0);

                                // Update pool data
                                existing_pool
                                    .amounts
                                    .insert(first_asset_id.to_string(), new_amount0);
                                existing_pool
                                    .amounts
                                    .insert(second_asset_id.to_string(), new_amount1);
                                existing_pool.total_supply =
                                    total_supply.saturating_add(mint_amount);

                                total_collateralized.push(new_amount0);
                                total_collateralized.push(new_amount1);

                                if !self.is_read_only {
                                    self.database.lock().await.put(
                                        COLLATERALIZED_CONTRACT_DATA,
                                        &pool_key,
                                        existing_pool,
                                    );
                                }

                                out_value = mint_amount;
                            }
                            // If pool doesn't exist, initialize it using constant product logic
                            Err(DatabaseError::NotFound) => {
                                // For initial mint, validate pubkey if required
                                if let Some(_) = &proportional_type.inital_mint_pointer_to_key {
                                    let state_key_found = self
                                        .unallocated_inputs
                                        .state_keys
                                        .contract_ids
                                        .remove(contract_id);
                                    if !state_key_found {
                                        return Some(Flaw::StateKeyNotFound);
                                    }

                                    if let Some(pointer_to_key) = mint_option.pointer_to_key {
                                        if let Some(flaw) = self.validate_pointer(pointer_to_key, tx) {
                                            return Some(flaw);
                                        }
                                        self.allocate_new_state_key(pointer_to_key, contract_id)
                                            .await;
                                    }
                                }

                                // Initialize pool with first deposit
                                let initial_supply =
                                    (input_first_asset.saturating_mul(input_second_asset)).sqrt();

                                let mut amounts = HashMap::new();
                                amounts.insert(first_asset_id.to_string(), input_first_asset);
                                amounts.insert(second_asset_id.to_string(), input_second_asset);

                                let new_pool = CollateralizedAssetData {
                                    amounts,
                                    total_supply: initial_supply,
                                };

                                total_collateralized.push(input_first_asset);
                                total_collateralized.push(input_second_asset);

                                if !self.is_read_only {
                                    self.database.lock().await.put(
                                        COLLATERALIZED_CONTRACT_DATA,
                                        &pool_key,
                                        new_pool,
                                    );
                                }

                                out_value = initial_supply;
                            }
                            Err(_) => return Some(Flaw::FailedDeserialization),
                        }
                    },
                    RatioModel::ConstantSum => {
                        // New branch for constant sum pool minting
                        let first_asset_id: BlockTx;
                        let second_asset_id: BlockTx;
                        if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[0] {
                            first_asset_id = BlockTx::from_tuple(asset_id)
                        } else {
                            return Some(Flaw::PoolNotFound);
                        }
                        if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[1] {
                            second_asset_id = BlockTx::from_tuple(asset_id)
                        } else {
                            return Some(Flaw::PoolNotFound);
                        }

                        let input_first_asset = self
                            .unallocated_inputs
                            .asset_list
                            .list
                            .remove(&first_asset_id.to_string())
                            .unwrap_or(0);
                        let input_second_asset = self
                            .unallocated_inputs
                            .asset_list
                            .list
                            .remove(&second_asset_id.to_string())
                            .unwrap_or(0);

                        let pool_key = BlockTx::from_tuple(*contract_id).to_string();
                        let pool_data: Result<CollateralizedAssetData, DatabaseError> = self
                            .database
                            .lock()
                            .await
                            .get(COLLATERALIZED_CONTRACT_DATA, &pool_key);

                        match pool_data {
                            Ok(mut existing_pool) => {
                                let existing_pool_amounts0 = existing_pool
                                    .amounts
                                    .get(&first_asset_id.to_string());
                                let existing_pool_amounts1 = existing_pool
                                    .amounts
                                    .get(&second_asset_id.to_string());
                                if existing_pool_amounts0.is_none() || existing_pool_amounts1.is_none() {
                                    return Some(Flaw::PoolNotFound);
                                }
                                let existing_pool_amounts0 = existing_pool_amounts0.unwrap().clone();
                                let existing_pool_amounts1 = existing_pool_amounts1.unwrap().clone();

                                // For a constant-sum pool, the total reserve is the sum of both assets.
                                let total_reserves_old = existing_pool_amounts0.saturating_add(existing_pool_amounts1);
                                let new_amount0 = existing_pool_amounts0.saturating_add(input_first_asset);
                                let new_amount1 = existing_pool_amounts1.saturating_add(input_second_asset);
                                let total_reserves_new = new_amount0.saturating_add(new_amount1);

                                let total_supply = existing_pool.total_supply;
                                let mint_amount = if total_reserves_old == 0 {
                                    // Should not happen if pool exists, but fallback to sum of deposits.
                                    input_first_asset.saturating_add(input_second_asset)
                                } else {
                                    (input_first_asset.saturating_add(input_second_asset))
                                        .saturating_mul(total_supply)
                                        .saturating_div(total_reserves_old)
                                };

                                // Update pool data
                                existing_pool.amounts.insert(first_asset_id.to_string(), new_amount0);
                                existing_pool.amounts.insert(second_asset_id.to_string(), new_amount1);
                                existing_pool.total_supply = total_supply.saturating_add(mint_amount);

                                total_collateralized.push(new_amount0);
                                total_collateralized.push(new_amount1);

                                if !self.is_read_only {
                                    self.database.lock().await.put(
                                        COLLATERALIZED_CONTRACT_DATA,
                                        &pool_key,
                                        existing_pool,
                                    );
                                }
                                out_value = mint_amount;
                            }
                            Err(DatabaseError::NotFound) => {
                                if let Some(_) = &proportional_type.inital_mint_pointer_to_key {
                                    let state_key_found = self
                                        .unallocated_inputs
                                        .state_keys
                                        .contract_ids
                                        .remove(contract_id);
                                    if !state_key_found {
                                        return Some(Flaw::StateKeyNotFound);
                                    }
                                    if let Some(pointer_to_key) = mint_option.pointer_to_key {
                                        if let Some(flaw) = self.validate_pointer(pointer_to_key, tx) {
                                            return Some(flaw);
                                        }
                                        self.allocate_new_state_key(pointer_to_key, contract_id)
                                            .await;
                                    }
                                }
                                // Initialize pool for constant sum with total LP = sum of deposits.
                                let initial_supply = input_first_asset.saturating_add(input_second_asset);
                                let mut amounts = HashMap::new();
                                amounts.insert(first_asset_id.to_string(), input_first_asset);
                                amounts.insert(second_asset_id.to_string(), input_second_asset);
                                let new_pool = CollateralizedAssetData {
                                    amounts,
                                    total_supply: initial_supply,
                                };
                                total_collateralized.push(input_first_asset);
                                total_collateralized.push(input_second_asset);
                                if !self.is_read_only {
                                    self.database.lock().await.put(
                                        COLLATERALIZED_CONTRACT_DATA,
                                        &pool_key,
                                        new_pool,
                                    );
                                }
                                out_value = initial_supply;
                            }
                            Err(_) => return Some(Flaw::FailedDeserialization),
                        }
                    }
                }
            }
        }

        if let Some(assert_values) = &mint_option.assert_values {
            if let Some(flaw) = self.validate_assert_values(
                &Some(assert_values.clone()),
                input_values,
                Some(total_collateralized),
                out_value,
            ) {
                return Some(flaw);
            }
        }

        // update the mint data
        if let Some(flaw) = self
            .validate_and_update_supply_cap(
                contract_id,
                mba.supply_cap.clone(),
                out_value,
                true,
                false,
                None,
            )
            .await
        {
            return Some(flaw);
        }

        // check pointer overflow and allocate new asset for the mint
        if let Some(pointer) = mint_option.pointer {
            if let Some(flaw) = self.validate_pointer(pointer, tx) {
                return Some(flaw);
            }
            self.allocate_new_asset(pointer, contract_id, out_value).await;
        }

        None
    }


    pub async fn process_close_account(
        &mut self,
        tx: &Transaction,
        _block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        close_account_option: &CloseAccountOption,
    ) -> Option<Flaw> {
        let collateral_account: Option<CollateralAccount> = self
            .unallocated_inputs
            .collateral_accounts
            .collateral_accounts
            .get(&BlockTx::from_tuple(*contract_id).to_string())
            .cloned();

        if collateral_account.is_none() {
            return Some(Flaw::CollateralAccountNotFound);
        }

        let collateral_account = collateral_account.unwrap();

        // Validate LTV is 0 and no outstanding amounts
        if collateral_account.ltv.0 != 0 {
            return Some(Flaw::LtvMustBeZero);
        }

        if collateral_account.amount_outstanding != 0 {
            return Some(Flaw::OutstandingMustBeZero);
        }

        // Validate pointer for output
        if let Some(flaw) = self.validate_pointer(close_account_option.pointer, tx) {
            return Some(flaw);
        }

        // Return collateral assets to user
        for (asset_id, amount) in collateral_account.collateral_amounts {
            self.allocate_new_asset(close_account_option.pointer, &asset_id, amount)
                .await;
        }

        // Delete collateral account
        self.unallocated_inputs
            .collateral_accounts
            .collateral_accounts
            .remove(&BlockTx::from_tuple(*contract_id).to_string());

        None
    }

    // call together with transfer
    pub async fn process_open_account(
        &mut self,
        tx: &Transaction,
        _block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        open_account_option: &OpenAccountOption,
        message: Result<OpReturnMessage, Flaw>,
    ) -> Option<Flaw> {
        // Get the MBA contract
        let mut collateral_amounts = Vec::new();
        let mut total_collateral_amount: u128 = 0;

        match message {
            Ok(op_return_message) => {
                let contract_creation = op_return_message.contract_creation?;
                match contract_creation.contract_type {
                    ContractType::Mba(mba) => {
                        if let Some(collateralized) = mba.mint_mechanism.collateralized {
                            if !matches!(collateralized.mint_structure, MintStructure::Account(_)) {
                                return Some(Flaw::InvalidContractType);
                            };
                            for input_asset in collateralized.input_assets {
                                if let InputAsset::GlittrAsset(asset_id) = input_asset {
                                    let burned_amount = self
                                        .unallocated_inputs
                                        .asset_list
                                        .list
                                        .remove(&BlockTx::from_tuple(asset_id).to_string())
                                        .unwrap_or(0);

                                    if burned_amount > 0 {
                                        collateral_amounts.push(((asset_id), burned_amount));
                                        total_collateral_amount += burned_amount;
                                    }
                                }
                            }
                        } else {
                            return Some(Flaw::InvalidContractType);
                        }
                    }
                    _ => return Some(Flaw::InvalidContractType),
                };
            }
            Err(_) => return Some(Flaw::ContractNotMatch),
        }

        let collateral_account = CollateralAccount {
            collateral_amounts,
            total_collateral_amount,
            share_amount: open_account_option.share_amount.0,
            ltv: (0, 100),
            amount_outstanding: 0,
        };

        if let Some(flaw) = self.validate_pointer(open_account_option.pointer_to_key, tx) {
            return Some(flaw);
        }

        self.allocate_new_collateral_accounts(
            open_account_option.pointer_to_key,
            &collateral_account,
            BlockTx::from_tuple(*contract_id).to_string(),
        )
        .await;
        None
    }

    pub async fn process_swap(
        &mut self,
        tx: &Transaction,
        _block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        swap_option: &SwapOption,
        message: Result<OpReturnMessage, Flaw>,
    ) -> Option<Flaw> {
        let contract_creation = match message {
            Ok(op_return_message) => op_return_message.contract_creation?,
            Err(flaw) => return Some(flaw),
        };

        match contract_creation.contract_type {
            ContractType::Mba(mba) => {
                if let Some(collateralized) = mba.mint_mechanism.collateralized {
                    if let MintStructure::Proportional(proportional_type) =
                        collateralized.mint_structure
                    {
                        // Gather input asset and amount
                        let mut input_asset_id = None;
                        let mut input_amount = 0u128;

                        for input_asset in collateralized.input_assets.clone() {
                            if let InputAsset::GlittrAsset(asset_id) = input_asset {
                                let amount = self
                                    .unallocated_inputs
                                    .asset_list
                                    .list
                                    .remove(&BlockTx::from_tuple(asset_id).to_string())
                                    .unwrap_or(0);

                                if amount > 0 {
                                    input_asset_id = Some(asset_id);
                                    input_amount = amount;
                                    break;
                                }
                            }
                        }

                        if input_asset_id.is_none() {
                            return Some(Flaw::InsufficientInputAmount);
                        }
                        let input_asset_id = input_asset_id.unwrap();

                        // Identify the other asset in the pair
                        let other_asset_id = collateralized
                            .input_assets
                            .clone()
                            .iter()
                            .find_map(|asset| {
                                if let InputAsset::GlittrAsset(asset_id) = asset {
                                    if asset_id != &input_asset_id {
                                        return Some(*asset_id);
                                    }
                                }
                                None
                            })
                            .unwrap();

                        // Get pool data (determine first and second asset IDs)
                        let first_asset_id: BlockTx;
                        let second_asset_id: BlockTx;
                        if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[0] {
                            first_asset_id = BlockTx::from_tuple(asset_id)
                        } else {
                            return Some(Flaw::PoolNotFound);
                        }
                        if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[1] {
                            second_asset_id = BlockTx::from_tuple(asset_id)
                        } else {
                            return Some(Flaw::PoolNotFound);
                        }

                        let pool_key = BlockTx::from_tuple(*contract_id).to_string();
                        let mut pool_data: CollateralizedAssetData = self
                            .database
                            .lock()
                            .await
                            .get(COLLATERALIZED_CONTRACT_DATA, &pool_key)
                            .unwrap();

                        // Identify which asset is the input (in_id) and which is the output (out_id)
                        let out_id = if other_asset_id == first_asset_id.to_tuple() {
                            first_asset_id.clone()
                        } else {
                            second_asset_id.clone()
                        };
                        let in_id = if input_asset_id == first_asset_id.to_tuple() {
                            first_asset_id.clone()
                        } else {
                            second_asset_id.clone()
                        };

                        match proportional_type.ratio_model {
                            RatioModel::ConstantProduct => {
                                // Existing constant product logic:
                                // out_amount = (reserve_out * input_amount) / (reserve_in + input_amount)
                                let existing_amount_out = pool_data
                                    .amounts
                                    .get(&out_id.to_string())
                                    .cloned()
                                    .unwrap_or(0);
                                let existing_amount_in = pool_data
                                    .amounts
                                    .get(&in_id.to_string())
                                    .cloned()
                                    .unwrap_or(0);

                                if existing_amount_in == 0 || existing_amount_out == 0 {
                                    return Some(Flaw::PoolNotFound);
                                }

                                let numerator = existing_amount_out.saturating_mul(input_amount);
                                let denominator = existing_amount_in.saturating_add(input_amount);
                                let out_value = numerator.saturating_div(denominator);
                                if out_value == 0 {
                                    return Some(Flaw::InsufficientOutputAmount);
                                }

                                // Update pool balances
                                let new_amount_in = existing_amount_in.saturating_add(input_amount);
                                let new_amount_out = existing_amount_out.saturating_sub(out_value);
                                pool_data
                                    .amounts
                                    .insert(in_id.to_string(), new_amount_in);
                                pool_data
                                    .amounts
                                    .insert(out_id.to_string(), new_amount_out);

                                // Validate minimum k invariant
                                let new_k = new_amount_in.saturating_mul(new_amount_out);
                                let old_k = existing_amount_in.saturating_mul(existing_amount_out);
                                if new_k < old_k {
                                    return Some(Flaw::InvalidConstantProduct);
                                }

                                if let Some(assert_values) = &swap_option.assert_values {
                                    if let Some(flaw) = self.validate_assert_values(
                                        &Some(assert_values.clone()),
                                        vec![input_amount],
                                        Some(vec![new_amount_in, new_amount_out]),
                                        out_value,
                                    ) {
                                        return Some(flaw);
                                    }
                                }

                                if !self.is_read_only {
                                    self.database.lock().await.put(
                                        COLLATERALIZED_CONTRACT_DATA,
                                        &pool_key,
                                        pool_data,
                                    );
                                }

                                if let Some(flaw) = self.validate_pointer(swap_option.pointer, tx) {
                                    return Some(flaw);
                                }

                                // Allocate output asset
                                self.allocate_new_asset(
                                    swap_option.pointer,
                                    &other_asset_id,
                                    out_value,
                                )
                                .await;

                                return None;
                            }
                            RatioModel::ConstantSum => {
                                // New CSMM logic:
                                // For a constant-sum pool, the output is simply the minimum of the input amount and the available counter asset.
                                let existing_amount_out = pool_data
                                    .amounts
                                    .get(&out_id.to_string())
                                    .cloned()
                                    .unwrap_or(0);
                                let existing_amount_in = pool_data
                                    .amounts
                                    .get(&in_id.to_string())
                                    .cloned()
                                    .unwrap_or(0);

                                if existing_amount_out == 0 {
                                    return Some(Flaw::PoolNotFound);
                                }

                                let out_value = min(input_amount, existing_amount_out);
                                if out_value == 0 {
                                    return Some(Flaw::InsufficientOutputAmount);
                                }

                                // Update pool balances: add the input amount to the input asset and subtract out_value from the output asset.
                                let new_amount_in = existing_amount_in.saturating_add(input_amount);
                                let new_amount_out = existing_amount_out.saturating_sub(out_value);
                                pool_data
                                    .amounts
                                    .insert(in_id.to_string(), new_amount_in);
                                pool_data
                                    .amounts
                                    .insert(out_id.to_string(), new_amount_out);

                                if let Some(assert_values) = &swap_option.assert_values {
                                    if let Some(flaw) = self.validate_assert_values(
                                        &Some(assert_values.clone()),
                                        vec![input_amount],
                                        Some(vec![new_amount_in, new_amount_out]),
                                        out_value,
                                    ) {
                                        return Some(flaw);
                                    }
                                }

                                if !self.is_read_only {
                                    self.database.lock().await.put(
                                        COLLATERALIZED_CONTRACT_DATA,
                                        &pool_key,
                                        pool_data,
                                    );
                                }

                                if let Some(flaw) = self.validate_pointer(swap_option.pointer, tx) {
                                    return Some(flaw);
                                }

                                // Allocate output asset
                                self.allocate_new_asset(
                                    swap_option.pointer,
                                    &other_asset_id,
                                    out_value,
                                )
                                .await;

                                return None;
                            }
                        }
                    }
                }
            }
            _ => return Some(Flaw::InvalidContractType),
        }

        None
    }

}
