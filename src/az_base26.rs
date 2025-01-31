use core::fmt;
use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::varuint::Varuint;

// (chars, separator)
#[derive(Debug, BorshSerialize, BorshDeserialize, Clone)]
pub struct AZBase26(pub Varuint<u128>, pub Option<Varuint<u32>>);

impl Display for AZBase26 {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let mut n = self.0 .0;
        if n == u128::MAX {
            return write!(f, "BCGDENLQRQWDSLRUGSNLBTMFIJAV");
        }

        n += 1;
        let mut symbol = String::new();
        while n > 0 {
            symbol.push(
                "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
                    .chars()
                    .nth(((n - 1) % 26) as usize)
                    .unwrap(),
            );
            n = (n - 1) / 26;
        }

        for (i, c) in symbol.chars().rev().enumerate() {
            if let Some(spacers) = self.1 {
                let flag: u32 = 1 << i;
                if spacers.0 & flag != 0 {
                    write!(f, "•")?;
                }
            }

            write!(f, "{c}")?;
        }

        Ok(())
    }
}

impl FromStr for AZBase26 {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_uppercase();

        let mut spacers_value = 0u32;

        let mut x = 0u128;
        let mut chars_index = 0;
        for c in s.chars() {
            match c {
                'A'..='Z' => {
                    if chars_index > 0 {
                        x = x.checked_add(1).ok_or("Invalid character length")?;
                    }
                    x = x.checked_mul(26).ok_or("Invalid character length")?;
                    x = x
                        .checked_add(c as u128 - 'A' as u128)
                        .ok_or("Invalid character length")?;

                    chars_index += 1;
                }
                '.' | '•' => {
                    let flag: u32 = 1 << chars_index;
                    if spacers_value & flag == 0 {
                        spacers_value |= flag;
                    }
                }
                _ => return Err(format!("Invalid character {}", c).into()),
            }
        }

        let mut spacers = None;
        if spacers_value > 0 {
            spacers = Some(Varuint(spacers_value));
        }

        Ok(AZBase26(Varuint(x), spacers))
    }
}

// for now, the serde parser is only used for JSON.
impl Serialize for AZBase26 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// for now, the serde parser is only used for JSON.
impl<'de> Deserialize<'de> for AZBase26 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        println!("AZBASE26: {}", s);
        Ok(Self::from_str(&s).map_err(|err| serde::de::Error::custom(err.to_string()))?)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    pub fn az_base26_no_spacers() {
        let ticker_encoded = AZBase26::from_str("POHONPISANG").unwrap();
        let ticker_decoded = ticker_encoded.to_string();
        assert_eq!(ticker_decoded, "POHONPISANG");
    }

    #[test]
    pub fn az_base26_spacers() {
        let ticker_encoded = AZBase26::from_str("P.O.H.O.N.P.I.S.A.N.G").unwrap();
        let ticker_decoded = ticker_encoded.to_string();
        assert_eq!(ticker_decoded, "P•O•H•O•N•P•I•S•A•N•G");
    }
}
