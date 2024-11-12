use asset_contract::Preallocated;
use bitcoin::hex::{Case, DisplayHex};
use message::MintOption;
use std::str::FromStr;

use super::*;

impl Updater {
    async fn mint_free_mint(
        &mut self,
        asset_contract: &AssetContract,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: &MintOption,
    ) -> Option<Flaw> {
        // check livetime
        if asset_contract.asset.live_time > block_tx.block {
            return Some(Flaw::LiveTimeNotReached);
        }

        let mut free_mint_data = match self.get_asset_contract_data(contract_id).await {
            Ok(data) => data,
            Err(flaw) => return Some(flaw),
        };

        let free_mint = asset_contract
            .distribution_schemes
            .free_mint
            .as_ref()
            .unwrap();
        // check the supply
        if let Some(supply_cap) = &free_mint.supply_cap {
            let next_supply = free_mint_data
                .minted_supply
                .saturating_add(free_mint.amount_per_mint.0);

            if next_supply > supply_cap.0 {
                return Some(Flaw::SupplyCapExceeded);
            }
        }
        free_mint_data.minted_supply = free_mint_data
            .minted_supply
            .saturating_add(free_mint.amount_per_mint.0);

        // check pointer overflow
        if mint_option.pointer >= tx.output.len() as u32 {
            return Some(Flaw::PointerOverflow);
        }
        // check invalid pointer if the target index is op_return output
        if self.is_op_return_index(&tx.output[mint_option.pointer as usize]) {
            return Some(Flaw::InvalidPointer);
        }

        // allocate enw asset for the mint
        self.allocate_new_asset(
            mint_option.pointer,
            contract_id,
            free_mint.amount_per_mint.0,
        )
        .await;

        // update the mint data
        self.set_asset_contract_data(contract_id, &free_mint_data)
            .await;

        None
    }

