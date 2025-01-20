use std::io::Read;

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::transaction::Version;
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness};
use borsh::io::{Error, ErrorKind};
use borsh::{BorshDeserialize, BorshSerialize};

use crate::{BitcoinOutpoint, BitcoinTransaction, U128};

pub const ERROR_UNEXPECTED_LENGTH_OF_INPUT: &str = "Unexpected length of input";

pub fn unexpected_eof_to_unexpected_length_of_input(e: Error) -> Error {
    if e.kind() == ErrorKind::UnexpectedEof {
        Error::new(ErrorKind::InvalidData, ERROR_UNEXPECTED_LENGTH_OF_INPUT)
    } else {
        e
    }
}

// U128
impl BorshSerialize for U128 {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let bytes = self.0.to_le_bytes();
        writer.write_all(&bytes)
    }
}

impl BorshDeserialize for U128 {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut bytes = [0u8; 16];

        reader
            .read_exact(&mut bytes)
            .map_err(unexpected_eof_to_unexpected_length_of_input)?;

        Ok(U128(u128::from_le_bytes(bytes)))
    }
}

// Bitcoin Outpoint (OutPoint)
impl BorshSerialize for BitcoinOutpoint {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let hash_bytes = self.0.txid.to_byte_array();
        let vout_bytes = self.0.vout.to_le_bytes();

        // the total size of the outpoint is 36 bytes
        let bytes: Vec<u8> = [&hash_bytes[..], &vout_bytes[..]].concat();
        writer.write_all(&bytes)
    }
}

impl BorshDeserialize for BitcoinOutpoint {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut bytes = [0u8; 36];

        reader
            .read_exact(&mut bytes)
            .map_err(unexpected_eof_to_unexpected_length_of_input)?;

        let txid = Txid::from_slice(&bytes[0..32]).unwrap();
        let vout = u32::from_le_bytes(bytes[32..].to_vec().try_into().unwrap());

        Ok(BitcoinOutpoint(OutPoint::new(txid, vout)))
    }
}

impl BorshSerialize for BitcoinTransaction {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize each field of the Transaction
        let version = self.0.version.0.to_le_bytes();
        let lock_time = self.0.lock_time.to_consensus_u32().to_le_bytes();
        BorshSerialize::serialize(&version, writer)?;
        BorshSerialize::serialize(&lock_time, writer)?;

        // Serialize inputs
        BorshSerialize::serialize(&(self.0.input.len() as u32), writer)?;
        for input in &self.0.input {
            let txid = input.previous_output.txid.to_byte_array();
            let vout = input.previous_output.vout.to_le_bytes();
            let script_sig = input.script_sig.as_bytes();
            let sequence = input.sequence.0.to_le_bytes();
            let witness = input.witness.to_vec();
            BorshSerialize::serialize(&txid, writer)?;
            BorshSerialize::serialize(&vout, writer)?;
            BorshSerialize::serialize(&script_sig, writer)?;
            BorshSerialize::serialize(&sequence, writer)?;
            BorshSerialize::serialize(&witness, writer)?;
        }
        // Serialize outputs
        BorshSerialize::serialize(&(self.0.output.len() as u32), writer)?;
        for output in &self.0.output {
            let value = output.value.to_sat().to_le_bytes();
            let script_pubkey = output.script_pubkey.as_bytes();
            BorshSerialize::serialize(&value, writer)?;
            BorshSerialize::serialize(&script_pubkey, writer)?;
        }
        Ok(())
    }
}

impl BorshDeserialize for BitcoinTransaction {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, Error> {
        // Deserialize each field of the Transaction
        let version = i32::deserialize_reader(reader)?;
        let lock_time_bytes = u32::deserialize_reader(reader)?;
        let lock_time = LockTime::from_consensus(lock_time_bytes);

        // Deserialize inputs
        let input_count = u32::deserialize_reader(reader)?;
        let mut inputs = Vec::with_capacity(input_count as usize);
        for _ in 0..input_count {
            let mut txid_buf = [0u8; 32];
            reader.read_exact(&mut txid_buf)?;

            let txid = Txid::from_byte_array(txid_buf);
            let vout = u32::deserialize_reader(reader)?;
            let script_sig = Vec::<u8>::deserialize_reader(reader)?;
            let sequence = u32::deserialize_reader(reader)?;
            let witness: Vec<Vec<u8>> = Vec::deserialize_reader(reader)?;
            inputs.push(TxIn {
                previous_output: OutPoint::new(txid, vout),
                script_sig: ScriptBuf::from_bytes(script_sig),
                sequence: Sequence(sequence),
                witness: Witness::from_slice(&witness),
            });
        }

        // Deserialize outputs
        let output_count = u32::deserialize_reader(reader)?;
        let mut outputs = Vec::with_capacity(output_count as usize);
        for _ in 0..output_count {
            let value = u64::deserialize_reader(reader)?;
            let script_pubkey = Vec::<u8>::deserialize_reader(reader)?;

            outputs.push(TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::from_bytes(script_pubkey),
            });
        }

        Ok(BitcoinTransaction(Transaction {
            version: Version(version),
            input: inputs,
            output: outputs,
            lock_time,
        }))
    }
}
