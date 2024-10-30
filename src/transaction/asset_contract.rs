use std::str::FromStr;

use bitcoin::{Address, XOnlyPublicKey};
use flaw::Flaw;

use super::*;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct FreeMint {
    pub supply_cap: Option<u32>,
    // TODO change the type to u128, need to check the serialization and deserialization since JSON
    // has a MAX number limitation.
    pub amount_per_mint: u32,
}

impl FreeMint {
    pub fn validate(&self, asset_contract: &AssetContract) -> Option<Flaw> {
        if let Some(supply_cap) = self.supply_cap {
            if self.amount_per_mint > supply_cap {
                return Some(Flaw::OverflowAmountPerMint);
            }

            if let Some(super_supply_cap) = asset_contract.asset.supply_cap {
                if super_supply_cap < supply_cap {
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
    pub supply_cap: Option<u32>,
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
    pub preallocated: Option<()>,
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
        pubkey: Vec<u8>, // compressed public key
        setting: OracleSetting,
    },
}

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

        if let Some(freemint) = &self.distribution_schemes.free_mint {
            return freemint.validate(self);
        }

        if let Some(purchase) = &self.distribution_schemes.purchase {
            if let InputAsset::GlittrAsset(block_tx_tuple) = purchase.input_asset {
                if block_tx_tuple.1 == 0 {
                    return Some(Flaw::InvalidBlockTxPointer);
                }
            }

            if let TransferScheme::Purchase(bitcoin_address) = &purchase.transfer_scheme {
                if Address::from_str(bitcoin_address.as_str()).is_err() {
                    return Some(Flaw::InvalidBitcoinAddress);
                }
            }

            match &purchase.transfer_ratio_type {
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
        }

        None
    }
}
