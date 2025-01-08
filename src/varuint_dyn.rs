use crate::borsh_serde::unexpected_eof_to_unexpected_length_of_input;
use borsh::io::{Error, ErrorKind};
use borsh::{BorshDeserialize, BorshSerialize};
use num_traits::{FromPrimitive, One, ToPrimitive, Unsigned, Zero};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::{
    io::Read,
    ops::{BitAnd, BitOr, Shl, Shr},
};

#[derive(PartialEq, Debug, Clone, PartialOrd)]
pub struct Varuint<T: Unsigned>(pub T);

#[derive(Debug)]
pub enum VaruintFlaw {
    Overlong,
    Unterminated,
}

impl<T> Varuint<T>
where
    T: Clone
        + PartialOrd
        + BitAnd<Output = T>
        + BitOr<Output = T>
        + Shl<u32, Output = T>
        + Shr<u32, Output = T>
        + FromPrimitive
        + ToPrimitive
        + Zero
        + One
        + Unsigned,
{
    pub fn decode(buffer: &[u8]) -> Result<T, VaruintFlaw> {
        let mut n = T::zero();
        let mut shift = 0u32;

        for (i, &byte) in buffer.iter().enumerate() {
            if i > 18 {
                return Err(VaruintFlaw::Overlong);
            }

            let value = T::from_u8(byte & 0b0111_1111).unwrap();

            n = n | (value << shift);
            shift += 7;

            if byte & 0b1000_0000 == 0 {
                return Ok(n);
            }
        }

        Err(VaruintFlaw::Unterminated)
    }

    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut results: Vec<u8> = Vec::new();
        let mut value = self.0.clone();

        let mask = T::from_u8(0b0111_1111).unwrap();
        let msb = T::from_u8(0b1000_0000).unwrap();

        while value > mask {
            let byte = (value.clone() & mask.clone()) | msb.clone();
            results.push(byte.to_u8().unwrap());
            value = value >> 7;
        }

        results.push(value.to_u8().unwrap());

        results
    }
}

// for now, the serde parser is only used for JSON.
impl<T> Serialize for Varuint<T>
where
    T: Unsigned + ToString,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

// for now, the serde parser is only used for JSON.
impl<'de, T> Deserialize<'de> for Varuint<T>
where
    T: Unsigned + FromStr,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        Ok(Self(
            // TODO: map proper error
            str::parse::<T>(&s)
                .map_err(|_| serde::de::Error::custom("Varuint serde decode error"))?,
        ))
    }
}

impl<T> BorshSerialize for Varuint<T>
where
    T: Clone
        + PartialOrd
        + BitAnd<Output = T>
        + BitOr<Output = T>
        + Shl<u32, Output = T>
        + Shr<u32, Output = T>
        + FromPrimitive
        + ToPrimitive
        + Zero
        + One
        + Unsigned,
{
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let varuint = self.encode_to_vec();
        writer.write_all(&varuint)
    }
}

impl<T> BorshDeserialize for Varuint<T>
where
    T: Clone
        + PartialOrd
        + BitAnd<Output = T>
        + BitOr<Output = T>
        + Shl<u32, Output = T>
        + Shr<u32, Output = T>
        + FromPrimitive
        + ToPrimitive
        + Zero
        + One
        + Unsigned,
{
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

        let decoded = Varuint::<T>::decode(&bytes);
        if decoded.is_err() {
            // TODO: specific error of varuint decode flaw
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Varuint borsh decode error",
            ));
        }

        Ok(Varuint(decoded.unwrap()))
    }
}
