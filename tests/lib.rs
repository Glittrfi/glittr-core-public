use bitcoin::Witness;
use mockcore::{Handle, TransactionTemplate};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{sync::Mutex, task::JoinHandle, time::sleep};

use glittr::{
    asset_contract::{
        AssetContract, AssetContractFreeMint, InputAsset, TransferRatioType, TransferScheme,
    },
    database::{
        Database, DatabaseError, ASSET_CONTRACT_DATA_PREFIX, ASSET_LIST_PREFIX,
        INDEXER_LAST_BLOCK_PREFIX, MESSAGE_PREFIX,
    },
    message::{CallType, ContractType, MintOption, OpReturnMessage, TxType, TxTypeTransfer},
    AssetContractData, AssetList, BlockTx, Flaw, Indexer, MessageDataOutcome, Outpoint,
};

// Test utilities
struct TestContext {
    indexer: Arc<Mutex<Indexer>>,
    core: Handle,
    _tempdir: TempDir,
}

impl TestContext {
    async fn new() -> Self {
        // env_logger::init();
        let tempdir = TempDir::new().unwrap();
        let core = tokio::task::spawn_blocking(|| mockcore::spawn())
            .await
            .expect("Task panicked");

        // Initial setup
        core.mine_blocks(2);

        let indexer =
            spawn_test_indexer(tempdir.path().to_str().unwrap().to_string(), core.url()).await;

        Self {
            indexer,
            core,
            _tempdir: tempdir,
        }
    }

