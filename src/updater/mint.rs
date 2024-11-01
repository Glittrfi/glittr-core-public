use asset_contract::Preallocated;
use bitcoin::hex::{Case, DisplayHex};
use message::MintOption;
use std::str::FromStr;

use super::*;

impl Updater {
    async fn update_asset_list_for_mint(
        &self,
        contract_id: &BlockTxTuple,
        outpoint: &Outpoint,
        amount: u128,
    ) -> Option<Flaw> {
        let mut asset_list = match self.get_asset_list(outpoint).await {
            Ok(data) => data,
            Err(flaw) => return Some(flaw),
        };
        let block_tx = BlockTx::from_tuple(*contract_id);
        let previous_amount = asset_list.list.get(&block_tx.to_str()).unwrap_or(&0);

        asset_list
            .list
            .insert(block_tx.to_str(), previous_amount.saturating_add(amount));
        self.set_asset_list(outpoint, &asset_list).await;

        None
    }

    async fn mint_free_mint(
        &self,
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
        let outpoint = Outpoint {
            txid: tx.compute_txid().to_string(),
            vout: mint_option.pointer,
        };

        // set the outpoint
        let flaw = self
            .update_asset_list_for_mint(contract_id, &outpoint, free_mint.amount_per_mint.0)
            .await;
        if flaw.is_some() {
            return flaw;
        }

        // update the mint data
        self.set_asset_contract_data(contract_id, &free_mint_data)
            .await;

        None
    }

