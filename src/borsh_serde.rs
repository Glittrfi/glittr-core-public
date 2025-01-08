use std::io::Read;

use bitcoin::hashes::Hash;
use bitcoin::{OutPoint, Txid};
use borsh::io::{Error, ErrorKind};
use borsh::{BorshDeserialize, BorshSerialize};

use crate::{varuint::Varuint, BitcoinOutpoint, U128};

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

impl BorshSerialize for Varuint {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let varuint = self.encode_to_vec();
        writer.write_all(&varuint)
    }
}

impl BorshDeserialize for Varuint {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut bytes: Vec<u8> = Vec::new();

        loop {
            let mut byte = [0u8; 1];
            reader
                .read_exact(&mut byte)
                .map_err(unexpected_eof_to_unexpected_length_of_input)?;

            bytes.push(byte[0]);

            if byte[0] & 128 == 0 {
                break;
            }
        }

        let decoded = Varuint::decode(&bytes);
        if decoded.is_err() {
            // TODO: specific error of varuint decode flaw
            return Err(Error::new(ErrorKind::InvalidData, "Varuint decode error"));
        }

        Ok(Varuint(decoded.unwrap()))
    }
}
