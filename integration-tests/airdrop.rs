mod utils;
use utils::{encrypt_message, get_bitcoin_address, start_indexer, TestContext};

use bitcoin::{
    key::Secp256k1,
    secp256k1::{self},
    Witness,
};
use glittr::{
    az_base26::AZBase26,
    bloom_filter_to_compressed_vec,
    message::{
        ArgsCommitment, CallType, Commitment, CommitmentMessage, ContractCall, ContractCreation,
        ContractType, MintBurnOption, OpReturnMessage,
    },
    mint_only_asset::{MOAMintMechanisms, MintOnlyAssetContract},
    transaction_shared::{AllocationType, BloomFilterArgType, FreeMint, Preallocated},
    varint::Varint,
    varuint::Varuint,
    BlockTx,
};
use growable_bloom_filter::GrowableBloom;
use mockcore::TransactionTemplate;
use std::{collections::HashMap, str::FromStr, sync::Arc};

#[tokio::test]
async fn test_integration_glittr_airdrop() {
    let mut ctx = TestContext::new().await;
    let (user_address, _) = get_bitcoin_address();

    // Create admin keypair for encryption/decryption
    let secp: Secp256k1<secp256k1::All> = Secp256k1::new();
    let admin_secret_key = bitcoin::secp256k1::SecretKey::new(&mut secp256k1::rand::thread_rng());

    // Create the corresponding public key
    let admin_public_key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &admin_secret_key);

    // 1. Admin creates first MOA with commitment
    let first_moa_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: None, // Unlimited supply
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    free_mint: Some(FreeMint {
                        supply_cap: None,
                        amount_per_mint: Varuint(1),
                    }),
                    preallocated: None,
                    purchase: None,
                },
                commitment: Some(Commitment {
                    public_key: admin_public_key.serialize().to_vec(),
                    args: ArgsCommitment {
                        fixed_string: AZBase26::from_str("GLITTRAIRDROP").unwrap(),
                        string: "username".to_string(),
                    },
                }),
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let first_moa_contract = ctx.build_and_mine_message(&first_moa_message).await;

    // 2. User mints first MOA with commitment
    let username = "alice123";
    let commitment_string = format!("GLITTRAIRDROP:{}", username);

    // Encrypt commitment using admin's public key
    let encrypted_commitment = encrypt_message(&admin_public_key, &commitment_string).unwrap();

    let mint_first_moa_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(first_moa_contract.to_tuple()),
            call_type: CallType::Mint(MintBurnOption {
                pointer: Some(Varuint(1)),
                oracle_message: None,
                pointer_to_key: None,
                assert_values: None,
                commitment_message: Some(CommitmentMessage {
                    public_key: admin_public_key.serialize().to_vec(),
                    args: encrypted_commitment,
                }),
            }),
        }),
        transfer: None,
        contract_creation: None,
    };

    let height = ctx.core.height();

    let txid = ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[((height - 1) as usize, 0, 0, Witness::new())],
        op_return: Some(mint_first_moa_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[0, 1000],
        outputs: 2,
        p2tr: false,
        recipient: None,
    });

    ctx.core.mine_blocks(1);

    let first_mint_tx = BlockTx {
        block: Varuint(height + 1),
        tx: Varuint(1),
    };

    // 3. Admin creates bloom filter with user's txid:vout
    let mut filter = GrowableBloom::new(0.05, 1000);
    let key = format!("{}:{}", txid, 1); // Using tx:vout as key
    filter.insert(key.clone());
    println!("contains {} {} ", filter.contains(key.clone()), key);
    let compressed_filter = bloom_filter_to_compressed_vec(filter);

    let mut allocations = HashMap::new();

    allocations.insert(
        Varuint(100),
        AllocationType::BloomFilter {
            filter: compressed_filter,
            arg: BloomFilterArgType::TxId,
        },
    );

    // 4. Admin creates second MOA with preallocated using bloom filter
    let second_moa_message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Moa(MintOnlyAssetContract {
                ticker: None,
                supply_cap: Some(Varuint(100)),
                divisibility: 18,
                live_time: Varint(0),
                end_time: None,
                mint_mechanism: MOAMintMechanisms {
                    preallocated: Some(Preallocated {
                        allocations,
                        vesting_plan: None,
                    }),
                    free_mint: None,
                    purchase: None,
                },
                commitment: None,
            }),
        }),
        transfer: None,
        contract_call: None,
    };

    let second_moa_contract = ctx.build_and_mine_message(&second_moa_message).await;

    // 5. User mints second MOA using first MOA as proof
    let mint_second_moa_message = OpReturnMessage {
        contract_call: Some(ContractCall {
            contract: Some(second_moa_contract.to_tuple()),
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

    ctx.core.broadcast_tx(TransactionTemplate {
        fee: 0,
        inputs: &[
            (first_mint_tx.block.0 as usize, 1, 1, Witness::new()), // UTXO containing first MOA
            (first_mint_tx.block.0 as usize, 0, 0, Witness::new()),
        ],
        op_return: Some(mint_second_moa_message.into_script()),
        op_return_index: Some(0),
        op_return_value: Some(0),
        output_values: &[1000, 1000],
        outputs: 2,
        p2tr: false,
        recipient: Some(user_address),
    });
    ctx.core.mine_blocks(1);

    let second_mint_tx = BlockTx {
        block: Varuint(ctx.core.height()),
        tx: Varuint(1),
    };

    start_indexer(Arc::clone(&ctx.indexer)).await;

    // Verify outcomes
    let first_contract_outcome = ctx.get_and_verify_message_outcome(first_moa_contract).await;
    assert!(first_contract_outcome.flaw.is_none());

    let first_mint_outcome = ctx.get_and_verify_message_outcome(first_mint_tx).await;
    assert!(first_mint_outcome.flaw.is_none());

    let second_contract_outcome = ctx
        .get_and_verify_message_outcome(second_moa_contract)
        .await;
    assert!(second_contract_outcome.flaw.is_none());

    let second_mint_outcome = ctx.get_and_verify_message_outcome(second_mint_tx).await;
    println!("{:?}", second_mint_outcome.flaw);
    assert!(second_mint_outcome.flaw.is_none());

    // Verify asset allocations
    let asset_map = ctx.get_asset_map().await;

    // Verify first MOA allocation
    let first_moa_amount = asset_map
        .values()
        .find_map(|list| list.list.get(&first_moa_contract.to_string()))
        .expect("First MOA should exist");
    assert_eq!(*first_moa_amount, 1);

    // Verify second MOA allocation
    let second_moa_amount = asset_map
        .values()
        .find_map(|list| list.list.get(&second_moa_contract.to_string()))
        .expect("Second MOA should exist");
    assert_eq!(*second_moa_amount, 100);

    ctx.drop().await;
}
