use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use store::database::{DatabaseError, MESSAGE_PREFIX, TRANSACTION_TO_BLOCK_TX_PREFIX};
use transaction::message::OpReturnMessage;
use super::*;


// TODO: The database lock could possibly slowing down indexing. Add cache or rate limit for the API.
pub struct APIState {
    pub database: Arc<Mutex<Database>>,
}
pub async fn run_api(database: Arc<Mutex<Database>>) -> Result<(), std::io::Error> {
    let shared_state = Arc::new(APIState { database });
    let app = Router::new()
        .route("/health", get(health))
        .route("/tx/:txid", get(tx_result))
        .with_state(shared_state.clone())
        .route("/blocktx/:block/:tx", get(get_block_tx))
        .with_state(shared_state);
    log::info!("API is listening on {}", CONFIG.api_url);
    let listener = tokio::net::TcpListener::bind(CONFIG.api_url.clone()).await?;

    axum::serve(listener, app).await
}

async fn tx_result(
    State(mut state): State<Arc<APIState>>,
    Path(txid): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let state = Arc::get_mut(&mut state).unwrap();
    let blocktx: Result<BlockTxTuple, DatabaseError> = state
        .database
        .lock()
        .await
        .get(TRANSACTION_TO_BLOCK_TX_PREFIX, txid.as_str());

    let blocktx = if let Ok(blocktx) = blocktx {
        blocktx
    } else {
        return Err(StatusCode::NOT_FOUND);
    };

    let message: Result<OpReturnMessage, DatabaseError> = state.database.lock().await.get(
        MESSAGE_PREFIX,
        BlockTx {
            block: blocktx.0,
            tx: blocktx.1,
        }
        .to_string()
        .as_str(),
    );

    if let Ok(message) = message {
        Ok(Json(json!({"valid": true, "message": message})))
    } else {
        return Err(StatusCode::NOT_FOUND);
    }
}

async fn get_block_tx(
    State(mut state): State<Arc<APIState>>,
    Path(block): Path<u64>,
    Path(tx): Path<u32>,
) -> Result<Json<Value>, StatusCode> {
    let state = Arc::get_mut(&mut state).unwrap();
    let message: Result<OpReturnMessage, DatabaseError> = state
        .database
        .lock()
        .await
        .get(MESSAGE_PREFIX, BlockTx { block, tx }.to_string().as_str());

    if let Ok(message) = message {
        Ok(Json(json!({"valid": true, "message": message})))
    } else {
        return Err(StatusCode::NOT_FOUND);
    }
}

async fn health() -> &'static str {
    "OK"
}
