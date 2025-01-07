use std::fmt;

use super::*;
use bitcoin::{
    opcodes,
    script::{self, Instruction, PushBytes},
    ScriptBuf, Transaction,
};
use bitcoincore_rpc::jsonrpc::serde_json::{self};
use compression::{Brotli, Compression};
use constants::OP_RETURN_MAGIC_PREFIX;
use flaw::Flaw;
use mint_burn_asset::MintBurnAssetContract;
use mint_only_asset::MintOnlyAssetContract;
use spec::SpecContract;

use borsh::{BorshDeserialize, BorshSerialize};

#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ContractType {
    Moa(MintOnlyAssetContract),
    Mba(MintBurnAssetContract),
    Spec(SpecContract),
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct MintBurnOption {
    pub pointer: Option<u32>,
    pub oracle_message: Option<OracleMessageSigned>,
    pub pointer_to_key: Option<u32>,
    pub assert_values: Option<AssertValues>,
    pub commitment_message: Option<CommitmentMessage>,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct SwapOption {
    pub pointer: u32,
    pub assert_values: Option<AssertValues>,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct AssertValues {
    pub input_values: Option<Vec<U128>>,
    pub total_collateralized: Option<Vec<U128>>,
    pub min_out_value: Option<U128>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    Mint(MintBurnOption),
    Burn(MintBurnOption),
    Swap(SwapOption),
    // Collateralized assets
    OpenAccount(OpenAccountOption),
    CloseAccount(CloseAccountOption), // TODO: partial return & fee
}

#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct OpenAccountOption {
    pub pointer_to_key: u32,
    pub share_amount: U128, // representation of total value of the inputs
}

#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct CloseAccountOption {
    pub pointer: u32,
}

#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct OracleMessageSigned {
    pub signature: Vec<u8>,
    pub message: OracleMessage,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct OracleMessage {
    /// the input_outpoint dictates which UTXO is being evaluated by the Oracle
    pub input_outpoint: Option<BitcoinOutpoint>,
    /// min_in_value represents what the input valued at (minimum because btc value could differ (-fee))
    pub min_in_value: Option<U128>,
    /// out_value represents the oracle's valuation of the input e.g. for 1 btc == 72000 wusd, 72000 is the out_value
    pub out_value: Option<U128>,
    /// rune's BlockTx if rune
    pub asset_id: Option<String>,
    // for raw_btc input it is more straightforward using ratio, output = received_value * ratio
    pub ratio: Option<Fraction>,
    pub ltv: Option<Fraction>,
    pub outstanding: Option<U128>,
    // the bitcoin block height when the message is signed
    pub block_height: u64,
}

#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct Commitment {
    pub public_key: Pubkey,
    pub args: ArgsCommitment,
}

#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct ArgsCommitment {
    pub fixed_string: String,
    pub string: String,
}

#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct CommitmentMessage {
    pub public_key: Pubkey,
    pub args: Vec<u8>,
}

/// Transfer
/// Asset: This is a block:tx reference to the contract where the asset was created
/// Output index of output to receive asset
/// Amount: value assigning shares of the transfer to the appropriate UTXO output
#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct TxTypeTransfer {
    pub asset: BlockTxTuple,
    pub output: u32,
    pub amount: U128,
}

// TxTypes: Transfer, ContractCreation, ContractCall
#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct Transfer {
    pub transfers: Vec<TxTypeTransfer>,
}

#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ContractCreation {
    pub contract_type: ContractType,
    pub spec: Option<BlockTxTuple>,
}

#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ContractCall {
    pub contract: Option<BlockTxTuple>,
    pub call_type: CallType,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct OpReturnMessage {
    pub transfer: Option<Transfer>,
    pub contract_creation: Option<ContractCreation>,
    pub contract_call: Option<ContractCall>,
}

pub trait ContractValidator {
    fn validate(&self) -> Option<Flaw>;
}

impl ContractValidator for CallType {
    fn validate(&self) -> Option<Flaw> {
        None
    }
}

impl OpReturnMessage {
    pub fn parse_tx(tx: &Transaction) -> Result<OpReturnMessage, Flaw> {
        let mut payload = Vec::new();

        for output in tx.output.iter() {
            let mut instructions = output.script_pubkey.instructions();

            if instructions.next() != Some(Ok(Instruction::Op(opcodes::all::OP_RETURN))) {
                continue;
            }

            let signature = instructions.next();
            if let Some(Ok(Instruction::PushBytes(glittr_message))) = signature {
                if glittr_message.as_bytes() != OP_RETURN_MAGIC_PREFIX.as_bytes() {
                    continue;
                }
            } else {
                continue;
            }

            for result in instructions {
                match result {
                    Ok(Instruction::PushBytes(push)) => {
                        payload.extend_from_slice(push.as_bytes());
                    }
                    Ok(Instruction::Op(op)) => {
                        return Err(Flaw::InvalidInstruction(op.to_string()));
                    }
                    Err(_) => {
                        return Err(Flaw::InvalidScript);
                    }
                }
            }
            break;
        }

        if payload.is_empty() {
            return Err(Flaw::NonGlittrMessage);
        }

        // TODO: singleton for brotli
        let mut brotli = Brotli::new(11, 22, 4096);
        let decompress = brotli.decompress(&payload).unwrap();
        let message = borsh::from_slice::<OpReturnMessage>(&decompress);

        match message {
            Ok(message) => {
                if message.contract_call.is_none()
                    && message.contract_creation.is_none()
                    && message.transfer.is_none()
                {
                    return Err(Flaw::FailedDeserialization);
                }
                Ok(message)
            }
            Err(_) => Err(Flaw::FailedDeserialization),
        }
    }

