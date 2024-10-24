use std::{sync::Arc, time::Duration};

use bitcoin::Witness;
use mockcore::TransactionTemplate;
use tempfile::TempDir;

use tokio::{sync::Mutex, task::JoinHandle, time::sleep};

use glittr::{
    asset_contract::{AssetContract, AssetContractFreeMint},
    database::{Database, DatabaseError, INDEXER_LAST_BLOCK_PREFIX, MESSAGE_PREFIX},
    message::{ContractType, OpReturnMessage, TxType},
    BlockTx, Indexer, MessageDataOutcome,
};

pub async fn spawn_test_indexer(
    db_path: String,
    rpc_url: String,
) -> (Arc<Mutex<Indexer>>, JoinHandle<()>) {
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

    let indexer_clone = Arc::clone(&indexer);
    let handle = tokio::spawn(async move {
        println!("running indexer");
        indexer
            .lock()
            .await
            .run_indexer()
            .await
            .expect("Run indexer");
    });

    sleep(Duration::from_millis(100)).await; // let the indexer run first

    (indexer_clone, handle)
}

#[tokio::test]
pub async fn test_integration_broadcast_op_return_message_success() {
    let tempdir = TempDir::new().unwrap();

    let core = tokio::task::spawn_blocking(|| mockcore::spawn())
        .await
        .expect("Task panicked");

    core.mine_blocks(2);
    let height = core.height();

    let dummy_message = OpReturnMessage {
        tx_type: TxType::ContractCreation {
            contract_type: ContractType::Asset(AssetContract::FreeMint(AssetContractFreeMint {
                supply_cap: Some(1000),
                amount_per_mint: 10,
                divisibility: 18,
                live_time: 0,
            })),
        },
    };
    core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(dummy_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 100],
        outputs: 2,
        p2tr: false,
        recipient: None,
    });

    core.mine_blocks(1);

    let (indexer, indexer_run_handle) =
        spawn_test_indexer(tempdir.path().to_str().unwrap().to_string(), core.url()).await;

    indexer_run_handle.abort();

    let last_block: u64 = indexer
        .lock()
        .await
        .database
        .lock()
        .await
        .get(INDEXER_LAST_BLOCK_PREFIX, "")
        .unwrap();

    assert_eq!(last_block, 3);

    let message: Result<MessageDataOutcome, DatabaseError> =
        indexer.lock().await.database.lock().await.get(
            MESSAGE_PREFIX,
            BlockTx { block: 3, tx: 1 }.to_string().as_str(),
        );

    assert!(message.is_ok());

    tokio::task::spawn_blocking(|| drop(core))
        .await
        .expect("Drop failed");
}

#[test]
pub fn test_integration_freemint() {}

#[test]
pub fn test_integration_preallocated() {}

#[test]
pub fn test_integration_mint() {}

#[test]
pub fn test_integration_transfer() {}

#[test]
pub fn test_integration_burn() {}

#[test]
pub fn test_integration_swap() {}
