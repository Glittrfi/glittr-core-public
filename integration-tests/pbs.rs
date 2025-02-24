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
        CallType, ContractCall, ContractCreation, ContractType, MintBurnOption, OpReturnMessage,
        OracleMessage, OracleMessageSigned,
    },
    mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract},
    transaction_shared::{InputAsset, OracleSetting, PurchaseBurnSwap, RatioType},
    varint::Varint,
    varuint::Varuint,
    BlockTx,
};
use mockcore::TransactionTemplate;
use std::sync::Arc;

#[tokio::test]
async fn test_integration_purchaseburnswap() {
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

/// test_raw_btc_to_glittr_asset_burn e.g. raw btc to wbtc by burn
#[tokio::test]
async fn test_raw_btc_to_glittr_asset_burn() {
    let mut ctx = TestContext::new().await;

    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: None,
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
        }),
        transfer: None,
        contract_call: None,
    };

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let (minter_address, _) = get_bitcoin_address();

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
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
        block: Varuint(height + 1),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none());

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == (bitcoin_value - fee - dust) as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

///Example: most basic gBTC implementation
/// {
///     TxType: Contract,
///     SimpleAsset:{
///         supplyCap: 21000000,
///         Divisibility: 100000000,
///         Livetime: -10
///     },
///     Purchase:{
///         inputAsset: BTC,
///         payTo: pubkey,
///         Ratio: 1,
///     }
///     }
/// test_raw_btc_to_glittr_asset_purchase e.g. raw btc to wbtc by purchase
#[tokio::test]
async fn test_raw_btc_to_glittr_asset_purchase_gbtc() {
    let mut ctx = TestContext::new().await;

    let (contract_treasury, contract_treasury_pub_key) = get_bitcoin_address();
    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(21_000_000 * 10u128.pow(8))),
                divisibility: 8,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: Some(contract_treasury_pub_key.to_bytes()),
                        ratio: RatioType::Fixed {
                            ratio: (Varuint(1), Varuint(1)),
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
    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
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

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    // NOTE: can only declare 1 output here
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[bitcoin_value - fee],
        outputs: 1,
        p2tr: false,
        recipient: Some(contract_treasury),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: Varuint(height + 1),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none());

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == (bitcoin_value - fee) as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

// test_raw_btc_to_glittr_asset_purchase e.g. raw btc to busd
#[tokio::test]
async fn test_raw_btc_to_glittr_asset_burn_oracle() {
    let mut ctx = TestContext::new().await;

    let secp = Secp256k1::new();
    let oracle_keypair = Keypair::new(&secp, &mut rand::thread_rng());
    let oracle_xonly = XOnlyPublicKey::from_keypair(&oracle_keypair);
    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: None,
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: None,
                        ratio: RatioType::Oracle {
                            setting: OracleSetting {
                                pubkey: oracle_xonly.0.serialize().to_vec(),
                                asset_id: Some("btc".to_string()),
                                block_height_slippage: 5,
                            },
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

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let (minter_address, _) = get_bitcoin_address();

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;
    let dust = 546;
    let ratio = 1000;
    let oracle_out_value = (bitcoin_value - dust - fee) * ratio;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    let oracle_message = OracleMessage {
        asset_id: Some("btc".to_string()),
        block_height: Varuint(height),
        input_outpoint: None,
        min_in_value: None,
        out_value: None,
        ratio: Some((Varuint(1000), Varuint(1))), // 1 sat = 1000 usd,
        ltv: None,
        outstanding: None,
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(Varuint(1)),
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(bitcoin_value - fee - dust),
        output_values: &[dust],
        outputs: 1,
        p2tr: false,
        recipient: Some(minter_address),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: Varuint(height + 1),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none(), "{:?}", mint_outcome.flaw);

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == oracle_out_value as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

/// Example: most basic Hermetica implementation
/// {
/// TxType: contract,
/// simpleAsset:{
/// 	supplyCap: -1,
/// 	Divisibility: 100,
/// 	Livetime: -10,
/// },
/// Purchase:{
/// 	inputAsset: BTC,
/// 	payTo: pubkey,
/// 	Ratio: {
/// 		Oracle: {
/// 	        oracleKey: pubKey,
/// 	        Args:{
///               satsPerDollar: int,
///               purchaseValue: float,
///               BLOCKHEIGHT: {AllowedBlockSlippage: -5},
///             },
///        },
///     },
/// },
/// }
///
#[tokio::test]
async fn test_raw_btc_to_glittr_asset_oracle_purchase() {
    let mut ctx = TestContext::new().await;

    let secp = Secp256k1::new();
    let oracle_keypair = Keypair::new(&secp, &mut rand::thread_rng());
    let oracle_xonly = XOnlyPublicKey::from_keypair(&oracle_keypair);

    let (treasury_address, treasury_address_pub_key) = get_bitcoin_address();
    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(21_000_000 * 10u128.pow(8))),
                divisibility: 8,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::RawBtc,
                        pay_to_key: Some(treasury_address_pub_key.to_bytes()),
                        ratio: RatioType::Oracle {
                            setting: OracleSetting {
                                pubkey: oracle_xonly.0.serialize().to_vec(),
                                asset_id: Some("btc".to_string()),
                                block_height_slippage: 5,
                            },
                        },
                    }),
                    preallocated: None,
                    free_mint: None,
                },
                commitment: None,
            }),
        }),
        contract_call: None,
        transfer: None,
    };

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;
    let oracle_out_value = (bitcoin_value - fee) * 1000;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    let oracle_message = OracleMessage {
        input_outpoint: None,
        min_in_value: None,
        out_value: None,
        asset_id: Some("btc".to_string()),
        block_height: Varuint(height),
        ratio: Some((Varuint(1000), Varuint(1))), // 1 sats = 1000 glittr asset
        ltv: None,
        outstanding: None,
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(Varuint(1)),
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        contract_creation: None,
        transfer: None,
    };
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[bitcoin_value - fee],
        outputs: 1,
        p2tr: false,
        recipient: Some(treasury_address),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: Varuint(height + 1),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none(), "{:?}", mint_outcome.flaw);

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == oracle_out_value as u128);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}

