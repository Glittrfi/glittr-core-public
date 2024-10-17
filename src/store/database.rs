use super::*;
use bitcoincore_rpc::jsonrpc::serde_json::{self, Deserializer};
use rocksdb::DB;

pub const INDEXER_LAST_BLOCK_PREFIX: &str = "last_block";
pub const MESSAGE_PREFIX: &str = "message";

pub struct Database {
    db: DB,
}

impl Database {
    pub fn new() -> Self {
        Self {
            db: DB::open_default(CONFIG.rocks_db_path.clone()).unwrap(),
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

    pub fn get<T: for<'a> Deserialize<'a>>(&self, prefix: &str, key: &str) -> Option<T> {
        let value = self.db.get(format!("{}{}", prefix, key)).unwrap();

        value.map(|value| {
            let message = T::deserialize(&mut Deserializer::from_slice(value.as_slice()));

            match message {
                Ok(message) => Some(message),
                Err(_) => None,
            }
        })?
    }
}
