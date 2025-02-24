#![allow(dead_code)]

use aes_gcm::{aead::Aead, AeadCore, Aes256Gcm, KeyInit};
use bitcoin::{
    key::{rand, Secp256k1},
    secp256k1::{self, SecretKey},
    Address, OutPoint, PrivateKey, PublicKey, ScriptBuf, Transaction, Witness,
};
use bitcoincore_rpc::{Auth, Client, RpcApi};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

use tokio::{sync::Mutex, task::JoinHandle, time::sleep};

use mockcore::{Handle, TransactionTemplate};

use tempfile::TempDir;

use glittr::{
    database::{
        Database, DatabaseError, ASSET_LIST_PREFIX, COLLATERAL_ACCOUNTS_PREFIX,
        INDEXER_LAST_BLOCK_PREFIX, MESSAGE_PREFIX,
    },
    message::OpReturnMessage,
    varuint::Varuint,
    AssetList, BlockTx, CollateralAccounts, Indexer, LastIndexedBlock, MessageDataOutcome,
};

use std::{collections::HashMap, sync::Arc, time::Duration};

pub fn get_bitcoin_address() -> (Address, PublicKey) {
    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();

    let secret_key = SecretKey::new(&mut secp256k1::rand::thread_rng());

    let private_key = PrivateKey {
        compressed: true,
        network: bitcoin::NetworkKind::Test,
        inner: secret_key,
    };

    // Create the corresponding public key
    let public_key = PublicKey::from_private_key(&secp, &private_key);
    (
        Address::from_script(
            &ScriptBuf::new_p2wpkh(&public_key.wpubkey_hash().unwrap()),
            bitcoin::Network::Regtest,
        )
        .unwrap(),
        public_key,
    )
}

// ECIES
pub fn encrypt_message(
    public_key: &bitcoin::secp256k1::PublicKey,
    message: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let secp = Secp256k1::new();
    let mut rng = OsRng;

    // Generate ephemeral key pair
    let ephemeral_sk = SecretKey::new(&mut rng);
    let ephemeral_pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &ephemeral_sk);

    // Perform ECDH to get shared secret
    let shared_point = public_key.mul_tweak(&secp, &ephemeral_sk.into()).unwrap();

    // Derive symmetric key using SHA-256
    let mut hasher = Sha256::new();
    hasher.update(&shared_point.serialize());
    let symmetric_key = hasher.finalize();

    // Generate random 96-bit nonce
    let nonce: [u8; 12] = Aes256Gcm::generate_nonce(&mut OsRng).into();

    // Encrypt message using AES-GCM
    let cipher = Aes256Gcm::new_from_slice(&symmetric_key).unwrap();
    let ciphertext = cipher.encrypt(&nonce.into(), message.as_bytes()).unwrap();

    // Combine ephemeral public key, nonce, and ciphertext
    let mut encrypted = Vec::new();
    encrypted.extend_from_slice(&ephemeral_pk.serialize());
    encrypted.extend_from_slice(&nonce);
    encrypted.extend_from_slice(&ciphertext);

    Ok(encrypted)
}

pub fn decrypt_message(
    secret_key: &bitcoin::secp256k1::SecretKey,
    encrypted: &[u8],
) -> Result<String, Box<dyn std::error::Error>> {
    let secp = Secp256k1::new();

    // Split input into components
    let ephemeral_pk = bitcoin::secp256k1::PublicKey::from_slice(&encrypted[..33])?;
    let nonce = <[u8; 12]>::try_from(&encrypted[33..45])?;
    let ciphertext = &encrypted[45..];

    // Perform ECDH to get shared secret
    let scalar = secp256k1::Scalar::from_be_bytes(*secret_key.as_ref()).unwrap();

    let shared_point = ephemeral_pk.mul_tweak(&secp, &scalar).unwrap();

    // Derive symmetric key using SHA-256
    let mut hasher = Sha256::new();
    hasher.update(&shared_point.serialize());
    let symmetric_key = hasher.finalize();

    // Decrypt message using AES-GCM
    let cipher = Aes256Gcm::new_from_slice(&symmetric_key).unwrap();
    let plaintext = cipher.decrypt(&nonce.into(), ciphertext).unwrap();

    Ok(String::from_utf8(plaintext)?)
}

pub struct TestContext {
    pub indexer: Arc<Mutex<Indexer>>,
    pub core: Handle,
    pub _tempdir: TempDir,
}

impl TestContext {
    pub async fn new() -> Self {
        let tempdir = TempDir::new().unwrap();
        let core = tokio::task::spawn_blocking(mockcore::spawn)
            .await
            .expect("Task panicked");

        // Initial setup
        core.mine_blocks(2);

        let database = Arc::new(Mutex::new(Database::new(
            tempdir.path().to_str().unwrap().to_string(),
        )));
        let indexer = spawn_test_indexer(&database, core.url()).await;

        Self {
            indexer,
            core,
            _tempdir: tempdir,
        }
    }