#[tokio::test]
async fn test_metaprotocol_to_glittr_asset() {
    let mut ctx = TestContext::new().await;

    let secp = Secp256k1::new();
    let oracle_keypair = Keypair::new(&secp, &mut rand::thread_rng());
    let oracle_xonly = XOnlyPublicKey::from_keypair(&oracle_keypair);
    let contract_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: None,
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    purchase: Some(PurchaseBurnSwap {
                        input_asset: InputAsset::Rune,
                        pay_to_key: None,
                        ratio: RatioType::Oracle {
                            setting: OracleSetting {
                                pubkey: oracle_xonly.0.serialize().to_vec(),
                                asset_id: Some("rune:840000:3".to_string()),
                                block_height_slippage: 5,
                            },
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

    let contract_id = ctx.build_and_mine_message(&contract_message).await;

    let (minter_address, _) = get_bitcoin_address();

    // prepare btc
    let bitcoin_value = 50000;
    let fee = 100;
    let dust = 546;
    let oracle_out_value = 10000;

    ctx.core.mine_blocks_with_subsidy(1, bitcoin_value);
    let height = ctx.core.height();

    let input_txid = ctx.core.tx((height - 1) as usize, 0).compute_txid();
    let oracle_message = OracleMessage {
        input_outpoint: Some(
            OutPoint {
                txid: input_txid,
                vout: 0,
            }
            .into(),
        ),
        min_in_value: Some(Varuint(0)),
        out_value: Some(Varuint(oracle_out_value)),
        asset_id: Some("rune:840000:3".to_string()),
        block_height: Varuint(height),
        ratio: None,
        ltv: None,
        outstanding: None,
    };

    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let msg = Message::from_digest_slice(
        sha256::Hash::hash(serde_json::to_string(&oracle_message).unwrap().as_bytes())
            .as_byte_array(),
    )
    .unwrap();

    let signature = secp.sign_schnorr(&msg, &oracle_keypair);

    let mint_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(contract_id.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(Varuint(1)),
                oracle_message: Some(OracleMessageSigned {
                    signature: signature.serialize().to_vec(),
                    message: oracle_message.clone(),
                }),
                pointer_to_key: None,
                assert_values: None,
                commitment_message: None,
            }),
        }),
        transfer: None,
        contract_creation: None,
    };
    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(bitcoin_value - fee - dust),
        output_values: &[dust],
        outputs: 1,
        p2tr: false,
        recipient: Some(minter_address),
    });

    ctx.core.mine_blocks(1);

    let mint_block_tx = BlockTx {
        block: Varuint(height + 1),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;
    ctx.get_and_verify_message_outcome(contract_id).await;
    let mint_outcome = ctx.get_and_verify_message_outcome(mint_block_tx).await;
    assert!(mint_outcome.flaw.is_none(), "{:?}", mint_outcome.flaw);

    let tx = ctx.get_transaction_from_block_tx(mint_block_tx).unwrap();

    let asset_lists = ctx.get_asset_list().await;

    for (k, v) in &asset_lists {
        println!("Mint output: {}: {:?}", k, v);
    }

    let outpoint_str = asset_lists[0].0.clone();
    let out_value = *asset_lists[0].1.list.get(&contract_id.to_string()).unwrap();

    assert!(out_value == oracle_out_value);
    assert!(
        outpoint_str
            == format!(
                "{}:{}",
                "asset_list",
                OutPoint {
                    txid: tx.compute_txid(),
                    vout: 1
                }
                .to_string()
            )
    );

    ctx.drop().await;
}
