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

    // transfer
    OutputOverflow(Vec<u32>),

    // call type
    MessageInvalid,
    ContractNotMatch,
    ContractNotFound,
    AssetContractDataNotFound,
    PointerOverflow,
    InvalidPointer,

    // call type::mint
    SupplyCapExceeded,
    LiveTimeNotReached,
    OracleMintFailed,
    OracleMintInputNotFound,
    OracleMintBelowMinValue,
    OracleMintSignatureFailed,
    OracleMintBlockSlippageExceeded,

    // asset contract
    OverflowAmountPerMint,
    DivideByZero,
    PubkeyInvalid,
    OracleMessageFormatInvalid,
    SupplyCapInvalid,

    NotImplemented,
}
