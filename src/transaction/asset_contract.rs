use std::str::FromStr;

use bitcoin::{Address, XOnlyPublicKey};
use flaw::Flaw;

use super::*;

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
    // TODO: optimize for multiple pubkey getting the same allocation
    allocations: Vec<(U128, Pubkey)>, // (allocation, pubkey)
    vesting_plan: VestingPlan,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum VestingPlan {
    Timelock(RelativeOrAbsoluteBlockHeight),
    Scheduled(Vec<(Ratio, RelativeOrAbsoluteBlockHeight)>),
}

impl Preallocated {
    pub fn validate(&self, asset_contract: &AssetContract) -> Option<Flaw> {
        if let VestingPlan::Scheduled(schedules) = &self.vesting_plan {
            if self.allocations.len() != schedules.len() {
                return Some(Flaw::PreallocatedLengthInvalid);
            }
        }

        if let Some(supply_cap) = &asset_contract.asset.supply_cap {
            let mut total_allocations = 0;
            for alloc in &self.allocations {
                total_allocations += alloc.0 .0;
            }

            if total_allocations > supply_cap.0 {
                return Some(Flaw::SupplyCapInvalid);
            }

            if total_allocations < supply_cap.0 {
                let remainder = supply_cap.0 - total_allocations;
                if let Some(free_mint) = &asset_contract.distribution_schemes.free_mint {
                    if let Some(free_mint_supply_cap) = &free_mint.supply_cap {
                        if free_mint_supply_cap.0 > remainder {
                            return Some(Flaw::SupplyCapInvalid);
                        }
                    } else {
                        return Some(Flaw::SupplyCapInvalid);
                    }
                } else {
                    return Some(Flaw::PreallocatedSupplyRemainderWithoutFreeMint);
                }
            }
        } else {
            return Some(Flaw::SupplyCapInvalid);
        }

        None
    }
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct FreeMint {
    pub supply_cap: Option<U128>,
    // TODO change the type to u128, need to check the serialization and deserialization since JSON
    // has a MAX number limitation.
    pub amount_per_mint: U128,
}

impl FreeMint {
    pub fn validate(&self, asset_contract: &AssetContract) -> Option<Flaw> {
        if let Some(supply_cap) = &self.supply_cap {
            if self.amount_per_mint.0 > supply_cap.0 {
                return Some(Flaw::OverflowAmountPerMint);
            }

            if let Some(super_supply_cap) = &asset_contract.asset.supply_cap {
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

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct SimpleAsset {
    pub supply_cap: Option<U128>,
    pub divisibility: u8,
    pub live_time: BlockHeight,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct AssetContract {
    pub asset: SimpleAsset,
    pub distribution_schemes: DistributionSchemes,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct DistributionSchemes {
    pub preallocated: Option<Preallocated>,
    pub free_mint: Option<FreeMint>,
    pub purchase: Option<PurchaseBurnSwap>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct PurchaseBurnSwap {
    pub input_asset: InputAsset,
    pub transfer_scheme: TransferScheme,
    pub transfer_ratio_type: TransferRatioType,
}

impl PurchaseBurnSwap {
    pub fn validate(&self) -> Option<Flaw> {
        if let InputAsset::GlittrAsset(block_tx_tuple) = self.input_asset {
            if block_tx_tuple.1 == 0 {
                return Some(Flaw::InvalidBlockTxPointer);
            }
        }

        if let TransferScheme::Purchase(bitcoin_address) = &self.transfer_scheme {
            if Address::from_str(bitcoin_address.as_str()).is_err() {
                return Some(Flaw::InvalidBitcoinAddress);
            }
        }

        match &self.transfer_ratio_type {
            TransferRatioType::Fixed { ratio } => {
                if ratio.1 == 0 {
                    return Some(Flaw::DivideByZero);
                }
            }
            TransferRatioType::Oracle { pubkey, setting: _ } => {
                if XOnlyPublicKey::from_slice(pubkey).is_err() {
                    return Some(Flaw::PubkeyInvalid);
                }
            }
        }

        None
    }
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum InputAsset {
    RawBTC,
    GlittrAsset(BlockTxTuple),
    Metaprotocol,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransferScheme {
    Purchase(BitcoinAddress),
    /// NOTE: btc burned must go to op_return, bitcoind must set maxburnamount
    Burn,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransferRatioType {
    Fixed {
        ratio: Ratio, // out_value = input_value * ratio
    },
    Oracle {
        pubkey: Pubkey, // compressed public key
        setting: OracleSetting,
    },
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct OracleSetting {
    /// set asset_id to none to fully trust the oracle, ordinal_number if ordinal, rune's block_tx if rune, etc
    pub asset_id: Option<String>,
    /// delta block_height in which the oracle message still valid
    pub block_height_slippage: u8,
}

/// Mix of distribution schemes only applicable for preallocated and free_mint or preallocated and purchase
impl AssetContract {
    pub fn validate(&self) -> Option<Flaw> {
        if self.distribution_schemes.purchase.is_some()
            && self.distribution_schemes.free_mint.is_some()
        {
            return Some(Flaw::NotImplemented);
        }

        if let Some(preallocated) = &self.distribution_schemes.preallocated {
            return preallocated.validate(self);
        }

        if let Some(freemint) = &self.distribution_schemes.free_mint {
            return freemint.validate(self);
        }

        if let Some(purchase) = &self.distribution_schemes.purchase {
            return purchase.validate();
        }

        None
    }
}