    pub async fn mint_purchase_burn_swap(
        &mut self,
        asset_contract: &AssetContract,
        purchase: &PurchaseBurnSwap,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: MintOption,
    ) -> Option<Flaw> {
        let mut total_unallocated_glittr_asset: u128 = 0;
        let mut total_received_value: u128 = 0;

        let mut out_value: u128 = 0;

        // VALIDATE INPUT
        match purchase.input_asset {
            InputAsset::GlittrAsset(asset_contract_id) => {
                if let Some(amount) = self
                    .unallocated_asset_list
                    .list
                    .get(&BlockTx::from_tuple(asset_contract_id).to_str())
                {
                    total_unallocated_glittr_asset = *amount;
                }
            }
            // No need to validate RawBtc
            InputAsset::RawBtc => {}
            // Validated below on oracle mint
            InputAsset::Metaprotocol => {}
        };

        // VALIDATE OUTPUT
        match &purchase.transfer_scheme {
            // Ensure that the asset is set to burn
            asset_contract::TransferScheme::Burn => match &purchase.input_asset {
                InputAsset::RawBtc => {
                    for output in tx.output.iter() {
                        let mut instructions = output.script_pubkey.instructions();

                        if instructions.next() == Some(Ok(Instruction::Op(opcodes::all::OP_RETURN)))
                        {
                            total_received_value = output.value.to_sat() as u128;
                        }
                    }
                }
                InputAsset::GlittrAsset(_) => {
                    total_received_value = total_unallocated_glittr_asset;
                }
                InputAsset::Metaprotocol => {}
            },
            asset_contract::TransferScheme::Purchase(ref bitcoin_address) => {
                for (pos, output) in tx.output.iter().enumerate() {
                    let address = Address::from_str(bitcoin_address.as_str())
                        .unwrap()
                        .assume_checked();
                    // TODO: bitcoin network from CONFIG
                    let address_from_script = Address::from_script(
                        output.script_pubkey.as_script(),
                        bitcoin::Network::Regtest,
                    );

                    if let Ok(address_from_script) = address_from_script {
                        if address == address_from_script {
                            match purchase.input_asset {
                                InputAsset::RawBtc => {
                                    total_received_value = output.value.to_sat() as u128;
                                }
                                InputAsset::GlittrAsset(asset_contract_id) => {
                                    if let Some(asset_list) =
                                        self.allocated_asset_list.get(&(pos as u32))
                                    {
                                        if let Some(amount) = asset_list
                                            .list
                                            .get(&BlockTx::from_tuple(asset_contract_id).to_str())
                                        {
                                            total_received_value = *amount;
                                        }
                                    }
                                }
                                InputAsset::Metaprotocol => {
                                    // TODO(transfer): handle transfer for each and every metaprotocol (ordinal, runes)
                                    // need to copy the transfer mechanics for each, make sure the sat is burned
                                }
                            }
                        }
                    }
                }
            }
        }

        // VALIDATE OUT_VALUE
        match &purchase.transfer_ratio_type {
            asset_contract::TransferRatioType::Fixed { ratio } => {
                out_value = (total_received_value * ratio.0 as u128) / ratio.1 as u128;
            }
            asset_contract::TransferRatioType::Oracle { pubkey, setting } => {
                if let Some(oracle_message_signed) = mint_option.oracle_message {
                    if setting.asset_id == oracle_message_signed.message.asset_id {
                        let pubkey: XOnlyPublicKey =
                            XOnlyPublicKey::from_slice(pubkey.as_slice()).unwrap();

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

                            let mut is_btc = false;
                            if let Some(asset_id) = oracle_message_signed.message.asset_id {
                                if asset_id == "btc".to_string() {
                                    is_btc = true;
                                    if let Some(ratio) = oracle_message_signed.message.ratio {
                                        out_value = total_received_value
                                            .saturating_mul(ratio.0 as u128)
                                            .saturating_div(ratio.1 as u128)
                                    }
                                }
                            }

                            // For non-BTC assets or no asset_id specified
                            if !is_btc {
                                if let Some(_out_value) = oracle_message_signed.message.out_value {
                                    out_value = _out_value.0;
                                } else {
                                    return Some(Flaw::OracleMintInfoFailed);
                                }

                                if let Some(input_outpoint) =
                                    oracle_message_signed.message.input_outpoint
                                {
                                    let mut input_found = false;
                                    for txin in tx.input.iter() {
                                        if txin.previous_output == input_outpoint {
                                            input_found = true;
                                        }
                                    }
                                    if !input_found {
                                        return Some(Flaw::OracleMintInputNotFound);
                                    }
                                } else {
                                    return Some(Flaw::OracleMintInfoFailed);
                                }

                                if let Some(min_in_value) =
                                    oracle_message_signed.message.min_in_value
                                {
                                    if total_received_value < min_in_value.0 {
                                        return Some(Flaw::OracleMintBelowMinValue);
                                    }
                                } else {
                                    return Some(Flaw::OracleMintInfoFailed);
                                }
                            }

                            if block_tx.block - oracle_message_signed.message.block_height
                                > setting.block_height_slippage as u64
                            {
                                return Some(Flaw::OracleMintBlockSlippageExceeded);
                            }

                            if !pubkey.verify(&secp, &msg, &signature).is_ok() {
                                return Some(Flaw::OracleMintSignatureFailed);
                            }
                        } else {
                            return Some(Flaw::OracleMintFailed);
                        }
                    } else {
                        return Some(Flaw::OracleMintFailed);
                    }
                } else {
                    return Some(Flaw::OracleMintFailed);
                };
            }
        }

        // check pointer overflow
        if mint_option.pointer >= tx.output.len() as u32 {
            return Some(Flaw::PointerOverflow);
        }
        // check invalid pointer if the target index is op_return output
        if self.is_op_return_index(&tx.output[mint_option.pointer as usize]) {
            return Some(Flaw::InvalidPointer);
        }

        if out_value == 0 {
            return Some(Flaw::MintedZero);
        }

        // If transfer is burn and using glittr asset input, remove it from unallocated and add burned
        if let asset_contract::TransferScheme::Burn = &purchase.transfer_scheme {
            if let InputAsset::GlittrAsset(asset_contract_id) = purchase.input_asset {
                let burned_amount = self
                    .unallocated_asset_list
                    .list
                    .remove(&BlockTx::from_tuple(asset_contract_id).to_str())
                    .unwrap_or(0);

                let mut asset_contract_data_input =
                    match self.get_asset_contract_data(&asset_contract_id).await {
                        Ok(data) => data,
                        Err(flaw) => return Some(flaw),
                    };

                asset_contract_data_input.burned_supply = asset_contract_data_input
                    .burned_supply
                    .saturating_add(burned_amount);
                self.set_asset_contract_data(&asset_contract_id, &asset_contract_data_input)
                    .await;
            }
        }

