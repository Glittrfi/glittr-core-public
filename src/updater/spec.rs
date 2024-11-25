use database::SPEC_CONTRACT_OWNED_PREFIX;
use message::ContractCreation;

use crate::spec::{MintOnlyAssetSpecPegInType, SpecContract, SpecContractType};

use super::*;

impl Updater {
    pub async fn create_spec(
        &mut self,
        block_height: u64,
        tx_index: u32,
        tx: &Transaction,
        spec_contract: &SpecContract,
    ) -> Option<Flaw> {
        if let Some(flaw) = self.validate_pointer(spec_contract.pointer, tx) {
            return Some(flaw);
        }

        let spec_contract_id = BlockTxTuple::from((block_height, tx_index));
        self.allocate_new_spec(spec_contract.pointer, &spec_contract_id)
            .await;

        None
    }

    pub async fn allocate_new_spec(&mut self, vout: u32, spec_contract_id: &BlockTxTuple) {
        let allocation = self.allocated_outputs.entry(vout).or_default();
        allocation.spec_owned.specs.insert(*spec_contract_id);
    }

    pub async fn move_spec_allocation(&mut self, vout: u32, spec_contract_id: &BlockTxTuple) {
        if self
            .unallocated_inputs
            .spec_owned
            .specs
            .remove(spec_contract_id)
        {
            self.allocate_new_spec(vout, spec_contract_id).await;
        };
    }

    pub async fn set_spec_contract_owned(
        &self,
        outpoint: &Outpoint,
        spec_contract_owned: &SpecContractOwned,
    ) {
        if !self.is_read_only {
            self.database.lock().await.put(
                SPEC_CONTRACT_OWNED_PREFIX,
                &outpoint.to_string(),
                spec_contract_owned,
            );
        }
    }

    pub async fn delete_spec_contract_owned(&self, outpoint: &Outpoint) {
        if !self.is_read_only {
            self.database
                .lock()
                .await
                .delete(SPEC_CONTRACT_OWNED_PREFIX, &outpoint.to_string());
        }
    }

    pub async fn get_spec_contract_owned(
        &self,
        outpoint: &Outpoint,
    ) -> Result<SpecContractOwned, Flaw> {
        let data: Result<SpecContractOwned, DatabaseError> = self
            .database
            .lock()
            .await
            .get(SPEC_CONTRACT_OWNED_PREFIX, &outpoint.to_string());

        match data {
            Ok(data) => Ok(data),
            Err(DatabaseError::NotFound) => Ok(SpecContractOwned::default()),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    pub async fn validate_contract_by_spec(
        &self,
        spec_contract_id: &BlockTxTuple,
        contract_type: &ContractType,
    ) -> Option<Flaw> {
        let spec = match self.get_spec(spec_contract_id).await {
            Ok(spec) => spec,
            Err(err) => return Some(err),
        };

        match spec.spec {
            SpecContractType::MintOnlyAsset(mint_only_asset_spec) => match contract_type {
                ContractType::Asset(mint_only_asset) => {
                    if let Some(purchase) = &mint_only_asset.mint_mechanism.purchase {
                        if purchase.input_asset != mint_only_asset_spec.input_asset.unwrap() {
                            return Some(Flaw::SpecCriteriaInvalid);
                        }

                        match mint_only_asset_spec.peg_in_type.unwrap() {
                            MintOnlyAssetSpecPegInType::Burn => {
                                if purchase.pay_to_key.is_some() {
                                    return Some(Flaw::SpecCriteriaInvalid);
                                }
                            }
                            MintOnlyAssetSpecPegInType::Pubkey(pubkey_spec) => {
                                if let Some(pubkey) = &purchase.pay_to_key {
                                    if *pubkey != pubkey_spec {
                                        return Some(Flaw::SpecCriteriaInvalid);
                                    }
                                } else {
                                    return Some(Flaw::SpecCriteriaInvalid);
                                }
                            }
                        }
                    } else {
                        return Some(Flaw::SpecCriteriaInvalid);
                    }
                }
                _ => return Some(Flaw::SpecCriteriaInvalid),
            },
            SpecContractType::MintBurnAsset(_) => return Some(Flaw::NotImplemented),
        };

        None
    }

    pub async fn get_spec(&self, contract_id: &BlockTxTuple) -> Result<SpecContract, Flaw> {
        let message = match self.get_message(&contract_id).await {
            Ok(message) => message,
            Err(Flaw::MessageInvalid) => return Err(Flaw::ReferencingFlawedBlockTx),
            Err(err) => return Err(err),
        };

        let contract_type = match message.contract_creation {
            Some(contract_creation) => contract_creation.contract_type,
            None => return Err(Flaw::SpecNotFound),
        };

        return match contract_type {
            ContractType::Spec(spec_contract) => Ok(spec_contract),
            _ => return Err(Flaw::SpecNotFound),
        };
    }

    pub async fn update_spec(
        &mut self,
        tx: &Transaction,
        spec_contract_id: &BlockTxTuple,
        spec_contract: &SpecContract,
    ) -> Option<Flaw> {
        let spec_owned = &self.unallocated_inputs.spec_owned;
        if !spec_owned.specs.contains(spec_contract_id) {
            return Some(Flaw::SpecUpdateNotAllowed);
        }

        let mut prev_spec_contract = match self.get_spec(spec_contract_id).await {
            Ok(spec) => spec,
            Err(err) => return Some(err),
        };

        match &spec_contract.spec {
            SpecContractType::MintOnlyAsset(_) => {
                return Some(Flaw::NotImplemented);
            }
            SpecContractType::MintBurnAsset(mba_spec) => {
                let mut prev_mba_spec = match prev_spec_contract.spec {
                    SpecContractType::MintBurnAsset(mba_spec) => mba_spec,
                    _ => return Some(Flaw::SpecNotFound),
                };

                // update assets value
                if !prev_mba_spec._mutable_assets {
                    return Some(Flaw::SpecNotMutable);
                } else {
                    prev_mba_spec.input_assets = mba_spec.input_assets.clone()
                }

                prev_spec_contract.spec = SpecContractType::MintBurnAsset(prev_mba_spec)
            }
        };

        let message = OpReturnMessage {
            contract_creation: Some(ContractCreation {
                contract_type: ContractType::Spec(prev_spec_contract),
                spec: None,
            }),
            contract_call: None,
            transfer: None,
        };

        self.set_message(&spec_contract_id, &message).await;
        if let Some(flaw) = self.validate_pointer(spec_contract.pointer, tx) {
            return Some(flaw);
        }
        self.move_spec_allocation(spec_contract.pointer, spec_contract_id).await;

        None
    }
}
