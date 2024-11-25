use bitcoin::PublicKey;
use mint_only_asset::InputAsset;

use super::*;

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MintBurnAssetSpecMint {
    Proportional,
    Fixed,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MintBurnAssetSpec {
    // if this is true, the assets are mutable
    // and can be updated
    pub _mutable_assets: bool,
    pub input_assets: Option<Vec<InputAsset>>,
    pub mint: Option<MintBurnAssetSpecMint>,
}

impl MintBurnAssetSpec {
    pub fn validate(&self) -> Option<Flaw> {
        if self.input_assets.is_none() {
            return Some(Flaw::SpecFieldRequired("input_assets".to_string()));
        }

        if self.mint.is_none() {
            return Some(Flaw::SpecFieldRequired("mint".to_string()));
        }

        None
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MintOnlyAssetSpecPegInType {
    Pubkey(Pubkey),
    Burn,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MintOnlyAssetSpec {
    pub input_asset: Option<InputAsset>,
    pub peg_in_type: Option<MintOnlyAssetSpecPegInType>,
}

impl MintOnlyAssetSpec {
    pub fn validate(&self) -> Option<Flaw> {
        if self.input_asset.is_none() {
            return Some(Flaw::SpecFieldRequired("input_asset".to_string()));
        }

        if let Some(peg_in_type) = &self.peg_in_type {
            if let MintOnlyAssetSpecPegInType::Pubkey(pubkey) = peg_in_type {
                if PublicKey::from_slice(&pubkey.as_slice()).is_err() {
                    return Some(Flaw::PubkeyInvalid);
                }
            }
        } else {
            return Some(Flaw::SpecFieldRequired("peg_in_type".to_string()));
        }

        None
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SpecContractType {
    MintOnlyAsset(MintOnlyAssetSpec),
    MintBurnAsset(MintBurnAssetSpec),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct SpecContract {
    pub spec: SpecContractType,
    // the target output index that holds the spec
    // if the pointer is None, it will point to the first non-op_return output
    pub pointer: Option<u32>,
    // if the block_tx is provided,
    // the spec would be updated based on valid field
    pub block_tx: Option<BlockTxTuple>,
}

impl SpecContract {
    pub fn validate(&self) -> Option<Flaw> {
        if self.block_tx.is_some() {
            // when updating the spec

            match &self.spec {
                SpecContractType::MintOnlyAsset(_) => {
                    return Some(Flaw::SpecNotMutable);
                }
                SpecContractType::MintBurnAsset(moa_spec) => {
                    if moa_spec._mutable_assets {
                        if moa_spec.input_assets.is_none() {
                            return Some(Flaw::SpecFieldRequired("input_assets".to_string()));
                        }
                    } else {
                        return Some(Flaw::SpecNotMutable);
                    }
                }
            }

            if self.pointer.is_none() {
                return Some(Flaw::SpecFieldRequired("pointer".to_string()));
            }

            return None;
        } else {
            // when creating the spec
            match &self.spec {
                SpecContractType::MintOnlyAsset(_) => {
                    if self.pointer.is_some() {
                        return Some(Flaw::SpecFieldNotNecessary("pointer".to_string()));
                    }
                }
                SpecContractType::MintBurnAsset(moa_spec) => {
                    if moa_spec._mutable_assets {
                        if self.pointer.is_none() {
                            return Some(Flaw::SpecFieldRequired("pointer".to_string()));
                        }
                    } else {
                        if self.pointer.is_some() {
                            return Some(Flaw::SpecFieldNotNecessary("pointer".to_string()));
                        }
                    }
                }
            }
        }

        return match &self.spec {
            SpecContractType::MintOnlyAsset(moa_spec) => moa_spec.validate(),
            SpecContractType::MintBurnAsset(moa_spec) => moa_spec.validate(),
        };
    }
}

#[cfg(test)]
mod test {
    use crate::{spec::{MintBurnAssetSpec, MintBurnAssetSpecMint}, BlockTxTuple, Flaw};

    use super::{
        mint_only_asset::InputAsset, MintOnlyAssetSpec, MintOnlyAssetSpecPegInType, SpecContract,
        SpecContractType,
    };

    #[test]
    pub fn validate_moa_create_spec_contract() {
        let spec = SpecContract {
            spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                input_asset: Some(InputAsset::Rune),
                peg_in_type: Some(MintOnlyAssetSpecPegInType::Burn),
            }),
            block_tx: None,
            pointer: None,
        };
        let flaw = spec.validate();
        assert_eq!(flaw, None);
    }

    #[test]
    pub fn validate_moa_create_spec_contract_pointer_exist() {
        let spec = SpecContract {
            spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                input_asset: Some(InputAsset::Rune),
                peg_in_type: Some(MintOnlyAssetSpecPegInType::Burn),
            }),
            block_tx: None,
            pointer: Some(1),
        };
        let flaw = spec.validate();
        assert_eq!(
            flaw,
            Some(Flaw::SpecFieldNotNecessary("pointer".to_string()))
        );
    }

    #[test]
    pub fn validate_moa_update_spec_contract_not_mutable() {
        let spec = SpecContract {
            spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                input_asset: Some(InputAsset::Rune),
                peg_in_type: Some(MintOnlyAssetSpecPegInType::Burn),
            }),
            block_tx: Some(BlockTxTuple::default()),
            pointer: Some(1),
        };
        let flaw = spec.validate();
        assert_eq!(flaw, Some(Flaw::SpecNotMutable));
    }

    #[test]
    pub fn validate_mba_create_spec_contract() {
        let spec = SpecContract {
            spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                _mutable_assets: false,
                input_assets: Some(vec![InputAsset::Rune]),
                mint: Some(MintBurnAssetSpecMint::Proportional),
            }),
            block_tx: None,
            pointer: None,
        };
        let flaw = spec.validate();
        assert_eq!(flaw, None);
    }

    #[test]
    pub fn validate_mba_create_spec_contract_mutable_pointer_required() {
        let spec = SpecContract {
            spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                _mutable_assets: true,
                input_assets: Some(vec![InputAsset::Rune]),
                mint: None,
            }),
            block_tx: None,
            pointer: None,
        };
        let flaw = spec.validate();
        assert_eq!(flaw, Some(Flaw::SpecFieldRequired("pointer".to_string())));
    }

    #[test]
    pub fn validate_mba_update_spec_contract() {
        let spec = SpecContract {
            spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                _mutable_assets: true,
                input_assets: Some(vec![InputAsset::Rune]),
                mint: None,
            }),
            block_tx: Some(BlockTxTuple::default()),
            pointer: Some(1),
        };
        let flaw = spec.validate();
        assert_eq!(flaw, None);
    }

    #[test]
    pub fn validate_mba_update_spec_contract_pointer_required() {
        let spec = SpecContract {
            spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                _mutable_assets: true,
                input_assets: Some(vec![InputAsset::Rune]),
                mint: None,
            }),
            block_tx: Some(BlockTxTuple::default()),
            pointer: None,
        };
        let flaw = spec.validate();
        assert_eq!(flaw, Some(Flaw::SpecFieldRequired("pointer".to_string())));
    }

}
