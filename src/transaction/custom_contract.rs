use serde::{Deserialize, Serialize};
use serde_json::json;
use flaw::Flaw;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct CustomContract {
    pub name: String,
    pub description: String,
    pub owner: String,
    pub supply_cap: u64,
    pub divisibility: u8,
}

impl CustomContract {
    pub fn validate(&self) -> Option<Flaw> {
        if self.name.is_empty() {
            return Some(Flaw::InvalidField("name".to_string()));
        }
        if self.description.is_empty() {
            return Some(Flaw::InvalidField("description".to_string()));
        }
        if self.owner.is_empty() {
            return Some(Flaw::InvalidField("owner".to_string()));
        }
        if self.supply_cap == 0 {
            return Some(Flaw::InvalidField("supply_cap".to_string()));
        }
        if self.divisibility > 18 {
            return Some(Flaw::InvalidField("divisibility".to_string()));
        }
        None
    }
}

// Example usage: Convert to JSON for devnet
impl CustomContract {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "name": self.name,
            "description": self.description,
            "owner": self.owner,
            "supply_cap": self.supply_cap,
            "divisibility": self.divisibility,
        })
    }
}