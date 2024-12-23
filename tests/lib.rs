use aes_gcm::{aead::Aead, AeadCore, Aes256Gcm, KeyInit};
use bitcoin::{
    hashes::{sha256, Hash},
    key::{rand, Keypair, Secp256k1},
    secp256k1::{self, Message, SecretKey},
    Address, OutPoint, PrivateKey, PublicKey, ScriptBuf, Transaction, Witness, XOnlyPublicKey,
};
use bitcoincore_rpc::{Auth, Client, RpcApi};
use glittr::{
    bloom_filter_to_compressed_vec,
    database::{
        Database, DatabaseError, ASSET_CONTRACT_DATA_PREFIX, ASSET_LIST_PREFIX,
        COLLATERAL_ACCOUNTS_PREFIX, INDEXER_LAST_BLOCK_PREFIX, MESSAGE_PREFIX,
        TICKER_TO_BLOCK_TX_PREFIX,
    },
    message::{
        ArgsCommitment, AssertValues, CallType, CloseAccountOption, Commitment, CommitmentMessage,
        ContractCall, ContractCreation, ContractType, MintBurnOption, OpReturnMessage,
        OpenAccountOption, OracleMessage, OracleMessageSigned, SwapOption, Transfer,
        TxTypeTransfer,
    },
    mint_burn_asset::{
        AccountType, BurnMechanisms, Collateralized, MBAMintMechanisms, MintBurnAssetContract,
        MintStructure, ProportionalType, RatioModel, ReturnCollateral, SwapMechanisms,
    },
    mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract},
    spec::{
        MintBurnAssetCollateralizedSpec, MintBurnAssetSpec, MintOnlyAssetSpec,
        MintOnlyAssetSpecPegInType, SpecContract, SpecContractType,
    },
    transaction_shared::{
        AllocationType, BloomFilterArgType, FreeMint, InputAsset, OracleSetting, Preallocated,
        PurchaseBurnSwap, RatioType, VestingPlan,
    },
    AssetContractData, AssetList, BlockTx, BlockTxTuple, CollateralAccounts, Flaw, Indexer,
    MessageDataOutcome, U128,
};
use growable_bloom_filter::GrowableBloom;
use mockcore::{Handle, TransactionTemplate};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{sync::Mutex, task::JoinHandle, time::sleep};

// Test utilities
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

struct TestContext {
    indexer: Arc<Mutex<Indexer>>,
    core: Handle,
    _tempdir: TempDir,
}

impl TestContext {
    async fn new() -> Self {
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

    fn get_transaction_from_block_tx(&self, block_tx: BlockTx) -> Result<Transaction, ()> {
        let rpc = Client::new(
            self.core.url().as_str(),
            Auth::UserPass("".to_string(), "".to_string()),
        )
        .unwrap();

        let block_hash = rpc.get_block_hash(block_tx.block).unwrap();
        let block = rpc.get_block(&block_hash).unwrap();

        for (pos, tx) in block.txdata.iter().enumerate() {
            if pos == block_tx.tx as usize {
                return Ok(tx.clone());
            }
        }

        Err(())
    }

    async fn build_and_mine_message(&mut self, message: &OpReturnMessage) -> BlockTx {
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
            block: height + 1,
            tx: 1,
        }
    }

    async fn get_and_verify_message_outcome(&self, block_tx: BlockTx) -> MessageDataOutcome {
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

    async fn get_asset_map(&self) -> HashMap<String, AssetList> {
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

    async fn get_collateralize_accounts(&self) -> HashMap<String, CollateralAccounts> {
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

    fn verify_asset_output(
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

    async fn verify_last_block(&self, expected_height: u64) {
        let last_block: u64 = self
            .indexer
            .lock()
            .await
            .database
            .lock()
            .await
            .get(INDEXER_LAST_BLOCK_PREFIX, "")
            .unwrap();

        assert_eq!(last_block, expected_height);
    }

    async fn drop(self) {
        tokio::task::spawn_blocking(|| drop(self.core))
            .await
            .expect("Drop failed");
    }

    async fn get_asset_list(&self) -> Vec<(String, AssetList)> {
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

async fn start_indexer(indexer: Arc<Mutex<Indexer>>) -> JoinHandle<()> {
    let handle = tokio::spawn(async move {
        indexer
            .lock()
            .await
            .run_indexer()
            .await
            .expect("Run indexer");
    });
    sleep(Duration::from_millis(100)).await; // let the indexer run first

    handle.abort();

    handle
}

async fn spawn_test_indexer(
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

#[tokio::test]
async fn test_integration_broadcast_op_return_message_success() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(10),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };
    println!("{}", message);

    let block_tx = ctx.build_and_mine_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.get_and_verify_message_outcome(block_tx).await;
    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_purchaseburnswap() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: None,
                        ratio: RatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx = ctx.build_and_mine_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.get_and_verify_message_outcome(block_tx).await;
    ctx.drop().await;
}

/// test_raw_btc_to_glittr_asset_burn e.g. raw btc to wbtc by burn
#[tokio::test]
async fn test_raw_btc_to_glittr_asset_burn() {
    let mut ctx = TestContext::new().await;

    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: None,
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: None,
                        ratio: RatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let (minter_address, _) = get_bitcoin_address();

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;
    let dust = 546;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(bitcoin_value - dust - fee),
        output_values: &[dust],
        outputs: 1,
        p2tr: false,
        recipient: Some(minter_address),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: height + 1,
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none());

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == (bitcoin_value - fee - dust) as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

///Example: most basic gBTC implementation
/// {
///     TxType: Contract,
///     SimpleAsset:{
///         supplyCap: 21000000,
///         Divisibility: 100000000,
///         Livetime: -10
///     },
///     Purchase:{
///         inputAsset: BTC,
///         payTo: pubkey,
///         Ratio: 1,
///     }
///     }
/// test_raw_btc_to_glittr_asset_purchase e.g. raw btc to wbtc by purchase
#[tokio::test]
async fn test_raw_btc_to_glittr_asset_purchase_gbtc() {
    let mut ctx = TestContext::new().await;

    let (contract_treasury, contract_treasury_pub_key) = get_bitcoin_address();
    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(21_000_000 * 10u128.pow(8))),
                divisibility: 8,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: Some(contract_treasury_pub_key.to_bytes()),
                        ratio: RatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };
    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    // NOTE: can only declare 1 output here
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[bitcoin_value - fee],
        outputs: 1,
        p2tr: false,
        recipient: Some(contract_treasury),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: height + 1,
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none());

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == (bitcoin_value - fee) as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

// test_raw_btc_to_glittr_asset_purchase e.g. raw btc to busd
#[tokio::test]
async fn test_raw_btc_to_glittr_asset_burn_oracle() {
    let mut ctx = TestContext::new().await;

    let secp = Secp256k1::new();
    let oracle_keypair = Keypair::new(&secp, &mut rand::thread_rng());
    let oracle_xonly = XOnlyPublicKey::from_keypair(&oracle_keypair);
    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: None,
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: None,
                        ratio: RatioType::Oracle {
                            setting: OracleSetting {
                                pubkey: oracle_xonly.0.serialize().to_vec(),
                                asset_id: Some("btc".to_string()),
                                block_height_slippage: 5,
                            },
                        },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let (minter_address, _) = get_bitcoin_address();

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;
    let dust = 546;
    let ratio = 1000;
    let oracle_out_value = (bitcoin_value - dust - fee) * ratio;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    let oracle_message = OracleMessage {
        asset_id: Some("btc".to_string()),
        block_height: height,
        input_outpoint: None,
        min_in_value: None,
        out_value: None,
        ratio: Some((1000, 1)), // 1 sat = 1000 usd,
        ltv: None,
        outstanding: None,
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(bitcoin_value - fee - dust),
        output_values: &[dust],
        outputs: 1,
        p2tr: false,
        recipient: Some(minter_address),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: height + 1,
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none(), "{:?}", mint_outcome.flaw);

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == oracle_out_value as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

/// Example: most basic Hermetica implementation
/// {
/// TxType: contract,
/// simpleAsset:{
/// 	supplyCap: -1,
/// 	Divisibility: 100,
/// 	Livetime: -10,
/// },
/// Purchase:{
/// 	inputAsset: BTC,
/// 	payTo: pubkey,
/// 	Ratio: {
/// 		Oracle: {
/// 	        oracleKey: pubKey,
/// 	        Args:{
///               satsPerDollar: int,
///               purchaseValue: float,
///               BLOCKHEIGHT: {AllowedBlockSlippage: -5},
///             },
///        },
///     },
/// },
/// }
///
#[tokio::test]
async fn test_raw_btc_to_glittr_asset_oracle_purchase() {
    let mut ctx = TestContext::new().await;

    let secp = Secp256k1::new();
    let oracle_keypair = Keypair::new(&secp, &mut rand::thread_rng());
    let oracle_xonly = XOnlyPublicKey::from_keypair(&oracle_keypair);

    let (treasury_address, treasury_address_pub_key) = get_bitcoin_address();
    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(21_000_000 * 10u128.pow(8))),
                divisibility: 8,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: Some(treasury_address_pub_key.to_bytes()),
                        ratio: RatioType::Oracle {
                            setting: OracleSetting {
                                pubkey: oracle_xonly.0.serialize().to_vec(),
                                asset_id: Some("btc".to_string()),
                                block_height_slippage: 5,
                            },
                        },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
        }),
        contract_call: None,
        transfer: None,
    };

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;
    let oracle_out_value = (bitcoin_value - fee) * 1000;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    let oracle_message = OracleMessage {
        input_outpoint: None,
        min_in_value: None,
        out_value: None,
        asset_id: Some("btc".to_string()),
        block_height: height,
        ratio: Some((1000, 1)), // 1 sats = 1000 glittr asset
        ltv: None,
        outstanding: None,
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        contract_creation: None,
        transfer: None,
    };
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[bitcoin_value - fee],
        outputs: 1,
        p2tr: false,
        recipient: Some(treasury_address),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: height + 1,
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none(), "{:?}", mint_outcome.flaw);

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == oracle_out_value as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

