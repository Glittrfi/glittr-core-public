use bitcoin::Witness;
use mockcore::{Handle, TransactionTemplate};
use std::{sync::Arc, time::Duration};
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
    message::{CallType, ContractType, MintOption, OpReturnMessage, TxType},
    AssetContractData, AssetList, BlockTx, Flaw, Indexer, MessageDataOutcome,
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
                live_time: 100,
            })),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(message).await;

    let total_mints = 10;

    for _ in 0..total_mints {
        let message = OpReturnMessage {
            tx_type: TxType::ContractCall {
                contract: block_tx_contract.to_tuple(),
                call_type: CallType::Mint(MintOption { pointer: 0 }),
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
                live_time: 100,
            })),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(message).await;

    // first mint
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 0 }),
        },
    };
    ctx.build_and_mine_message(message).await;

    // second mint should be execeeded the supply cap
    // and the total minted should be still 1
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 0 }),
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
async fn test_integration_mint_freemint_livetime_exceeded() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(1000),
                amount_per_mint: 50,
                divisibility: 18,
                live_time: 4,
            })),
        },
    };

    let block_tx_contract = ctx.build_and_mine_message(message).await;

    // first mint
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 0 }),
        },
    };
    ctx.build_and_mine_message(message).await;

    // second mint should be execeeded the livetime 
    // and the total minted should be still 1
    let message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: block_tx_contract.to_tuple(),
            call_type: CallType::Mint(MintOption { pointer: 0 }),
        },
    };
    let overflow_block_tx = ctx.build_and_mine_message(message).await;
    println!("Overflow block tx: {:?}", overflow_block_tx);

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
    assert_eq!(outcome.flaw.unwrap(), Flaw::LiveTimeExceeded);

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
