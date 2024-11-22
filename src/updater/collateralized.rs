use std::cmp::min;

use super::*;
use crate::updater::database::COLLATERAL_ACCOUNT_PREFIX;
use bitcoin::{OutPoint, Transaction};
use message::{CloseAccountOption, MintOption, OpenAccountOption};
use mint_burn_asset::{Collateralized, MintBurnAssetContract};



impl Updater {
    pub async fn mint_collateralized(
        &mut self,
        mba: &MintBurnAssetContract,
        collateralized: Collateralized,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: &MintOption,
    ) -> Option<Flaw> {
        let mut total_received_value: u128 = 0;
        let mut out_value: u128 = 0;

        // check livetime
        if mba.live_time > block_tx.block {
            return Some(Flaw::LiveTimeNotReached);
        }

        if mint_option.pointer_to_key.is_none() {
            return Some(Flaw::PointerKeyNotFound);
        }

        let mut asset_contract_data = match self.get_asset_contract_data(contract_id).await {
            Ok(data) => data,
            Err(flaw) => return Some(flaw),
        };

        match collateralized.mint_structure {
            mint_burn_asset::MintStructure::Ratio(ratio_type) => {
                for input_asset in collateralized.input_assets {
                    if let InputAsset::GlittrAsset(asset_id) = input_asset {
                        let burned_amount = self
                            .unallocated_asset_list
                            .list
                            .remove(&BlockTx::from_tuple(asset_id).to_str())
                            .unwrap_or(0);

                        if burned_amount > 0 {
                            total_received_value += burned_amount;
                        }
                    }
                }

                let process_ratio_result = self.process_ratio_type(
                    &ratio_type,
                    &total_received_value,
                    &mint_option,
                    &tx,
                    &block_tx,
                );

                if let Ok(_out_value) = process_ratio_result {
                    out_value = _out_value;
                } else {
                    return process_ratio_result.err();
                }
            }
            mint_burn_asset::MintStructure::Account(account_type) => {
                // get collateral account
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

                if collateral_account.is_none() {
                    return Some(Flaw::StateKeyNotFound);
                }

                let mut collateral_account = collateral_account.unwrap();
                let available_amount = collateral_account.total_collateral_amount
                    - (collateral_account
                        .total_collateral_amount
                        .saturating_mul(collateral_account.ltv.0 as u128))
                    .saturating_div(collateral_account.ltv.1 as u128);

                // Allowed amount = min(total_amount * max_ltv, available_amount)
                let allowed_amount = min(collateral_account
                    .total_collateral_amount
                    .saturating_mul(account_type.max_ltv.0 as u128)
                    .saturating_div(account_type.max_ltv.1 as u128), available_amount);

                match account_type.ratio {
                    shared::RatioType::Fixed { ratio } => {
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

                    self.database.lock().await.delete(
                        COLLATERAL_ACCOUNT_PREFIX,
                        collateral_account_outpoint.unwrap().to_string().as_str()
                    );

                    let new_outpoint = OutPoint { txid: tx.compute_txid(), vout: pointer_to_key };

                    // update collateral account
                    self.database.lock().await.put(
                        COLLATERAL_ACCOUNT_PREFIX,
                        new_outpoint.to_string().as_str(),
                        collateral_account
                    );

                } else {
                    return Some(Flaw::PointerKeyNotFound);
                }
            }
            mint_burn_asset::MintStructure::Proportional() => {}
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
        if let Some(flaw) = self.validate_pointer(mint_option.pointer, tx) {
            return Some(flaw);
        }

        // allocate enw asset for the mint
        self.allocate_new_asset(mint_option.pointer, contract_id, out_value)
            .await;

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
            self.allocate_new_asset(close_account_option.pointer, &asset_id, amount).await;
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
                            for input_asset in collateralized.input_assets {
                                if let InputAsset::GlittrAsset(asset_id) = input_asset {
                                    let burned_amount = self
                                        .unallocated_asset_list
                                        .list
                                        .remove(&BlockTx::from_tuple(asset_id).to_str())
                                        .unwrap_or(0);

                                    if burned_amount > 0 {
                                        collateral_amounts.push(((asset_id), burned_amount));
                                        total_collateral_amount += burned_amount;
                                    }
                                }
                            }
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
}
