use super::*;

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
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
    ContractNotMatch,
    ContractNotFound,
    PointerOverflow,

    // call type::mint
    SupplyCapExceeded,

    // asset contract
    OverflowAmountPerMint,
    DivideByZero,
    PubkeyLengthInvalid,
    OracleMessageFormatInvalid,
}
