use super::*;
use bitcoincore_rpc::jsonrpc::serde_json::{self, Deserializer};
use rocksdb::DB;

pub const INDEXER_LAST_BLOCK_PREFIX: &str = "last_block";
pub const MESSAGE_PREFIX: &str = "message";
pub const TRANSACTION_TO_BLOCK_TX_PREFIX: &str = "tx_to_blocktx";

pub struct Database {
    db: Arc<DB>,
}

pub enum DatabaseError {
    NotFound,
    DeserializeFailed,
}

impl Database {
    pub fn new() -> Self {
        Self {
            db: Arc::new(DB::open_default(CONFIG.rocks_db_path.clone()).unwrap()),
        }
    }

    pub fn put<T: Serialize>(
        &mut self,
        prefix: &str,
        key: &str,
        value: T,
    ) -> Result<(), rocksdb::Error> {
        self.db.put(
            format!("{}{}", prefix, key),
            serde_json::to_string(&value).unwrap(),
        )
    }

    pub fn get<T: for<'a> Deserialize<'a>>(
        &self,
        prefix: &str,
        key: &str,
    ) -> Result<T, DatabaseError> {
        let value = self.db.get(format!("{}{}", prefix, key)).unwrap();

        if let Some(value) = value {
            let message = T::deserialize(&mut Deserializer::from_slice(value.as_slice()));

            return match message {
                Ok(message) => Ok(message),
                Err(_) => Err(DatabaseError::DeserializeFailed),
            };
        }
        Err(DatabaseError::NotFound)
    }
}