        if let Some(supply_cap) = &asset_contract.asset.supply_cap {
            let mut asset_contract_data = match self.get_asset_contract_data(contract_id).await {
                Ok(data) => data,
                Err(flaw) => return Some(flaw),
            };

            // check the supply
            let next_supply = asset_contract_data.minted_supply.saturating_add(out_value);

            if next_supply > supply_cap.0 {
                return Some(Flaw::SupplyCapExceeded);
            }

            asset_contract_data.minted_supply =
                asset_contract_data.minted_supply.saturating_add(out_value);

            self.set_asset_contract_data(contract_id, &asset_contract_data)
                .await;
        }

        // set the outpoint

        self.allocate_new_asset(mint_option.pointer, contract_id, out_value)
            .await;

        None
    }

    pub async fn mint_preallocated(
        &mut self,
        asset_contract: &AssetContract,
        preallocated: &Preallocated,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: &MintOption,
    ) -> Option<Flaw> {
        let mut owner_pub_key: Vec<u8> = Vec::new();
        let mut total_allocation: u128 = 0;

        // validate input has utxo owned by one of the vestee
        // TODO: edge case when there are two input utxos by two vesting owners
        for txin in &tx.input {
            // P2WPKH, P2TR
            let pubkey = if !txin.witness.is_empty() {
                txin.witness.last().unwrap().to_vec()
            } else {
                // P2PKH, P2PK
                // [signature, public key]
                let mut instructions = txin.script_sig.instructions();

                if instructions.clone().count() >= 2 {
                    if let Ok(Instruction::PushBytes(pubkey_bytes)) = &instructions.nth(1).unwrap()
                    {
                        pubkey_bytes.as_bytes().to_vec()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            };

            for (allocation, pubkeys) in preallocated.allocations.iter() {
                if pubkeys.contains(&pubkey) {
                    owner_pub_key = pubkey;
                    total_allocation = allocation.0;
                    break;
                }
            }
        }

        if owner_pub_key.is_empty() {
            return Some(Flaw::VesteeNotFound);
        }

        let mut vesting_contract_data = self.get_vesting_contract_data(contract_id).await.unwrap();
        let mut claimed_allocation = *vesting_contract_data
            .claimed_allocations
            .get(&owner_pub_key.to_hex_string(Case::Lower))
            .unwrap_or(&0);

        let out_value: u128 = match preallocated.vesting_plan.clone() {
            VestingPlan::Timelock(block_height_relative_absolute) => {
                let vested_block_height = relative_block_height_to_block_height(
                    block_height_relative_absolute,
                    contract_id.0,
                );

                if block_tx.block < vested_block_height {
                    return Some(Flaw::VestingBlockNotReached);
                }

                total_allocation.saturating_sub(claimed_allocation)
            }
            VestingPlan::Scheduled(mut vesting_schedule) => {
                let mut vested_allocation: u128 = 0;

                vesting_schedule.sort_by(|a, b| {
                    let vested_block_height_a =
                        relative_block_height_to_block_height(a.1, contract_id.0);
                    let vested_block_height_b =
                        relative_block_height_to_block_height(b.1, contract_id.0);

                    vested_block_height_a.cmp(&vested_block_height_b)
                });

                for (ratio, block_height_relative_absolute) in vesting_schedule {
                    let vested_block_height = relative_block_height_to_block_height(
                        block_height_relative_absolute,
                        contract_id.0,
                    );

                    if block_tx.block >= vested_block_height {
                        vested_allocation = vested_allocation
                            .saturating_add((total_allocation * ratio.0 as u128) / ratio.1 as u128);
                    }
                }

                vested_allocation.saturating_sub(claimed_allocation)
            }
        };

        if let Some(supply_cap) = &asset_contract.asset.supply_cap {
            let mut asset_contract_data = match self.get_asset_contract_data(contract_id).await {
                Ok(data) => data,
                Err(flaw) => return Some(flaw),
            };

            // check the supply
            let next_supply = asset_contract_data.minted_supply.saturating_add(out_value);

            if next_supply > supply_cap.0 {
                return Some(Flaw::SupplyCapExceeded);
            }

            asset_contract_data.minted_supply =
                asset_contract_data.minted_supply.saturating_add(out_value);

            self.set_asset_contract_data(contract_id, &asset_contract_data)
                .await;
        }

        claimed_allocation = claimed_allocation.saturating_add(out_value);
        vesting_contract_data
            .claimed_allocations
            .insert(owner_pub_key.to_hex_string(Case::Lower), claimed_allocation);
        self.set_vesting_contract_data(contract_id, &vesting_contract_data)
            .await;

        // check pointer overflow
        if mint_option.pointer >= tx.output.len() as u32 {
            return Some(Flaw::PointerOverflow);
        }

        self.allocate_new_asset(mint_option.pointer, contract_id, out_value)
            .await;

        None
    }

    pub async fn mint(
        &mut self,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: &MintOption,
    ) -> Option<Flaw> {
        let message = self.get_message(contract_id).await;
        match message {
            Ok(op_return_message) => match op_return_message.contract_creation {
                Some(contract_creation) => match contract_creation.contract_type {
                    message::ContractType::Asset(asset_contract) => {
                        let result_preallocated = if let Some(preallocated) =
                            &asset_contract.distribution_schemes.preallocated
                        {
                            self.mint_preallocated(
                                &asset_contract,
                                preallocated,
                                tx,
                                block_tx,
                                contract_id,
                                mint_option,
                            )
                            .await
                        } else {
                            Some(Flaw::NotImplemented)
                        };

                        if result_preallocated.is_none() {
                            return result_preallocated;
                        }

                        let result_free_mint =
                            if asset_contract.distribution_schemes.free_mint.is_some() {
                                self.mint_free_mint(
                                    &asset_contract,
                                    tx,
                                    block_tx,
                                    contract_id,
                                    mint_option,
                                )
                                .await
                            } else {
                                Some(Flaw::NotImplemented)
                            };

                        if result_free_mint.is_none() {
                            return result_free_mint;
                        }

                        let result_purchase =
                            if let Some(purchase) = &asset_contract.distribution_schemes.purchase {
                                self.mint_purchase_burn_swap(
                                    &asset_contract,
                                    purchase,
                                    tx,
                                    block_tx,
                                    contract_id,
                                    mint_option.clone(),
                                )
                                .await
                            } else {
                                Some(Flaw::NotImplemented)
                            };

                        if result_purchase.is_none() {
                            return result_purchase;
                        }

                        if result_preallocated != Some(Flaw::NotImplemented) {
                            result_preallocated
                        } else if result_free_mint != Some(Flaw::NotImplemented) {
                            result_free_mint
                        } else {
                            result_purchase
                        }
                    }
                    message::ContractType::NFT(nft_contract) => {
                        let result_preallocated = if let Some(preallocated) =
                            &nft_contract.distribution_schemes.preallocated
                        {
                            self.mint_preallocated(
                                &nft_contract,
                                preallocated,
                                tx,
                                block_tx,
                                contract_id,
                                mint_option,
                            )
                            .await
                        } else {
                            Some(Flaw::NotImplemented)
                        };
            
                        if result_preallocated.is_none() {
                            return result_preallocated;
                        }
            
                        let result_free_mint =
                            if nft_contract.distribution_schemes.free_mint.is_some() {
                                self.mint_free_mint(
                                    &nft_contract,
                                    tx,
                                    block_tx,
                                    contract_id,
                                    mint_option,
                                )
                                .await
                            } else {
                                Some(Flaw::NotImplemented)
                            };
            
                        if result_free_mint.is_none() {
                            return result_free_mint;
                        }
            
                        let result_purchase =
                            if let Some(purchase) = &nft_contract.distribution_schemes.purchase {
                                self.mint_purchase_burn_swap(
                                    &nft_contract,
                                    purchase,
                                    tx,
                                    block_tx,
                                    contract_id,
                                    mint_option.clone(),
                                )
                                .await
                            } else {
                                Some(Flaw::NotImplemented)
                            };
            
                        if result_purchase.is_none() {
                            return result_purchase;
                        }
            
                        if result_preallocated != Some(Flaw::NotImplemented) {
                            result_preallocated
                        } else if result_free_mint != Some(Flaw::NotImplemented) {
                            result_free_mint
                        } else {
                            result_purchase
                        }
                    }
                },
                None => Some(Flaw::ContractNotMatch),
            },
            Err(flaw) => Some(flaw),
        }
    }
}

// TODO: move to helper file
pub fn relative_block_height_to_block_height(
    block_height_relative_absolute: RelativeOrAbsoluteBlockHeight,
    current_block_height: BlockHeight,
) -> BlockHeight {
    if block_height_relative_absolute < 0 {
        current_block_height.saturating_add(-block_height_relative_absolute as u64)
    } else {
        block_height_relative_absolute as u64
    }
}
