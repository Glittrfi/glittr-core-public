use std::{collections::HashMap, str::FromStr};

use crate::{APIState, BlockTx, BlockTxString, Updater};
/// Helper API is a collection of optional APIs that is not part of the core,
/// but will help the initial integration with dApps.
/// These helpers APIs will take more storage resources.
/// It is best for dApps to run their own API to ensure decentralization
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize)]
pub struct ContractInfo {
    pub ticker: Option<String>,
}

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
            let mut contract_info: HashMap<BlockTxString, ContractInfo> = HashMap::new();

            for contract_id in balance.summarized.keys() {
                let block_tx = BlockTx::from_str(contract_id).unwrap();
                let ticker = updater
                    .get_ticker_by_contract_block_tx(block_tx.to_tuple())
                    .await
                    .unwrap();
                contract_info.insert(contract_id.clone(), ContractInfo { ticker });
            }

            return Ok(Json(
                json!({ "balance": balance, "contract_info": contract_info }),
            ));
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}
