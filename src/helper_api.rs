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
use serde_json::{json, Value};
use crate::{APIState, Updater};

// TODO: get_token_supply_by_ticker, get_token_holders_by_ticker, get_address_balance_by_ticker
pub fn helper_routes() -> Router<APIState> {
    Router::new()
        .route("/helper/address/:address/balance", get(get_address_balance))
}

async fn get_address_balance(
    State(state): State<APIState>,
    Path(address): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let updater = Updater::new(state.database.clone(), true).await;
    match updater.get_address_balance(address).await {
        Ok(balance) => Ok(Json(json!({ "balance": balance }))),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}
