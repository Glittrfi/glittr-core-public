use num::integer::Roots;
use std::cmp::min;

use super::*;
use crate::updater::database::COLLATERAL_ACCOUNT_PREFIX;
use bitcoin::{OutPoint, Transaction};
use database::POOL_DATA_PREFIX;
use message::{CloseAccountOption, MintBurnOption, OpenAccountOption, SwapOption};
use mint_burn_asset::{Collateralized, MintBurnAssetContract, MintStructure, RatioModel};

#[derive(Serialize, Deserialize)]
struct PoolData {
    amounts: [u128; 2],
    total_supply: u128,
}

impl Updater {
    pub async fn mint_collateralized(
        &mut self,
        mba: &MintBurnAssetContract,
        collateralized: Collateralized,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: &MintBurnOption,
    ) -> Option<Flaw> {
        let mut out_value: u128 = 0;

        // check livetime
        if mba.live_time > block_tx.block {
            return Some(Flaw::LiveTimeNotReached);
        }

        let mut asset_contract_data = match self.get_asset_contract_data(contract_id).await {
            Ok(data) => data,
            Err(flaw) => return Some(flaw),
        };

        match collateralized.mint_structure {
            mint_burn_asset::MintStructure::Ratio(ratio_type) => {
                let available_amount =
                    if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[0] {
                        let burned_amount = self
                            .unallocated_inputs
                            .asset_list
                            .list
                            .remove(&BlockTx::from_tuple(asset_id).to_str())
                            .unwrap_or(0);

                        burned_amount
                    } else {
                        0
                    };

                let process_ratio_result = self.process_ratio_type(
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
                let mut collateral_account: Option<CollateralAccount> = None;
                let mut collateral_account_outpoint: Option<OutPoint> = None;
                for input in &tx.input {
                    // TODO: move collateral account fetching and saving to unallocate_input, commit_output
                    let result_collateral_account: Result<CollateralAccount, DatabaseError> =
                        self.database.lock().await.get(
                            COLLATERAL_ACCOUNT_PREFIX,
                            input.previous_output.to_string().as_str(),
                        );

                    if let Some(_collateral_account) = result_collateral_account.ok() {
                        collateral_account = Some(_collateral_account);
                        collateral_account_outpoint = Some(input.previous_output);
                        break;
                    }
                }

                if collateral_account.is_none() {
                    return Some(Flaw::StateKeyNotFound);
                }

                let mut collateral_account = collateral_account.unwrap();
                match account_type.ratio {
                    shared::RatioType::Fixed { ratio } => {
                        // get collateral account
                        let available_amount = collateral_account.total_collateral_amount
                            - (collateral_account
                                .total_collateral_amount
                                .saturating_mul(collateral_account.ltv.0 as u128))
                            .saturating_div(collateral_account.ltv.1 as u128);

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
                    shared::RatioType::Oracle { setting } => {
                        if let Some(oracle_message_signed) = &mint_option.oracle_message {
                            if let Some(expected_input_outpoint) =
                                oracle_message_signed.message.input_outpoint
                            {
                                if expected_input_outpoint != collateral_account_outpoint.unwrap() {
                                    return Some(Flaw::OracleMintFailed);
                                }

                                if block_tx.block - oracle_message_signed.message.block_height
                                    > setting.block_height_slippage as u64
                                {
                                    return Some(Flaw::OracleMintBlockSlippageExceeded);
                                }

                                // TODO: create a shared function to validate the oracle message
                                let pubkey: XOnlyPublicKey =
                                    XOnlyPublicKey::from_slice(&setting.pubkey.as_slice()).unwrap();

                                if let Ok(signature) =
                                    Signature::from_slice(&oracle_message_signed.signature)
                                {
                                    let secp = Secp256k1::new();

                                    let msg = Message::from_digest_slice(
                                        sha256::Hash::hash(
                                            serde_json::to_string(&oracle_message_signed.message)
                                                .unwrap()
                                                .as_bytes(),
                                        )
                                        .as_byte_array(),
                                    )
                                    .unwrap();

                                    if pubkey.verify(&secp, &msg, &signature).is_err() {
                                        return Some(Flaw::OracleMintSignatureFailed);
                                    }
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
                    if let Some(flaw) = self.validate_pointer(pointer_to_key, tx) {
                        return Some(flaw);
                    }

                    let new_outpoint = OutPoint {
                        txid: tx.compute_txid(),
                        vout: pointer_to_key,
                    };

                    // update collateral account
                    if !self.is_read_only {
                        self.database.lock().await.delete(
                            COLLATERAL_ACCOUNT_PREFIX,
                            collateral_account_outpoint.unwrap().to_string().as_str(),
                        );

                        self.database.lock().await.put(
                            COLLATERAL_ACCOUNT_PREFIX,
                            new_outpoint.to_string().as_str(),
                            collateral_account,
                        );
                    }
                } else {
                    return Some(Flaw::PointerKeyNotFound);
                }
            }
            mint_burn_asset::MintStructure::Proportional(proportional_type) => {
                match proportional_type.ratio_model {
                    RatioModel::ConstantProduct => {
                    let mut first_asset_id: BlockTxTuple = (0, 0);
                    let mut second_asset_id: BlockTxTuple = (0, 0);
                    if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[0] {
                        first_asset_id = asset_id
                    }

                    if let InputAsset::GlittrAsset(asset_id) = collateralized.input_assets[1] {
                        second_asset_id = asset_id
                    }

                    let input_first_asset = self
                        .unallocated_inputs
                        .asset_list
                        .list
                        .remove(&BlockTx::from_tuple(first_asset_id).to_str())
                        .unwrap_or(0);

                    let input_second_asset = self
                        .unallocated_inputs
                        .asset_list
                        .list
                        .remove(&BlockTx::from_tuple(second_asset_id).to_str())
                        .unwrap_or(0);

                    let pool_key = format!(
                        "{}:{}",
                        BlockTx::from_tuple(first_asset_id).to_str(),
                        BlockTx::from_tuple(second_asset_id).to_str()
                    );

                    let pool_data: Result<PoolData, DatabaseError> =
                        self.database.lock().await.get(POOL_DATA_PREFIX, &pool_key);

                    match pool_data {
                        // If pool exists, validate constant product
                        Ok(mut existing_pool) => {
                            // Calculate k = x * y
                            let k =
                                existing_pool.amounts[0].saturating_mul(existing_pool.amounts[1]);

                            // Add new liquidity
                            let new_amount0 =
                                existing_pool.amounts[0].saturating_add(input_first_asset);
                            let new_amount1 =
                                existing_pool.amounts[1].saturating_add(input_second_asset);

                            // Validate k increases proportionally
                            let new_k = new_amount0.saturating_mul(new_amount1);
                            if new_k <= k {
                                return Some(Flaw::InvalidConstantProduct);
                            }

                            // Calculate LP tokens to mint
                            let total_supply = existing_pool.total_supply;

                            // TODO: validate algo with ref-finance / uniswap
                            let mint_amount = (input_first_asset.saturating_mul(total_supply))
                                .saturating_div(existing_pool.amounts[0]);

                            // Update pool data
                            existing_pool.amounts[0] = new_amount0;
                            existing_pool.amounts[1] = new_amount1;
                            existing_pool.total_supply = total_supply.saturating_add(mint_amount);

                            if !self.is_read_only {
                                self.database.lock().await.put(
                                    POOL_DATA_PREFIX,
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
                                let mut found = false;
                                for input in &tx.input {
                                    let state_key: Result<StateKey, DatabaseError> =
                                        self.database.lock().await.get(
                                            STATE_KEY_PREFIX,
                                            input.previous_output.to_string().as_str(),
                                        );

                                    if state_key.is_ok() {
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    return Some(Flaw::StateKeyNotFound);
                                }
                            }

                            // Initialize pool with first deposit
                            let initial_supply =
                                (input_first_asset.saturating_mul(input_second_asset)).sqrt();

                            let new_pool = PoolData {
                                amounts: [input_first_asset, input_second_asset],
                                total_supply: initial_supply,
                            };

                            if !self.is_read_only {
                                self.database.lock().await.put(
                                    POOL_DATA_PREFIX,
                                    &pool_key,
                                    new_pool,
                                );
                            }

                            out_value = initial_supply;
                        }
                        Err(_) => return Some(Flaw::FailedDeserialization),
                    }
                }}
            }
        }

        // check the supply
        if let Some(supply_cap) = &mba.supply_cap {
            let next_supply = asset_contract_data.minted_supply.saturating_add(out_value);

            if next_supply > supply_cap.0 {
                return Some(Flaw::SupplyCapExceeded);
            }
        }

        asset_contract_data.minted_supply =
            asset_contract_data.minted_supply.saturating_add(out_value);

        // check pointer overflow
        if let Some(pointer) = mint_option.pointer {
            if let Some(flaw) = self.validate_pointer(pointer, tx) {
                return Some(flaw);
            }

            // allocate enw asset for the mint
            self.allocate_new_asset(pointer, contract_id, out_value)
                .await;
        }

        // update the mint data
        self.set_asset_contract_data(contract_id, &asset_contract_data)
            .await;

        None
    }

    pub async fn process_close_account(
        &mut self,
        tx: &Transaction,
        _block_tx: &BlockTx,
        _contract_id: &BlockTxTuple,
        close_account_option: &CloseAccountOption,
    ) -> Option<Flaw> {
        // Find collateral account in inputs
        let mut collateral_account: Option<CollateralAccount> = None;
        let mut collateral_account_outpoint: Option<OutPoint> = None;

        for input in &tx.input {
            let result_collateral_account: Result<CollateralAccount, DatabaseError> =
                self.database.lock().await.get(
                    COLLATERAL_ACCOUNT_PREFIX,
                    input.previous_output.to_string().as_str(),
                );

            if let Some(_collateral_account) = result_collateral_account.ok() {
                collateral_account = Some(_collateral_account);
                collateral_account_outpoint = Some(input.previous_output);
                break;
            }
        }

        // Validate collateral account exists in inputs
        if collateral_account.is_none() {
            return Some(Flaw::StateKeyNotFound);
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
        if !self.is_read_only {
            self.database.lock().await.delete(
                COLLATERAL_ACCOUNT_PREFIX,
                collateral_account_outpoint.unwrap().to_string().as_str(),
            );
        }

        None
    }

    // call together with transfer
    pub async fn process_open_account(
        &mut self,
        tx: &Transaction,
        _block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        open_account_option: &OpenAccountOption,
    ) -> Option<Flaw> {
        // Get the MBA contract
        let message = self.get_message(contract_id).await;
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
                                        .remove(&BlockTx::from_tuple(asset_id).to_str())
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
            contract_id: *contract_id,
            collateral_amounts,
            total_collateral_amount,
            share_amount: open_account_option.share_amount.0,
            ltv: (0, 100),
            amount_outstanding: 0,
        };

        if let Some(flaw) = self.validate_pointer(open_account_option.pointer_to_key, tx) {
            return Some(flaw);
        }

        let txid = tx.compute_txid().to_string();
        let outpoint = &Outpoint {
            txid,
            vout: open_account_option.pointer_to_key,
        };

        if !self.is_read_only {
            self.database.lock().await.put(
                COLLATERAL_ACCOUNT_PREFIX,
                outpoint.to_string().as_str(),
                collateral_account,
            );
        }

        None
    }

    pub async fn process_swap(
        &mut self,
        tx: &Transaction,
        _block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        swap_option: &SwapOption,
    ) -> Option<Flaw> {
        let message = self.get_message(contract_id).await;
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
                                        .remove(&BlockTx::from_tuple(asset_id).to_str())
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
                            let mut first_asset: BlockTxTuple = (0, 0);
                            let mut second_asset: BlockTxTuple = (0, 0);
                            if let InputAsset::GlittrAsset(asset_id) =
                                collateralized.input_assets[0]
                            {
                                first_asset = asset_id
                            }

                            if let InputAsset::GlittrAsset(asset_id) =
                                collateralized.input_assets[1]
                            {
                                second_asset = asset_id
                            }

                            let pool_key = format!(
                                "{}:{}",
                                BlockTx::from_tuple(first_asset).to_str(),
                                BlockTx::from_tuple(second_asset).to_str()
                            );

                            let mut pool_data: PoolData = self
                                .database
                                .lock()
                                .await
                                .get(POOL_DATA_PREFIX, &pool_key)
                                .unwrap();

                            let out_idx = if other_asset_id == first_asset { 0 } else { 1 };
                            let in_idx = if input_asset_id == first_asset { 0 } else { 1 };
                            // Calculate output amount using constant product formula
                            // out_amount = y * dx / (x + dx)
                            let numerator = pool_data.amounts[out_idx].saturating_mul(input_amount);
                            let denominator =
                                pool_data.amounts[in_idx].saturating_add(input_amount);

                            let out_value = numerator.saturating_div(denominator);
                            if out_value == 0 {
                                return Some(Flaw::InsufficientOutputAmount);
                            }

                            // Update pool balances
                            pool_data.amounts[in_idx] =
                                pool_data.amounts[in_idx].saturating_add(input_amount);
                            pool_data.amounts[out_idx] =
                                pool_data.amounts[out_idx].saturating_sub(out_value);

                            // Validate minimum k
                            let new_k = pool_data.amounts[0].saturating_mul(pool_data.amounts[1]);
                            let old_k = (pool_data.amounts[in_idx].saturating_sub(input_amount))
                                .saturating_mul(
                                    pool_data.amounts[out_idx].saturating_add(out_value),
                                );
                            if new_k < old_k {
                                return Some(Flaw::InvalidConstantProduct);
                            }

                            // Update pool state
                            if !self.is_read_only {
                                self.database.lock().await.put(
                                    POOL_DATA_PREFIX,
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
            _ => return Some(Flaw::InvalidContractType),
        }

        None
    }
}
