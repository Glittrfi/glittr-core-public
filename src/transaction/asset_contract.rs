use super::*;

#[serde(rename_all = "snake_case")]
#[derive(Deserialize, Serialize)]
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

#[serde(rename_all = "snake_case")]
#[derive(Deserialize, Serialize)]
pub enum InputAssetType {
    RawBTC,
    Rune,
    GlittrAsset,
}

#[serde(rename_all = "snake_case")]
#[derive(Deserialize, Serialize)]
pub enum TransferScheme {
    Purchase(BitcoinAddress),
    Burn,
}

#[serde(rename_all = "snake_case")]
#[derive(Deserialize, Serialize)]
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
    pub fn validate() {
        todo!("validate if all parameters are valid, e.g. ");
    }
}
