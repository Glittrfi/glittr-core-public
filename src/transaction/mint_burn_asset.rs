use super::*;
use transaction_shared::{FreeMint, InputAsset, OracleSetting, Preallocated, PurchaseBurnSwap, RatioType};

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
    pub swap_mechanism: SwapMechanisms,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MBAMintMechanisms {
    pub preallocated: Option<Preallocated>,
    pub free_mint: Option<FreeMint>,
    pub purchase: Option<PurchaseBurnSwap>,
    pub collateralized: Option<Collateralized>,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct BurnMechanisms {
    pub return_collateral: Option<ReturnCollateral>,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct SwapMechanisms {
    pub fee: Option<U128>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct Collateralized {
    pub input_assets: Vec<InputAsset>,
    pub _mutable_assets: bool,
    pub mint_structure: MintStructure,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MintStructure {
    Ratio(RatioType),
    Proportional(ProportionalType),
    Account(AccountType),
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct AccountType {
    pub max_ltv: Fraction,
    pub ratio: RatioType,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ProportionalType {
    pub ratio_model: RatioModel,
    // The initial mint can be restricted to a state key (utxo)
    pub inital_mint_pointer_to_key: Option<u32>, 
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RatioModel {
    ConstantProduct
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ReturnCollateral {
    pub fee: Option<Fraction>, // TODO: fee
    pub oracle_setting: Option<OracleSetting>,
}

impl Collateralized {
    pub fn validate(&self) -> Option<Flaw> {
        for input_asset in &self.input_assets {
            match input_asset {
                InputAsset::GlittrAsset(_) => {}
                _ => {
                    return Some(Flaw::NotImplemented);
                }
            }
        }

        match &self.mint_structure {
            MintStructure::Ratio(ratio_type) => {
                if self.input_assets.len() != 1 {
                    return Some(Flaw::InputAssetsOnlyOneForRatio)
                }
                return ratio_type.validate()
            }
                ,
            MintStructure::Proportional(_proportional_type) => {
                if self.input_assets.len() != 2 {
                    return Some(Flaw::InputAssetsOnlyTwoForProportional)
                }
            },
            MintStructure::Account(account) => {
                if account.max_ltv.0 > account.max_ltv.1 {
                    return Some(Flaw::FractionInvalid);
                }
                return account.ratio.validate();
            }
        }

        None
    }
}

impl ReturnCollateral {
    pub fn validate(&self) -> Option<Flaw> {
        if let Some(fee) = self.fee {
            if fee.0 > fee.1 {
                return Some(Flaw::FractionInvalid);
            }
        }
        None
    }
}

impl MintBurnAssetContract {
    pub fn validate(&self) -> Option<Flaw> {
        if self.mint_mechanism.purchase.is_some() && self.mint_mechanism.free_mint.is_some() {
            return Some(Flaw::NotImplemented);
        }

        if let Some(preallocated) = &self.mint_mechanism.preallocated {
            return preallocated.validate(&message::ContractType::Mba(self.clone()));
        }

        if let Some(freemint) = &self.mint_mechanism.free_mint {
            return freemint.validate(&message::ContractType::Mba(self.clone()));
        }

        if let Some(purchase) = &self.mint_mechanism.purchase {
            return purchase.validate();
        }

        if let Some(collateralized) = &self.mint_mechanism.collateralized {
            if let Some(return_collateral) = &self.burn_mechanism.return_collateral {
                let validate = return_collateral.validate();
                if validate.is_some() {
                    return validate;
                }
            }
            return collateralized.validate();
        }

        None
    }
}
