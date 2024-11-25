use super::*;
use bitcoincore_rpc::jsonrpc::serde_json::{self, Deserializer};
use rocksdb::{IteratorMode, DB};

pub const INDEXER_LAST_BLOCK_PREFIX: &str = "last_block";
pub const MESSAGE_PREFIX: &str = "message";
pub const TRANSACTION_TO_BLOCK_TX_PREFIX: &str = "tx_to_blocktx";

pub const ASSET_LIST_PREFIX: &str = "asset_list";
pub const ASSET_CONTRACT_DATA_PREFIX: &str = "asset_contract_data";
pub const VESTING_CONTRACT_DATA_PREFIX: &str = "vesting_contract_data";
pub const COLLATERAL_ACCOUNT_PREFIX: &str = "collateral_account";
pub const POOL_DATA_PREFIX: &str = "pool_data";
pub const STATE_KEY_PREFIX: &str = "state_key";
pub const SPEC_CONTRACT_OWNED_PREFIX: &str = "spec_contract_owned";

pub struct Database {
    db: Arc<DB>,
}

#[derive(Debug)]
pub enum DatabaseError {
    NotFound,
    DeserializeFailed,
}

// TODO:
// - implement error handling
// - hash the key
// - add transaction feature
impl Database {
    pub fn new(path: String) -> Self {
        Self {
            db: Arc::new(DB::open_default(path).unwrap()),
        }
    }

    pub fn put<T: Serialize>(&mut self, prefix: &str, key: &str, value: T) {
        self.db
            .put(
                format!("{}:{}", prefix, key),
                serde_json::to_string(&value).unwrap(),
            )
            .expect("Error putting data into database");
    }

    pub fn get<T: for<'a> Deserialize<'a>>(
        &self,
        prefix: &str,
        key: &str,
    ) -> Result<T, DatabaseError> {
        let value = self
            .db
            .get(format!("{}:{}", prefix, key))
            .expect("Error getting data from database");

        if let Some(value) = value {
            let message = T::deserialize(&mut Deserializer::from_slice(value.as_slice()));

            return match message {
                Ok(message) => Ok(message),
                Err(_) => Err(DatabaseError::DeserializeFailed),
            };
        }
        Err(DatabaseError::NotFound)
    }

    pub fn expensive_find_by_prefix<T: for<'a> Deserialize<'a>>(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, T)>, DatabaseError> {
        let mut results = Vec::new();
        let iter = self.db.iterator(IteratorMode::From(
            prefix.as_bytes(),
            rocksdb::Direction::Forward,
        ));

        for item in iter {
            match item {
                Ok((key, value)) => {
                    let key_str = String::from_utf8_lossy(&key);
                    if !key_str.starts_with(prefix) {
                        break; // Stop when we've moved past the prefix
                    }

                    match T::deserialize(&mut Deserializer::from_slice(&value)) {
                        Ok(deserialized) => results.push((key_str.to_string(), deserialized)),
                        Err(_) => return Err(DatabaseError::DeserializeFailed),
                    }
                }
                Err(_) => return Err(DatabaseError::DeserializeFailed),
            }
        }

        Ok(results)
    }

    pub fn delete(&mut self, prefix: &str, key: &str) {
        self.db
            .delete(format!("{}:{}", prefix, key))
            .expect("Error deleting data from database");
    }
}
