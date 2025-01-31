/// Helper API is a collection of optional APIs that is not part of the core,
/// but will help the initial integration with dApps.
/// These helpers APIs will take more storage resources.
/// It is best for dApps to run their own API to ensure decentralization
use crate::{
    az_base26::AZBase26, varuint::Varuint, APIState, BlockTx, BlockTxString, ContractInfo,
    MintType, Updater,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use bitcoin::{OutPoint, Txid};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, str::FromStr};

#[derive(Serialize, Deserialize)]
struct AssetBalance {
    contract_id: BlockTxString,
    balance: Varuint<u128>,
    ticker: Option<AZBase26>,
    divisibility: u8,
    r#type: MintType,
}

#[derive(Serialize, Deserialize)]
struct ValidOutput {
    address: String,
    output: String,
    asset_balances: Vec<AssetBalance>,
}

// TODO: get_token_supply_by_ticker, get_token_holders_by_ticker, get_address_balance_by_ticker
pub fn helper_routes() -> Router<APIState> {
    Router::new()
        .route(
            "/helper/address/:address/balance",
            get(helper_get_address_balance),
        )
        .route(
            "/helper/address/:address/balance-summary",
            get(helper_get_address_balance_summary),
        )
        .route(
            "/helper/address/:address/valid-outputs",
            get(helper_get_address_valid_outputs),
        )
        .route(
            "/helper/assets/:outpoint",
            get(helper_get_assets_in_outpoint),
        )
}

async fn helper_get_address_balance(
    State(state): State<APIState>,
    Path(address): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let updater = Updater::new(state.database.clone(), true).await;

    match updater.get_address_balance(address).await {
        Ok(balance) => {
            let mut contract_infos: HashMap<BlockTxString, Option<ContractInfo>> = HashMap::new();

            for contract_id in balance.summarized.keys() {
                let block_tx = BlockTx::from_str(contract_id).unwrap();

                let contract_info = updater
                    .get_contract_info_by_block_tx(block_tx.to_tuple())
                    .await
                    .unwrap();
                contract_infos.insert(contract_id.clone(), contract_info);
            }

            return Ok(Json(
                json!({ "balance": balance, "contract_info": contract_infos }),
            ));
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

async fn helper_get_address_balance_summary(
    State(state): State<APIState>,
    Path(address): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let updater = Updater::new(state.database.clone(), true).await;

    match updater.get_address_balance(address).await {
        Ok(balance) => {
            let mut result: Vec<AssetBalance> = Vec::new();

            for contract_id in balance.summarized.keys() {
                let block_tx = BlockTx::from_str(contract_id).unwrap();

                let contract_info = updater
                    .get_contract_info_by_block_tx(block_tx.to_tuple())
                    .await
                    .unwrap()
                    .unwrap();

                result.push(AssetBalance {
                    contract_id: block_tx.to_string(),
                    balance: balance.summarized.get(contract_id).unwrap().clone(),
                    ticker: contract_info.ticker,
                    divisibility: contract_info.divisibility,
                    r#type: contract_info.r#type,
                });
            }

            let block_height = updater.get_last_indexed_block().await;

            return Ok(Json(
                json!({ "data": result, "block_height": block_height.unwrap() }),
            ));
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

async fn helper_get_address_valid_outputs(
    State(state): State<APIState>,
    Path(address): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let updater = Updater::new(state.database.clone(), true).await;

    match updater.get_address_balance(address.clone()).await {
        Ok(balance) => {
            let mut result: Vec<ValidOutput> = Vec::new();

            for utxo in balance.utxos {
                let mut asset_balances: Vec<AssetBalance> = Vec::new();

                for (block_tx_str, balance) in utxo.assets {
                    let contract_info = updater
                        .get_contract_info_by_block_tx(
                            BlockTx::from_str(&block_tx_str).unwrap().to_tuple(),
                        )
                        .await
                        .unwrap()
                        .unwrap();

                    asset_balances.push(AssetBalance {
                        contract_id: block_tx_str,
                        balance,
                        ticker: contract_info.ticker,
                        divisibility: contract_info.divisibility,
                        r#type: contract_info.r#type,
                    })
                }

                result.push(ValidOutput {
                    address: address.clone(),
                    output: format!("{}:{}", utxo.txid, utxo.vout),
                    asset_balances,
                });
            }

            let block_height = updater.get_last_indexed_block().await;

            return Ok(Json(
                json!({ "data": result, "block_height": block_height.unwrap() }),
            ));
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

async fn helper_get_assets_in_outpoint(
    State(state): State<APIState>,
    Path(outpoint): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let updater = Updater::new(state.database, true).await;

    let outpoint_parts: Vec<&str> = outpoint.split(":").collect::<Vec<&str>>();
    let outpoint = OutPoint {
        txid: Txid::from_str(outpoint_parts[0]).unwrap(),
        vout: u32::from_str(outpoint_parts[1]).unwrap(),
    };

    match updater.get_asset_list(&outpoint).await {
        Ok(asset_list) => {
            let mut asset_balances: Vec<AssetBalance> = Vec::new();

            for (contract_id, balance) in asset_list.list {
                let block_tx = BlockTx::from_str(&contract_id).unwrap();
                let contract_info = updater
                    .get_contract_info_by_block_tx(block_tx.to_tuple())
                    .await
                    .unwrap()
                    .unwrap();

                asset_balances.push(AssetBalance {
                    contract_id,
                    balance: Varuint(balance),
                    ticker: contract_info.ticker,
                    divisibility: contract_info.divisibility,
                    r#type: contract_info.r#type,
                });
            }
            let block_height = updater.get_last_indexed_block().await;

            Ok(Json(
                json!({ "result": asset_balances, "block_height": block_height.unwrap() }),
            ))
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}
