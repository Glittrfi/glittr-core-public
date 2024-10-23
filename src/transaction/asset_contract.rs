use std::str::FromStr;

use bitcoin::Address;
use flaw::Flaw;
use serde_json::value::RawValue;

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
        input_asset: InputAsset,
        transfer_scheme: TransferScheme,
        transfer_ratio_type: TransferRatioType,
    },
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum InputAsset {
    RawBTC,
    Rune(BlockTxTuple),
    GlittrAsset(BlockTxTuple),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransferScheme {
    Purchase(BitcoinAddress),
    Burn, // NOTE: btc burned must go to op_return, bitcoind must set maxburnamount
}

#[derive(Deserialize, Serialize)]
pub struct OracleMessage<'a> {
    #[serde(borrow)]
    payload: &'a RawValue,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransferRatioType {
    Fixed {
        ratio: Ratio, // out_value = input_value * ratio
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
                input_asset,
                transfer_scheme,
                transfer_ratio_type,
            } => {
                if let InputAsset::GlittrAsset(block_tx_tuple) = input_asset {
                    if block_tx_tuple.1 == 0 {
                        return Some(Flaw::InvalidBlockTxPointer);
                    }
                } else if let InputAsset::Rune(block_tx_tuple) = input_asset {
                    if block_tx_tuple.1 == 0 {
                        return Some(Flaw::InvalidBlockTxPointer);
                    }
                }

                if let TransferScheme::Purchase(bitcoin_address) = transfer_scheme {
                    if let Err(_) = Address::from_str(bitcoin_address.as_str()) {
                        return Some(Flaw::InvalidBitcoinAddress);
                    }
                }

                match &transfer_ratio_type {
                    TransferRatioType::Fixed { ratio } => {
                        if ratio.1 == 0 {
                            return Some(Flaw::DivideByZero);
                        }
                    }
                    TransferRatioType::Oracle { pubkey, message } => {
                        if pubkey.len() != 33 || pubkey.len() != 32 {
                            return Some(Flaw::PubkeyLengthInvalid);
                        }

                        if let Err(_) = serde_json::from_str::<OracleMessage>(&message.as_str()) {
                            return Some(Flaw::OracleMessageFormatInvalid);
                        }
                    }
                }
            }
        }

        None
    }
}
