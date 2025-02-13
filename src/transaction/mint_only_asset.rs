use flaw::Flaw;
use message::{Commitment, ContractValidator};
use transaction_shared::{FreeMint, Preallocated, PurchaseBurnSwap};

use super::*;

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MOAMintMechanisms {
    pub preallocated: Option<Preallocated>,
    pub free_mint: Option<FreeMint>,
    pub purchase: Option<PurchaseBurnSwap>,
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MintOnlyAssetContract {
    pub ticker: Option<String>,
    pub supply_cap: Option<U128>,
    pub divisibility: u8,
    pub live_time: RelativeOrAbsoluteBlockHeight,
    pub end_time: Option<RelativeOrAbsoluteBlockHeight>,
    pub mint_mechanism: MOAMintMechanisms,
    pub commitment: Option<Commitment>
}

/// Mix of distribution schemes only applicable for preallocated and free_mint or preallocated and purchase
impl ContractValidator for MintOnlyAssetContract {
    fn validate(&self) -> Option<Flaw> {
        if let Some(end_time) = self.end_time {
            if end_time < self.live_time {
                return Some(Flaw::EndTimeIsLessThanLiveTime)
            }
        }

        if self.mint_mechanism.purchase.is_some() && self.mint_mechanism.free_mint.is_some() {
            return Some(Flaw::NotImplemented);
        }

        if let Some(preallocated) = &self.mint_mechanism.preallocated {
            return preallocated.validate(&message::ContractType::Moa(self.clone()));
        }

        if let Some(freemint) = &self.mint_mechanism.free_mint {
            return freemint.validate(&message::ContractType::Moa(self.clone()));
        }

        if let Some(purchase) = &self.mint_mechanism.purchase {
            return purchase.validate();
        }

        None
    }
}
