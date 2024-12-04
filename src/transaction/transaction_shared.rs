use super::*;
use std::collections::HashMap;

use bitcoin::{PublicKey, XOnlyPublicKey};
use message::ContractType;

pub trait ContractValidator {
    fn validate(&self) -> Option<Flaw>;
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MintMechanisms {
    pub preallocated: Option<Preallocated>,
    pub free_mint: Option<FreeMint>,
    pub purchase: Option<PurchaseBurnSwap>,
}

/// Pre-allocated (e.g. allowing anyone who owns certain ordinals to claim, or literally hardcoding for TGE)
/// * Allocations & amounts -> For now, just a list of public keys
///     Formatted {public key: amount} or if multiple keys getting same allocation {key, key, key: amount1}, {key, key: amount2}
/// If total allocation is less than supply cap, the remainder is a free mint
/// * Time lock or vesting schedule
///    - Time lock (Block height)
/// * Vesting schedule
///    - List of floats (percentage unlock)
///    - List of block heights
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct Preallocated {
    pub allocations: HashMap<U128, Vec<Pubkey>>,
    pub vesting_plan: Option<VestingPlan>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum VestingPlan {
    Timelock(RelativeOrAbsoluteBlockHeight),
    Scheduled(Vec<(Fraction, RelativeOrAbsoluteBlockHeight)>),
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct FreeMint {
    pub supply_cap: Option<U128>,
    pub amount_per_mint: U128,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct PurchaseBurnSwap {
    pub input_asset: InputAsset,
    pub pay_to_key: Option<Pubkey>, // None is burn
    pub ratio: RatioType,
}

impl Preallocated {
    pub fn validate(&self, contract: &ContractType) -> Option<Flaw> {
        let supply_cap: Option<U128>;
        let free_mint: Option<FreeMint>;
        match contract {
            ContractType::Moa(mint_only_asset_contract) => {
                supply_cap = mint_only_asset_contract.supply_cap.clone();
                free_mint = mint_only_asset_contract.mint_mechanism.free_mint.clone();
            }
            ContractType::Mba(mint_burn_asset_contract) => {
                supply_cap = mint_burn_asset_contract.supply_cap.clone();
                free_mint = mint_burn_asset_contract.mint_mechanism.free_mint.clone();
            }
            ContractType::Spec(_) => {
                supply_cap = None;
                free_mint = None;
            },
        }

        if let Some(supply_cap) = &supply_cap {
            let mut total_allocations: u128 = 0;
            for alloc in &self.allocations {
                total_allocations = total_allocations
                    .saturating_add(alloc.0 .0.saturating_mul(alloc.1.len() as u128));
            }

            if total_allocations > supply_cap.0 {
                return Some(Flaw::SupplyCapInvalid);
            }

            if total_allocations < supply_cap.0 {
                let mut remainder = supply_cap.0 - total_allocations;
                if let Some(free_mint) = &free_mint {
                    if let Some(free_mint_supply_cap) = &free_mint.supply_cap {
                        if free_mint_supply_cap.0 > remainder {
                            return Some(Flaw::SupplyCapInvalid);
                        }
                        remainder = remainder.saturating_sub(free_mint_supply_cap.0);
                    } else {
                        return Some(Flaw::SupplyCapInvalid);
                    }
                } else {
                    return Some(Flaw::SupplyRemainder);
                }

                if remainder > 0 {
                    return Some(Flaw::SupplyRemainder);
                }
            }
        } else {
            return Some(Flaw::SupplyCapInvalid);
        }

        None
    }
}

impl FreeMint {
    pub fn validate(&self, contract: &ContractType) -> Option<Flaw> {
        let super_supply_cap = match contract {
            ContractType::Moa(mint_only_asset_contract) => {
                mint_only_asset_contract.supply_cap.clone()
            }
            ContractType::Mba(mint_burn_asset_contract) => {
                mint_burn_asset_contract.supply_cap.clone()
            }
            ContractType::Spec(_) => {
                None
            },
        };

        if let Some(supply_cap) = &self.supply_cap {
            if self.amount_per_mint.0 > supply_cap.0 {
                return Some(Flaw::OverflowAmountPerMint);
            }

            if let Some(super_supply_cap) = super_supply_cap {
                if super_supply_cap.0 < supply_cap.0 {
                    return Some(Flaw::SupplyCapInvalid);
                }
            } else {
                return Some(Flaw::SupplyCapInvalid);
            }
        }
        None
    }
}

impl PurchaseBurnSwap {
    pub fn validate(&self) -> Option<Flaw> {
        if let InputAsset::GlittrAsset(block_tx_tuple) = self.input_asset {
            if block_tx_tuple.1 == 0 {
                return Some(Flaw::InvalidBlockTxPointer);
            }
        }

        if let Some(pubkey) = &self.pay_to_key {
            if PublicKey::from_slice(&pubkey.as_slice()).is_err() {
                return Some(Flaw::PubkeyInvalid);
            }
        }

        return self.ratio.validate();
    }
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InputAsset {
    RawBtc,
    GlittrAsset(BlockTxTuple),
    Rune,
    Ordinal,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RatioType {
    Fixed {
        ratio: Fraction, // out_value = input_value * ratio
    },
    Oracle {
        setting: OracleSetting,
    },
}

impl RatioType {
    pub fn validate(&self) -> Option<Flaw> {
        match &self {
            RatioType::Fixed { ratio } => {
                if ratio.1 == 0 {
                    return Some(Flaw::DivideByZero);
                }
            }
            RatioType::Oracle { setting } => {
                if XOnlyPublicKey::from_slice(&setting.pubkey).is_err() {
                    return Some(Flaw::PubkeyInvalid);
                }
            }
        }

        None
    }
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct OracleSetting {
    /// compressed public key
    pub pubkey: Pubkey, 
    /// set asset_id to null for fully trust the oracle, ordinal_number if ordinal, rune's block_tx if rune, etc
    pub asset_id: Option<String>,
    /// delta block_height in which the oracle message still valid
    pub block_height_slippage: u8,
}
