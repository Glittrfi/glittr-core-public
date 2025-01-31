use num::integer::Roots;
use std::cmp::min;
use varuint::Varuint;

use super::*;
use crate::updater::database::COLLATERAL_ACCOUNTS_PREFIX;
use bitcoin::{OutPoint, Transaction};
use database::COLLATERALIZED_CONTRACT_DATA;
use message::{CloseAccountOption, MintBurnOption, OpenAccountOption, SwapOption};
use mint_burn_asset::{Collateralized, MintBurnAssetContract, MintStructure, RatioModel};

#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Default)]
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
                        // get collateral account
                        let available_amount = collateral_account.total_collateral_amount
                            - (collateral_account
                                .total_collateral_amount
                                .saturating_mul(collateral_account.ltv.0 .0 as u128))
                            .saturating_div(collateral_account.ltv.1 .0 as u128);

                        input_values.push(available_amount);

                        // Allowed amount = min(total_amount * max_ltv, available_amount)
                        let allowed_amount = min(
                            collateral_account
                                .total_collateral_amount
                                .saturating_mul(account_type.max_ltv.0 .0 as u128)
                                .saturating_div(account_type.max_ltv.1 .0 as u128),
                            available_amount,
                        );

                        out_value = allowed_amount
                            .saturating_mul(ratio.0 .0 as u128)
                            .saturating_div(ratio.1 .0 as u128);

                        collateral_account.ltv = account_type.max_ltv;
                        collateral_account.amount_outstanding = out_value;
                    }
                    transaction_shared::RatioType::Oracle { setting } => {
                        if let Some(oracle_message_signed) = &mint_option.oracle_message {
                            if let Some(expected_input_outpoint) =
                                oracle_message_signed.message.input_outpoint
                            {
                                if expected_input_outpoint
                                    != collateral_account_outpoint.unwrap().into()
                                {
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
                                // LTV is based on off-chain currency (e.g. collateralized * USD)
                                // Outstanding could also include interest
                                if let Some(ltv) = oracle_message_signed.message.ltv {
                                    if ltv > account_type.max_ltv {
                                        return Some(Flaw::MaxLtvExceeded);
                                    }

                                    collateral_account.ltv = ltv;
                                } else {
                                    return Some(Flaw::LtvMustBeUpdated);
                                }

                                if let Some(outstanding) =
                                    &oracle_message_signed.message.outstanding
                                {
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
                    if let Some(flaw) = self.validate_pointer(pointer_to_key.0, tx) {
                        return Some(flaw);
                    }

                    self.allocate_new_collateral_accounts(
                        pointer_to_key.0,
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
                            // If pool exists, validate constant product
                            Ok(mut existing_pool) => {
                                // Calculate k = x * y
                                let existing_pool_amounts0 =
                                    existing_pool.amounts.get(&first_asset_id.to_string());
                                let existing_pool_amounts1 =
                                    existing_pool.amounts.get(&second_asset_id.to_string());
                                if existing_pool_amounts0.is_none() {
                                    return Some(Flaw::PoolNotFound);
                                }

                                if existing_pool_amounts1.is_none() {
                                    return Some(Flaw::PoolNotFound);
                                }

                                let existing_pool_amounts0 =
                                    existing_pool_amounts0.unwrap().clone();
                                let existing_pool_amounts1 =
                                    existing_pool_amounts1.unwrap().clone();

                                let k =
                                    existing_pool_amounts0.saturating_mul(existing_pool_amounts1);

                                // Add new liquidity
                                let new_amount0 =
                                    existing_pool_amounts0.saturating_add(input_first_asset);
                                let new_amount1 =
                                    existing_pool_amounts1.saturating_add(input_second_asset);

                                // Validate k increases proportionally
                                let new_k = new_amount0.saturating_mul(new_amount1);
                                if new_k <= k {
                                    return Some(Flaw::InvalidConstantProduct);
                                }

                                // Calculate LP tokens to mint
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
                            // If pool doesn't exist, initialize it
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
                                        if let Some(flaw) =
                                            self.validate_pointer(pointer_to_key.0, tx)
                                        {
                                            return Some(flaw);
                                        }

                                        self.allocate_new_state_key(pointer_to_key.0, contract_id)
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

        // check pointer overflow
        if let Some(pointer) = mint_option.pointer {
            if let Some(flaw) = self.validate_pointer(pointer.0, tx) {
                return Some(flaw);
            }

            // allocate enw asset for the mint
            self.allocate_new_asset(pointer.0, contract_id, out_value)
                .await;
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
        if collateral_account.ltv.0 .0 != 0 {
            return Some(Flaw::LtvMustBeZero);
        }

        if collateral_account.amount_outstanding != 0 {
            return Some(Flaw::OutstandingMustBeZero);
        }

        // Validate pointer for output
        if let Some(flaw) = self.validate_pointer(close_account_option.pointer.0, tx) {
            return Some(flaw);
        }

        // Return collateral assets to user
        for (asset_id, amount) in collateral_account.collateral_amounts {
            self.allocate_new_asset(close_account_option.pointer.0, &asset_id, amount)
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
            ltv: (Varuint(0), Varuint(100)),
            amount_outstanding: 0,
        };

        if let Some(flaw) = self.validate_pointer(open_account_option.pointer_to_key.0, tx) {
            return Some(flaw);
        }

        self.allocate_new_collateral_accounts(
            open_account_option.pointer_to_key.0,
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
                        if let RatioModel::ConstantProduct = proportional_type.ratio_model {
                            // Get input asset and amount from unallocated list
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

                            // Find the other asset in the pair
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

                            // Get pool data
                            let first_asset_id: BlockTx;
                            let second_asset_id: BlockTx;
                            if let InputAsset::GlittrAsset(asset_id) =
                                collateralized.input_assets[0]
                            {
                                first_asset_id = BlockTx::from_tuple(asset_id)
                            } else {
                                return Some(Flaw::PoolNotFound);
                            }

                            if let InputAsset::GlittrAsset(asset_id) =
                                collateralized.input_assets[1]
                            {
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

                            let out_id = if other_asset_id == first_asset_id.to_tuple() {
                                first_asset_id
                            } else {
                                second_asset_id
                            };
                            let in_id = if input_asset_id == first_asset_id.to_tuple() {
                                first_asset_id
                            } else {
                                second_asset_id
                            };
                            // Calculate output amount using constant product formula
                            // out_amount = y * dx / (x + dx)

                            let existing_amount_out = pool_data.amounts.get(&out_id.to_string());
                            let existing_amount_in = pool_data.amounts.get(&in_id.to_string());

                            if existing_amount_in.is_none() {
                                return Some(Flaw::PoolNotFound);
                            }

                            if existing_amount_out.is_none() {
                                return Some(Flaw::PoolNotFound);
                            }

                            let mut existing_amount_out = existing_amount_out.unwrap().clone();
                            let mut existing_amount_in = existing_amount_in.unwrap().clone();

                            let numerator = existing_amount_out.saturating_mul(input_amount);
                            let denominator = existing_amount_in.saturating_add(input_amount);

                            let out_value = numerator.saturating_div(denominator);
                            if out_value == 0 {
                                return Some(Flaw::InsufficientOutputAmount);
                            }

                            // Update pool balances
                            existing_amount_in = existing_amount_in.saturating_add(input_amount);
                            existing_amount_out = existing_amount_out.saturating_sub(out_value);

                            pool_data
                                .amounts
                                .insert(in_id.to_string(), existing_amount_in);
                            pool_data
                                .amounts
                                .insert(out_id.to_string(), existing_amount_out);

                            // Validate minimum k
                            let new_k = existing_amount_in.saturating_mul(existing_amount_out);
                            let old_k = (existing_amount_in.saturating_sub(input_amount))
                                .saturating_mul(existing_amount_out.saturating_add(out_value));
                            if new_k < old_k {
                                return Some(Flaw::InvalidConstantProduct);
                            }

                            if let Some(assert_values) = &swap_option.assert_values {
                                if let Some(flaw) = self.validate_assert_values(
                                    &Some(assert_values.clone()),
                                    vec![input_amount],
                                    Some(vec![existing_amount_in, existing_amount_out]),
                                    out_value,
                                ) {
                                    return Some(flaw);
                                }
                            }

                            // Update pool state
                            if !self.is_read_only {
                                self.database.lock().await.put(
                                    COLLATERALIZED_CONTRACT_DATA,
                                    &pool_key,
                                    pool_data,
                                );
                            }

                            if let Some(flaw) = self.validate_pointer(swap_option.pointer.0, tx) {
                                return Some(flaw);
                            }

                            // Allocate output asset
                            self.allocate_new_asset(
                                swap_option.pointer.0,
                                &other_asset_id,
                                out_value,
                            )
                            .await;

                            return None;
                        }
                    }
                }
            }
            _ => return Some(Flaw::InvalidContractType),
        }

        None
    }
}
