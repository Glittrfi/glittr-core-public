use serde::{Deserialize, Serialize};

// Varuint is only worth using if the uint value is larger than 2 bit
#[derive(PartialEq, Debug, Clone)]
pub struct Varuint(pub u128);

#[derive(PartialEq, Debug)]
pub enum VaruintFlaw {
    Overlong,
    Overflow,
    Unterminated,
}

impl Varuint {
    pub fn decode(buffer: &[u8]) -> Result<u128, VaruintFlaw> {
        let mut n = 0u128;

        for (i, &byte) in buffer.iter().enumerate() {
            if i > 18 {
                return Err(VaruintFlaw::Overlong);
            }

            let value = u128::from(byte) & 0b0111_1111;

            if i == 18 && value & 0b0111_1100 != 0 {
                return Err(VaruintFlaw::Overflow);
            }

            n |= value << (7 * i);

            if byte & 0b1000_0000 == 0 {
                return Ok(n);
            }
        }

        Err(VaruintFlaw::Unterminated)
    }

    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut results: Vec<u8> = Vec::new();
        let mut value = self.0.clone();

        while value >> 7 > 0 {
            results.push(value.to_le_bytes()[0] | 0b1000_0000);
            value >>= 7;
        }

        results.push(value.to_le_bytes()[0]);

        return results;
    }
}

// for now, the serde parser is only used for JSON.
impl Serialize for Varuint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

// for now, the serde parser is only used for JSON.
impl<'de> Deserialize<'de> for Varuint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        Ok(Self(str::parse::<u128>(&s).map_err(|err| {
            serde::de::Error::custom(err.to_string())
        })?))
    }
}
