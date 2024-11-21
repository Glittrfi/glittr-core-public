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
    pub input_assets: Vec<InputAsset>,
    pub mint: MintBurnAssetSpecMint,
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
    pub input_asset: InputAsset,
    pub peg_in_type: MintOnlyAssetSpecPegInType,
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
    // for now put this flag here, but it should be moved to the spec itself
    pub mutable_asset: bool,
}
