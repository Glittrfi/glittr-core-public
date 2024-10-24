use std::str::FromStr;

use bitcoin::{Address, XOnlyPublicKey};
use flaw::Flaw;

use super::*;

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct AssetContractFreeMint {
    supply_cap: Option<u32>,
    amount_per_mint: u32,
    divisibility: u8,
    live_time: BlockHeight,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum AssetContract {
    Preallocated {
        todo: Option<()>,
    },
    FreeMint {
        supply_cap: Option<u32>,
        amount_per_mint: u32,
        divisibility: u8,
        live_time: BlockHeight,
    },
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
            AssetContract::FreeMint {
                supply_cap,
                amount_per_mint,
                divisibility: _,
                live_time: _,
            } => {
                if let Some(supply_cap) = supply_cap {
                    if amount_per_mint > supply_cap {
                        return Some(Flaw::OverflowAmountPerMint);
                    }
                }

                // TODO: validate divisibility value
                // TODO: validate live_time value (block_height must be valid)
            }
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
