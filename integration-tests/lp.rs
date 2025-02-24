mod utils;
use utils::{get_bitcoin_address, start_indexer, TestContext};

use bitcoin::Witness;
use glittr::{
    message::{
        AssertValues, CallType, ContractCall, ContractCreation, ContractType, MintBurnOption,
        OpReturnMessage, SwapOption, Transfer, TxTypeTransfer,
    },
    mint_burn_asset::{
        BurnMechanisms, Collateralized, MBAMintMechanisms, MintBurnAssetContract, MintStructure,
        ProportionalType, RatioModel, ReturnCollateral, SwapMechanisms,
    },
    mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract},
    transaction_shared::{FreeMint, InputAsset},
    varint::Varint,
    varuint::Varuint,
    BlockTx,
};
use mockcore::TransactionTemplate;
use std::sync::Arc;

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
                supply_cap: Some(Varuint(1_000_000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(1_000_000)),
                        amount_per_mint: Varuint(100_000),
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
                supply_cap: Some(Varuint(1_000_000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(1_000_000)),
                        amount_per_mint: Varuint(50_000),
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

    let mint_token2_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(token2_contract.to_tuple()),
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
                live_time: Varint(0),
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

    // Broadcast open account transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (token1_mint_tx.block.0 as usize, 1, 1, Witness::new()),
            (token2_mint_tx.block.0 as usize, 1, 1, Witness::new()),
            (token2_mint_tx.block.0 as usize, 0, 0, Witness::new()),
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
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
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
                output: Varuint(2),
                amount: Varuint(100),
            }],
        }),
        contract_creation: None,
        contract_call: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (token1_mint_tx.block.0 as usize, 1, 1, Witness::new()),
            (token1_mint_tx.block.0 as usize, 0, 0, Witness::new()),
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
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let swap_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(lp_contract.to_tuple()),
            call_type: CallType::Swap(SwapOption {
                pointer: Varuint(1),
                assert_values: Some(AssertValues {
                    input_values: Some(vec![Varuint(100)]),
                    total_collateralized: None,
                    min_out_value: Some(Varuint(49)),
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
            (rebase_block_tx.block.0 as usize, 1, 2, Witness::new()),
            (rebase_block_tx.block.0 as usize, 0, 0, Witness::new()),
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
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
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

    // Broadcast burn transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (mint_lp_block_tx.block.0 as usize, 1, 1, Witness::new()),
            (mint_lp_block_tx.block.0 as usize, 0, 0, Witness::new()),
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
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
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
