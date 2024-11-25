use std::{error::Error, fmt, str::FromStr};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub struct BlockTx {
    pub block: u64,
    pub tx: u32,
}

pub type Fraction = (u64, u64);
pub type BlockTxTuple = (u64, u32);
pub type BlockHeight = u64;
pub type BitcoinAddress = String;
pub type BlockTxString = String;
pub type Pubkey = Vec<u8>;

/// negative indicates relative block height, delta from mined contract's block height
pub type RelativeOrAbsoluteBlockHeight = i64;

impl fmt::Display for BlockTx {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.block, self.tx)
    }
}

impl BlockTx {
    pub fn from_tuple(block_tx_tuple: BlockTxTuple) -> Self {
        BlockTx {
            block: block_tx_tuple.0,
            tx: block_tx_tuple.1,
        }
    }

    pub fn to_tuple(&self) -> BlockTxTuple {
        (self.block, self.tx)
    }

    pub fn to_str(&self) -> String {
        format!("{}:{}", self.block, self.tx)
    }
}

impl FromStr for BlockTx {
    type Err = Box<dyn Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (block, tx) = s.split_once(':').ok_or("Split error")?;

        Ok(BlockTx {
            block: block.parse().expect("Invalid block tx block format"),
            tx: tx.parse().expect("Invalid block tx block format"),
        })
    }
}

// TODO: remove this, use OutPoint from bitcoin lib instead
pub struct Outpoint {
    pub txid: String,
    pub vout: u32,
}

impl fmt::Display for Outpoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.txid, self.vout)
    }
}

impl FromStr for Outpoint {
    type Err = Box<dyn Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (txid, vout) = s.split_once(':').ok_or("Split error")?;
        Ok(Outpoint {
            txid: txid.to_string(),
            vout: vout.parse().expect("Invalid Outpoint vout format"),
        })
    }
}

/// U128 is wrapped u128, represented as string when serialized
/// This is because JSON only supports up to u32 as integer representation
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct U128(pub u128);

impl Serialize for U128 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for U128 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        Ok(Self(str::parse::<u128>(&s).map_err(|err| {
            serde::de::Error::custom(err.to_string())
        })?))
    }
}
