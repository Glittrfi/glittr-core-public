use super::*;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Flaw {
    // parse tx
    InvalidInstruction(String),
    InvalidScript,
    FailedDeserialization,

    InvalidBlockTxPointer,
    ReferencingFlawedBlockTx,
    InvalidBitcoinAddress,

    // call type
    MessageInvalid,
    ContractNotFound,
    ContractNotMatch,
    OracleMintFailed,
    InvalidMintPointer,

    // asset contract
    OverflowAmountPerMint,
    DivideByZero,
    PubkeyInvalid,
    OracleMessageFormatInvalid,

    NotImplemented,
}
