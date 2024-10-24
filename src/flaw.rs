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

    // asset contract
    OverflowAmountPerMint,
    DivideByZero,
    PubkeyLengthInvalid,
    OracleMessageFormatInvalid,

    NotImplemented,
}
