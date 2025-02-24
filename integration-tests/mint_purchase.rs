mod utils;
use utils::{get_bitcoin_address, start_indexer, TestContext};

use bitcoin::Witness;
use glittr::{
    message::{
        CallType, ContractCall, ContractCreation, ContractType, MintBurnOption, OpReturnMessage,
        Transfer, TxTypeTransfer,
    }, mint_burn_asset::{BurnMechanisms, MBAMintMechanisms, MintBurnAssetContract, SwapMechanisms}, mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract}, transaction_shared::{FreeMint, InputAsset, PurchaseBurnSwap, RatioType}, varint::Varint, varuint::Varuint
};
use mockcore::TransactionTemplate;
use std::sync::Arc;

#[tokio::test]
async fn test_integration_glittr_asset_mint_purchase() {
    let mut ctx = TestContext::new().await;

    // Create first contract with free mint
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
                        amount_per_mint: Varuint(100),
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

    let first_mint_tx = ctx.build_and_mine_message(&mint_message).await;

    // Create second contract that uses first as input asset
    let (treasury_address, treasury_pub_key) = get_bitcoin_address();
    let second_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(500)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::GlittrAsset(first_contract.to_tuple()),
                        pay_to_key: Some(treasury_pub_key.to_bytes()),
                        ratio: RatioType::Fixed {
                            ratio: (Varuint(2), Varuint(1)),
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

    let second_contract = ctx.build_and_mine_message(&second_message).await;

    // Mint second contract using first contract as input
    let second_mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(second_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(Varuint(1)),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: Some(Transfer {
            transfers: [TxTypeTransfer {
                asset: first_contract.to_tuple(),
                output: Varuint(1),
                amount: Varuint(100),
            }]
            .to_vec(),
        }),
        contract_creation: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (first_mint_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO contain assets
            (first_mint_tx.block.0 as usize, 0, 0, Witness::new()),
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
async fn test_integration_glittr_mba_mint_purchase() {
    let mut ctx = TestContext::new().await;

    // Create first contract with free mint
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Mba(MintBurnAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(1000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MBAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(1000)),
                        amount_per_mint: Varuint(100),
                    }),
                    preallocated: None,
                    purchase: None,
                    collateralized: None,
                },
                commitment: None,
                burn_mechanism: BurnMechanisms {
                    return_collateral: None,
                },
                swap_mechanism: SwapMechanisms { fee: None },
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

    let first_mint_tx = ctx.build_and_mine_message(&mint_message).await;

    // Create second contract that uses first as input asset
    let (treasury_address, treasury_pub_key) = get_bitcoin_address();
    let second_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Mba(MintBurnAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(500)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MBAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::GlittrAsset(first_contract.to_tuple()),
                        pay_to_key: Some(treasury_pub_key.to_bytes()),
                        ratio: RatioType::Fixed { ratio: (Varuint(2), Varuint(1)) },
                    }),
                    preallocated: None,
                    free_mint: None,
                    collateralized: None,
                },
                commitment: None,
                burn_mechanism: BurnMechanisms { return_collateral: None},
                swap_mechanism: SwapMechanisms {fee: None}
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
                pointer: Some(Varuint(1)),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: Some(Transfer {
            transfers: [TxTypeTransfer {
                asset: first_contract.to_tuple(),
                output: Varuint(1),
                amount: Varuint(100),
            }]
            .to_vec(),
        }),
        contract_creation: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (first_mint_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO contain assets
            (first_mint_tx.block.0 as usize, 0, 0, Witness::new()),
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

    assert_eq!(asset_lists[0].1.list.get(&first_contract.to_string()), Some(&100));
    assert_eq!(asset_lists[0].1.list.get(&second_contract.to_string()), Some(&50));

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    ctx.drop().await;
}