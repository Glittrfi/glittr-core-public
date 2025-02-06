use crate::borsh_serde::unexpected_eof_to_unexpected_length_of_input;
use borsh::io::{Error, ErrorKind};
use borsh::{BorshDeserialize, BorshSerialize};
use num_traits::{Bounded, FromPrimitive, Signed, ToPrimitive, Zero};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;
use std::{
    io::Read,
    ops::{BitAnd, BitOr, Shl, Shr},
};

#[derive(PartialEq, Debug, Clone, PartialOrd, Copy, Ord, Eq, Hash)]
pub struct Varint<T: Signed>(pub T);

#[derive(Debug)]
pub enum VarintFlaw {
    Overlong,
    Unterminated,
}

impl<T> Display for Varint<T>
where
    T: Signed + Display,
{
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> FromStr for Varint<T>
where
    T: Signed + FromStr,
{
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = T::from_str(s).ok().ok_or("Invalid varint number")?;
        Ok(Varint(value))
    }
}

impl<T> Default for Varint<T>
where
    T: Zero + Signed,
{
    fn default() -> Self {
        Varint(T::zero())
    }
}

impl<T> Varint<T>
where
    T: Clone
        + Copy
        + PartialOrd
        + BitAnd<Output = T>
        + BitOr<Output = T>
        + Shl<u32, Output = T>
        + Shr<u32, Output = T>
        + FromPrimitive
        + ToPrimitive
        + Zero
        + Signed
        + Bounded,
{
    pub fn decode(buffer: &[u8]) -> Result<T, VarintFlaw> {
        let mut n = 0u128;
        let mut shift = 0u32;

        for (i, &byte) in buffer.iter().enumerate() {
            if i > 18 {
                return Err(VarintFlaw::Overlong);
            }

            let value = u128::from_u8(byte & 0b0111_1111).unwrap();

            n = n | (value << shift);
            shift += 7;

            if byte & 0b1000_0000 == 0 {
                if n == 0 {
                    return Ok(T::zero());
                }

                if n & 1 == 1 {
                    // negative
                    n = (n + 1) >> 1;

                    match T::from_u128(n) {
                        Some(n) => return Ok(-n),
                        None => return Err(VarintFlaw::Overlong),
                    }
                } else {
                    // positive
                    n = n >> 1;

                    match T::from_u128(n) {
                        Some(n) => return Ok(n),
                        None => return Err(VarintFlaw::Overlong),
                    }
                }
            }
        }

        Err(VarintFlaw::Unterminated)
    }

    pub fn encode_to_vec(&self) -> Result<Vec<u8>, VarintFlaw> {
        let mut results: Vec<u8> = Vec::new();
        let value = self.0.to_i128().unwrap();

        // the minimum value supported for signed int is min_value + 1
        if self.0 == T::min_value() {
            return Err(VarintFlaw::Overlong);
        }

        // zigzag encoding to convert it to signed
        let unsigned = if value.is_positive() {
            let unsigned = (value << 1) as u128;
            unsigned
        } else {
            let mut unsigned: u128 = 0;
            if value != 0 {
                unsigned = (-value << 1) as u128 - 1;
            }
            unsigned
        };

        let mut value = unsigned;

        let mask = u128::from_u8(0b0111_1111).unwrap();
        let msb = u128::from_u8(0b1000_0000).unwrap();

        while value > mask {
            let byte = (value.clone() & mask.clone()) | msb.clone();
            results.push(byte.to_u8().unwrap());
            value = value >> 7;
        }

        results.push(value.to_u8().unwrap());

        Ok(results)
    }
}

// for now, the serde parser is only used for JSON.
impl<T> Serialize for Varint<T>
where
    T: Signed + ToString,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

// for now, the serde parser is only used for JSON.
impl<'de, T> Deserialize<'de> for Varint<T>
where
    T: Signed + FromStr,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        Ok(Self(
            // TODO: map proper error
            str::parse::<T>(&s)
                .map_err(|_| serde::de::Error::custom("Varint serde decode error"))?,
        ))
    }
}

impl<T> BorshSerialize for Varint<T>
where
    T: Clone
        + Copy
        + PartialOrd
        + BitAnd<Output = T>
        + BitOr<Output = T>
        + Shl<u32, Output = T>
        + Shr<u32, Output = T>
        + FromPrimitive
        + ToPrimitive
        + Zero
        + Signed
        + Bounded,
{
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let varint = self.encode_to_vec().map_err(|_| ErrorKind::InvalidData)?;
        writer.write_all(&varint)
    }
}

impl<T> BorshDeserialize for Varint<T>
where
    T: Clone
        + Copy
        + PartialOrd
        + BitAnd<Output = T>
        + BitOr<Output = T>
        + Shl<u32, Output = T>
        + Shr<u32, Output = T>
        + FromPrimitive
        + ToPrimitive
        + Zero
        + Signed
        + Bounded,
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

        let decoded = Varint::<T>::decode(&bytes);
        if decoded.is_err() {
            // TODO: specific error of varint decode flaw
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Varint borsh decode error",
            ));
        }

        Ok(Varint(decoded.unwrap()))
    }
}
