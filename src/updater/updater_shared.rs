use message::{MintBurnOption, OracleMessageSigned};
use transaction_shared::{OracleSetting, RatioType};

use super::*;

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

impl Updater {
    pub fn validate_pointer(&self, pointer: u32, tx: &Transaction) -> Option<Flaw> {
        if pointer >= tx.output.len() as u32 {
            return Some(Flaw::PointerOverflow);
        }
        if self.is_op_return_index(&tx.output[pointer as usize]) {
            return Some(Flaw::InvalidPointer);
        }
        None
    }

    pub fn validate_oracle_message(
        &self,
        oracle_message: &OracleMessageSigned,
        setting: &OracleSetting,
        block_tx: &BlockTx,
    ) -> Option<Flaw> {
        // Check asset ID matches
        if setting.asset_id.is_some() {
            if setting.asset_id != oracle_message.message.asset_id {
                return Some(Flaw::OracleMintFailed);
            }
        }

        // Check block height slippage
        if block_tx.block - oracle_message.message.block_height
            > setting.block_height_slippage as u64
        {
            return Some(Flaw::OracleMintBlockSlippageExceeded);
        }

        // Validate signature
        let pubkey = XOnlyPublicKey::from_slice(&setting.pubkey).unwrap();

        let msg = Message::from_digest_slice(
            sha256::Hash::hash(
                serde_json::to_string(&oracle_message.message)
                    .unwrap()
                    .as_bytes(),
            )
            .as_byte_array(),
        )
        .unwrap();

        let signature = Signature::from_slice(&oracle_message.signature);

        if signature.is_err() {
            return Some(Flaw::OracleMintSignatureFailed);
        }

        if pubkey
            .verify(&Secp256k1::new(), &msg, &signature.unwrap())
            .is_err()
        {
            return Some(Flaw::OracleMintSignatureFailed);
        }

        None
    }

    pub async fn validate_and_update_supply_cap(
        &mut self,
        contract_id: &BlockTxTuple,
        supply_cap: Option<U128>,
        amount: u128,
        is_mint: bool,
        is_free_mint: bool,
        free_mint_supply: Option<U128>
    ) -> Option<Flaw> {
        let mut data = match self.get_asset_contract_data(contract_id).await {
            Ok(data) => data,
            Err(flaw) => return Some(flaw),
        };

        if is_mint {
            let next_supply = data.minted_supply.saturating_add(amount);

            if let Some(cap) = supply_cap.clone() {
                if next_supply > cap.0 {
                    return Some(Flaw::SupplyCapExceeded);
                }
            }

            data.minted_supply = next_supply;

            if is_free_mint {
                let next_supply_free_mint = data.minted_supply_by_freemint.saturating_add(amount);
                if let Some(cap) = free_mint_supply {
                    if next_supply_free_mint > cap.0 {
                        return Some(Flaw::SupplyCapExceeded);
                    }
                }
                data.minted_supply_by_freemint = next_supply_free_mint;
            }
        } else {
            data.burned_supply = data.burned_supply.saturating_sub(amount);
        }

        self.set_asset_contract_data(contract_id, &data).await;
        None
    }

    pub fn process_ratio_type(
        &self,
        ratio: &RatioType,
        total_received_value: &u128,
        mint_option: &MintBurnOption,
        tx: &Transaction,
        block_tx: &BlockTx,
        is_burn: bool,
    ) -> Result<u128, Flaw> {
        match ratio {
            RatioType::Fixed { ratio } => {
                if !is_burn {
                    Ok((total_received_value * ratio.0 as u128) / ratio.1 as u128)
                } else {
                    Ok((total_received_value * ratio.1 as u128) / ratio.0 as u128)
                }
            }
            RatioType::Oracle { setting } => {
                if let Some(oracle_message_signed) = &mint_option.oracle_message {
                    if setting.asset_id == oracle_message_signed.message.asset_id {
                        let oracle_validate =
                            self.validate_oracle_message(oracle_message_signed, &setting, block_tx);

                        if oracle_validate.is_some() {
                            return Err(oracle_validate.unwrap());
                        }

                        let mut is_btc = false;
                        if let Some(asset_id) = &oracle_message_signed.message.asset_id {
                            if asset_id == "btc" {
                                is_btc = true;
                                if let Some(ratio) = oracle_message_signed.message.ratio {
                                    return Ok(total_received_value
                                        .saturating_mul(ratio.0 as u128)
                                        .saturating_div(ratio.1 as u128));
                                }
                            }
                        }

                        // For non-BTC assets or no asset_id specified
                        if !is_btc {
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
                                    return Err(Flaw::OracleMintInputNotFound);
                                }
                            } else {
                                return Err(Flaw::OracleMintInfoFailed);
                            }

                            if let Some(min_in_value) = &oracle_message_signed.message.min_in_value
                            {
                                if total_received_value < &min_in_value.0 {
                                    return Err(Flaw::OracleMintBelowMinValue);
                                }
                            } else {
                                return Err(Flaw::OracleMintInfoFailed);
                            }

                            if let Some(_out_value) = &oracle_message_signed.message.out_value {
                                return Ok(_out_value.0);
                            } else {
                                return Err(Flaw::OracleMintInfoFailed);
                            }
                        } else {
                            return Err(Flaw::OracleMintFailed);
                        }
                    } else {
                        return Err(Flaw::OracleMintFailed);
                    }
                } else {
                    return Err(Flaw::OracleMintFailed);
                }
            }
        }
    }
}
