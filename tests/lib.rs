use bitcoin::{
    hashes::{sha256, Hash},
    key::{rand, Keypair, Secp256k1},
    secp256k1::{self, Message, SecretKey},
    Address, OutPoint, PrivateKey, PublicKey, ScriptBuf, Transaction, Witness, XOnlyPublicKey,
};
use bitcoincore_rpc::{Auth, Client, RpcApi};
use mockcore::{Handle, TransactionTemplate};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{sync::Mutex, task::JoinHandle, time::sleep};

use glittr::{
    asset_contract::{
        AssetContract, DistributionSchemes, FreeMint, InputAsset, OracleSetting, Preallocated,
        PurchaseBurnSwap, SimpleAsset, TransferRatioType, TransferScheme, VestingPlan,
    },
    database::{
        Database, DatabaseError, ASSET_CONTRACT_DATA_PREFIX, ASSET_LIST_PREFIX,
        INDEXER_LAST_BLOCK_PREFIX, MESSAGE_PREFIX,
    },
    message::{
        CallType, ContractType, MintOption, OpReturnMessage, OracleMessage, OracleMessageSigned,
        TxType,
    },
    AssetContractData, AssetList, BlockTx, Flaw, Indexer, MessageDataOutcome, Outpoint, Pubkey,
    U128,
};

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

struct TestContext {
    indexer: Arc<Mutex<Indexer>>,
    core: Handle,
    _tempdir: TempDir,
}

impl TestContext {
    async fn new() -> Self {
        // env_logger::init();
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
            output_values: &[0, 100],
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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: Some(U128(1000)),
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(10),
                    }),
                    preallocated: None,
                    purchase: None,
                },
            }),
        },
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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: None,
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBTC,
                        transfer_scheme: TransferScheme::Burn,
                        transfer_ratio_type: TransferRatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
            }),
        },
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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: None,
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBTC,
                        transfer_scheme: TransferScheme::Burn,
                        transfer_ratio_type: TransferRatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
            }),
        },
    };

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let (minter_address, _) = get_bitcoin_address();

    let mint_message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: contract_id.to_tuple(),
            call_type: CallType::Mint(MintOption {
                pointer: 0,
                oracle_message: None,
            }),
        },
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

    let asset_list: Result<Vec<(String, AssetList)>, DatabaseError> = ctx
        .indexer
        .lock()
        .await
        .database
        .lock()
        .await
        .expensive_find_by_prefix(ASSET_LIST_PREFIX);
    let asset_lists = asset_list.expect("asset list should exist");

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_str()).unwrap();

    assert!(out_value == (bitcoin_value - fee - dust) as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                Outpoint {
                    txid: tx.compute_txid().to_string(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

// test_raw_btc_to_glittr_asset_purchase e.g. raw btc to wbtc by purchase
#[tokio::test]
async fn test_raw_btc_to_glittr_asset_purchase() {
    let mut ctx = TestContext::new().await;

    let (contract_treasury, _) = get_bitcoin_address();
    let contract_message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: None,
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBTC,
                        transfer_scheme: TransferScheme::Purchase(contract_treasury.to_string()),
                        transfer_ratio_type: TransferRatioType::Fixed { ratio: (1, 1) },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
            }),
        },
    };

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let mint_message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: contract_id.to_tuple(),
            call_type: CallType::Mint(MintOption {
                pointer: 0,
                oracle_message: None,
            }),
        },
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

    let asset_list: Result<Vec<(String, AssetList)>, DatabaseError> = ctx
        .indexer
        .lock()
        .await
        .database
        .lock()
        .await
        .expensive_find_by_prefix(ASSET_LIST_PREFIX);
    let asset_lists = asset_list.expect("asset list should exist");

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_str()).unwrap();

    assert!(out_value == (bitcoin_value - fee) as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                Outpoint {
                    txid: tx.compute_txid().to_string(),
                    vout: 0
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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: None,
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBTC,
                        transfer_scheme: TransferScheme::Burn,
                        transfer_ratio_type: TransferRatioType::Oracle {
                            pubkey: oracle_xonly.0.serialize().to_vec(),
                            setting: OracleSetting {
                                asset_id: Some("btc".to_string()),
                                block_height_slippage: 5,
                            },
                        },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
            }),
        },
    };

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let (minter_address, _) = get_bitcoin_address();

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;
    let dust = 546;
    let oracle_out_value = 1000;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    let input_txid = ctx.core.tx((height - 1) as usize, 0).compute_txid();
    let oracle_message = OracleMessage {
        input_outpoint: OutPoint {
            txid: input_txid,
            vout: 0,
        },
        min_in_value: (bitcoin_value - 1000) as u128,
        out_value: oracle_out_value,
        asset_id: Some("btc".to_string()),
        block_height: height,
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    let mint_message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: contract_id.to_tuple(),
            call_type: CallType::Mint(MintOption {
                pointer: 1,
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
            }),
        },
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

    let asset_list: Result<Vec<(String, AssetList)>, DatabaseError> = ctx
        .indexer
        .lock()
        .await
        .database
        .lock()
        .await
        .expensive_find_by_prefix(ASSET_LIST_PREFIX);
    let asset_lists = asset_list.expect("asset list should exist");

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_str()).unwrap();

    assert!(out_value == oracle_out_value);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                Outpoint {
                    txid: tx.compute_txid().to_string(),
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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: None,
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::Metaprotocol,
                        transfer_scheme: TransferScheme::Burn,
                        transfer_ratio_type: TransferRatioType::Oracle {
                            pubkey: oracle_xonly.0.serialize().to_vec(),
                            setting: OracleSetting {
                                asset_id: Some("rune:840000:3".to_string()),
                                block_height_slippage: 5,
                            },
                        },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
            }),
        },
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
        input_outpoint: OutPoint {
            txid: input_txid,
            vout: 0,
        },
        min_in_value: 0,
        out_value: oracle_out_value,
        asset_id: Some("rune:840000:3".to_string()),
        block_height: height,
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    let mint_message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: contract_id.to_tuple(),
            call_type: CallType::Mint(MintOption {
                pointer: 1,
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
            }),
        },
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

    let asset_list: Result<Vec<(String, AssetList)>, DatabaseError> = ctx
        .indexer
        .lock()
        .await
        .database
        .lock()
        .await
        .expensive_find_by_prefix(ASSET_LIST_PREFIX);
    let asset_lists = asset_list.expect("asset list should exist");

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_str()).unwrap();

    assert!(out_value == oracle_out_value);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                Outpoint {
                    txid: tx.compute_txid().to_string(),
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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: Some(U128(1000)),
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(10),
                    }),
                    preallocated: None,
                    purchase: None,
                },
            }),
        },
    };

    let block_tx = ctx.build_and_mine_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.get_and_verify_message_outcome(block_tx).await;
    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_preallocated() {
    // TODO: Implement using TestContext
}

