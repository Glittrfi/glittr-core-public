use std::sync::Arc;

use super::*;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use bitcoin::{consensus::deserialize, Transaction};
use serde_json::{json, Value};
use store::database::{DatabaseError, MESSAGE_PREFIX, TRANSACTION_TO_BLOCK_TX_PREFIX};
use transaction::message::OpReturnMessage;

// TODO: The database lock could possibly slowing down indexing. Add cache or rate limit for the API.
#[derive(Clone)]
pub struct APIState {
    pub database: Arc<Mutex<Database>>,
}
pub async fn run_api(database: Arc<Mutex<Database>>) -> Result<(), std::io::Error> {
    let shared_state = APIState { database };
    let app = Router::new()
        .route("/health", get(health))
        .route("/tx/:txid", get(tx_result))
        .route("/blocktx/:block/:tx", get(get_block_tx))
        .route("/assets/:txid/:vout", get(get_assets))
        .route("/asset-contract/:block/:tx", get(get_asset_contract))
        .route("/validate-tx", post(validate_tx))
        .with_state(shared_state);
    println!("API is listening on {}", CONFIG.api_url);
    let listener = tokio::net::TcpListener::bind(CONFIG.api_url.clone()).await?;

    axum::serve(listener, app).await
}

async fn tx_result(
    State(state): State<APIState>,
    Path(txid): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let blocktx: Result<BlockTxTuple, DatabaseError> = state
        .database
        .lock()
        .await
        .get(TRANSACTION_TO_BLOCK_TX_PREFIX, txid.as_str());

    let blocktx = if let Ok(blocktx) = blocktx {
        BlockTx {
            block: blocktx.0,
            tx: blocktx.1,
        }
    } else {
        return Err(StatusCode::NOT_FOUND);
    };

    let message: Result<MessageDataOutcome, DatabaseError> = state
        .database
        .lock()
        .await
        .get(MESSAGE_PREFIX, blocktx.to_string().as_str());

    if let Ok(message) = message {
        if let Some(_) = message.flaw {
            Ok(Json(
                json!({"is_valid": false, "message": message, "block_tx": blocktx.to_str()}),
            ))
        } else {
            Ok(Json(
                json!({"is_valid": true, "message": message, "block_tx": blocktx.to_str()}),
            ))
        }
    } else {
        return Err(StatusCode::NOT_FOUND);
    }
}

async fn get_block_tx(
    State(state): State<APIState>,
    Path((block, tx)): Path<(u64, u32)>,
) -> Result<Json<Value>, StatusCode> {
    let message: Result<MessageDataOutcome, DatabaseError> = state
        .database
        .lock()
        .await
        .get(MESSAGE_PREFIX, BlockTx { block, tx }.to_string().as_str());

    if let Ok(message) = message {
        Ok(Json(json!({"is_valid": true, "message": message})))
    } else {
        return Err(StatusCode::NOT_FOUND);
    }
}

async fn get_assets(
    State(state): State<APIState>,
    Path((txid, vout)): Path<(String, u32)>,
) -> Result<Json<Value>, StatusCode> {
    let updater = Updater::new(state.database, true).await;
    let outpoint = Outpoint { txid, vout };
    if let Some(asset_list) = updater.get_asset_list(&outpoint).await.ok() {
        Ok(Json(json!({"assets": asset_list})))
    } else {
        return Err(StatusCode::NOT_FOUND);
    }
}

async fn get_asset_contract(
    State(state): State<APIState>,
    Path((block, tx)): Path<(u64, u32)>,
) -> Result<Json<Value>, StatusCode> {
    let updater = Updater::new(state.database, true).await;
    if let Some(asset_contract_data) = updater.get_asset_contract_data(&(block, tx)).await.ok() {
        Ok(Json(json!({"asset": asset_contract_data})))
    } else {
        return Err(StatusCode::NOT_FOUND);
    }
}

async fn validate_tx(
    State(state): State<APIState>,
    body: String,
) -> Result<Json<Value>, StatusCode> {
    let tx_bytes = if let Some(tx_bytes) = hex::decode(body).ok() {
        tx_bytes
    } else {
        return Ok(Json(
            json!({"is_valid": false, "msg": "Cannot decode hex string"}),
        ));
    };

    let tx: Transaction = if let Some(tx) = deserialize(&tx_bytes).ok() {
        tx
    } else {
        return Ok(Json(
            json!({"is_valid": false, "msg": "Cannot deserialize to bitcoin transaction"}),
        ));
    };

    return if let Some(op_return_message) = OpReturnMessage::parse_tx(&tx).ok() {
        let mut temp_updater = Updater::new(Arc::clone(&state.database), true).await;
        if let Some(outcome) = temp_updater
            .index(0, 0, &tx, Ok(op_return_message))
            .await
            .ok()
        {
            if let Some(flaw) = outcome.flaw {
                Ok(Json(json!({"is_valid": false, "msg": flaw})))
            } else {
                Ok(Json(json!({"is_valid": true})))
            }
        } else {
            Ok(Json(json!({"is_valid": false, "msg": "Error"})))
        }
    } else {
        Ok(Json(
            json!({"is_valid": false, "msg": "Not a valid Glittr message"}),
        ))
    };
}

async fn health() -> &'static str {
    "OK"
}