    pub fn validate(&self) -> Option<Flaw> {
        if let Some(contract_creation) = &self.contract_creation {
            return match &contract_creation.contract_type {
                ContractType::Moa(mint_only_asset_contract) => mint_only_asset_contract.validate(),
                ContractType::Mba(mint_burn_asset_contract) => mint_burn_asset_contract.validate(),
                ContractType::Spec(spec_contract) => spec_contract.validate(),
            };
        }

        if let Some(contract_call) = &self.contract_call {
            return contract_call.call_type.validate();
        }

        None
    }

    pub fn into_script(&self) -> ScriptBuf {
        let mut builder = script::Builder::new().push_opcode(opcodes::all::OP_RETURN);
        let magic_prefix: &PushBytes = OP_RETURN_MAGIC_PREFIX.as_bytes().try_into().unwrap();

        let binding = borsh::to_vec(self).unwrap();
        // TODO: singleton for brotli
        let mut brotli = Brotli::new(11, 22, 4096);
        let binding_compressed = brotli.compress(&binding).unwrap();

        let script_bytes: &PushBytes = binding_compressed.as_slice().try_into().unwrap();

        builder = builder.push_slice(magic_prefix);
        builder = builder.push_slice(script_bytes);

        builder.into_script()
    }
}

impl fmt::Display for OpReturnMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(&self).unwrap())
    }
}

#[cfg(test)]
mod test {
    use bitcoin::consensus::deserialize;
    use bitcoin::{locktime, transaction::Version, Amount, Transaction, TxOut};
    use bitcoin::{OutPoint, Txid};
    use bitcoincore_rpc::RawTx;
    use std::str::FromStr;

    use crate::transaction::message::ContractType;
    use crate::transaction::mint_only_asset::MintOnlyAssetContract;
    use crate::U128;

    use super::mint_only_asset::MOAMintMechanisms;
    use super::transaction_shared::FreeMint;
    use super::varuint::Varuint;
    use super::{ContractCreation, OpReturnMessage};

    fn create_dummy_tx() -> Transaction {
        let dummy_message = OpReturnMessage {
            transfer: None,
            contract_creation: Some(ContractCreation {
                contract_type: ContractType::Moa(MintOnlyAssetContract {
                    ticker: None,
                    supply_cap: Some(Varuint(1000_000_000)),
                    divisibility: 18,
                    live_time: 0,
                    end_time: None,
                    mint_mechanism: MOAMintMechanisms {
                        free_mint: Some(FreeMint {
                            supply_cap: Some(U128(1000)),
                            amount_per_mint: U128(10),
                        }),
                        preallocated: None,
                        purchase: None,
                    },
                    commitment: None,
                }),
                spec: None,
            }),
            contract_call: Some(super::ContractCall {
                contract: None,
                call_type: super::CallType::Mint(super::MintBurnOption {
                    pointer: Some(2),
                    oracle_message: Some(super::OracleMessageSigned {
                        signature: [].to_vec(),
                        message: super::OracleMessage {
                            input_outpoint: Some(OutPoint {
                                txid: Txid::from_str("2fec618aad64988b5369873a4c44e7be7f32437518b60d29b30be6325d36b33d").unwrap(),
                                vout: 99,
                            }.into()),
                            min_in_value: None,
                            out_value: None,
                            asset_id: None,
                            block_height: 0,
                            ratio: None,
                            ltv: None,
                            outstanding: None,
                        },
                    }),
                    pointer_to_key: Some(1),
                    assert_values: None,
                    commitment_message: None,
                }),
            }),
        };

        Transaction {
            input: Vec::new(),
            lock_time: locktime::absolute::LockTime::ZERO,
            output: vec![TxOut {
                script_pubkey: dummy_message.into_script(),
                value: Amount::from_int_btc(0),
            }],
            version: Version(2),
        }
    }

    #[test]
    pub fn parse_op_return_message_success() {
        let tx = create_dummy_tx();

        let parsed = OpReturnMessage::parse_tx(&tx);
        print!("{:?}", parsed.clone().unwrap());

        if let Some(contract_creation) = parsed.unwrap().contract_creation {
            match contract_creation.contract_type {
                ContractType::Moa(mint_only_asset_contract) => {
                    let free_mint = mint_only_asset_contract.mint_mechanism.free_mint.unwrap();
                    assert_eq!(
                        mint_only_asset_contract.supply_cap,
                        Some(Varuint(1000_000_000))
                    );
                    assert_eq!(mint_only_asset_contract.divisibility, 18);
                    assert_eq!(mint_only_asset_contract.live_time, 0);
                    assert_eq!(free_mint.supply_cap, Some(U128(1000)));
                    assert_eq!(free_mint.amount_per_mint, U128(10));
                }
                _ => panic!("Invalid contract type"),
            }
        }
    }

    #[test]
    pub fn validate_op_return_message_from_tx_hex_success() {
        let tx = create_dummy_tx();

        let tx_bytes = hex::decode(tx.raw_hex()).unwrap();

        let tx: Transaction = deserialize(&tx_bytes).unwrap();

        let op_return_message = OpReturnMessage::parse_tx(&tx);

        assert!(op_return_message.is_ok());
    }
}
