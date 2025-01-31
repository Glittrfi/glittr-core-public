use crate::varuint::Varuint;
use bitcoin::{OutPoint, Transaction};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Default)]
pub struct BlockTx {
    pub block: Varuint<u64>,
    pub tx: Varuint<u32>,
}

pub type Fraction = (Varuint<u64>, Varuint<u64>);
pub type BlockTxTuple = (Varuint<u64>, Varuint<u32>);
pub type BlockHeight = Varuint<u64>;
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

    pub fn to_string(&self) -> String {
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

// internal wrapper for bitcoin outpoint
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub struct BitcoinOutpoint(pub OutPoint);

impl From<OutPoint> for BitcoinOutpoint {
    fn from(outpoint: OutPoint) -> Self {
        BitcoinOutpoint(outpoint)
    }
}

impl Into<OutPoint> for BitcoinOutpoint {
    fn into(self) -> OutPoint {
        self.0
    }
}

// internal wrapper for bitcoin transaction
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct BitcoinTransaction(pub Transaction);

impl From<Transaction> for BitcoinTransaction {
    fn from(tx: Transaction) -> Self {
        BitcoinTransaction(tx)
    }
}

impl Into<Transaction> for BitcoinTransaction {
    fn into(self) -> Transaction {
        self.0
    }
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct LastIndexedBlock(pub u64);
