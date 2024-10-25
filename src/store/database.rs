use super::*;
use bitcoincore_rpc::jsonrpc::serde_json::{self, Deserializer};
use rocksdb::{IteratorMode, DB};

pub const INDEXER_LAST_BLOCK_PREFIX: &str = "last_block";
pub const MESSAGE_PREFIX: &str = "message";
pub const TRANSACTION_TO_BLOCK_TX_PREFIX: &str = "tx_to_blocktx";

// MINT
pub const MINT_OUTPUT_PREFIX: &str = "mint_output";
pub const MINT_DATA_PREFIX: &str = "mint_data";

pub struct Database {
    db: Arc<DB>,
}

#[derive(Debug)]
pub enum DatabaseError {
    NotFound,
    DeserializeFailed,
}

// TODO: implment error handling
impl Database {
    pub fn new(path: String) -> Self {
        Self {
            db: Arc::new(DB::open_default(path).unwrap()),
        }
    }

    pub fn put<T: Serialize>(
        &mut self,
        prefix: &str,
        key: &str,
        value: T,
    ) -> Result<(), rocksdb::Error> {
        self.db.put(
            format!("{}:{}", prefix, key),
            serde_json::to_string(&value).unwrap(),
        )
    }

    pub fn get<T: for<'a> Deserialize<'a>>(
        &self,
        prefix: &str,
        key: &str,
    ) -> Result<T, DatabaseError> {
        let value = self.db.get(format!("{}:{}", prefix, key)).unwrap();

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
}
