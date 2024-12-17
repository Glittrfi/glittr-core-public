/// Helper API is a collection of optional APIs that is not part of the core,
/// but will help the initial integration with dApps.
/// These helpers APIs will take more storage resources.
/// It is best for dApps to run their own API to ensure decentralization
use std::{collections::HashMap, str::FromStr};
use crate::{APIState, BlockTx, BlockTxString, ContractInfo, Updater};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};


// TODO: get_token_supply_by_ticker, get_token_holders_by_ticker, get_address_balance_by_ticker
pub fn helper_routes() -> Router<APIState> {
    Router::new().route("/helper/address/:address/balance", get(get_address_balance))
}

async fn get_address_balance(
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
