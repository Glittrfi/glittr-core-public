mod utils;
use utils::{start_indexer, TestContext};

use bitcoin::{OutPoint, Witness};
use glittr::{
    message::{
        CallType, ContractCall, ContractCreation, ContractType, MintBurnOption, OpReturnMessage,
        Transfer, TxTypeTransfer,
    },
    mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract},
    transaction_shared::FreeMint,
    varint::Varint,
    varuint::Varuint,
    BlockTx, Flaw,
};
use mockcore::TransactionTemplate;
use std::sync::Arc;

#[tokio::test]
async fn test_integration_transfer_normal() {
    let mut ctx = TestContext::new().await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(100_000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(100_000)),
                        amount_per_mint: Varuint(20_000),
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
    let mint_block_tx = ctx.build_and_mine_message(&message).await;
    let message = OpReturnMessage {
        transfer: Some(Transfer {
            transfers: [
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: Varuint(1),
                    amount: Varuint(10_000),
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: Varuint(2),
                    amount: Varuint(2_000),
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: Varuint(2),
                    amount: Varuint(500),
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: Varuint(3),
                    amount: Varuint(8_000),
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
            (mint_block_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO contain assets
            (mint_block_tx.block.0 as usize, 0, 0, Witness::new()),
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
                supply_cap: Some(Varuint(100_000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(100_000)),
                        amount_per_mint: Varuint(20_000),
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
    let mint_block_tx = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        transfer: Some(Transfer {
            transfers: [
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: Varuint(1),
                    amount: Varuint(10_000),
                },
                TxTypeTransfer {
                    asset: block_tx_contract.to_tuple(),
                    output: Varuint(2),
                    amount: Varuint(2_000),
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
            (mint_block_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO contain assets
            (mint_block_tx.block.0 as usize, 0, 0, Witness::new()),
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
            block: Varuint(height),
            tx: Varuint(1),
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
                supply_cap: Some(Varuint(100_000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(100_000)),
                        amount_per_mint: Varuint(20_000),
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
    let mint_block_tx = ctx.build_and_mine_message(&message).await;

    // outputs have 2 outputs
    // - index 0: output target (default fallback) first non_op_return output
    // - index 1: output
    let new_output_txid = ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (mint_block_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO contain assets
            (mint_block_tx.block.0 as usize, 0, 0, Witness::new()),
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
