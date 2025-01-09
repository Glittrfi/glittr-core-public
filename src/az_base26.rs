use core::fmt;
use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::varuint_dyn::Varuint;

#[derive(Debug, BorshSerialize, BorshDeserialize, Clone)]
pub struct AZBase26(pub Varuint<u128>);

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

        for c in symbol.chars().rev() {
            write!(f, "{c}")?;
        }

        Ok(())
    }
}

impl FromStr for AZBase26 {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_uppercase();

        let mut x = 0u128;
        for (i, c) in s.chars().enumerate() {
            if i > 0 {
                x = x.checked_add(1).ok_or("Invalid character length")?;
            }
            x = x.checked_mul(26).ok_or("Invalid character length")?;
            match c {
                'A'..='Z' => {
                    x = x
                        .checked_add(c as u128 - 'A' as u128)
                        .ok_or("Invalid character length")?;
                }
                _ => return Err(format!("Invalid character {}", c).into()),
            }
        }
        Ok(AZBase26(Varuint(x)))
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
