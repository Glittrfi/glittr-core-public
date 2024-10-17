use std::fmt;

pub struct BlockTx {
    pub block: u64,
    pub tx: u32,
}

pub type BlockTxTuple = (u64, u32);
pub type BlockHeight = u32;
pub type BitcoinAddress = String;

impl fmt::Display for BlockTx {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.block, self.tx)
    }
}

impl BlockTx {
    pub fn to_tuple(&self) -> BlockTxTuple {
        (self.block, self.tx)
    }
}
