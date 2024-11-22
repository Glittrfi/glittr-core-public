use bitcoin::OutPoint;
use database::COLLATERAL_ACCOUNT_PREFIX;
use message::BurnOption;
use mint_burn_asset::{MintBurnAssetContract, ReturnCollateral};

use super::*;

impl Updater {
    pub async fn burn_return_collateral(
        &mut self,
        mba: &MintBurnAssetContract,
        return_collateral: &ReturnCollateral,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        burn_option: &BurnOption,
    ) -> Option<Flaw> {
        if mba.live_time > block_tx.block {
            return Some(Flaw::LiveTimeNotReached);
        }

        if burn_option.pointer_to_key.is_none() {
            return Some(Flaw::PointerKeyNotFound);
        }

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

        if let Some(oracle_message_signed) = &burn_option.oracle_message {
            if let Some(expected_input_outpoint) = oracle_message_signed.message.input_outpoint {
                if expected_input_outpoint != collateral_account_outpoint.unwrap() {
                    return Some(Flaw::OracleMintFailed);
                }

                if block_tx.block - oracle_message_signed.message.block_height
                    > return_collateral.oracle_setting.block_height_slippage as u64
                {
                    return Some(Flaw::OracleMintBlockSlippageExceeded);
                }

                let pubkey: XOnlyPublicKey =
                    XOnlyPublicKey::from_slice(&return_collateral.oracle_setting.pubkey.as_slice())
                        .unwrap();

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

                    if pubkey.verify(&secp, &msg, &signature).is_err() {
                        return Some(Flaw::OracleMintSignatureFailed);
                    }
                }

                // LTV and outstanding always updated by the oracle
                // LTV is based on off-chain currency (e.g. collateralized * USD)
                // Outstanding could also include interest
                if let Some(ltv) = oracle_message_signed.message.ltv {
                    collateral_account.ltv = ltv;
                } else {
                    return Some(Flaw::LtvMustBeUpdated);
                }

                if let Some(outstanding) = &oracle_message_signed.message.outstanding {
                    collateral_account.amount_outstanding = outstanding.0;
                } else {
                    return Some(Flaw::OutstandingMustBeUpdated);
                }

                // verify burned asset

                let burned_amount = self
                    .unallocated_asset_list
                    .list
                    .get(&BlockTx::from_tuple(*contract_id).to_str())
                    .unwrap_or(&0);

                if let Some(out_value) = &oracle_message_signed.message.out_value {
                    if burned_amount < &out_value.0 {
                        return Some(Flaw::BurnValueIncorrect);
                    }

                    let burned_amount = burned_amount - out_value.0;

                    if burned_amount == 0 {
                        self.unallocated_asset_list
                            .list
                            .remove(&BlockTx::from_tuple(*contract_id).to_str());
                    } else {
                        self.unallocated_asset_list
                            .list
                            .insert(BlockTx::from_tuple(*contract_id).to_str(), burned_amount);
                    }
                } else {
                    return Some(Flaw::OutValueNotFound);
                }

                if let Some(pointer_to_key) = burn_option.pointer_to_key {
                    if let Some(flaw) = self.validate_pointer(pointer_to_key, tx) {
                        return Some(flaw);
                    }

                    self.database.lock().await.delete(
                        COLLATERAL_ACCOUNT_PREFIX,
                        collateral_account_outpoint.unwrap().to_string().as_str(),
                    );

                    let new_outpoint = OutPoint {
                        txid: tx.compute_txid(),
                        vout: pointer_to_key,
                    };

                    // update collateral account
                    self.database.lock().await.put(
                        COLLATERAL_ACCOUNT_PREFIX,
                        new_outpoint.to_string().as_str(),
                        collateral_account,
                    );
                } else {
                    return Some(Flaw::PointerKeyNotFound);
                }
            }
        } else {
            return Some(Flaw::OracleMintInfoFailed);
        }

        None
    }

    pub async fn burn(
        &mut self,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        burn_option: &BurnOption,
    ) -> Option<Flaw> {
        let message = self.get_message(contract_id).await;
        match message {
            Ok(op_return_message) => match op_return_message.contract_creation {
                Some(contract_creation) => match contract_creation.contract_type {
                    ContractType::Moa(_moa) => None,
                    ContractType::Mba(mba) => {
                        if let Some(return_collateral) = &mba.burn_mechanism.return_collateral {
                            return self
                                .burn_return_collateral(
                                    &mba,
                                    return_collateral,
                                    tx,
                                    block_tx,
                                    contract_id,
                                    burn_option,
                                )
                                .await;
                        } else {
                            return Some(Flaw::NotImplemented);
                        }
                    }
                },
                None => Some(Flaw::ContractNotMatch),
            },
            Err(flaw) => Some(flaw),
        }
    }
}