    async fn build_and_mine_message(&mut self, message: OpReturnMessage) -> BlockTx {
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

    async fn verify_message(&self, block_tx: BlockTx) -> MessageDataOutcome {
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

    fn verify_asset_output(
        &self,
        asset_lists: &HashMap<String, AssetList>,
        block_tx_contract: &BlockTx,
        outpoint: &Outpoint,
        expected_value: u32,
    ) {
        let asset_output = asset_lists
            .get(&outpoint.to_string())
            .expect(&format!("Asset output {} should exist", outpoint.vout));

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

async fn spawn_test_indexer(db_path: String, rpc_url: String) -> Arc<Mutex<Indexer>> {
    let database = Arc::new(Mutex::new(Database::new(db_path)));

    let indexer = Arc::new(Mutex::new(
        Indexer::new(
            Arc::clone(&database),
            rpc_url,
            "".to_string(),
            "".to_string(),
        )
        .await
        .unwrap(),
    ));

    indexer
}

#[tokio::test]
async fn test_integration_broadcast_op_return_message_success() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(1000),
                amount_per_mint: 10,
                divisibility: 18,
                live_time: 0,
            })),
        },
    };

    let block_tx = ctx.build_and_mine_message(message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.verify_message(block_tx).await;
    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_purchaseburnswap() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::PurchaseBurnSwap {
                input_asset: InputAsset::RawBTC,
                transfer_scheme: TransferScheme::Burn,
                transfer_ratio_type: TransferRatioType::Fixed { ratio: (1, 1) },
            }),
        },
    };

    let block_tx = ctx.build_and_mine_message(message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.verify_message(block_tx).await;
    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_freemint() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(1000),
                amount_per_mint: 10,
                divisibility: 18,
                live_time: 0,
            })),
        },
    };

    let block_tx = ctx.build_and_mine_message(message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.verify_message(block_tx).await;
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
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(1000),
                amount_per_mint: 10,
                divisibility: 18,
                live_time: 0,
            })),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(message).await;

    let total_mints = 10;

    for _ in 0..total_mints {
        let message = OpReturnMessage {
            tx_type: TxType::ContractCall {
                contract: block_tx_contract.to_tuple(),
                call_type: CallType::Mint(MintOption { pointer: 1 }),
            },
        };
        ctx.build_and_mine_message(message).await;
    }

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_contract_data: Result<AssetContractData, DatabaseError> =
        ctx.indexer.lock().await.database.lock().await.get(
            ASSET_CONTRACT_DATA_PREFIX,
            block_tx_contract.to_string().as_str(),
        );
    let data_free_mint = match asset_contract_data.expect("Free mint data should exist") {
        AssetContractData::FreeMint(free_mint) => free_mint,
    };

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

    assert_eq!(data_free_mint.minted, total_mints);
    assert_eq!(asset_lists.len() as u32, total_mints);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_mint_freemint_supply_cap_exceeded() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(50),
                amount_per_mint: 50,
                divisibility: 18,
                live_time: 0,
            })),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(message).await;

    // first mint
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 1 }),
        },
    };
    ctx.build_and_mine_message(message).await;

    // second mint should be execeeded the supply cap
    // and the total minted should be still 1
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 1 }),
        },
    };
    let overflow_block_tx = ctx.build_and_mine_message(message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_contract_data: Result<AssetContractData, DatabaseError> =
        ctx.indexer.lock().await.database.lock().await.get(
            ASSET_CONTRACT_DATA_PREFIX,
            block_tx_contract.to_string().as_str(),
        );
    let data_free_mint = match asset_contract_data.expect("Free mint data should exist") {
        AssetContractData::FreeMint(free_mint) => free_mint,
    };

    assert_eq!(data_free_mint.minted, 1);

    let outcome = ctx.verify_message(overflow_block_tx).await;
    assert_eq!(outcome.flaw.unwrap(), Flaw::SupplyCapExceeded);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_mint_freemint_livetime_notreached() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(1000),
                amount_per_mint: 50,
                divisibility: 18,
                live_time: 5,
            })),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(message).await;

    // first mint not reach the live time
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 1 }),
        },
    };
    let notreached_block_tx = ctx.build_and_mine_message(message).await;
    println!("Not reached livetime block tx: {:?}", notreached_block_tx);

    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 1 }),
        },
    };
    ctx.build_and_mine_message(message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_contract_data: Result<AssetContractData, DatabaseError> =
        ctx.indexer.lock().await.database.lock().await.get(
            ASSET_CONTRACT_DATA_PREFIX,
            block_tx_contract.to_string().as_str(),
        );
    let data_free_mint = match asset_contract_data.expect("Free mint data should exist") {
        AssetContractData::FreeMint(free_mint) => free_mint,
    };

    let outcome = ctx.verify_message(notreached_block_tx).await;
    assert_eq!(outcome.flaw.unwrap(), Flaw::LiveTimeNotReached);

    assert_eq!(data_free_mint.minted, 1);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_mint_freemint_invalidpointer() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(1000),
                amount_per_mint: 50,
                divisibility: 18,
                live_time: 0,
            })),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(message).await;

    // set pointer to index 0 (op_return output), it should be error
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 0 }),
        },
    };
    let invalid_pointer_block_tx = ctx.build_and_mine_message(message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let outcome = ctx.verify_message(invalid_pointer_block_tx).await;
    assert_eq!(outcome.flaw.unwrap(), Flaw::InvalidPointer);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_transfer_normal() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(100_000),
                amount_per_mint: 20_000,
                divisibility: 18,
                live_time: 0,
            })),
        },
    };
    let block_tx_contract = ctx.build_and_mine_message(message).await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 1 }),
        },
    };
    let mint_block_tx = ctx.build_and_mine_message(message).await;

    let message = OpReturnMessage {
        tx_type: TxType::Transfer(
            [
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 1,
                    amount: 10_000,
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 2,
                    amount: 2_000,
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 2,
                    amount: 500,
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 3,
                    amount: 8_000,
                },
            ]
            .to_vec(),
        ),
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
        &Outpoint {
            txid: new_output_txid.to_string(),
            vout: 1,
        },
        10_000,
    );

    ctx.verify_asset_output(
        &asset_map,
        &block_tx_contract,
        &Outpoint {
            txid: new_output_txid.to_string(),
            vout: 2,
        },
        2500,
    );

    // this expected 7500 not 8_000 because the remaining asset or unallocated < amount
    // transferred value picks MIN(remainder, amount)
    ctx.verify_asset_output(
        &asset_map,
        &block_tx_contract,
        &Outpoint {
            txid: new_output_txid.to_string(),
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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(100_000),
                amount_per_mint: 20_000,
                divisibility: 18,
                live_time: 0,
            })),
        },
    };
    let block_tx_contract = ctx.build_and_mine_message(message).await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 1 }),
        },
    };
    let mint_block_tx = ctx.build_and_mine_message(message).await;

    let message = OpReturnMessage {
        tx_type: TxType::Transfer(
            [
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 1,
                    amount: 10_000,
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: 2, // will overflow the output
                    amount: 2_000,
                },
            ]
            .to_vec(),
        ),
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
        .verify_message(BlockTx {
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
        &Outpoint {
            txid: new_output_txid.to_string(),
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
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(100_000),
                amount_per_mint: 20_000,
                divisibility: 18,
                live_time: 0,
            })),
        },
    };
    let block_tx_contract = ctx.build_and_mine_message(message).await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 1 }),
        },
    };
    let mint_block_tx = ctx.build_and_mine_message(message).await;

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
        &Outpoint {
            txid: new_output_txid.to_string(),
            vout: 0,
        },
        20_000,
    );

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_burn() {
    // TODO: Implement using TestContext
}

#[tokio::test]
async fn test_integration_swap() {
    // TODO: Implement using TestContext
}
