use super::*;
use borsh::{from_slice, to_vec, BorshDeserialize, BorshSerialize};
use rocksdb::DB;

pub const INDEXER_LAST_BLOCK_PREFIX: &str = "last_block";

pub struct Database {
    db: DB,
}

impl Database {
    pub fn new() -> Self {
        Self {
            db: DB::open_default(CONFIG.rocks_db_path.clone()).unwrap(),
        }
    }

    pub fn put<T: BorshSerialize>(
        &mut self,
        prefix: &str,
        key: &str,
        value: T,
    ) -> Result<(), rocksdb::Error> {
        self.db
            .put(format!("{}{}", prefix, key), to_vec(&value).unwrap())
    }

    pub fn get<T: BorshDeserialize>(&self, prefix: &str, key: &str) -> Option<T> {
        let value = self.db.get(format!("{}{}", prefix, key)).unwrap();

        value.map(|value| from_slice(value.as_slice()).unwrap())
    }
}
