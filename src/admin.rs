use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::{Json, Router, routing::{delete, get, patch, post}};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use crate::scheduler::{CreateScheduleRequest, Schedule, SchedulePhase};
use crate::state::AppState;

pub fn admin_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/config", get(get_config).patch(patch_config))
        .route("/schedules", get(list_schedules).post(create_schedule))
        .route("/schedules/archives", get(list_archives))
        .route("/schedules/{id}", delete(delete_schedule))
        .route("/schedules/{id}/config", patch(patch_schedule))
        .route("/schedules/{id}/stop", post(stop_schedule))
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
        "enabled": state.is_enabled(),
        "redis_url": config.redis_url,
        "branding": {
            "page_title": config.branding.page_title,
            "logo_url": config.branding.logo_url,
        },
    }))
}

#[derive(Deserialize)]
struct PatchConfigRequest {
    max_active_users: Option<u32>,
    session_ttl_secs: Option<u64>,
    origin_url: Option<String>,
}

async fn patch_config(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PatchConfigRequest>,
) -> impl IntoResponse {
    // Update runtime config
    {
        let mut config = state.config.write();
        if let Some(v) = req.max_active_users {
            config.max_active_users = v;
        }
        if let Some(v) = req.session_ttl_secs {
            config.session_ttl_secs = v;
        }
        if let Some(v) = &req.origin_url {
            config.origin_url = v.clone();
        }
    }

    // Also update the active schedule's overrides so the scheduler
    // doesn't overwrite our changes on the next tick
    {
        let mut schedules = state.schedules.write();
        if let Some(active) = schedules.iter_mut().find(|s| s.phase == SchedulePhase::Active) {
            if let Some(v) = req.max_active_users {
                active.max_active_users = Some(v);
            }
            if let Some(v) = req.session_ttl_secs {
                active.session_ttl_secs = Some(v);
            }
            if let Some(v) = req.origin_url {
                active.origin_url = Some(v);
            }
        }
    }

    // Persist to Redis if available
    crate::schedule_store::save_all_schedules(&state).await;

    let config = state.config.read();
    Json(serde_json::json!({
        "status": "updated",
        "max_active_users": config.max_active_users,
        "session_ttl_secs": config.session_ttl_secs,
        "origin_url": config.origin_url,
    }))
}

#[derive(Deserialize)]
struct PatchScheduleRequest {
    max_active_users: Option<u32>,
    session_ttl_secs: Option<u64>,
    origin_url: Option<String>,
}

async fn patch_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PatchScheduleRequest>,
) -> impl IntoResponse {
    let found = {
        let mut schedules = state.schedules.write();
        if let Some(schedule) = schedules.iter_mut().find(|s| s.id == id) {
            if let Some(v) = req.max_active_users {
                schedule.max_active_users = Some(v);
            }
            if let Some(v) = req.session_ttl_secs {
                schedule.session_ttl_secs = Some(v);
            }
            if let Some(v) = req.origin_url {
                schedule.origin_url = Some(v);
            }
            Some(schedule.clone())
        } else {
            None
        }
    };

    match found {
        Some(schedule) => {
            crate::schedule_store::save_all_schedules(&state).await;
            (StatusCode::OK, Json(serde_json::json!({
                "status": "updated",
                "schedule": schedule,
            }))).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn stop_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let found = {
        let mut schedules = state.schedules.write();
        if let Some(schedule) = schedules.iter_mut().find(|s| s.id == id) {
            if schedule.phase == SchedulePhase::Ended {
                return (StatusCode::CONFLICT, Json(serde_json::json!({
                    "error": "schedule already ended"
                }))).into_response();
            }
            schedule.end_at = Utc::now();
            Some(schedule.clone())
        } else {
            None
        }
    };

    match found {
        Some(schedule) => {
            crate::schedule_store::save_all_schedules(&state).await;
            (StatusCode::OK, Json(serde_json::json!({
                "status": "stopped",
                "schedule": schedule,
            }))).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// --- Schedule endpoints ---

async fn list_schedules(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let schedules = crate::schedule_store::load_schedules(&state).await;
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

    // Check for overlap with existing schedules (exclude ended ones)
    let existing = crate::schedule_store::load_schedules(&state).await;
    for s in &existing {
        if s.phase == SchedulePhase::Ended {
            continue;
        }
        // Two intervals overlap if one starts before the other ends and vice versa
        if req.start_at < s.end_at && req.end_at > s.start_at {
            return Err((StatusCode::CONFLICT, Json(serde_json::json!({
                "error": format!("기존 스케줄 '{}' (id: {}) 일정과 겹칩니다", s.name, s.id)
            }))));
        }
    }

    let schedule = Schedule::new(req);
    let response = serde_json::json!({
        "status": "created",
        "schedule": schedule,
    });
    crate::schedule_store::save_schedule(&state, &schedule).await;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn list_archives(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let archives = crate::archive_store::load_archives(&state).await;
    Json(serde_json::json!({"archives": archives}))
}

async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if crate::schedule_store::remove_schedule(&state, &id).await {
        Json(serde_json::json!({"status": "deleted", "id": id}))
    } else {
        Json(serde_json::json!({"status": "not_found", "id": id}))
    }
}
