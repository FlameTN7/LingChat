//! Authenticated loopback API for deterministic memory validation.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::harness::validate_real;
use super::scenarios;
use super::scripted_provider::ScriptedProvider;
use crate::ai_service::types::GameMemoryBank;

#[derive(Clone)]
pub struct ApiState {
    pub token: Arc<str>,
    pub shutdown: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
    pub busy: Arc<std::sync::Mutex<bool>>,
}

struct BusyGuard(Arc<std::sync::Mutex<bool>>);
impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Ok(mut busy) = self.0.lock() {
            *busy = false;
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ValidateRequest {
    #[serde(default)]
    pub scenario: String,
    #[serde(default = "default_role_id")]
    pub role_id: i32,
    #[serde(default = "default_display_name")]
    pub display_name: String,
    #[serde(default)]
    pub initial_bank: GameMemoryBank,
    #[serde(default = "default_line_count")]
    pub line_count: usize,
    #[serde(default = "default_update_interval")]
    pub update_interval: usize,
    #[serde(default)]
    pub delay_ms: u64,
    #[serde(default)]
    pub fail_section: Option<String>,
    #[serde(default)]
    pub empty_section: Option<String>,
    #[serde(default)]
    pub panic_section: Option<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub append_during_update: bool,
    #[serde(default)]
    pub persistence_roundtrip: bool,
}

fn default_role_id() -> i32 {
    7
}
fn default_display_name() -> String {
    "Test AI".into()
}
fn default_line_count() -> usize {
    4
}
fn default_update_interval() -> usize {
    1
}
fn default_timeout_ms() -> u64 {
    10_000
}

#[derive(Clone, Debug, Serialize)]
pub struct ValidateResponse {
    pub outcome: String,
    pub scenario: String,
    pub triggered: bool,
    pub committed: bool,
    pub calls: usize,
    pub bank: GameMemoryBank,
    pub last_processed_global_idx: i64,
    pub unprocessed_tail_lines: usize,
    pub updating: bool,
    pub persistence_roundtrip: Option<bool>,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    busy: bool,
    mode: &'static str,
    api_version: u32,
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|candidate| candidate == token)
}
fn unauthorized() -> impl IntoResponse {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error":"unauthorized"})),
    )
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/memory/validate", post(validate))
        .route("/v1/scenarios/:name", post(validate_scenario))
        .route("/shutdown", post(shutdown))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(state)
}

async fn health(State(state): State<ApiState>, headers: HeaderMap) -> impl IntoResponse {
    if !authorized(&headers, &state.token) {
        return unauthorized().into_response();
    }
    let busy = state.busy.lock().map(|g| *g).unwrap_or(true);
    Json(HealthResponse {
        ok: true,
        busy,
        mode: "scripted",
        api_version: 1,
    })
    .into_response()
}

async fn validate_scenario(
    State(state): State<ApiState>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    validate_inner(state, headers, body, Some(name)).await
}

async fn validate(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    validate_inner(state, headers, body, None).await
}

