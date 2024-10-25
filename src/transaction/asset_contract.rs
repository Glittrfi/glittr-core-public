use std::str::FromStr;

use bitcoin::{Address, XOnlyPublicKey};
use flaw::Flaw;

use super::*;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct AssetContractFreeMint {
    pub supply_cap: Option<u32>,
    // TODO change the type to u128, need to check the serialization and deserialization since JSON
    // has a MAX number limitation.
    pub amount_per_mint: u32,
    pub divisibility: u8,
    pub live_time: BlockHeight,
}
impl AssetContractFreeMint {
    pub fn validate(&self) -> Option<Flaw> {
        if let Some(supply_cap) = self.supply_cap {
            if self.amount_per_mint > supply_cap {
                return Some(Flaw::OverflowAmountPerMint);
            }
        }
        None
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum AssetContract {
    Preallocated { todo: Option<()> },
    FreeMint(AssetContractFreeMint),
    PurchaseBurnSwap(AssetContractPurchaseBurnSwap),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct AssetContractPurchaseBurnSwap {
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
pub struct OracleSetting {
    /// set asset_id to none to fully trust the oracle, ordinal_number if ordinal, rune's block_tx if rune, etc
    pub asset_id: Option<String>,
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

impl AssetContract {
    pub fn validate(&self) -> Option<Flaw> {
        match self {
            AssetContract::Preallocated { todo: _ } => {
                // TODO: add and validate preallocated
            }
            AssetContract::FreeMint(free_mint) => return free_mint.validate(),
            AssetContract::PurchaseBurnSwap(AssetContractPurchaseBurnSwap {
                input_asset,
                transfer_scheme,
                transfer_ratio_type,
            }) => {
                if let InputAsset::GlittrAsset(block_tx_tuple) = input_asset {
                    if block_tx_tuple.1 == 0 {
                        return Some(Flaw::InvalidBlockTxPointer);
                    }
                }

                if let TransferScheme::Purchase(bitcoin_address) = transfer_scheme {
                    if Address::from_str(bitcoin_address.as_str()).is_err() {
                        return Some(Flaw::InvalidBitcoinAddress);
                    }
                }

                match &transfer_ratio_type {
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
        }

        None
    }
}
