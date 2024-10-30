use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub struct BlockTx {
    pub block: u64,
    pub tx: u32,
}

pub type Ratio = (u128, u128);
pub type BlockTxTuple = (u64, u32);
pub type BlockHeight = u64;
pub type BitcoinAddress = String;

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

    // TODO: handle error
    pub fn from_str(s: &str) -> Self {
        let (block, tx) = s.split_once(':').expect("Invalid block tx format");

        BlockTx {
            block: block.parse().expect("Invalid block tx block format"),
            tx: tx.parse().expect("Invalid block tx block format"),
        }
    }
}

pub struct Outpoint {
    pub txid: String,
    pub vout: u32,
}

impl fmt::Display for Outpoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.txid, self.vout)
    }
}

impl Outpoint {
    pub fn to_string(&self) -> String {
        format!("{}:{}", self.txid, self.vout)
    }

    // TODO: handle error
    pub fn from_str(s: &str) -> Self {
        let (txid, vout) = s.split_once(':').expect("Invalid Outpoint format");
        Outpoint {
            txid: txid.to_string(),
            vout: vout.parse().expect("Invalid Outpoint vout format"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
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
