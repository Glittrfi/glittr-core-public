use std::fmt;

use super::*;
use asset_contract::AssetContract;
use bitcoin::{opcodes, script::Instruction, Transaction};
use bitcoincore_rpc::jsonrpc::serde_json::{self, Deserializer};
use dummy_contract::DummyContract;
use store::database::MESSAGE_PREFIX;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractType {
    Asset(AssetContract),
    Dummy(DummyContract),
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    Mint,
    Burn,
    Swap
}

/// Transfer
/// Asset: This is a block:tx reference to the contract where the asset was created
/// N outputs: Number of output utxos to receive assets
/// Amount: Vector of values assigning shares of the transfer to the appropriate UTXO outputs
#[derive(Deserialize, Serialize)]
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
        call_type: CallType
    },
}

#[derive(Deserialize, Serialize)]
pub struct OpReturnMessage {
    pub tx_type: TxType,
}

impl OpReturnMessage {
    pub fn parse_tx(tx: &Transaction) -> Result<OpReturnMessage, Box<dyn Error>> {
        let mut payload = Vec::new();
        for output in tx.output.iter() {
            let mut instructions = output.script_pubkey.instructions();

            if instructions.next() != Some(Ok(Instruction::Op(opcodes::all::OP_RETURN))) {
                continue;
            }

            let signature = instructions.next();
            if let Some(Ok(Instruction::PushBytes(glittr_message))) = signature {
                if glittr_message.as_bytes() != "GLITTR".as_bytes() {
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
                        return Err(format!("Invalid instruction {}", op).into());
                    }
                    Err(_) => {
                        return Err("Invalid script".into());
                    }
                }
            }
        }

        let message =
            OpReturnMessage::deserialize(&mut Deserializer::from_slice(payload.as_slice()));

        match message {
            Ok(message) => Ok(message),
            Err(error) => Err(error.into()),
        }
    }

    pub fn put(&self, database: &mut Database, key: BlockTx) -> Result<(), rocksdb::Error> {
        return database.put(MESSAGE_PREFIX, key.to_string().as_str(), self);
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
        locktime, opcodes,
        script::{self, PushBytes},
        transaction::Version,
        Amount, Transaction, TxOut,
    };

    use crate::transaction::asset_contract::AssetContract;
    use crate::transaction::message::ContractType;
    use crate::transaction::message::TxType;

    use super::OpReturnMessage;

    #[test]
    pub fn parse_op_return_message_success() {
        let dummy_message = OpReturnMessage {
            tx_type: TxType::ContractCreation {
                contract_type: ContractType::Asset(AssetContract::FreeMint {
                    supply_cap: Some(1000),
                    amount_per_mint: 10,
                    divisibility: 18,
                    live_time: 0,
                }),
            },
        };

        let mut builder = script::Builder::new().push_opcode(opcodes::all::OP_RETURN);

        println!("{}", dummy_message.to_string());
        for slice in [dummy_message.to_string().as_bytes()] {
            let Ok(push): Result<&PushBytes, _> = slice.try_into() else {
                continue;
            };

            let Ok(push_glittr): Result<&PushBytes, _> = "GLITTR".as_bytes().try_into() else {
                continue;
            };
            builder = builder.push_slice(push_glittr);
            builder = builder.push_slice(push);
        }

        let tx = Transaction {
            input: Vec::new(),
            lock_time: locktime::absolute::LockTime::ZERO,
            output: vec![TxOut {
                script_pubkey: builder.into_script(),
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
                    AssetContract::FreeMint {
                        supply_cap,
                        amount_per_mint,
                        divisibility,
                        live_time,
                    } => {
                        assert_eq!(supply_cap, Some(1000));
                        assert_eq!(amount_per_mint, 10);
                        assert_eq!(divisibility, 18);
                        assert_eq!(live_time, 0);
                    }
                    AssetContract::PurchaseBurnSwap {
                        input_asset_type: _,
                        input_asset: _,
                        transfer_scheme: _,
                        transfer_ratio_type: _,
                    } => panic!("not purchase burn swap"),
                },
                ContractType::Dummy(_dummy_contract) => panic!("not dummy contract"),
            },
            TxType::ContractCall { contract, call_type } => panic!("not contract call"),
        }
    }
}
