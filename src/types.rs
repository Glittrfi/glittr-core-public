use std::fmt;

#[derive(Clone, Copy, Debug)]
pub struct BlockTx {
    pub block: u64,
    pub tx: u32,
}

pub type Ratio = (u32, u32);
pub type BlockTxTuple = (u64, u32);
pub type BlockHeight = u32;
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
    pub fn to_str(&self) -> String {
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