#[tokio::test]
async fn test_metaprotocol_to_glittr_asset() {
    let mut ctx = TestContext::new().await;

    let secp = Secp256k1::new();
    let oracle_keypair = Keypair::new(&secp, &mut rand::thread_rng());
    let oracle_xonly = XOnlyPublicKey::from_keypair(&oracle_keypair);
    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: None,
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::Rune,
                        pay_to_key: None,
                        ratio: RatioType::Oracle {
                            setting: OracleSetting {
                                pubkey: oracle_xonly.0.serialize().to_vec(),
                                asset_id: Some("rune:840000:3".to_string()),
                                block_height_slippage: 5,
                            },
                        },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let (minter_address, _) = get_bitcoin_address();

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;
    let dust = 546;
    let oracle_out_value = 10000;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    let input_txid = ctx.core.tx((height - 1) as usize, 0).compute_txid();
    let oracle_message = OracleMessage {
        input_outpoint: Some(
            OutPoint {
                txid: input_txid,
                vout: 0,
            }
            .into(),
        ),
        min_in_value: Some(U128(0)),
        out_value: Some(U128(oracle_out_value)),
        asset_id: Some("rune:840000:3".to_string()),
        block_height: height,
        ratio: None,
        ltv: None,
        outstanding: None,
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(bitcoin_value - fee - dust),
        output_values: &[dust],
        outputs: 1,
        p2tr: false,
        recipient: Some(minter_address),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: height + 1,
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none(), "{:?}", mint_outcome.flaw);

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == oracle_out_value);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_freemint() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(10),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx = ctx.build_and_mine_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.get_and_verify_message_outcome(block_tx).await;
    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_mint_freemint() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(10),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let total_mints = 10;

    for _ in 0..total_mints {
        let message = OpReturnMessage {
            contract_call: Some(ContractCall {
                contract: Some(block_tx_contract.to_tuple()),
                call_type: CallType::Mint(MintBurnOption {
                    pointer: Some(1),
                    oracle_message: None,
                    pointer_to_key: None,
                    assert_values: None,
                    commitment_message: None,
                }),
            }),
            transfer: None,
            contract_creation: None,
        };
        ctx.build_and_mine_message(&message).await;
    }

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_contract_data: Result<AssetContractData, DatabaseError> =
        ctx.indexer.lock().await.database.lock().await.get(
            ASSET_CONTRACT_DATA_PREFIX,
            block_tx_contract.to_string().as_str(),
        );
    let data_free_mint = asset_contract_data.expect("Free mint data should exist");

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    assert_eq!(data_free_mint.minted_supply, total_mints * 10);
    assert_eq!(asset_lists.len() as u32, total_mints as u32);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_mint_freemint_supply_cap_exceeded() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(50)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(50)),
                        amount_per_mint: U128(50),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    // first mint
    let message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(block_tx_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        contract_creation: None,
        transfer: None,
    };
    ctx.build_and_mine_message(&message).await;

    // second mint should be execeeded the supply cap
    // and the total minted should be still 1
    let message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(block_tx_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(0),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        contract_creation: None,
        transfer: None,
    };
    let overflow_block_tx = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_contract_data: Result<AssetContractData, DatabaseError> =
        ctx.indexer.lock().await.database.lock().await.get(
            ASSET_CONTRACT_DATA_PREFIX,
            block_tx_contract.to_string().as_str(),
        );
    let data_free_mint = asset_contract_data.expect("Free mint data should exist");

    assert_eq!(data_free_mint.minted_supply, 50);

    let outcome = ctx.get_and_verify_message_outcome(overflow_block_tx).await;
    assert_eq!(outcome.flaw.unwrap(), Flaw::SupplyCapExceeded);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_mint_freemint_livetime_notreached() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 5,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(50),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    // first mint not reach the live time
    let message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(block_tx_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };
    let notreached_block_tx = ctx.build_and_mine_message(&message).await;
    println!("Not reached livetime block tx: {:?}", notreached_block_tx);

    let message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(block_tx_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };
    ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_contract_data: Result<AssetContractData, DatabaseError> =
        ctx.indexer.lock().await.database.lock().await.get(
            ASSET_CONTRACT_DATA_PREFIX,
            block_tx_contract.to_string().as_str(),
        );
    let data_free_mint = asset_contract_data.expect("Free mint data should exist");

    let outcome = ctx
        .get_and_verify_message_outcome(notreached_block_tx)
        .await;
    assert_eq!(outcome.flaw.unwrap(), Flaw::ContractIsNotLive);

    assert_eq!(data_free_mint.minted_supply, 50);

    ctx.drop().await;
}

