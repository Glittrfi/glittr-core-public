use flaw::Flaw;
use message::ContractValidator;

use super::*;

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct NftAssetContract {
    pub asset: Vec<u8>,
    pub supply_cap: Option<U128>,
    pub live_time: RelativeOrAbsoluteBlockHeight,
    pub end_time: Option<RelativeOrAbsoluteBlockHeight>,
    // the target output index that holds the nft
    pub pointer: Option<u32>,
}

impl ContractValidator for NftAssetContract {
    fn validate(&self) -> Option<Flaw> {
        if let Some(end_time) = self.end_time {
            if end_time < self.live_time {
                return Some(Flaw::EndTimeIsLessThanLiveTime);
            }
        }
        None
    }
}