#[tokio::test]
async fn test_integration_mint_freemint() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: Some(U128(1000)),
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(10),
                    }),
                    preallocated: None,
                    purchase: None,
                },
            }),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let total_mints = 10;

    for _ in 0..total_mints {
        let message = OpReturnMessage {
            tx_type: TxType::ContractCall {
                contract: block_tx_contract.to_tuple(),
                call_type: CallType::Mint(MintOption {
                    pointer: 0,
                    oracle_message: None,
                }),
            },
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

    let asset_list: Result<Vec<(String, AssetList)>, DatabaseError> = ctx
        .indexer
        .lock()
        .await
        .database
        .lock()
        .await
        .expensive_find_by_prefix(ASSET_LIST_PREFIX);
    let asset_lists = asset_list.expect("asset list should exist");

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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: Some(U128(50)),
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(50)),
                        amount_per_mint: U128(50),
                    }),
                    preallocated: None,
                    purchase: None,
                },
            }),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    // first mint
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption {
                pointer: 0,
                oracle_message: None,
            }),
        },
    };
    ctx.build_and_mine_message(&message).await;

    // second mint should be execeeded the supply cap
    // and the total minted should be still 1
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption {
                pointer: 0,
                oracle_message: None,
            }),
        },
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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: Some(U128(1000)),
                    divisibility: 18,
                    live_time: 5,
                },
                distribution_schemes: DistributionSchemes {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(1000)),
                        amount_per_mint: U128(50),
                    }),
                    preallocated: None,
                    purchase: None,
                },
            }),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    // first mint not reach the live time
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption {
                pointer: 0,
                oracle_message: None,
            }),
        },
    };
    let notreached_block_tx = ctx.build_and_mine_message(&message).await;
    println!("Not reached livetime block tx: {:?}", notreached_block_tx);

    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption {
                pointer: 0,
                oracle_message: None,
            }),
        },
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
    assert_eq!(outcome.flaw.unwrap(), Flaw::LiveTimeNotReached);

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

    let mut allocations: HashMap<U128, Vec<Pubkey>> = HashMap::new();
    allocations.insert(
        U128(100),
        vec![
            pubkey_1.to_bytes(),
            pubkey_2.to_bytes(),
            pubkey_3.to_bytes(),
            pubkey_4.to_bytes(),
        ],
    );
    allocations.insert(U128(300), vec![pubkey_reserve.to_bytes()]);

    let vesting_plan =
        VestingPlan::Scheduled(vec![((1, 4), -1), ((1, 4), -2), ((1, 4), -3), ((1, 4), -4)]);

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract {
                asset: SimpleAsset {
                    supply_cap: Some(U128(1000)),
                    divisibility: 18,
                    live_time: 0,
                },
                distribution_schemes: DistributionSchemes {
                    preallocated: Some(Preallocated {
                        // total 400 + 300 = 700
                        allocations,
                        vesting_plan,
                    }),
                    free_mint: Some(FreeMint {
                        supply_cap: Some(U128(300)),
                        amount_per_mint: U128(1),
                    }),
                    purchase: None,
                },
            }),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;


    let mint_message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption {
                pointer: 1,
                oracle_message: None,
            }),
        },
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
        inputs: &[((height+1) as usize, 1, 0, witness)],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546],
        outputs: 2,
        p2tr: false,
        recipient: Some(address_1)
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

    let asset_list: Result<Vec<(String, AssetList)>, DatabaseError> = ctx
        .indexer
        .lock()
        .await
        .database
        .lock()
        .await
        .expensive_find_by_prefix(ASSET_LIST_PREFIX);
    let asset_lists = asset_list.expect("asset list should exist");

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    assert_eq!(data_free_mint.minted_supply, 10 + 50);
    assert_eq!(asset_lists.len() as u32, total_mints as u32 + 1);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_transfer() {
    // TODO: Implement using TestContext
}

#[tokio::test]
async fn test_integration_burn() {
    // TODO: Implement using TestContext
}

#[tokio::test]
async fn test_integration_swap() {
    // TODO: Implement using TestContext
}
