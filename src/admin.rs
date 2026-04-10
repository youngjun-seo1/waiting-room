use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::{Json, Router, routing::{delete, get, post}};
use serde::Deserialize;
use std::sync::Arc;
use crate::scheduler::{CreateScheduleRequest, Schedule};
use crate::state::AppState;

pub fn admin_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/config", get(get_config).put(update_config))
        .route("/enable", post(enable))
        .route("/disable", post(disable))
        .route("/stats", get(stats))
        .route("/flush", post(flush))
        .route("/schedules", get(list_schedules).post(create_schedule))
        .route("/schedules/{id}", delete(delete_schedule))
        .layer(axum::middleware::from_fn_with_state(state, auth_middleware))
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    let expected = state.config.read().admin_api_key.clone();
    let provided = req
        .headers()
        .get("X-Api-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if provided != expected {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read();
    Json(serde_json::json!({
        "listen_addr": config.listen_addr.to_string(),
        "origin_url": config.origin_url,
        "max_active_users": config.max_active_users,
        "session_ttl_secs": config.session_ttl_secs,
        "queue_cookie_name": config.queue_cookie_name,
        "enabled": config.enabled,
        "redis_url": config.redis_url,
        "branding": {
            "page_title": config.branding.page_title,
            "logo_url": config.branding.logo_url,
        },
    }))
}

#[derive(Deserialize)]
struct ConfigUpdate {
    max_active_users: Option<u32>,
    session_ttl_secs: Option<u64>,
    page_title: Option<String>,
    logo_url: Option<String>,
}

async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(update): Json<ConfigUpdate>,
) -> impl IntoResponse {
    let mut config = state.config.write();
    if let Some(v) = update.max_active_users {
        config.max_active_users = v;
    }
    if let Some(v) = update.session_ttl_secs {
        config.session_ttl_secs = v;
    }
    if let Some(v) = update.page_title {
        config.branding.page_title = v;
    }
    if let Some(v) = update.logo_url {
        config.branding.logo_url = v;
    }
    Json(serde_json::json!({"status": "updated"}))
}

async fn enable(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.config.write().enabled = true;
    Json(serde_json::json!({"enabled": true}))
}

async fn disable(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.config.write().enabled = false;
    state.queue.flush().await;
    Json(serde_json::json!({"enabled": false}))
}

async fn stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let s = state.queue.stats().await;
    Json(serde_json::json!({
        "active_users": s.active_count,
        "queue_length": s.waiting_count,
        "avg_active_duration_secs": s.avg_active_duration_secs,
        "total_admitted": s.total_admitted,
    }))
}

async fn flush(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.queue.flush().await;
    state.notify_queue_update();
    Json(serde_json::json!({"status": "flushed"}))
}

// --- Schedule endpoints ---

async fn list_schedules(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let schedules = state.schedules.read().clone();
    Json(serde_json::json!({"schedules": schedules}))
}

async fn create_schedule(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if req.start_at >= req.end_at {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "start_at must be before end_at"
        }))));
    }

    let schedule = Schedule::new(req);
    let response = serde_json::json!({
        "status": "created",
        "schedule": schedule,
    });
    state.schedules.write().push(schedule);
    Ok((StatusCode::CREATED, Json(response)))
}

async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut schedules = state.schedules.write();
    let before = schedules.len();
    schedules.retain(|s| s.id != id);
    if schedules.len() < before {
        Json(serde_json::json!({"status": "deleted", "id": id}))
    } else {
        Json(serde_json::json!({"status": "not_found", "id": id}))
    }
}
