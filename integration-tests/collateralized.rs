mod utils;
use utils::{get_bitcoin_address, start_indexer, TestContext};

use bitcoin::{
    hashes::{sha256, Hash},
    key::{rand, Keypair, Secp256k1},
    secp256k1::{self, Message},
    OutPoint, Witness, XOnlyPublicKey,
};
use glittr::{
    message::{
        CallType, CloseAccountOption, ContractCall, ContractCreation, ContractType, MintBurnOption,
        OpReturnMessage, OpenAccountOption, OracleMessage, OracleMessageSigned,
    },
    mint_burn_asset::{
        AccountType, BurnMechanisms, Collateralized, MBAMintMechanisms, MintBurnAssetContract,
        MintStructure, ReturnCollateral, SwapMechanisms,
    },
    mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract},
    transaction_shared::{FreeMint, InputAsset, OracleSetting, RatioType},
    varint::Varint,
    varuint::Varuint,
    BlockTx,
};
use mockcore::TransactionTemplate;
use std::sync::Arc;

#[tokio::test]
async fn test_integration_collateralized_mba() {
    let mut ctx = TestContext::new().await;

    let (owner_address, _) = get_bitcoin_address();

    // Create oracle keypair
    let secp = Secp256k1::new();
    let oracle_keypair = Keypair::new(&secp, &mut rand::thread_rng());
    let oracle_xonly = XOnlyPublicKey::from_keypair(&oracle_keypair);

    // Create MOA (collateral token) with free mint
    let collateral_message = OpReturnMessage {
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

    let collateral_contract = ctx.build_and_mine_message(&collateral_message).await;

    // Mint collateral tokens
    let mint_collateral_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(collateral_contract.to_tuple()),
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

    let collateral_mint_tx = ctx.build_and_mine_message(&mint_collateral_message).await;

    let mba_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Mba(MintBurnAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(500_000)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MBAMintMechanisms {
                    preallocated: None,
                    free_mint: None,
                    purchase: None,
                    collateralized: Some(Collateralized {
                        input_assets: vec![InputAsset::GlittrAsset(collateral_contract.to_tuple())],
                        _mutable_assets: false,
                        mint_structure: MintStructure::Account(AccountType {
                            max_ltv: (Varuint(7), Varuint(10)),
                            ratio: RatioType::Oracle {
                                setting: OracleSetting {
                                    pubkey: oracle_xonly.0.serialize().to_vec(),
                                    block_height_slippage: 5,
                                    asset_id: None,
                                },
                            },
                        }),
                    }),
                },
                burn_mechanism: BurnMechanisms {
                    return_collateral: Some(ReturnCollateral {
                        oracle_setting: Some(OracleSetting {
                            pubkey: oracle_xonly.0.serialize().to_vec(),
                            asset_id: None,
                            block_height_slippage: 5,
                        }),
                        fee: None,
                    }),
                },
                swap_mechanism: SwapMechanisms { fee: None },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let mba_contract = ctx.build_and_mine_message(&mba_message).await;

    let open_account_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::OpenAccount(OpenAccountOption {
                pointer_to_key: Varuint(1),
                share_amount: Varuint(100),
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (collateral_mint_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO containing collateral
            (collateral_mint_tx.block.0 as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(open_account_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000], // Values for outputs
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let account_block_tx = BlockTx {
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
    };

    // Create oracle message for mint
    let oracle_message = OracleMessage {
        asset_id: None,
        block_height: Varuint(ctx.core.height()),
        input_outpoint: Some(
            OutPoint {
                txid: ctx
                    .core
                    .tx(account_block_tx.block.0 as usize, 1)
                    .compute_txid(),
                vout: 1,
            }
            .into(),
        ),
        min_in_value: None,
        out_value: Some(Varuint(50_000)), // Amount to mint
        ratio: None,
        ltv: Some((Varuint(5), Varuint(10))), // 50% LTV
        outstanding: Some(Varuint(50_000)),
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    // Mint using oracle message
    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(Varuint(2)),
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
                pointer_to_key: Some(Varuint(1)),
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast mint transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (account_block_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO containing collateral account
            (account_block_tx.block.0 as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546, 546], // Values for outputs
        outputs: 4,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify all outcomes
    let collateral_outcome = ctx
        .get_and_verify_message_outcome(collateral_contract)
        .await;
    assert!(collateral_outcome.flaw.is_none());

    let mba_outcome = ctx.get_and_verify_message_outcome(mba_contract).await;
    assert!(mba_outcome.flaw.is_none());

    let account_outcome = ctx.get_and_verify_message_outcome(account_block_tx).await;
    assert!(account_outcome.flaw.is_none());

    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none(), "{:?}", mint_outcome.flaw);

    // Verify minted assets
    let asset_lists = ctx.get_asset_map().await;
    for (k, v) in &asset_lists {
        println!("Asset output: {}: {:?}", k, v);
    }

    // Find and verify the minted asset amount
    let minted_amount = asset_lists
        .values()
        .find_map(|list| list.list.get(&mba_contract.to_string()))
        .expect("Minted asset should exist");

    assert_eq!(*minted_amount, 50_000); // Amount specified in oracle message

    // Verify collateral accounts
    let collateral_accounts = ctx.get_collateralize_accounts().await;
    println!("{:?}", collateral_accounts);
    assert_eq!(collateral_accounts.len(), 1);

    let accounts = collateral_accounts.values().next().unwrap();
    let collateral_account = accounts
        .collateral_accounts
        .get(&mba_contract.to_string())
        .unwrap();
    assert_eq!(collateral_account.share_amount, 100);
    assert_eq!(
        collateral_account.collateral_amounts,
        [((Varuint(3), Varuint(1)), 100000)]
    );
    assert_eq!(collateral_account.ltv, (Varuint(5), Varuint(10))); // LTV from oracle message
    assert_eq!(collateral_account.amount_outstanding, 50_000); // Outstanding amount from oracle message

    // Create oracle message for burn
    let burn_oracle_message = OracleMessage {
        asset_id: None,
        block_height: Varuint(ctx.core.height()),
        input_outpoint: Some(
            OutPoint {
                txid: ctx
                    .core
                    .tx(mint_block_tx.block.0 as usize, 1)
                    .compute_txid(),
                vout: 1,
            }
            .into(),
        ),
        min_in_value: None,
        out_value: Some(Varuint(25_000)), // Amount to burn
        ratio: None,
        ltv: Some((Varuint(3), Varuint(10))), // Updated LTV after partial repayment
        outstanding: Some(Varuint(25_000)),   // Remaining outstanding amount
    };

    let burn_msg = Message::from_digest_slice(
        sha256::Hash::hash(
            serde_json::to_string(&burn_oracle_message)
                .unwrap()
                .as_bytes(),
        )
        .as_byte_array(),
    )
    .unwrap();

    let burn_signature = secp.sign_schnorr(&burn_msg, &oracle_keypair);

    // Create burn message
    let burn_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::Burn(MintBurnOption {
                oracle_message: Some(OracleMessageSigned {
                    signature: burn_signature.serialize().to_vec(),
                    message: burn_oracle_message.clone(),
                }),
                pointer_to_key: Some(Varuint(1)),
                pointer: None,
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
            (mint_block_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO containing collateral account
            (mint_block_tx.block.0 as usize, 1, 2, Witness::new()), // UTXO containing mint
            (mint_block_tx.block.0 as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(burn_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546], // Values for outputs
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

    // Verify remaining assets after burn
    let asset_lists_after_burn = ctx.get_asset_map().await;
    for (k, v) in &asset_lists_after_burn {
        println!("Asset output after burn: {}: {:?}", k, v);
    }

    // Find and verify the remaining asset amount
    let remaining_amount = asset_lists_after_burn
        .values()
        .find_map(|list| list.list.get(&mba_contract.to_string()))
        .expect("Remaining asset should exist");

    assert_eq!(*remaining_amount, 25_000); // Original amount (50,000) - burned amount (25,000)

    // Verify updated collateral account after burn
    let collateral_accounts_after_burn = ctx.get_collateralize_accounts().await;
    assert_eq!(collateral_accounts_after_burn.len(), 1);

    let accounts = collateral_accounts_after_burn.values().next().unwrap();
    let account_after_burn = accounts
        .collateral_accounts
        .get(&mba_contract.to_string())
        .unwrap();
    assert_eq!(account_after_burn.share_amount, 100);
    assert_eq!(
        account_after_burn.collateral_amounts,
        [((Varuint(3), Varuint(1)), 100000)]
    ); // Collateral amount unchanged
    assert_eq!(account_after_burn.ltv, (Varuint(3), Varuint(10))); // Updated LTV from oracle message
    assert_eq!(account_after_burn.amount_outstanding, 25_000); // Updated outstanding amount from oracle message

    // Create final burn message to clear outstanding amount
    let final_burn_oracle_message = OracleMessage {
        asset_id: None,
        block_height: Varuint(ctx.core.height()),
        input_outpoint: Some(
            OutPoint {
                txid: ctx
                    .core
                    .tx(burn_block_tx.block.0 as usize, 1)
                    .compute_txid(),
                vout: 1,
            }
            .into(),
        ),
        min_in_value: None,
        out_value: Some(Varuint(25_000)), // Burn remaining amount
        ratio: None,
        ltv: Some((Varuint(0), Varuint(10))), // Set LTV to 0
        outstanding: Some(Varuint(0)),        // Set outstanding to 0
    };

    let final_burn_msg = Message::from_digest_slice(
        sha256::Hash::hash(
            serde_json::to_string(&final_burn_oracle_message)
                .unwrap()
                .as_bytes(),
        )
        .as_byte_array(),
    )
    .unwrap();

    let final_burn_signature = secp.sign_schnorr(&final_burn_msg, &oracle_keypair);

    // Create final burn message
    let final_burn_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::Burn(MintBurnOption {
                oracle_message: Some(OracleMessageSigned {
                    signature: final_burn_signature.serialize().to_vec(),
                    message: final_burn_oracle_message.clone(),
                }),
                pointer_to_key: Some(Varuint(1)),
                pointer: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast final burn transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (burn_block_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO containing collateral account
            (burn_block_tx.block.0 as usize, 1, 2, Witness::new()), // UTXO containing remaining minted tokens
            (burn_block_tx.block.0 as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(final_burn_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546], // Values for outputs
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let final_burn_block_tx = BlockTx {
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
    };

    // Create close account message
    let close_account_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(mba_contract.to_tuple()),
            call_type: CallType::CloseAccount(CloseAccountOption {
                pointer: Varuint(1), // Output index for returned collateral
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    // Broadcast close account transaction
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (final_burn_block_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO containing collateral account
            (final_burn_block_tx.block.0 as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(close_account_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 546], // Values for outputs
        outputs: 2,
        p2tr: false,
        recipient: Some(owner_address.clone()),
    });
    ctx.core.mine_blocks(1);

    let close_account_block_tx = BlockTx {
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify final burn outcome
    let final_burn_outcome = ctx
        .get_and_verify_message_outcome(final_burn_block_tx)
        .await;
    assert!(
        final_burn_outcome.flaw.is_none(),
        "{:?}",
        final_burn_outcome.flaw
    );

    // Verify close account outcome
    let close_account_outcome = ctx
        .get_and_verify_message_outcome(close_account_block_tx)
        .await;
    assert!(
        close_account_outcome.flaw.is_none(),
        "{:?}",
        close_account_outcome.flaw
    );

    // Verify all minted tokens are burned
    let final_asset_lists = ctx.get_asset_map().await;
    let remaining_minted = final_asset_lists
        .values()
        .find_map(|list| list.list.get(&mba_contract.to_string()))
        .unwrap_or(&0);
    assert_eq!(*remaining_minted, 0);

    // Verify collateral account is deleted
    let final_collateral_accounts = ctx.get_collateralize_accounts().await;
    assert_eq!(final_collateral_accounts.len(), 0);

    // Verify collateral tokens are returned
    let returned_collateral = final_asset_lists
        .values()
        .find_map(|list| list.list.get(&collateral_contract.to_string()))
        .expect("Returned collateral should exist");
    assert_eq!(*returned_collateral, 100_000); // Original collateral amount

    ctx.drop().await;
}
