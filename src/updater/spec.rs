use message::ContractCreation;

use crate::spec::{SpecContract, SpecContractType};

use super::*;

impl Updater {
    // TODO: handle spec only can be updated by the creator
    pub async fn update_spec(&mut self, spec_contract: &SpecContract) -> Option<Flaw> {
        let contract_id = spec_contract
            .block_tx
            .expect("Error block_tx, something is wrong with the indexer");

        let prev_message = match self.get_message(&contract_id).await {
            Ok(message) => message,
            Err(Flaw::MessageInvalid) => return Some(Flaw::ReferencingFlawedBlockTx),
            Err(err) => return Some(err),
        };

        let prev_contract_type = match prev_message.contract_creation {
            Some(contract_creation) => contract_creation.contract_type,
            None => return Some(Flaw::SpecInvalidContract),
        };

        let mut prev_spec_contract = match prev_contract_type {
            ContractType::Spec(spec_contract) => spec_contract,
            _ => return Some(Flaw::SpecInvalidContract),
        };

        match &spec_contract.spec {
            SpecContractType::MintOnlyAsset(_) => {
                // TODO: implemented in the future if needed
                return Some(Flaw::NotImplemented);
            }
            SpecContractType::MintBurnAsset(mba_spec) => {
                let mut prev_mba_spec = match prev_spec_contract.spec {
                    SpecContractType::MintBurnAsset(mba_spec) => mba_spec,
                    _ => return Some(Flaw::SpecInvalidContract),
                };

                // update assets
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
            }),
            contract_call: None,
            transfer: None,
        };

        self.set_message(&contract_id, &message).await;

        None
    }
}
