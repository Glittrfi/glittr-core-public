mod utils;
use utils::{get_bitcoin_address, start_indexer, TestContext};

use bitcoin::Witness;
use glittr::{
    database::{DatabaseError, ASSET_CONTRACT_DATA_PREFIX},
    message::{
        CallType, ContractCall, ContractCreation, ContractType, MintBurnOption, OpReturnMessage,
    },
    mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract},
    transaction_shared::{AllocationType, FreeMint, Preallocated, VestingPlan},
    varint::Varint,
    varuint::Varuint,
    AssetContractData, Flaw,
};
use mockcore::TransactionTemplate;
use std::{collections::HashMap, sync::Arc};

#[tokio::test]
async fn test_integration_freemint() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(1000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(1000)),
                        amount_per_mint: Varuint(10),
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
    ctx.verify_last_block(block_tx.block.0).await;
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
                supply_cap: Some(Varuint(1000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(1000)),
                        amount_per_mint: Varuint(10),
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
                    pointer: Some(Varuint(1)),
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
                supply_cap: Some(Varuint(50)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(50)),
                        amount_per_mint: Varuint(50),
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
                pointer: Some(Varuint(1)),
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
                pointer: Some(Varuint(0)),
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
                supply_cap: Some(Varuint(1000)),
                divisibility: 18,
                live_time: Varint(5),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(1000)),
                        amount_per_mint: Varuint(50),
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
                pointer: Some(Varuint(1)),
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
                pointer: Some(Varuint(1)),
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

    let mut allocations: HashMap<Varuint<u128>, AllocationType> = HashMap::new();
    allocations.insert(
        Varuint(100),
        AllocationType::VecPubkey(vec![
            pubkey_1.to_bytes(),
            pubkey_2.to_bytes(),
            pubkey_3.to_bytes(),
            pubkey_4.to_bytes(),
        ]),
    );
    allocations.insert(
        Varuint(300),
        AllocationType::VecPubkey(vec![pubkey_reserve.to_bytes()]),
    );

    let vesting_plan = VestingPlan::Scheduled(vec![
        ((Varuint(1), Varuint(4)), Varint(-4)),
        ((Varuint(1), Varuint(4)), Varint(-2)),
        ((Varuint(1), Varuint(4)), Varint(-3)),
        ((Varuint(1), Varuint(4)), Varint(-1)),
    ]);

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(1000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    preallocated: Some(Preallocated {
                        // total 400 + 300 = 700
                        allocations,
                        vesting_plan: Some(vesting_plan),
                    }),
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(300)),
                        amount_per_mint: Varuint(1),
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
                pointer: Some(Varuint(1)),
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
                supply_cap: Some(Varuint(1000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(1000)),
                        amount_per_mint: Varuint(50),
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
                pointer: Some(Varuint(0)),
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
