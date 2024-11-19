use std::fmt;

use super::*;
use asset_contract::AssetContract;
use bitcoin::{
    opcodes,
    script::{self, Instruction, PushBytes},
    OutPoint, ScriptBuf, Transaction,
};
use bitcoincore_rpc::jsonrpc::serde_json::{self, Deserializer};
use constants::OP_RETURN_MAGIC_PREFIX;
use flaw::Flaw;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ContractType {
    Asset(AssetContract),
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct MintOption {
    pub pointer: u32,
    pub oracle_message: Option<OracleMessageSigned>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    Mint(MintOption),
    Burn,
    Swap,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct OracleMessageSigned {
    pub signature: Vec<u8>,
    pub message: OracleMessage,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct OracleMessage {
    /// the input_outpoint dictates which UTXO is being evaluated by the Oracle
    pub input_outpoint: Option<OutPoint>,
    /// min_in_value represents what the input valued at (minimum because btc value could differ (-fee))
    pub min_in_value: Option<U128>,
    /// out_value represents the oracle's valuation of the input e.g. for 1 btc == 72000 wusd, 72000 is the out_value
    pub out_value: Option<U128>,
    /// rune's BlockTx if rune
    pub asset_id: Option<String>,
    // for raw_btc input it is more straightforward using ratio, output = received_value * ratio
    pub ratio: Option<Ratio>,
    // the bitcoin block height when the message is signed
    pub block_height: u64,
}

/// Transfer
/// Asset: This is a block:tx reference to the contract where the asset was created
/// Output index of output to receive asset
/// Amount: value assigning shares of the transfer to the appropriate UTXO output
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct TxTypeTransfer {
    pub asset: BlockTxTuple,
    pub output: u32,
    pub amount: U128,
}

// TxTypes: Transfer, ContractCreation, ContractCall
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct Transfer {
    pub transfers: Vec<TxTypeTransfer>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ContractCreation {
    pub contract_type: ContractType,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ContractCall {
    pub contract: BlockTxTuple,
    pub call_type: CallType,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct OpReturnMessage {
    pub transfer: Option<Transfer>,
    pub contract_creation: Option<ContractCreation>,
    pub contract_call: Option<ContractCall>,
}

impl CallType {
    pub fn validate(&self) -> Option<Flaw> {
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

        let message =
            OpReturnMessage::deserialize(&mut Deserializer::from_slice(payload.as_slice()));

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
            match &contract_creation.contract_type {
                ContractType::Asset(asset_contract) => {
                    return asset_contract.validate();
                }
            }
        }

        if let Some(contract_call) = &self.contract_call {
            return contract_call.call_type.validate();
        }

        None
    }

    pub fn into_script(&self) -> ScriptBuf {
        let mut builder = script::Builder::new().push_opcode(opcodes::all::OP_RETURN);
        let magic_prefix: &PushBytes = OP_RETURN_MAGIC_PREFIX.as_bytes().try_into().unwrap();
        let binding = self.to_string();
        let script_bytes: &PushBytes = binding.as_bytes().try_into().unwrap();

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

    use crate::asset_contract::DistributionSchemes;
    use crate::asset_contract::FreeMint;
    use crate::asset_contract::SimpleAsset;
    use crate::transaction::asset_contract::AssetContract;
    use crate::transaction::message::ContractType;
    use crate::U128;

    use super::{ContractCreation, OpReturnMessage};

    #[test]
    pub fn parse_op_return_message_success() {
        let dummy_message = OpReturnMessage {
            transfer: None,
            contract_creation: Some(ContractCreation {
                contract_type: ContractType::Asset(AssetContract {
                    asset: SimpleAsset {
                        supply_cap: Some(U128(1000)),
                        divisibility: 18,
                        live_time: 0,
                    },
                    distribution_schemes: DistributionSchemes {
                        free_mint: Some(FreeMint {
                            supply_cap: Some(U128(1000)),
                            amount_per_mint: U128(10),
                        }),
                        preallocated: None,
                        purchase: None,
                    },
                }),
            }),
            contract_call: None,
        };

        let tx = Transaction {
            input: Vec::new(),
            lock_time: locktime::absolute::LockTime::ZERO,
            output: vec![TxOut {
                script_pubkey: dummy_message.into_script(),
                value: Amount::from_int_btc(0),
            }],
            version: Version(2),
        };

        let parsed = OpReturnMessage::parse_tx(&tx);

        if let Some(contract_creation) = parsed.unwrap().contract_creation {
            match contract_creation.contract_type {
                ContractType::Asset(asset_contract) => {
                    let free_mint = asset_contract.distribution_schemes.free_mint.unwrap();
                    assert_eq!(asset_contract.asset.supply_cap, Some(U128(1000)));
                    assert_eq!(asset_contract.asset.divisibility, 18);
                    assert_eq!(asset_contract.asset.live_time, 0);
                    assert_eq!(free_mint.supply_cap, Some(U128(1000)));
                    assert_eq!(free_mint.amount_per_mint, U128(10));
                }
            }
        }
    }

    #[test]
    pub fn validate_op_return_message_from_tx_hex_success() {
        let tx_bytes = hex::decode("02000000031824d7a443ef2c52d76e0ff243cd4a02e9c758897f656ac76e3f2a485e162578000000006b483045022100a2ecd650f46049c41f30305e6930cdb5365964d345b9e05943f1ee1ff136a0e002206704f49c9c7158bf1f197f95f615242ff5529fc2b68ec99f5317711e51aa31aa0121032bcbd9cfbdbd9eff2bda9935f6cc2a6fa0c908da3aaa50aed80d68b0afb3451affffffffb2d572197db334f724d68e14465803e05398249b8f80f63b381feec0b9b0c468010000006b483045022100ed23e2191ae275aef00b11cb907e8702c1ed49bc066a94cdbeba2d24e2f2932802204adf56e12df0eb1de35e3013ccedee1f68f9c3708a8b15176377e19dc597bb930121032bcbd9cfbdbd9eff2bda9935f6cc2a6fa0c908da3aaa50aed80d68b0afb3451affffffff1824d7a443ef2c52d76e0ff243cd4a02e9c758897f656ac76e3f2a485e162578020000006b483045022100a1b52d39338cd0929187dd40cea207e8857881dbbdd36c13fd8c35f06293b8240220507e51ce8e97f84f5f4904eb480293985fc2db2d92199f080a58eac776be96a30121032bcbd9cfbdbd9eff2bda9935f6cc2a6fa0c908da3aaa50aed80d68b0afb3451affffffff0322020000000000001976a9147bbfdf910e1d5f7b2fa2172315eb712f8ab30ae488ac0000000000000000fd31016a06474c495454524d26017b22636f6e74726163745f6372656174696f6e223a7b22636f6e74726163745f74797065223a7b226173736574223a7b226173736574223a7b22737570706c795f636170223a223231303030303030222c2264697669736962696c697479223a382c226c6976655f74696d65223a307d2c22646973747269627574696f6e5f736368656d6573223a7b227075726368617365223a7b22696e7075745f6173736574223a227261775f627463222c227472616e736665725f736368656d65223a7b227075726368617365223a226d726f4847457456424c784b6f6f33344853486248646d4b7a316f6f4a6441336577227d2c227472616e736665725f726174696f5f74797065223a7b226669786564223a7b22726174696f223a5b312c315d7d7d7d7d7d7d7d7d09c0f405000000001976a9147bbfdf910e1d5f7b2fa2172315eb712f8ab30ae488ac00000000").unwrap();

        let tx: Transaction = deserialize(&tx_bytes).unwrap();

        let op_return_message = OpReturnMessage::parse_tx(&tx);

        assert!(op_return_message.is_ok());
    }
}
