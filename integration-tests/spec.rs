mod utils;
use utils::{get_bitcoin_address, start_indexer, TestContext};

use bitcoin::Witness;
use glittr::{
    message::{ContractCreation, ContractType, OpReturnMessage},
    mint_burn_asset::MintStructure,
    mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract},
    spec::{
        MintBurnAssetCollateralizedSpec, MintBurnAssetSpec, MintOnlyAssetSpec,
        MintOnlyAssetSpecPegInType, SpecContract, SpecContractType,
    },
    transaction_shared::{InputAsset, PurchaseBurnSwap, RatioType},
    varint::Varint,
    varuint::Varuint,
    BlockTx, Flaw,
};
use mockcore::TransactionTemplate;
use std::sync::Arc;

#[tokio::test]
async fn test_integration_spec() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                    input_asset: Some(InputAsset::Rune),
                    peg_in_type: Some(MintOnlyAssetSpecPegInType::Burn),
                }),
                block_tx: None,
                pointer: None,
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    ctx.verify_last_block(block_tx_contract.block.0).await;
    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, None);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_update() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![InputAsset::Rune].into(),
                        mint_structure: Some(MintStructure::Ratio(RatioType::Fixed {
                            ratio: (Varuint(10), Varuint(10)),
                        })),
                    }),
                }),
                block_tx: None,
                pointer: Some(Varuint(1)),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![
                            InputAsset::Rune,
                            InputAsset::RawBtc,
                            InputAsset::Ordinal,
                        ]
                        .into(),
                        mint_structure: None,
                    }),
                }),
                block_tx: Some(block_tx_contract.to_tuple()),
                pointer: Some(Varuint(1)),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (block_tx_contract.block.0 as usize, 1, 1, Witness::new()), // spec owner
            (block_tx_contract.block.0 as usize, 0, 0, Witness::new()),
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
    let block_tx_first_update = BlockTx {
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
    };

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![InputAsset::Rune, InputAsset::RawBtc].into(),
                        mint_structure: None,
                    }),
                }),
                block_tx: Some(block_tx_contract.to_tuple()),
                pointer: Some(Varuint(1)),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (block_tx_first_update.block.0 as usize, 1, 1, Witness::new()), // spec owner
            (block_tx_first_update.block.0 as usize, 0, 0, Witness::new()),
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
    let block_tx_second_update = BlockTx {
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    let message = ctx
        .get_and_verify_message_outcome(block_tx_first_update)
        .await;
    assert_eq!(message.flaw, None);

    let message = ctx
        .get_and_verify_message_outcome(block_tx_second_update)
        .await;
    assert_eq!(message.flaw, None);

    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, None);

    if let ContractType::Spec(spec_contract) = message
        .message
        .unwrap()
        .contract_creation
        .unwrap()
        .contract_type
    {
        if let SpecContractType::MintBurnAsset(mba_spec) = spec_contract.spec {
            let prev_input_assets = mba_spec.collateralized.unwrap().input_assets.unwrap();
            assert_eq!(prev_input_assets.len(), 2);
            itertools::assert_equal(
                prev_input_assets.iter(),
                vec![InputAsset::Rune, InputAsset::RawBtc].iter(),
            );
        } else {
            panic!("Invalid spec contract type")
        }
    } else {
        panic!("Invalid contract type");
    };

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_update_not_allowed() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![InputAsset::Rune].into(),
                        mint_structure: Some(MintStructure::Ratio(RatioType::Fixed {
                            ratio: (Varuint(10), Varuint(10)),
                        })),
                    }),
                }),
                block_tx: None,
                pointer: Some(Varuint(1)),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                    collateralized: Some(MintBurnAssetCollateralizedSpec {
                        _mutable_assets: true,
                        input_assets: vec![
                            InputAsset::Rune,
                            InputAsset::RawBtc,
                            InputAsset::Ordinal,
                        ]
                        .into(),
                        mint_structure: None,
                    }),
                }),
                block_tx: Some(block_tx_contract.to_tuple()),
                pointer: Some(Varuint(1)),
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    // the tx would be fail and has a flaw because the UTXO isn't the owner of the spec.
    let block_tx_update = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let message = ctx.get_and_verify_message_outcome(block_tx_update).await;
    assert_eq!(message.flaw, Some(Flaw::SpecUpdateNotAllowed));

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_moa_valid_contract_creation() {
    let (_, address_pubkey) = get_bitcoin_address();
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                    input_asset: Some(InputAsset::Rune),
                    peg_in_type: Some(MintOnlyAssetSpecPegInType::Pubkey(
                        address_pubkey.to_bytes(),
                    )),
                }),
                block_tx: None,
                pointer: None,
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_spec = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(1000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::Rune,
                        pay_to_key: Some(address_pubkey.to_bytes()),
                        ratio: RatioType::Fixed {
                            ratio: (Varuint(1), Varuint(1)),
                        },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
            // use the spec
            spec: Some(block_tx_spec.to_tuple()),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, None);

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_moa_input_asset_invalid() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                    input_asset: Some(InputAsset::Rune),
                    peg_in_type: Some(MintOnlyAssetSpecPegInType::Burn),
                }),
                block_tx: None,
                pointer: None,
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_spec = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(1000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
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
                commitment: None,
            }),
            // use the spec
            spec: Some(block_tx_spec.to_tuple()),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, Some(Flaw::SpecCriteriaInvalid));

    ctx.drop().await;
}

#[tokio::test]
async fn test_integration_spec_moa_peg_in_type_invalid() {
    let (_, address_pubkey) = get_bitcoin_address();

    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Spec(SpecContract {
                spec: SpecContractType::MintOnlyAsset(MintOnlyAssetSpec {
                    input_asset: Some(InputAsset::Rune),
                    peg_in_type: Some(MintOnlyAssetSpecPegInType::Pubkey(
                        address_pubkey.to_bytes(),
                    )),
                }),
                block_tx: None,
                pointer: None,
            }),
            spec: None,
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_spec = ctx.build_and_mine_message(&message).await;

    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(1000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::Rune,
                        pay_to_key: None,
                        ratio: RatioType::Fixed {
                            ratio: (Varuint(1), Varuint(1)),
                        },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
            // use the spec
            spec: Some(block_tx_spec.to_tuple()),
        }),
        transfer: None,
        contract_call: None,
    };

    let block_tx_contract = ctx.build_and_mine_message(&message).await;

    start_indexer(Arc::clone(&ctx.indexer)).await;

    let message = ctx.get_and_verify_message_outcome(block_tx_contract).await;
    assert_eq!(message.flaw, Some(Flaw::SpecCriteriaInvalid));

    ctx.drop().await;
}
