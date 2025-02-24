mod utils;
use utils::{start_indexer, TestContext};

use glittr::{
    message::{
        CallType, ContractCall, ContractCreation, ContractType, MintBurnOption, OpReturnMessage,
    },
    nft::NftAssetContract,
    varint::Varint,
    varuint::Varuint,
};
use std::sync::Arc;

#[tokio::test]
async fn test_integration_mint_nft() {
    let mut ctx = TestContext::new().await;
    let message = OpReturnMessage {
        contract_creation: Some(ContractCreation {
            spec: None,
            contract_type: ContractType::Nft(NftAssetContract {
                supply_cap: Some(Varuint(1)),
                live_time: Varint(0),
                end_time: None,
                asset: vec![0],
                pointer: None,
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

    let block_tx_contract = ctx.build_and_mine_message(&message).await;
    start_indexer(Arc::clone(&ctx.indexer)).await;

    let asset_lists = ctx.get_asset_list().await;
    assert_eq!(
        asset_lists[0].1.list.get(&block_tx_contract.to_string()),
        Some(&1)
    );

    ctx.drop().await;
}
