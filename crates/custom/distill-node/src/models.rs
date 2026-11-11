//! Model catalog endpoint for Postudio.

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use serde_json::{Value, json};

use crate::keeper::Keeper;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub current_model_id: Option<String>,
    pub available_models: Vec<Model>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub model_id: String,
    pub name: String,
}

pub fn routes(keeper: Keeper) -> Router {
    Router::new()
        .route("/api/models", get(list))
        .with_state(keeper)
}

async fn list(State(keeper): State<Keeper>) -> Response {
    match keeper
        .handle()
        .request("_x.ai/models/list", json!({}))
        .await
        .and_then(|response| normalize(response).map_err(anyhow::Error::msg))
    {
        Ok(catalog) => Json(catalog).into_response(),
        Err(error) => fail(StatusCode::BAD_GATEWAY, error),
    }
}

fn normalize(response: Value) -> Result<ModelCatalog, String> {
    let envelope = if response.get("error").is_some() {
        &response
    } else {
        response.get("result").unwrap_or(&response)
    };
    if let Some(error) = envelope.get("error").filter(|value| !value.is_null()) {
        return Err(match error {
            Value::String(message) => message.clone(),
            value => value.to_string(),
        });
    }
    let result = envelope.get("result").unwrap_or(envelope);
    let available_models = result
        .get("availableModels")
        .and_then(Value::as_array)
        .ok_or_else(|| "models/list response has no availableModels array".to_owned())?
        .iter()
        .filter_map(|model| {
            let model_id = model.get("modelId")?.as_str()?;
            if model_id.trim().is_empty() {
                return None;
            }
            let name = model
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(model_id);
            Some(Model {
                model_id: model_id.to_owned(),
                name: name.to_owned(),
            })
        })
        .collect();
    let current_model_id = result
        .get("currentModelId")
        .and_then(Value::as_str)
        .filter(|model_id| !model_id.trim().is_empty())
        .map(str::to_owned);
    Ok(ModelCatalog {
        current_model_id,
        available_models,
    })
}

fn fail(status: StatusCode, error: impl std::fmt::Display) -> Response {
    (status, Json(json!({"error": error.to_string()}))).into_response()
}
