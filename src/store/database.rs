use super::*;
use borsh::{BorshDeserialize, BorshSerialize};
use rocksdb::{IteratorMode, Options, DB};

pub const INDEXER_LAST_BLOCK_PREFIX: &str = "last_block";
pub const MESSAGE_PREFIX: &str = "message";
pub const TRANSACTION_TO_BLOCK_TX_PREFIX: &str = "tx_to_blocktx";
pub const TICKER_TO_BLOCK_TX_PREFIX: &str = "ticker_to_blocktx";

pub const ASSET_LIST_PREFIX: &str = "asset_list";
pub const ASSET_CONTRACT_DATA_PREFIX: &str = "asset_contract_data";
pub const VESTING_CONTRACT_DATA_PREFIX: &str = "vesting_contract_data";
pub const COLLATERAL_ACCOUNTS_PREFIX: &str = "collateral_account";
pub const COLLATERALIZED_CONTRACT_DATA: &str = "pool_data";
pub const STATE_KEYS_PREFIX: &str = "state_key";
pub const SPEC_CONTRACT_OWNED_PREFIX: &str = "spec_contract_owned";

#[cfg(feature = "helper-api")]
pub const ADDRESS_ASSET_LIST_PREFIX: &str = "address_asset_list";

#[cfg(feature = "helper-api")]
pub const TXID_TO_TRANSACTION_PREFIX: &str = "txid_to_transaction";

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
// - add transaction feature
impl Database {
    pub fn new(path: String) -> Self {
        let mut opts = Options::default();
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_bottommost_compression_type(rocksdb::DBCompressionType::Zstd);
        opts.create_if_missing(true);

        Self {
            db: Arc::new(DB::open(&opts, path).unwrap()),
        }
    }

    pub fn put<T: BorshSerialize>(&mut self, prefix: &str, key: &str, value: T) {
        self.db
            .put(
                format!("{}:{}", prefix, key),
                borsh::to_vec(&value).unwrap(),
            )
            .expect("Error putting data into database");
    }

    pub fn get<T: BorshDeserialize>(&self, prefix: &str, key: &str) -> Result<T, DatabaseError> {
        let value = self
            .db
            .get(format!("{}:{}", prefix, key))
            .expect("Error getting data from database");

        if let Some(value) = value {
            let message = borsh::from_slice(value.as_slice());

            return match message {
                Ok(message) => Ok(message),
                Err(_) => Err(DatabaseError::DeserializeFailed),
            };
        }
        Err(DatabaseError::NotFound)
    }

    pub fn expensive_find_by_prefix<T: BorshDeserialize>(
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

                    match borsh::from_slice(&value) {
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
