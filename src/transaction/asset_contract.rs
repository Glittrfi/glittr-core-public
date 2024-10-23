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
    PurchaseBurnSwap {
        input_asset_type: InputAssetType,
        input_asset: Option<BlockTxTuple>,
        transfer_scheme: TransferScheme,
        transfer_ratio_type: TransferRatioType,
    },
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum InputAssetType {
    RawBTC,
    Rune,
    GlittrAsset,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransferScheme {
    Purchase(BitcoinAddress),
    Burn,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransferRatioType {
    Fixed {
        ratio: u16,
    },
    Oracle {
        pubkey: Vec<u8>, // compressed public key
        message: String, // todo: Arguments expected in oracle message
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
            AssetContract::PurchaseBurnSwap {
                input_asset_type: _,
                input_asset: _,
                transfer_scheme: _,
                transfer_ratio_type: _,
            } => {
                // TODO: validation
            }
        }

        None
    }
}
