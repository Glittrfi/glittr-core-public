use crate::config::get_bitcoin_network;

pub const GLITTR_FIRST_BLOCK_HEIGHT_REGTEST: u64 = 0;
pub const GLITTR_FIRST_BLOCK_HEIGHT_MAINNET: u64 = 0;
pub const GLITTR_FIRST_BLOCK_HEIGHT_TESTNET: u64 = 0;
pub const GLITTR_FIRST_BLOCK_HEIGHT_SIGNET: u64 = 0;
pub const OP_RETURN_MAGIC_PREFIX: &str = "GLITTR";

pub fn first_glittr_height() -> u64 {
    let bitcoin_network = get_bitcoin_network();

    match bitcoin_network{
        bitcoin::Network::Bitcoin => GLITTR_FIRST_BLOCK_HEIGHT_MAINNET, 
        bitcoin::Network::Testnet => GLITTR_FIRST_BLOCK_HEIGHT_TESTNET,
        bitcoin::Network::Signet => GLITTR_FIRST_BLOCK_HEIGHT_SIGNET,
        bitcoin::Network::Regtest => GLITTR_FIRST_BLOCK_HEIGHT_REGTEST,
        _ => GLITTR_FIRST_BLOCK_HEIGHT_REGTEST
    }
}
