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
    OracleMintInfoFailed,
    MintedZero,
    VestingBlockNotReached,
    VesteeNotFound,
    PointerKeyNotFound,
    StateKeyNotFound,
    LtvMustBeUpdated,
    MaxLtvExceeded,
    OutValueNotFound,
    OutstandingMustBeUpdated,

    // call type::burn
    BurnValueIncorrect,

    // call type::close account
    LtvMustBeZero,
    OutstandingMustBeZero,
    
    // call type: swap
    InsufficientInputAmount,
    InsufficientOutputAmount,

    // asset contract
    OverflowAmountPerMint,
    DivideByZero,
    PubkeyInvalid,
    OracleMessageFormatInvalid,
    SupplyCapInvalid,
    SupplyRemainder,
    FractionInvalid,
    InvalidContractType,

    // asset contract: mba
    InvalidInputAssetCount,
    InvalidConstantProduct,
    InputAssetsOnlyOneForRatio,
    InputAssetsOnlyTwoForProportional,

    // spec
    SpecNotMutable,
    SpecFieldRequired(String),
    SpecFieldNotNecessary(String),
    SpecNotFound,
    SpecCriteriaInvalid,
    SpecOwnerNotFound, // should be never happened
    SpecUpdateNotAllowed,

    NotImplemented,
    NonGlittrMessage,
    PoolNotFound,
}
