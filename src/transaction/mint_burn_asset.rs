use super::*;
use shared::{FreeMint, InputAsset, Preallocated, PurchaseBurnSwap, RatioType};

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MintBurnAssetContract {
    pub ticker: Option<String>,
    pub supply_cap: Option<U128>,
    pub divisibility: u8,
    pub live_time: BlockHeight,
    pub mint_mechanism: MBAMintMechanisms,
    pub burn_mechanism: BurnMechanisms,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MBAMintMechanisms {
    pub preallocated: Option<Preallocated>,
    pub free_mint: Option<FreeMint>,
    pub purchase: Option<PurchaseBurnSwap>,
    pub collateralized: Option<Collateralized>
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct BurnMechanisms {
    pub return_collateral: Option<ReturnCollateral>
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct Collateralized {
    pub input_assets: Vec<InputAsset>,
    pub is_asset_mutable: bool,
    pub mint_structure: MintStructure
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MintStructure {
    Ratio(RatioType),
    Proportional(), // TODO: proportional MBA (for AMM)
    Account(AccountType),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct AccountType {
    pub max_ltv: Fraction,
    pub ratio: RatioType
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ReturnCollateral {
    pub fee: Option<Fraction>
}