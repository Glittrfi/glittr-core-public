use super::*;
use bitcoin::{opcodes, script::Instruction, Transaction};
use borsh::BorshDeserialize;
use borsh_derive::{BorshDeserialize, BorshSerialize};

pub type BlockTx = String;

pub const DUMMY_GLYPH_PREFIX: &str = "dummy_glyph";

#[derive(BorshDeserialize, BorshSerialize)]
pub enum TxType {
    Etch,
    Transfer,
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct DummyGlyph {
    pub tx_type: TxType,
    pub ticker: String,
    pub supply: u32,
    pub premine: u32,
    pub receiver_pointer: u32,
}
impl DummyGlyph {
    pub fn parse_tx(tx: &Transaction) -> Result<DummyGlyph, Box<dyn Error>> {
        let mut payload = Vec::new();
        for output in tx.output.iter() {
            let mut instructions = output.script_pubkey.instructions();

            if instructions.next() != Some(Ok(Instruction::Op(opcodes::all::OP_RETURN))) {
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

        let dummy_glyph = DummyGlyph::deserialize(&mut payload.as_slice());

        match dummy_glyph {
            Ok(dummy_glyph) => Ok(dummy_glyph),
            Err(error) => Err(error.into()),
        }
    }

    pub fn put(&self, database: &mut Database, key: BlockTx) -> Result<(), rocksdb::Error> {
        return database.put(DUMMY_GLYPH_PREFIX, key.as_str(), self);
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
    use borsh::to_vec;

    use super::DummyGlyph;

    #[test]
    pub fn parse_dummy_glyph_tx_success() {
        let dummy_glyph = DummyGlyph {
            tx_type: super::TxType::Etch,
            ticker: "sample".to_string(),
            supply: 1000000,
            premine: 10000,
            receiver_pointer: 1,
        };

        let mut builder = script::Builder::new().push_opcode(opcodes::all::OP_RETURN);

        for slice in vec![to_vec(&dummy_glyph).unwrap()] {
            let Ok(push): Result<&PushBytes, _> = slice.as_slice().try_into() else {
                continue;
            };
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

        let parsed = DummyGlyph::parse_tx(&tx);

        assert!(parsed.unwrap().supply == dummy_glyph.supply);
    }
}
