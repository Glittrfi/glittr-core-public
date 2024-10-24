use bitcoin::{
    key::Secp256k1,
    secp256k1::{self, SecretKey},
    Address, PrivateKey, PublicKey, ScriptBuf, Transaction, Witness,
};
use bitcoincore_rpc::{Auth, Client, RpcApi};
use mockcore::{Handle, TransactionTemplate};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{sync::Mutex, task::JoinHandle, time::sleep};

use glittr::{
    asset_contract::{
        AssetContract, AssetContractPurchaseBurnSwap, InputAsset, TransferRatioType, TransferScheme,
    },
    database::{Database, DatabaseError, INDEXER_LAST_BLOCK_PREFIX, MESSAGE_PREFIX},
    message::{CallType, ContractType, OpReturnMessage, TxType},
    BlockTx, Indexer, MessageDataOutcome, Updater,
};

// Test utilities
pub fn get_bitcoin_address() -> Address {
    let secp = Secp256k1::new();

    let secret_key = SecretKey::new(&mut secp256k1::rand::thread_rng());

    let private_key = PrivateKey {
        compressed: true,
        network: bitcoin::NetworkKind::Test,
        inner: secret_key,
    };

    // Create the corresponding public key
    let public_key = PublicKey::from_private_key(&secp, &private_key);
    return Address::from_script(
        &ScriptBuf::new_p2wpkh(&public_key.wpubkey_hash().unwrap()),
        bitcoin::Network::Regtest,
    )
    .unwrap();
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

    async fn get_transaction_from_block_tx(&self, block_tx: BlockTx) -> Result<Transaction, ()> {
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

    async fn build_and_mine_contract_message(&mut self, message: &OpReturnMessage) -> BlockTx {
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

    async fn get_message_outcome(&self, block_tx: BlockTx) -> MessageDataOutcome {
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
            contract_type: ContractType::Asset(AssetContract::FreeMint {
                supply_cap: Some(1000),
                amount_per_mint: 10,
                divisibility: 18,
                live_time: 0,
            }),
        },
    };

    let block_tx = ctx.build_and_mine_contract_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.get_message_outcome(block_tx).await;
    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_purchaseburnswap() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::PurchaseBurnSwap(
                AssetContractPurchaseBurnSwap {
                    input_asset: InputAsset::RawBTC,
                    transfer_scheme: TransferScheme::Burn,
                    transfer_ratio_type: TransferRatioType::Fixed { ratio: (1, 1) },
                },
            )),
        },
    };

    let block_tx = ctx.build_and_mine_contract_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.get_message_outcome(block_tx).await;
    ctx.drop().await;
}

#[tokio::test]
async fn test_raw_btc_to_wbtc_burn() {
    let mut ctx = TestContext::new().await;

    let contract_message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::PurchaseBurnSwap(
                AssetContractPurchaseBurnSwap {
                    input_asset: InputAsset::RawBTC,
                    transfer_scheme: TransferScheme::Burn,
                    transfer_ratio_type: TransferRatioType::Fixed { ratio: (1, 1) },
                },
            )),
        },
    };

    let contract_init_block_tx = ctx.build_and_mine_contract_message(&contract_message).await;

    let minter_address = get_bitcoin_address();

    let mint_message = OpReturnMessage {
        tx_type: TxType::ContractCall {
            contract: contract_init_block_tx.to_tuple(),
            call_type: CallType::Mint,
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
    ctx.get_message_outcome(contract_init_block_tx).await;
    let mint_outcome = ctx.get_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none());

    let tx = ctx
        .get_transaction_from_block_tx(mint_block_tx)
        .await
        .unwrap();
    let pbs = if let TxType::ContractCreation {
        contract_type: ContractType::Asset(AssetContract::PurchaseBurnSwap(pbs)),
    } = contract_message.tx_type.clone()
    {
        Some(pbs.clone())
    } else {
        None
    };

    let mint_outcome = Updater::get_mint_purchase_burn_swap(pbs.unwrap(), &tx)
        .await
        .unwrap();
    assert!(mint_outcome.out_value == (bitcoin_value - fee - dust) as u128);
    assert!(mint_outcome.txout == 0);

    ctx.drop().await;
}

#[tokio::test]
async fn test_raw_btc_to_wusd() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::PurchaseBurnSwap(
                AssetContractPurchaseBurnSwap {
                    input_asset: InputAsset::RawBTC,
                    transfer_scheme: TransferScheme::Burn,
                    transfer_ratio_type: TransferRatioType::Oracle {
                        pubkey: Vec::new(),
                        message: "".to_string(),
                    },
                },
            )),
        },
    };

    let block_tx = ctx.build_and_mine_contract_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.get_message_outcome(block_tx).await;
    ctx.drop().await;
}

#[tokio::test]
async fn test_rune_to_wrune() {
    let mut ctx = TestContext::new().await;
    let rune_location: BlockTx = BlockTx { block: 1, tx: 1 };
    let contract_treasury = get_bitcoin_address();

    let message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::PurchaseBurnSwap(
                AssetContractPurchaseBurnSwap {
                    input_asset: InputAsset::Rune(rune_location.to_tuple()),
                    transfer_scheme: TransferScheme::Purchase(contract_treasury.to_string()),
                    transfer_ratio_type: TransferRatioType::Oracle {
                        pubkey: Vec::new(),
                        message: "".to_string(),
                    },
                },
            )),
        },
    };

    let block_tx = ctx.build_and_mine_contract_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block).await;
    ctx.get_message_outcome(block_tx).await;
    ctx.drop().await;
}

// Template for additional tests
#[tokio::test]
async fn test_integration_freemint() {
    // TODO: Implement using TestContext
}

#[tokio::test]
async fn test_integration_preallocated() {
    // TODO: Implement using TestContext
}

#[tokio::test]
async fn test_integration_mint() {
    // TODO: Implement using TestContext
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
