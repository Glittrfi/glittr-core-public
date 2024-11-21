use flaw::Flaw;
use shared::MintMechanisms;

use super::*;

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MintOnlyAssetContract {
    pub ticker: Option<String>,
    pub supply_cap: Option<U128>,
    pub divisibility: u8,
    pub live_time: BlockHeight,
    pub mint_mechanism: MintMechanisms,
}

/// Mix of distribution schemes only applicable for preallocated and free_mint or preallocated and purchase
impl MintOnlyAssetContract {
    pub fn validate(&self) -> Option<Flaw> {
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
