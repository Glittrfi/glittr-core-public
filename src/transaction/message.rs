use std::fmt;

use super::*;
use asset_contract::AssetContract;
use bitcoin::{
    opcodes,
    script::{self, Instruction, PushBytes},
    ScriptBuf, Transaction,
};
use bitcoincore_rpc::jsonrpc::serde_json::{self, Deserializer};
use constants::OP_RETURN_MAGIC_PREFIX;
use flaw::Flaw;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ContractType {
    Asset(AssetContract),
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MintOption {
    pub pointer: u32
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    Mint(MintOption),
    Burn,
    Swap,
}

/// Transfer
/// Asset: This is a block:tx reference to the contract where the asset was created
/// N outputs: Number of output utxos to receive assets
/// Amount: Vector of values assigning shares of the transfer to the appropriate UTXO outputs
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TxType {
    Transfer {
        asset: BlockTxTuple,
        n_outputs: u32,
        amounts: Vec<u32>,
    },
    ContractCreation {
        contract_type: ContractType,
    },
    ContractCall {
        contract: BlockTxTuple,
        call_type: CallType,
    },
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct OpReturnMessage {
    pub tx_type: TxType,
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

        let message =
            OpReturnMessage::deserialize(&mut Deserializer::from_slice(payload.as_slice()));

        match message {
            Ok(message) => Ok(message),
            Err(_) => Err(Flaw::FailedDeserialization),
        }
    }

    pub fn validate(&self) -> Option<Flaw> {
        match self.tx_type.clone() {
            TxType::Transfer {
                asset: _,
                n_outputs: _,
                amounts: _,
            } => {
                // TODO: validate if asset exist
                // TODO: validate n_outputs <  max outputs in transactions
                // TODO: validate if amounts from input
            }
            TxType::ContractCreation { contract_type } => match contract_type {
                ContractType::Asset(asset_contract) => {
                    return asset_contract.validate();
                }
            },
            TxType::ContractCall {
                call_type,
                ..
            } => {
                return call_type.validate();
            }

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

        return builder.into_script();
    }
}

impl fmt::Display for OpReturnMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(&self).unwrap())
    }
}

#[cfg(test)]
mod test {
    use bitcoin::{
        consensus::deserialize, locktime, transaction::Version, Amount, Transaction, TxOut
    };
    use crate::asset_contract::AssetContractFreeMint;
    use crate::transaction::asset_contract::AssetContract;
    use crate::transaction::message::ContractType;
    use crate::transaction::message::TxType;

    use super::OpReturnMessage;

    #[test]
    pub fn parse_op_return_message_success() {
        let dummy_message = OpReturnMessage {
            tx_type: TxType::ContractCreation {
                contract_type: ContractType::Asset(AssetContract::FreeMint(
                    AssetContractFreeMint {
                        supply_cap: Some(1000),
                        amount_per_mint: 10,
                        divisibility: 18,
                        live_time: 0,
                    },
                )),
            },
        };

        println!("{}", dummy_message.to_string());

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

        match parsed.unwrap().tx_type {
            TxType::Transfer {
                asset: _,
                n_outputs: _,
                amounts: _,
            } => panic!("not transfer"),
            TxType::ContractCreation { contract_type } => match contract_type {
                ContractType::Asset(asset_contract) => match asset_contract {
                    AssetContract::Preallocated { todo: _ } => panic!("not preallocated"),
                    AssetContract::FreeMint(free_mint) => {
                        assert_eq!(free_mint.supply_cap, Some(1000));
                        assert_eq!(free_mint.amount_per_mint, 10);
                        assert_eq!(free_mint.divisibility, 18);
                        assert_eq!(free_mint.live_time, 0);
                    }
                    AssetContract::PurchaseBurnSwap {
                        input_asset: _,
                        transfer_scheme: _,
                        transfer_ratio_type: _,
                    } => panic!("not purchase burn swap"),
                },
            },
            TxType::ContractCall {
                contract: _,
                call_type: _,
            } => panic!("not contract call"),
        }
    }

    #[test]
    pub fn validate_op_return_message_from_tx_hex_success() {
        let tx_bytes = hex::decode("0200000001ce9f8af57c1988692745ff72a5d190b04e1137a5d29855f71beda45c8032f1db010000006a47304402206e9d49fea6da1d4f2925787b4b291f8450dccd18387205f5b7a7c91e6d61b25602201e7a99d167c0da5efeff9a381d888d27bd6b95e2d5dec261a0e7f20f72db3c8e0121032bcbd9cfbdbd9eff2bda9935f6cc2a6fa0c908da3aaa50aed80d68b0afb3451affffffff0200000000000000009e6a06474c495454524c947b2274785f74797065223a7b22636f6e74726163745f6372656174696f6e223a7b22636f6e74726163745f74797065223a7b226173736574223a7b22667265655f6d696e74223a7b22737570706c795f636170223a313030302c22616d6f756e745f7065725f6d696e74223a31302c2264697669736962696c697479223a31382c226c6976655f74696d65223a307d7d7d7d7d7d98ecfa02000000001976a9147bbfdf910e1d5f7b2fa2172315eb712f8ab30ae488ac00000000").unwrap();

        let tx: Transaction = deserialize(&tx_bytes).unwrap();

        let op_return_message = OpReturnMessage::parse_tx(&tx);

        assert!(op_return_message.is_ok());
    }
}