    pub fn get_transaction_from_block_tx(&self, block_tx: BlockTx) -> Result<Transaction, ()> {
        let rpc = Client::new(
            self.core.url().as_str(),
            Auth::UserPass("".to_string(), "".to_string()),
        )
        .unwrap();

        let block_hash = rpc.get_block_hash(block_tx.block.0).unwrap();
        let block = rpc.get_block(&block_hash).unwrap();

        for (pos, tx) in block.txdata.iter().enumerate() {
            if pos == block_tx.tx.0 as usize {
                return Ok(tx.clone());
            }
        }

        Err(())
    }

    pub async fn build_and_mine_message(&mut self, message: &OpReturnMessage) -> BlockTx {
        let height = self.core.height();

        self.core.broadcast_tx(TransactionTemplate {
            fee: 0,
            inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
            op_return: Some(message.into_script()),
            op_return_index: Some(0),
            op_return_value: Some(0),
            output_values: &[0, 1000],
            outputs: 2,
            p2tr: false,
            recipient: None,
        });

        self.core.mine_blocks(1);

        BlockTx {
            block: Varuint(height + 1),
            tx: Varuint(1),
        }
    }

    pub async fn get_and_verify_message_outcome(&self, block_tx: BlockTx) -> MessageDataOutcome {
        let message: Result<MessageDataOutcome, DatabaseError> = self
            .indexer
            .lock()
            .await
            .database
            .lock()
            .await
            .get(MESSAGE_PREFIX, block_tx.to_string().as_str());

        message.expect("Message should exist")
    }

    pub async fn get_asset_map(&self) -> HashMap<String, AssetList> {
        let asset_map: Result<HashMap<String, AssetList>, DatabaseError> = self
            .indexer
            .lock()
            .await
            .database
            .lock()
            .await
            .expensive_find_by_prefix(ASSET_LIST_PREFIX)
            .map(|vec| {
                vec.into_iter()
                    .map(|(k, v)| {
                        (
                            k.trim_start_matches(&format!("{}:", ASSET_LIST_PREFIX))
                                .to_string(),
                            v,
                        )
                    })
                    .collect()
            });
        asset_map.expect("asset map should exist")
    }

    pub async fn get_collateralize_accounts(&self) -> HashMap<String, CollateralAccounts> {
        let collateralize_account: Result<HashMap<String, CollateralAccounts>, DatabaseError> =
            self.indexer
                .lock()
                .await
                .database
                .lock()
                .await
                .expensive_find_by_prefix(COLLATERAL_ACCOUNTS_PREFIX)
                .map(|vec| {
                    vec.into_iter()
                        .map(|(k, v)| {
                            (
                                k.trim_start_matches(&format!("{}:", COLLATERAL_ACCOUNTS_PREFIX))
                                    .to_string(),
                                v,
                            )
                        })
                        .collect()
                });
        collateralize_account.expect("collateral account should exist")
    }

    pub fn verify_asset_output(
        &self,
        asset_lists: &HashMap<String, AssetList>,
        block_tx_contract: &BlockTx,
        outpoint: &OutPoint,
        expected_value: u128,
    ) {
        let asset_output = asset_lists
            .get(&outpoint.to_string())
            .unwrap_or_else(|| panic!("Asset output {} should exist", outpoint.vout));

        let value_output = asset_output
            .list
            .get(&block_tx_contract.to_string())
            .unwrap();
        assert_eq!(*value_output, expected_value);
    }

    pub async fn verify_last_block(&self, expected_height: u64) {
        let last_block: LastIndexedBlock = self
            .indexer
            .lock()
            .await
            .database
            .lock()
            .await
            .get(INDEXER_LAST_BLOCK_PREFIX, "")
            .unwrap();

        assert_eq!(last_block.0, expected_height);
    }

    pub async fn drop(self) {
        tokio::task::spawn_blocking(|| drop(self.core))
            .await
            .expect("Drop failed");
    }

    pub async fn get_asset_list(&self) -> Vec<(String, AssetList)> {
        let asset_list: Result<Vec<(String, AssetList)>, DatabaseError> = self
            .indexer
            .lock()
            .await
            .database
            .lock()
            .await
            .expensive_find_by_prefix(ASSET_LIST_PREFIX);
        asset_list.expect("asset list should exist")
    }
}

pub async fn start_indexer(indexer: Arc<Mutex<Indexer>>) -> JoinHandle<()> {
    let handle = tokio::spawn(async move {
        indexer
            .lock()
            .await
            .run_indexer(Arc::new(Mutex::new(false)))
            .await
            .expect("Run indexer");
    });
    sleep(Duration::from_millis(100)).await; // let the indexer run first

    handle.abort();

    handle
}

pub async fn spawn_test_indexer(
    database: &Arc<Mutex<Database>>,
    rpc_url: String,
) -> Arc<Mutex<Indexer>> {
    Arc::new(Mutex::new(
        Indexer::new(
            Arc::clone(database),
            rpc_url,
            "".to_string(),
            "".to_string(),
        )
        .await
        .unwrap(),
    ))
}
