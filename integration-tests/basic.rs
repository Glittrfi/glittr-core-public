mod utils;
use utils::{get_bitcoin_address, start_indexer, TestContext};

use bitcoin::Witness;
use glittr::{
    az_base26::AZBase26,
    database::{DatabaseError, TICKER_TO_BLOCK_TX_PREFIX},
    message::{
        CallType, ContractCall, ContractCreation, ContractType, MintBurnOption, OpReturnMessage,
    },
    mint_burn_asset::{
        BurnMechanisms, Collateralized, MBAMintMechanisms, MintBurnAssetContract, MintStructure,
        ProportionalType, RatioModel, ReturnCollateral, SwapMechanisms,
    },
    mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract},
    transaction_shared::{FreeMint, InputAsset, PurchaseBurnSwap, RatioType},
    varint::Varint,
    varuint::Varuint,
    BlockTx, BlockTxTuple, Flaw,
};
use mockcore::TransactionTemplate;
use std::{str::FromStr, sync::Arc};

#[tokio::test]
async fn test_integration_broadcast_op_return_message_success() {
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
    println!("{}", message);

    let block_tx = ctx.build_and_mine_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.verify_last_block(block_tx.block.0).await;
    ctx.get_and_verify_message_outcome(block_tx).await;
    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_contract_ticker() {
    let mut ctx = TestContext::new().await;

    let ticker = AZBase26::from_str("POHON.PISANG").unwrap();

    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: Some(ticker.clone()),
                supply_cap: None,
                divisibility: 18,
                live_time: Varint(0),
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: None,
                        ratio: RatioType::Fixed {
                            ratio: (Varuint(1), Varuint(1)),
                        },
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
                live_time: Varint(0),
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: None,
                        ratio: RatioType::Fixed {
                            ratio: (Varuint(1), Varuint(1)),
                        },
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
        .get(TICKER_TO_BLOCK_TX_PREFIX, &ticker.to_string());

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
                supply_cap: Some(Varuint(1_000_000)),
                divisibility: 18,
                live_time: Varint(0),
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(1_000_000)),
                        amount_per_mint: Varuint(100_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                end_time: None,
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
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: Some(Varuint(1_000_000)),
                        amount_per_mint: Varuint(50_000),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                end_time: None,
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
                supply_cap: None,
                divisibility: 18,
                live_time: Varint(0),
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
                end_time: None,
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: Some(ContractCall {
            contract: None,
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(Varuint(1)),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
    };

    // Broadcast lp creation and minting
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (token1_mint_tx.block.0 as usize, 1, 1, Witness::new()),
            (token2_mint_tx.block.0 as usize, 1, 1, Witness::new()),
            (token2_mint_tx.block.0 as usize, 0, 0, Witness::new()),
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
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
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