// Example: pre-allocation with free mint
// {
// TxType: Contract,
// simpleAsset:{
// 	supplyCap: 1000,
// 	Divisibility: 100,
// 	liveTime: -100,
// },
// Allocation:{
// {100:pk1,pk2, pk3, pk4},
// {300:reservePubKey},
// {200:freeMint}
// vestingSchedule:{
// 		Fractions: [.25, .25, .25, .25],
// 		Blocks: [-10000, -20000, -30000, -40000]
// },
// FreeMint:{
// 		mintCap: 1,
// },
// },
// }

#[tokio::test]
async fn test_integration_mint_preallocated_freemint() {
    let mut ctx = TestContext::new().await;

    let (address_1, pubkey_1) = get_bitcoin_address();
    let (_address_2, pubkey_2) = get_bitcoin_address();
    let (_address_3, pubkey_3) = get_bitcoin_address();
    let (_address_4, pubkey_4) = get_bitcoin_address();
    let (_address_reserve, pubkey_reserve) = get_bitcoin_address();

    println!("pub key {:?}", pubkey_1.to_bytes());

    let mut allocations: HashMap<U128, AllocationType> = HashMap::new();
    allocations.insert(
        U128(100),
        AllocationType::VecPubkey(vec![
            pubkey_1.to_bytes(),
            pubkey_2.to_bytes(),
            pubkey_3.to_bytes(),
            pubkey_4.to_bytes(),
        ]),
    );
    allocations.insert(
        U128(300),
        AllocationType::VecPubkey(vec![pubkey_reserve.to_bytes()]),
    );

    let vesting_plan =
        VestingPlan::Scheduled(vec![((1, 4), -4), ((1, 4), -2), ((1, 4), -3), ((1, 4), -1)]);

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    preallocated: Some(Preallocated {
                        // total 400 + 300 = 700
                        allocations,
                        vesting_plan: Some(vesting_plan),
                    }),
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(300)),
                        amount_per_mint: U128(1),
                    }),
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(block_tx_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };
    // give vestee money and mint
    let height = ctx.core.height();

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: None,
        op_return_index: None,
        op_return_value: None,
        output_values: &[1000],
        outputs: 1,
        p2tr: false,
        recipient: Some(address_1.clone()),
    });
    ctx.core.mine_blocks(1);

    let mut witness = Witness::new();
    witness.push(pubkey_1.to_bytes());
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height + 1) as usize, 1, 0, witness)],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546],
        outputs: 2,
        p2tr: false,
        recipient: Some(address_1),
    });

    ctx.core.mine_blocks(1);

    let total_mints = 10;
    // free mints
    for _ in 0..total_mints {
        ctx.build_and_mine_message(&mint_message).await;
    }

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_contract_data: Result<AssetContractData, DatabaseError> =
        ctx.indexer.lock().await.database.lock().await.get(
            ASSET_CONTRACT_DATA_PREFIX,
            block_tx_contract.to_string().as_str(),
        );
    let data_free_mint = asset_contract_data.expect("Free mint data should exist");

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    assert_eq!(data_free_mint.minted_supply, 10 + 50);
    assert_eq!(asset_lists.len() as u32, total_mints as u32 + 1);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_mint_freemint_invalidpointer() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(50),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        contract_call: None,
        transfer: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    // set pointer to index 0 (op_return output), it should be error
    let message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(block_tx_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(0),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };
    let invalid_pointer_block_tx = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let outcome = ctx
        .get_and_verify_message_outcome(invalid_pointer_block_tx)
        .await;
    assert_eq!(outcome.flaw.unwrap(), Flaw::InvalidPointer);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_transfer_normal() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(100_000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(100_000)),
                        amount_per_mint: U128(20_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };
    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(block_tx_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        contract_creation: None,
        transfer: None,
    };
    let mint_block_tx = ctx.build_and_mine_message(&message).await;
    let message = OpReturnMessage {
        transfer: Some(Transfer {
            transfers: [
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 1,
                    amount: U128(10_000),
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 2,
                    amount: U128(2_000),
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 2,
                    amount: U128(500),
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 3,
                    amount: U128(8_000),
                },
            ]
            .to_vec(),
        }),
        contract_call: None,
        contract_creation: None,
    };

    // outputs have 4 outputs
    // - index 0: op_return
    // - index 1..3: outputs
    let new_output_txid = ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (mint_block_tx.block as usize, 1, 1, Witness::new()), // UTXO contain assets
            (mint_block_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000, 1000],
        outputs: 3,
        p2tr: false,
        recipient: None,
    });
    ctx.core.mine_blocks(1);

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_map = ctx.get_asset_map().await;

    let mut total_amount = 0;
    for (_, v) in &asset_map {
        let value = v.list.get(&block_tx_contract.to_string()).unwrap();
        total_amount += value
    }
    // the total amount should be 20,000, same as before splitting
    assert_eq!(total_amount, 20_000);

    ctx.verify_asset_output(
        &asset_map,
        &block_tx_contract,
        &OutPoint {
            txid: new_output_txid,
            vout: 1,
        },
        10_000,
    );

    ctx.verify_asset_output(
        &asset_map,
        &block_tx_contract,
        &OutPoint {
            txid: new_output_txid,
            vout: 2,
        },
        2500,
    );

    // this expected 7500 not 8_000 because the remaining asset or unallocated < amount
    // transferred value picks MIN(remainder, amount)
    ctx.verify_asset_output(
        &asset_map,
        &block_tx_contract,
        &OutPoint {
            txid: new_output_txid,
            vout: 3,
        },
        7500,
    );

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_transfer_overflow_output() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(100_000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(100_000)),
                        amount_per_mint: U128(20_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };
    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(block_tx_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };
    let mint_block_tx = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        transfer: Some(Transfer {
            transfers: [
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 1,
                    amount: U128(10_000),
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 2,
                    amount: U128(2_000),
                },
            ]
            .to_vec(),
        }),
        contract_call: None,
        contract_creation: None,
    };

    // outputs have 2 outputs
    // - index 0: op_return
    // - index 1: output
    let new_output_txid = ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (mint_block_tx.block as usize, 1, 1, Witness::new()), // UTXO contain assets
            (mint_block_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000],
        outputs: 1,
        p2tr: false,
        recipient: None,
    });
    ctx.core.mine_blocks(1);
    let height = ctx.core.height();

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let outcome = ctx
        .get_and_verify_message_outcome(BlockTx {
            block: height,
            tx: 1,
        })
        .await;
    assert_eq!(outcome.flaw.unwrap(), Flaw::OutputOverflow([1].to_vec()));

    let asset_map = ctx.get_asset_map().await;

    let mut total_amount = 0;
    for (_, v) in &asset_map {
        let value = v.list.get(&block_tx_contract.to_string()).unwrap();
        total_amount += value
    }
    // the total amount should be 20,000, same as before splitting
    assert_eq!(total_amount, 20_000);

    // transfer: 10_000
    // remainder asset to the first non_op_return index (this index or output): 10_000
    // total: 20_000
    ctx.verify_asset_output(
        &asset_map,
        &block_tx_contract,
        &OutPoint {
            txid: new_output_txid,
            vout: 1,
        },
        20_000,
    );

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_transfer_utxo() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(100_000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(100_000)),
                        amount_per_mint: U128(20_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        contract_call: None,
        transfer: None,
    };
    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(block_tx_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        contract_creation: None,
        transfer: None,
    };
    let mint_block_tx = ctx.build_and_mine_message(&message).await;

    // outputs have 2 outputs
    // - index 0: output target (default fallback) first non_op_return output
    // - index 1: output
    let new_output_txid = ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (mint_block_tx.block as usize, 1, 1, Witness::new()), // UTXO contain assets
            (mint_block_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: None,
        op_return_index: None,
        op_return_value: None,
        output_values: &[1000, 1000],
        outputs: 2,
        p2tr: false,
        recipient: None,
    });
    ctx.core.mine_blocks(1);

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_map = ctx.get_asset_map().await;

    let mut total_amount = 0;
    for (_, v) in &asset_map {
        let value = v.list.get(&block_tx_contract.to_string()).unwrap();
        total_amount += value
    }
    // the total amount should be 20,000, same as before splitting
    assert_eq!(total_amount, 20_000);

    ctx.verify_asset_output(
        &asset_map,
        &block_tx_contract,
        &OutPoint {
            txid: new_output_txid,
            vout: 0,
        },
        20_000,
    );

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_glittr_asset_mint_purchase() {
    let mut ctx = TestContext::new().await;

    // Create first contract with free mint
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(100),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let first_contract = ctx.build_and_mine_message(&message).await;

    // Mint first contract
    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(first_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        contract_creation: None,
        transfer: None,
    };

    let first_mint_tx = ctx.build_and_mine_message(&mint_message).await;

    // Create second contract that uses first as input asset
    let (treasury_address, treasury_pub_key) = get_bitcoin_address();
    let second_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(500)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::GlittrAsset(first_contract.to_tuple()),
                        pay_to_key: Some(treasury_pub_key.to_bytes()),
                        ratio: RatioType::Fixed { ratio: (2, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let second_contract = ctx.build_and_mine_message(&second_message).await;

    // Mint second contract using first contract as input
    let second_mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(second_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: Some(Transfer {
            transfers: [TxTypeTransfer {
                asset: first_contract.to_tuple(),
                output: 1,
                amount: U128(100),
            }]
            .to_vec(),
        }),
        contract_creation: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (first_mint_tx.block as usize, 1, 1, Witness::new()), // UTXO contain assets
            (first_mint_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(second_mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000, 1000],
        outputs: 3,
        p2tr: false,
        recipient: Some(treasury_address),
    });

    ctx.core.mine_blocks(1);

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify outcomes
    let first_outcome = ctx.get_and_verify_message_outcome(first_contract).await;
    assert!(first_outcome.flaw.is_none());

    let first_mint_outcome = ctx.get_and_verify_message_outcome(first_mint_tx).await;
    assert!(first_mint_outcome.flaw.is_none());

    let second_outcome = ctx.get_and_verify_message_outcome(second_contract).await;
    assert!(second_outcome.flaw.is_none());

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_collateralized_mba() {
    let mut ctx = TestContext::new().await;

    let (owner_address, _) = get_bitcoin_address();

    // Create oracle keypair
    let secp = Secp256k1::new();
    let oracle_keypair = Keypair::new(&secp, &mut rand::thread_rng());
    let oracle_xonly = XOnlyPublicKey::from_keypair(&oracle_keypair);

    // Create MOA (collateral token) with free mint
    let collateral_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1_000_000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1_000_000)),
                        amount_per_mint: U128(100_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let collateral_contract = ctx.build_and_mine_message(&collateral_message).await;

    // Mint collateral tokens
    let mint_collateral_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(collateral_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    let collateral_mint_tx = ctx.build_and_mine_message(&mint_collateral_message).await;

    let mba_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Mba(MintBurnAssetContract {
                ticker: None,
                supply_cap: Some(U128(500_000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MBAMintMechanisms {
                    preallocated: None,
                    free_mint: None,
                    purchase: None,
                    collateralized: Some(Collateralized {
                        input_assets: vec![InputAsset::GlittrAsset(collateral_contract.to_tuple())],
                        _mutable_assets: false,
                        mint_structure: MintStructure::Account(AccountType {
                            max_ltv: (7, 10),
                            ratio: RatioType::Oracle {
                                setting: OracleSetting {
                                    pubkey: oracle_xonly.0.serialize().to_vec(),
                                    block_height_slippage: 5,
                                    asset_id: None,
                                },
                            },
                        }),
                    }),
                },
                burn_mechanism: BurnMechanisms {
                    return_collateral: Some(ReturnCollateral {
                        oracle_setting: Some(OracleSetting {
                            pubkey: oracle_xonly.0.serialize().to_vec(),
                            asset_id: None,
                            block_height_slippage: 5,
                        }),
                        fee: None,
                    }),
                },
                swap_mechanism: SwapMechanisms { fee: None },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let mba_contract = ctx.build_and_mine_message(&mba_message).await;

    let open_account_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::OpenAccount(OpenAccountOption {
                pointer_to_key: 1,
                share_amount: U128(100),
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (collateral_mint_tx.block as usize, 1, 1, Witness::new()), // UTXO containing collateral
            (collateral_mint_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(open_account_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000], // Values for outputs
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let account_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    // Create oracle message for mint
    let oracle_message = OracleMessage {
        asset_id: None,
        block_height: ctx.core.height(),
        input_outpoint: Some(
            OutPoint {
                txid: ctx
                    .core
                    .tx(account_block_tx.block as usize, 1)
                    .compute_txid(),
                vout: 1,
            }
            .into(),
        ),
        min_in_value: None,
        out_value: Some(U128(50_000)), // Amount to mint
        ratio: None,
        ltv: Some((5, 10)), // 50% LTV
        outstanding: Some(U128(50_000)),
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    // Mint using oracle message
    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(2),
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
                pointer_to_key: Some(1),
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast mint transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (account_block_tx.block as usize, 1, 1, Witness::new()), // UTXO containing collateral account
            (account_block_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546, 546], // Values for outputs
        outputs: 4,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify all outcomes
    let collateral_outcome = ctx
        .get_and_verify_message_outcome(collateral_contract)
        .await;
    assert!(collateral_outcome.flaw.is_none());

    let mba_outcome = ctx.get_and_verify_message_outcome(mba_contract).await;
    assert!(mba_outcome.flaw.is_none());

    let account_outcome = ctx.get_and_verify_message_outcome(account_block_tx).await;
    assert!(account_outcome.flaw.is_none());

    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none(), "{:?}", mint_outcome.flaw);

    // Verify minted assets
    let asset_lists = ctx.get_asset_map().await;
    for (k, v) in &asset_lists {
        println!("Asset output: {}: {:?}", k, v);
    }

    // Find and verify the minted asset amount
    let minted_amount = asset_lists
        .values()
        .find_map(|list| list.list.get(&mba_contract.to_string()))
        .expect("Minted asset should exist");

    assert_eq!(*minted_amount, 50_000); // Amount specified in oracle message

    // Verify collateral accounts
    let collateral_accounts = ctx.get_collateralize_accounts().await;
    println!("{:?}", collateral_accounts);
    assert_eq!(collateral_accounts.len(), 1);

    let accounts = collateral_accounts.values().next().unwrap();
    let collateral_account = accounts
        .collateral_accounts
        .get(&mba_contract.to_string())
        .unwrap();
    assert_eq!(collateral_account.share_amount, 100);
    assert_eq!(collateral_account.collateral_amounts, [((3, 1), 100000)]);
    assert_eq!(collateral_account.ltv, (5, 10)); // LTV from oracle message
    assert_eq!(collateral_account.amount_outstanding, 50_000); // Outstanding amount from oracle message

    // Create oracle message for burn
    let burn_oracle_message = OracleMessage {
        asset_id: None,
        block_height: ctx.core.height(),
        input_outpoint: Some(
            OutPoint {
                txid: ctx.core.tx(mint_block_tx.block as usize, 1).compute_txid(),
                vout: 1,
            }
            .into(),
        ),
        min_in_value: None,
        out_value: Some(U128(25_000)), // Amount to burn
        ratio: None,
        ltv: Some((3, 10)),              // Updated LTV after partial repayment
        outstanding: Some(U128(25_000)), // Remaining outstanding amount
    };

    let burn_msg = Message::from_digest_slice(
        sha256::Hash::hash(
            serde_json::to_string(&burn_oracle_message)
                .unwrap()
                .as_bytes(),
        )
        .as_byte_array(),
    )
    .unwrap();

    let burn_signature = secp.sign_schnorr(&burn_msg, &oracle_keypair);

    // Create burn message
    let burn_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::Burn(MintBurnOption {
                oracle_message: Some(OracleMessageSigned {
                    signature: burn_signature.serialize().to_vec(),
                    message: burn_oracle_message.clone(),
                }),
                pointer_to_key: Some(1),
                pointer: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast burn transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (mint_block_tx.block as usize, 1, 1, Witness::new()), // UTXO containing collateral account
            (mint_block_tx.block as usize, 1, 2, Witness::new()), // UTXO containing mint
            (mint_block_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(burn_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546], // Values for outputs
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let burn_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify burn outcome
    let burn_outcome = ctx.get_and_verify_message_outcome(burn_block_tx).await;
    assert!(burn_outcome.flaw.is_none(), "{:?}", burn_outcome.flaw);

    // Verify remaining assets after burn
    let asset_lists_after_burn = ctx.get_asset_map().await;
    for (k, v) in &asset_lists_after_burn {
        println!("Asset output after burn: {}: {:?}", k, v);
    }

    // Find and verify the remaining asset amount
    let remaining_amount = asset_lists_after_burn
        .values()
        .find_map(|list| list.list.get(&mba_contract.to_string()))
        .expect("Remaining asset should exist");

    assert_eq!(*remaining_amount, 25_000); // Original amount (50,000) - burned amount (25,000)

    // Verify updated collateral account after burn
    let collateral_accounts_after_burn = ctx.get_collateralize_accounts().await;
    assert_eq!(collateral_accounts_after_burn.len(), 1);

    let accounts = collateral_accounts_after_burn.values().next().unwrap();
    let account_after_burn = accounts
        .collateral_accounts
        .get(&mba_contract.to_string())
        .unwrap();
    assert_eq!(account_after_burn.share_amount, 100);
    assert_eq!(account_after_burn.collateral_amounts, [((3, 1), 100000)]); // Collateral amount unchanged
    assert_eq!(account_after_burn.ltv, (3, 10)); // Updated LTV from oracle message
    assert_eq!(account_after_burn.amount_outstanding, 25_000); // Updated outstanding amount from oracle message

    // Create final burn message to clear outstanding amount
    let final_burn_oracle_message = OracleMessage {
        asset_id: None,
        block_height: ctx.core.height(),
        input_outpoint: Some(
            OutPoint {
                txid: ctx.core.tx(burn_block_tx.block as usize, 1).compute_txid(),
                vout: 1,
            }
            .into(),
        ),
        min_in_value: None,
        out_value: Some(U128(25_000)), // Burn remaining amount
        ratio: None,
        ltv: Some((0, 10)),         // Set LTV to 0
        outstanding: Some(U128(0)), // Set outstanding to 0
    };

    let final_burn_msg = Message::from_digest_slice(
        sha256::Hash::hash(
            serde_json::to_string(&final_burn_oracle_message)
                .unwrap()
                .as_bytes(),
        )
        .as_byte_array(),
    )
    .unwrap();

    let final_burn_signature = secp.sign_schnorr(&final_burn_msg, &oracle_keypair);

    // Create final burn message
    let final_burn_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::Burn(MintBurnOption {
                oracle_message: Some(OracleMessageSigned {
                    signature: final_burn_signature.serialize().to_vec(),
                    message: final_burn_oracle_message.clone(),
                }),
                pointer_to_key: Some(1),
                pointer: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast final burn transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (burn_block_tx.block as usize, 1, 1, Witness::new()), // UTXO containing collateral account
            (burn_block_tx.block as usize, 1, 2, Witness::new()), // UTXO containing remaining minted tokens
            (burn_block_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(final_burn_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546], // Values for outputs
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let final_burn_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    // Create close account message
    let close_account_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::CloseAccount(CloseAccountOption {
                pointer: 1, // Output index for returned collateral
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast close account transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (final_burn_block_tx.block as usize, 1, 1, Witness::new()), // UTXO containing collateral account
            (final_burn_block_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(close_account_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546], // Values for outputs
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let close_account_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify final burn outcome
    let final_burn_outcome = ctx
        .get_and_verify_message_outcome(final_burn_block_tx)
        .await;
    assert!(
        final_burn_outcome.flaw.is_none(),
        "{:?}",
        final_burn_outcome.flaw
    );

    // Verify close account outcome
    let close_account_outcome = ctx
        .get_and_verify_message_outcome(close_account_block_tx)
        .await;
    assert!(
        close_account_outcome.flaw.is_none(),
        "{:?}",
        close_account_outcome.flaw
    );

    // Verify all minted tokens are burned
    let final_asset_lists = ctx.get_asset_map().await;
    let remaining_minted = final_asset_lists
        .values()
        .find_map(|list| list.list.get(&mba_contract.to_string()))
        .unwrap_or(&0);
    assert_eq!(*remaining_minted, 0);

    // Verify collateral account is deleted
    let final_collateral_accounts = ctx.get_collateralize_accounts().await;
    assert_eq!(final_collateral_accounts.len(), 0);

    // Verify collateral tokens are returned
    let returned_collateral = final_asset_lists
        .values()
        .find_map(|list| list.list.get(&collateral_contract.to_string()))
        .expect("Returned collateral should exist");
    assert_eq!(*returned_collateral, 100_000); // Original collateral amount

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_proportional_mba_lp() {
    let mut ctx = TestContext::new().await;
    let (owner_address, _) = get_bitcoin_address();

    // Create two MOA tokens to be used in the liquidity pool
    let token1_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1_000_000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1_000_000)),
                        amount_per_mint: U128(100_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let token2_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1_000_000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1_000_000)),
                        amount_per_mint: U128(50_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let token1_contract = ctx.build_and_mine_message(&token1_message).await;
    let token2_contract = ctx.build_and_mine_message(&token2_message).await;

    // Mint both tokens
    let mint_token1_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(token1_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    let mint_token2_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(token2_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    let token1_mint_tx = ctx.build_and_mine_message(&mint_token1_message).await;
    let token2_mint_tx = ctx.build_and_mine_message(&mint_token2_message).await;

    // Create LP token contract
    let lp_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Mba(MintBurnAssetContract {
                ticker: None,
                supply_cap: None, // No supply cap for LP tokens
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MBAMintMechanisms {
                    preallocated: None,
                    free_mint: None,
                    purchase: None,
                    collateralized: Some(Collateralized {
                        input_assets: vec![
                            InputAsset::GlittrAsset(token1_contract.to_tuple()),
                            InputAsset::GlittrAsset(token2_contract.to_tuple()),
                        ],
                        _mutable_assets: false,
                        mint_structure: MintStructure::Proportional(ProportionalType {
                            ratio_model: RatioModel::ConstantProduct,
                            inital_mint_pointer_to_key: None,
                        }),
                    }),
                },
                burn_mechanism: BurnMechanisms {
                    return_collateral: Some(ReturnCollateral {
                        fee: None,
                        oracle_setting: None,
                    }),
                },
                swap_mechanism: SwapMechanisms { fee: None },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let lp_contract = ctx.build_and_mine_message(&lp_message).await;

    // Provide liquidity and mint LP tokens
    let mint_lp_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(lp_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast open account transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (token1_mint_tx.block as usize, 1, 1, Witness::new()),
            (token2_mint_tx.block as usize, 1, 1, Witness::new()),
            (token2_mint_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(mint_lp_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000],
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let mint_lp_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify initial setup
    let mint_lp_outcome = ctx.get_and_verify_message_outcome(mint_lp_block_tx).await;
    assert!(mint_lp_outcome.flaw.is_none(), "{:?}", mint_lp_outcome.flaw);

    let asset_lists = ctx.get_asset_map().await;

    let lp_minted_amount = asset_lists
        .values()
        .find_map(|list| list.list.get(&lp_contract.to_string()))
        .expect("Minted asset should exist");

    // Minted LP: https://github.com/Uniswap/v2-core/blob/master/contracts/UniswapV2Pair.sol#L120-L123
    assert_eq!(*lp_minted_amount, 70710);

    // Test swap functionality

    let token1_mint_tx = ctx.build_and_mine_message(&mint_token1_message).await;

    let rebase_token1_message = OpReturnMessage {
        transfer: Some(Transfer {
            transfers: vec![TxTypeTransfer {
                asset: token1_contract.to_tuple(),
                output: 2,
                amount: U128(100),
            }],
        }),
        contract_creation: None,
        contract_call: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (token1_mint_tx.block as usize, 1, 1, Witness::new()),
            (token1_mint_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(rebase_token1_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000, 1000],
        outputs: 3,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let rebase_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let swap_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(lp_contract.to_tuple()),
            call_type: CallType::Swap(SwapOption {
                pointer: 1,
                assert_values: Some(AssertValues {
                    input_values: Some(vec![U128(100)]),
                    total_collateralized: None,
                    min_out_value: Some(U128(49)),
                }),
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast swap transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (rebase_block_tx.block as usize, 1, 2, Witness::new()),
            (rebase_block_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(swap_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000],
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let swap_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify swap outcome
    let swap_outcome = ctx.get_and_verify_message_outcome(swap_block_tx).await;
    assert!(swap_outcome.flaw.is_none(), "{:?}", swap_outcome.flaw);

    let asset_lists = ctx.get_asset_map().await;
    let token_2_swapped = asset_lists
        .values()
        .find_map(|list| list.list.get(&token2_contract.to_string()))
        .expect("Minted asset should exist");

    // token_1_input = 100
    // token_1_total = 100_000
    // token_2_total = 50_000
    // k_before == k_after
    // token_1_total * token_2_total = (token_1_total + token_1_input) * (token_2_total - token_2_out)
    // token_2_out = token_2_total - (token_1_total * token_2_total) / (token_1_total + token_1_input )
    // token_2_out = 50_000 - (100_000 * 50_000) / (100_100)
    // token_2_out = 49
    assert_eq!(*token_2_swapped, 49);

    // Burn LP
    let burn_lp_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(lp_contract.to_tuple()),
            call_type: CallType::Burn(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast burn transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (mint_lp_block_tx.block as usize, 1, 1, Witness::new()),
            (mint_lp_block_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(burn_lp_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000],
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let burn_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify burn outcome
    let burn_outcome = ctx.get_and_verify_message_outcome(burn_block_tx).await;
    assert!(burn_outcome.flaw.is_none(), "{:?}", burn_outcome.flaw);

    // Verify final state
    let final_collateral_accounts = ctx.get_collateralize_accounts().await;
    assert_eq!(final_collateral_accounts.len(), 0);

    // Verify returned assets
    let final_assets = ctx.get_asset_map().await;
    for (k, v) in &final_assets {
        println!("Final assets: {}: {:?}", k, v);
    }

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                    input_asset: Some(InputAsset::Rune),
                    peg_in_type: Some(MintOnlyAssetSpecPegInType::Burn),
                }),
                block_tx: None,
                pointer: None,
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    ctx.verify_last_block(block_tx_contract.block).await;
    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, None);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_update() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![InputAsset::Rune].into(),
                        mint_structure: Some(MintStructure::Ratio(RatioType::Fixed {
                            ratio: (10, 10),
                        })),
                    }),
                }),
                block_tx: None,
                pointer: Some(1),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![
                            InputAsset::Rune,
                            InputAsset::RawBtc,
                            InputAsset::Ordinal,
                        ]
                        .into(),
                        mint_structure: None,
                    }),
                }),
                block_tx: Some(block_tx_contract.to_tuple()),
                pointer: Some(1),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (block_tx_contract.block as usize, 1, 1, Witness::new()), // spec owner
            (block_tx_contract.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000, 1000],
        outputs: 3,
        p2tr: false,
        recipient: None,
    });
    ctx.core.mine_blocks(1);
    let block_tx_first_update = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![InputAsset::Rune, InputAsset::RawBtc].into(),
                        mint_structure: None,
                    }),
                }),
                block_tx: Some(block_tx_contract.to_tuple()),
                pointer: Some(1),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (block_tx_first_update.block as usize, 1, 1, Witness::new()), // spec owner
            (block_tx_first_update.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000, 1000],
        outputs: 3,
        p2tr: false,
        recipient: None,
    });
    ctx.core.mine_blocks(1);
    let block_tx_second_update = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    let message = ctx
        .get_and_verify_message_outcome(block_tx_first_update)
        .await;
    assert_eq!(message.flaw, None);

    let message = ctx
        .get_and_verify_message_outcome(block_tx_second_update)
        .await;
    assert_eq!(message.flaw, None);

    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, None);

    if let ContractType::Spec(spec_contract) = message
        .message
        .unwrap()
        .contract_creation
        .unwrap()
        .contract_type
    {
        if let SpecContractType::MintBurnAsset(mba_spec) = spec_contract.spec {
            let prev_input_assets = mba_spec.collateralized.unwrap().input_assets.unwrap();
            assert_eq!(prev_input_assets.len(), 2);
            itertools::assert_equal(
                prev_input_assets.iter(),
                vec![InputAsset::Rune, InputAsset::RawBtc].iter(),
            );
        } else {
            panic!("Invalid spec contract type")
        }
    } else {
        panic!("Invalid contract type");
    };

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_update_not_allowed() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![InputAsset::Rune].into(),
                        mint_structure: Some(MintStructure::Ratio(RatioType::Fixed {
                            ratio: (10, 10),
                        })),
                    }),
                }),
                block_tx: None,
                pointer: Some(1),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![
                            InputAsset::Rune,
                            InputAsset::RawBtc,
                            InputAsset::Ordinal,
                        ]
                        .into(),
                        mint_structure: None,
                    }),
                }),
                block_tx: Some(block_tx_contract.to_tuple()),
                pointer: Some(1),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    // the tx would be fail and has a flaw because the UTXO isn't the owner of the spec.
    let block_tx_update = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let message = ctx.get_and_verify_message_outcome(block_tx_update).await;
    assert_eq!(message.flaw, Some(Flaw::SpecUpdateNotAllowed));

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_moa_valid_contract_creation() {
    let (_, address_pubkey) = get_bitcoin_address();
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                    input_asset: Some(InputAsset::Rune),
                    peg_in_type: Some(MintOnlyAssetSpecPegInType::Pubkey(
                        address_pubkey.to_bytes(),
                    )),
                }),
                block_tx: None,
                pointer: None,
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_spec = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::Rune,
                        pay_to_key: Some(address_pubkey.to_bytes()),
                        ratio: RatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
            // use the spec
            spec: Some(block_tx_spec.to_tuple()),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, None);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_moa_input_asset_invalid() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                    input_asset: Some(InputAsset::Rune),
                    peg_in_type: Some(MintOnlyAssetSpecPegInType::Burn),
                }),
                block_tx: None,
                pointer: None,
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_spec = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: None,
                        ratio: RatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
            // use the spec
            spec: Some(block_tx_spec.to_tuple()),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, Some(Flaw::SpecCriteriaInvalid));

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_moa_peg_in_type_invalid() {
    let (_, address_pubkey) = get_bitcoin_address();

    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                    input_asset: Some(InputAsset::Rune),
                    peg_in_type: Some(MintOnlyAssetSpecPegInType::Pubkey(
                        address_pubkey.to_bytes(),
                    )),
                }),
                block_tx: None,
                pointer: None,
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_spec = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1000)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::Rune,
                        pay_to_key: None,
                        ratio: RatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
            // use the spec
            spec: Some(block_tx_spec.to_tuple()),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, Some(Flaw::SpecCriteriaInvalid));

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_glittr_airdrop() {
    let mut ctx = TestContext::new().await;
    let (user_address, _) = get_bitcoin_address();

    // Create admin keypair for encryption/decryption
    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let admin_secret_key = bitcoin::secp256k1::SecretKey::new(&mut secp256k1::rand::thread_rng());

    // Create the corresponding public key
    let admin_public_key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &admin_secret_key);

    // 1. Admin creates first MOA with commitment
    let first_moa_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: None, // Unlimited supply
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: None,
                        amount_per_mint: U128(1),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: Some(Commitment {
                    public_key: admin_public_key.serialize().to_vec(),
                    args: ArgsCommitment {
                        fixed_string: "GLITTRAIRDROP".to_string(),
                        string: "username".to_string(),
                    },
                }),
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let first_moa_contract = ctx.build_and_mine_message(&first_moa_message).await;

    // 2. User mints first MOA with commitment
    let username = "alice123";
    let commitment_string = format!("GLITTRAIRDROP:{}", username);

    // Encrypt commitment using admin's public key
    let encrypted_commitment = encrypt_message(&admin_public_key, &commitment_string).unwrap();

    let mint_first_moa_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: first_moa_contract.to_tuple(),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: Some(CommitmentMessage {
                    public_key: admin_public_key.serialize().to_vec(),
                    args: encrypted_commitment,
                }),
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    let height = ctx.core.height();

    let txid = ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_first_moa_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 1000],
        outputs: 2,
        p2tr: false,
        recipient: None,
    });

    ctx.core.mine_blocks(1);

    let first_mint_tx = BlockTx {
        block: height + 1,
        tx: 1,
    };

    // 3. Admin creates bloom filter with user's txid:vout
    let mut filter = GrowableBloom::new(0.05, 1000);
    let key = format!("{}:{}", txid, 1); // Using tx:vout as key
    filter.insert(key.clone());
    println!("contains {} {} ", filter.contains(key.clone()), key);
    let compressed_filter = bloom_filter_to_compressed_vec(filter);

    let mut allocations = HashMap::new();

    allocations.insert(
        U128(100),
        AllocationType::BloomFilter {
            filter: compressed_filter,
            arg: BloomFilterArgType::TxId,
        },
    );

    // 4. Admin creates second MOA with preallocated using bloom filter
    let second_moa_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(100)),
                divisibility: 18,
                live_time: 0,
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    preallocated: Some(Preallocated {
                        allocations,
                        vesting_plan: None,
                    }),
                    free_mint: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let second_moa_contract = ctx.build_and_mine_message(&second_moa_message).await;

    // 5. User mints second MOA using first MOA as proof
    let mint_second_moa_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: second_moa_contract.to_tuple(),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (first_mint_tx.block as usize, 1, 1, Witness::new()), // UTXO containing first MOA
            (first_mint_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(mint_second_moa_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000],
        outputs: 2,
        p2tr: false,
        recipient: Some(user_address),
    });
    ctx.core.mine_blocks(1);

    let second_mint_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify outcomes
    let first_contract_outcome = ctx.get_and_verify_message_outcome(first_moa_contract).await;
    assert!(first_contract_outcome.flaw.is_none());

    let first_mint_outcome = ctx.get_and_verify_message_outcome(first_mint_tx).await;
    assert!(first_mint_outcome.flaw.is_none());

    let second_contract_outcome = ctx
        .get_and_verify_message_outcome(second_moa_contract)
        .await;
    assert!(second_contract_outcome.flaw.is_none());

    let second_mint_outcome = ctx.get_and_verify_message_outcome(second_mint_tx).await;
    println!("{:?}", second_mint_outcome.flaw);
    assert!(second_mint_outcome.flaw.is_none());

    // Verify asset allocations
    let asset_map = ctx.get_asset_map().await;

    // Verify first MOA allocation
    let first_moa_amount = asset_map
        .values()
        .find_map(|list| list.list.get(&first_moa_contract.to_str()))
        .expect("First MOA should exist");
    assert_eq!(*first_moa_amount, 1);

    // Verify second MOA allocation
    let second_moa_amount = asset_map
        .values()
        .find_map(|list| list.list.get(&second_moa_contract.to_str()))
        .expect("Second MOA should exist");
    assert_eq!(*second_moa_amount, 100);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_contract_ticker() {
    let mut ctx = TestContext::new().await;

    let ticker = "POHON_PISANG".to_string();

    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: Some(ticker.clone()),
                supply_cap: None,
                divisibility: 18,
                live_time: 0,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: None,
                        ratio: RatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                end_time: None,
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx = ctx.build_and_mine_message(&contract_message).await;

    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: Some(ticker.clone()),
                supply_cap: None,
                divisibility: 18,
                live_time: 0,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: None,
                        ratio: RatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                end_time: None,
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_error = ctx.build_and_mine_message(&contract_message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(block_tx).await;

    let block_tx_by_ticker: Result<BlockTxTuple, DatabaseError> = ctx
        .indexer
        .lock()
        .await
        .database
        .lock()
        .await
        .get(TICKER_TO_BLOCK_TX_PREFIX, &ticker);

    assert_eq!(block_tx_by_ticker.unwrap(), block_tx.to_tuple());

    let message_error = ctx.get_and_verify_message_outcome(block_tx_error).await;
    assert_eq!(message_error.flaw, Some(Flaw::TickerAlreadyExist));

    ctx.drop().await;
}

#[tokio::test]
async fn test_contract_creation_and_mint() {
    let mut ctx = TestContext::new().await;
    let (owner_address, _) = get_bitcoin_address();

    // Create two MOA tokens to be used in the liquidity pool
    let token1_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1_000_000)),
                divisibility: 18,
                live_time: 0,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1_000_000)),
                        amount_per_mint: U128(100_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let token2_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(U128(1_000_000)),
                divisibility: 18,
                live_time: 0,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1_000_000)),
                        amount_per_mint: U128(50_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let token1_contract = ctx.build_and_mine_message(&token1_message).await;
    let token2_contract = ctx.build_and_mine_message(&token2_message).await;

    // Mint both tokens
    let mint_token1_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(token1_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    let mint_token2_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(token2_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    let token1_mint_tx = ctx.build_and_mine_message(&mint_token1_message).await;
    let token2_mint_tx = ctx.build_and_mine_message(&mint_token2_message).await;

    // Create LP token contract
    let lp_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Mba(MintBurnAssetContract {
                ticker: None,
                supply_cap: None, // No supply cap for LP tokens
                divisibility: 18,
                live_time: 0,
                mint_mechanism: MBAMintMechanisms {
                    preallocated: None,
                    free_mint: None,
                    purchase: None,
                    collateralized: Some(Collateralized {
                        input_assets: vec![
                            InputAsset::GlittrAsset(token1_contract.to_tuple()),
                            InputAsset::GlittrAsset(token2_contract.to_tuple()),
                        ],
                        _mutable_assets: false,
                        mint_structure: MintStructure::Proportional(ProportionalType {
                            ratio_model: RatioModel::ConstantProduct,
                            inital_mint_pointer_to_key: None,
                        }),
                    }),
                },
                burn_mechanism: BurnMechanisms {
                    return_collateral: Some(ReturnCollateral {
                        fee: None,
                        oracle_setting: None,
                    }),
                },
                swap_mechanism: SwapMechanisms { fee: None },
            }),
        }),
        transfer: None,
        contract_call: Some(ContractCall {
            contract: None,
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(1),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
            }),
        }),
    };

    // Broadcast lp creation and minting
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (token1_mint_tx.block as usize, 1, 1, Witness::new()),
            (token2_mint_tx.block as usize, 1, 1, Witness::new()),
            (token2_mint_tx.block as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(lp_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000],
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let contract_create_and_mint_lp_block_tx = BlockTx {
        block: ctx.core.height(),
        tx: 1,
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify initial setup
    let mint_lp_outcome = ctx
        .get_and_verify_message_outcome(contract_create_and_mint_lp_block_tx)
        .await;
    assert!(mint_lp_outcome.flaw.is_none(), "{:?}", mint_lp_outcome.flaw);

    let asset_lists = ctx.get_asset_map().await;

    let lp_minted_amount = asset_lists
        .values()
        .find_map(|list| {
            list.list
                .get(&contract_create_and_mint_lp_block_tx.to_string())
        })
        .expect("Minted asset should exist");

    // Minted LP: https://github.com/Uniswap/v2-core/blob/master/contracts/UniswapV2Pair.sol#L120-L123
    assert_eq!(*lp_minted_amount, 70710);

    ctx.drop().await;
}