async fn validate_inner(
    state: ApiState,
    headers: HeaderMap,
    body: Bytes,
    route_scenario: Option<String>,
) -> axum::response::Response {
    if !authorized(&headers, &state.token) {
        return unauthorized().into_response();
    }
    let mut request: ValidateRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"invalid_json"})),
            )
                .into_response();
        },
    };
    if let Some(scenario) = route_scenario {
        request.scenario = scenario;
    }
    if !scenarios::known(&request.scenario) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error":"unknown_scenario"})),
        )
            .into_response();
    }
    // Built-in scenario names are executable behavior, not just a whitelist.
    if request.scenario == "one-section-fails" && request.fail_section.is_none() {
        request.fail_section = Some("promises".into());
    }
    if request.scenario == "empty-section-fails" && request.empty_section.is_none() {
        request.empty_section = Some("promises".into());
    }
    if request.scenario == "append-during-update" {
        request.append_during_update = true;
        if request.delay_ms == 0 {
            request.delay_ms = 10;
        }
    }
    if request.scenario == "persistence-roundtrip" {
        request.persistence_roundtrip = true;
    }
    if request.role_id <= 0 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error":"invalid_role_id"})),
        )
            .into_response();
    }
    if request.line_count > 10_000 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error":"too_many_lines"})),
        )
            .into_response();
    }
    if request.update_interval == 0
        || request.timeout_ms == 0
        || request.timeout_ms > 120_000
        || request.delay_ms > 60_000
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error":"invalid_limits"})),
        )
            .into_response();
    }
    let acquired = match state.busy.lock() {
        Ok(mut busy) if !*busy => {
            *busy = true;
            true
        },
        Ok(_) => false,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"busy_state_unavailable"})),
            )
                .into_response();
        },
    };
    if !acquired {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error":"validation_busy"})),
        )
            .into_response();
    }
    let _busy_guard = BusyGuard(state.busy.clone());
    let failed = request.fail_section.is_some()
        || request.empty_section.is_some()
        || request.panic_section.is_some();
    let provider = ScriptedProvider {
        delay_ms: request.delay_ms,
        fail_section: request.fail_section.clone(),
        empty_section: request.empty_section.clone(),
        panic_section: request.panic_section.clone(),
        ..Default::default()
    };
    let timeout = Duration::from_millis(request.timeout_ms);
    let run = validate_real(
        provider.clone(),
        request.initial_bank,
        request.role_id,
        request.line_count,
        request.update_interval,
        timeout,
        request.append_during_update,
    );
    let result = tokio::time::timeout(timeout, run).await;
    match result {
        Ok(Ok(result)) => {
            let mut persistence_roundtrip = None;
            let mut persistence_error = None;
            if request.persistence_roundtrip && result.committed {
                match super::temp_db::TemporaryDatabase::open().await {
                    Ok(db) => match db
                        .seed_save_role(request.role_id, &request.display_name)
                        .await
                    {
                        Ok((save_id, role_id)) => match db
                            .round_trip(save_id, role_id, &result.bank)
                            .await
                        {
                            Ok(loaded) if loaded == result.bank => {
                                persistence_roundtrip = Some(true)
                            },
                            Ok(_) => persistence_error = Some("round-trip bank mismatch".into()),
                            Err(error) => persistence_error = Some(error.to_string()),
                        },
                        Err(error) => persistence_error = Some(error.to_string()),
                    },
                    Err(error) => persistence_error = Some(error.to_string()),
                }
            }
            let outcome = if failed {
                "not_committed"
            } else if persistence_error.is_some() {
                "persistence_failed"
            } else if result.committed {
                "succeeded"
            } else {
                "not_committed"
            };
            Json(ValidateResponse {
                outcome: outcome.into(),
                scenario: request.scenario,
                triggered: result.triggered,
                committed: result.committed,
                calls: result.calls,
                bank: result.bank,
                last_processed_global_idx: result.processed_idx,
                unprocessed_tail_lines: result.tail_lines,
                updating: result.updating,
                persistence_roundtrip,
                error_code: if failed {
                    Some("compression_failed".into())
                } else {
                    persistence_error.map(|_| "persistence_failed".into())
                },
            })
            .into_response()
        },
        Ok(Err(_error)) => Json(ValidateResponse {
            outcome: "not_committed".into(),
            scenario: request.scenario,
            triggered: false,
            committed: false,
            calls: provider.calls(),
            bank: GameMemoryBank::default(),
            last_processed_global_idx: 0,
            unprocessed_tail_lines: request.line_count,
            updating: false,
            persistence_roundtrip: None,
            error_code: Some("validation_failed".into()),
        })
        .into_response(),
        Err(_) => (
            StatusCode::REQUEST_TIMEOUT,
            Json(serde_json::json!({"error":"timed_out","outcome":"timed_out"})),
        )
            .into_response(),
    }
}

async fn shutdown(State(state): State<ApiState>, headers: HeaderMap) -> impl IntoResponse {
    if !authorized(&headers, &state.token) {
        return unauthorized().into_response();
    }
    if let Some(sender) = state
        .shutdown
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
    {
        let _ = sender.send(());
    }
    Json(serde_json::json!({"ok":true})).into_response()
}

pub async fn serve(token: Arc<str>) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state = ApiState {
        token,
        shutdown: Arc::new(std::sync::Mutex::new(Some(shutdown_tx))),
        busy: Arc::new(std::sync::Mutex::new(false)),
    };
    println!(
        "{}",
        serde_json::json!({"event":"ready","host":"127.0.0.1","port":address.port(),"token":state.token,"api_version":1})
    );
    use std::io::Write;
    let _ = std::io::stdout().flush();
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await
}