    pub async fn mint_purchase_burn_swap(
        &self,
        asset_contract: &AssetContract,
        purchase: &PurchaseBurnSwap,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: MintOption,
    ) -> Option<Flaw> {
        // TODO: All glittr asset transfer logic here is temporary, MUST change when we have transfer logic implemented
        let mut total_in_value_glittr_asset: u128 = 0;
        let mut total_received_value: u128 = 0;

        let mut out_value: u128 = 0;
        let mut vout: Option<u32> = None;

        // VALIDATE INPUT
        match purchase.input_asset {
            InputAsset::GlittrAsset(asset_contract_id) => {
                for txin in tx.input.iter() {
                    if let Ok(asset_list) = self
                        .get_asset_list(&Outpoint {
                            txid: txin.previous_output.txid.to_string(),
                            vout: txin.previous_output.vout,
                        })
                        .await
                    {
                        let block_tx = BlockTx::from_tuple(asset_contract_id);
                        let amount = asset_list.list.get(&block_tx.to_str()).unwrap_or(&0);
                        total_in_value_glittr_asset += *amount;
                    }
                }
            }
            // No need to validate RawBtc
            InputAsset::RawBTC => {}
            // Validated below on oracle mint
            InputAsset::Metaprotocol => {}
        };

        // VALIDATE OUTPUT
        match &purchase.transfer_scheme {
            // Ensure that the asset is set to burn
            asset_contract::TransferScheme::Burn => {
                for (pos, output) in tx.output.iter().enumerate() {
                    let mut instructions = output.script_pubkey.instructions();

                    if instructions.next() == Some(Ok(Instruction::Op(opcodes::all::OP_RETURN))) {
                        match purchase.input_asset {
                            InputAsset::RawBTC => {
                                total_received_value = output.value.to_sat() as u128;
                            }
                            InputAsset::GlittrAsset(..) => {
                                // TODO(transfer): simulate transfer function, check if assets are sent here
                                // for now assume transfering glittr asset success
                                total_received_value = total_in_value_glittr_asset;
                            }
                            InputAsset::Metaprotocol => {
                                // TODO(transfer): handle transfer for each and every metaprotocol (ordinal, runes)
                                // need to copy the transfer mechanics for each, make sure the sat is burned
                            }
                        }

                        if mint_option.pointer != pos as u32 {
                            // NOTE: all asset always went to the first non-op-return txout if the pointer is invalid (for burn)
                            vout = Some(mint_option.pointer);
                            break;
                        }
                    } else if vout.is_none() {
                        vout = Some(pos as u32);
                    }
                }
            }
            asset_contract::TransferScheme::Purchase(bitcoin_address) => {
                for output in tx.output.iter() {
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
                                InputAsset::RawBTC => {
                                    total_received_value = output.value.to_sat() as u128;
                                }
                                InputAsset::GlittrAsset(..) => {
                                    // TODO(transfer): simulate transfer function, check if assets are sent here
                                    // for now assume transfering glittr asset success
                                    total_received_value = total_in_value_glittr_asset;
                                }
                                InputAsset::Metaprotocol => {
                                    // TODO(transfer): handle transfer for each and every metaprotocol (ordinal, runes)
                                    // need to copy the transfer mechanics for each, make sure the sat is burned
                                }
                            }
                        }
                    }
                }
                vout = Some(mint_option.pointer);
            }
        }

        // VALIDATE OUT_VALUE
        if let asset_contract::TransferRatioType::Fixed { ratio } = purchase.transfer_ratio_type {
            out_value = (total_received_value * ratio.0 as u128) / ratio.1 as u128;
        } else if let asset_contract::TransferRatioType::Oracle { pubkey, setting } =
            purchase.transfer_ratio_type.clone()
        {
            if let Some(oracle_message_signed) = mint_option.oracle_message {
                if setting.asset_id == oracle_message_signed.message.asset_id {
                    let pubkey: XOnlyPublicKey =
                        XOnlyPublicKey::from_slice(pubkey.as_slice()).unwrap();

                    if let Ok(signature) = Signature::from_slice(&oracle_message_signed.signature) {
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

                        out_value = oracle_message_signed.message.out_value;

                        let mut input_found = false;
                        for txin in tx.input.iter() {
                            if txin.previous_output == oracle_message_signed.message.input_outpoint
                            {
                                input_found = true;
                            }
                        }

                        if block_tx.block - oracle_message_signed.message.block_height
                            > setting.block_height_slippage as u64
                        {
                            return Some(Flaw::OracleMintBlockSlippageExceeded);
                        }

                        if total_received_value < oracle_message_signed.message.min_in_value {
                            return Some(Flaw::OracleMintBelowMinValue);
                        }

                        if !input_found {
                            return Some(Flaw::OracleMintInputNotFound);
                        }

                        if pubkey.verify(&secp, &msg, &signature).is_err() {
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

        if vout.unwrap() > tx.output.len() as u32 {
            return Some(Flaw::PointerOverflow);
        }

        let outpoint = Outpoint {
            txid: tx.compute_txid().to_string(),
            vout: vout.unwrap(),
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

        // set the outpoint
        let flaw = self
            .update_asset_list_for_mint(contract_id, &outpoint, out_value)
            .await;
        if flaw.is_some() {
            return flaw;
        }

        None
    }

    pub async fn mint_preallocated(
        &self,
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

        let out_value: u128 = match &preallocated.vesting_plan {
            VestingPlan::Timelock(block_height_relative_absolute) => {
                let vested_block_height = relative_block_height_to_block_height(
                    *block_height_relative_absolute,
                    contract_id.0,
                );

                if block_tx.block < vested_block_height {
                    return Some(Flaw::VestingBlockNotReached);
                }

                total_allocation.saturating_sub(claimed_allocation)
            }
            VestingPlan::Scheduled(vesting_schedule) => {
                let mut vested_allocation = 0;

                for (ratio, block_height_relative_absolute) in vesting_schedule {
                    let vested_block_height = relative_block_height_to_block_height(
                        *block_height_relative_absolute,
                        contract_id.0,
                    );

                    if block_tx.block >= vested_block_height {
                        vested_allocation += (total_allocation * ratio.0 as u128) / ratio.1 as u128
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

        claimed_allocation += out_value;
        vesting_contract_data
            .claimed_allocations
            .insert(owner_pub_key.to_hex_string(Case::Lower), claimed_allocation);
        self.set_vesting_contract_data(contract_id, &vesting_contract_data)
            .await;

        // check pointer overflow
        if mint_option.pointer >= tx.output.len() as u32 {
            return Some(Flaw::PointerOverflow);
        }
        let outpoint = Outpoint {
            txid: tx.compute_txid().to_string(),
            vout: mint_option.pointer,
        };

        // set the outpoint
        let flaw = self
            .update_asset_list_for_mint(contract_id, &outpoint, out_value)
            .await;
        if flaw.is_some() {
            return flaw;
        }

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
            Ok(op_return_message) => match op_return_message.tx_type {
                TxType::ContractCreation { contract_type } => match contract_type {
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
                },
                _ => Some(Flaw::ContractNotMatch),
            },
            Err(flaw) => Some(flaw),
        }
    }
}

pub fn relative_block_height_to_block_height(
    block_height_relative_absolute: RelativeOrAbsoluteBlockHeight,
    current_block_height: BlockHeight,
) -> BlockHeight {
    if block_height_relative_absolute < 0 {
        current_block_height + -block_height_relative_absolute as u64
    } else {
        block_height_relative_absolute as u64
    }
}
