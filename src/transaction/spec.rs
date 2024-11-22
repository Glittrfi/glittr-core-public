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
    pub pointer: u32,
    // if the block_tx is provided,
    // the spec would be updated based on valid field
    pub block_tx: Option<BlockTxTuple>,
}

impl SpecContract {
    pub fn validate(&self) -> Option<Flaw> {
        if self.block_tx.is_some() {
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

            return None;
        }

        return match &self.spec {
            SpecContractType::MintOnlyAsset(moa_spec) => moa_spec.validate(),
            SpecContractType::MintBurnAsset(moa_spec) => moa_spec.validate(),
        };
    }
}
