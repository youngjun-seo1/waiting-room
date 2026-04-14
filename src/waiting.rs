use axum::body::Body;
use axum::extract::State;
use axum::http::{Response, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use serde::Serialize;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::queue::SessionId;
use crate::state::AppState;

static TEMPLATE_SRC: &str = include_str!("templates/waiting.html");

#[derive(Serialize)]
struct SseData {
    position: usize,
    total_waiting: usize,
    eta_seconds: f64,
    progress_pct: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redirect_url: Option<String>,
}

pub async fn serve_waiting_page(state: &Arc<AppState>, session_id: SessionId) -> Response<Body> {
    let config = state.config.read().clone();

    let (position, eta_seconds, total_waiting) =
        if let Some(pos) = state.queue.get_position(&session_id).await {
            (pos.position, pos.eta_seconds, pos.total_waiting)
        } else {
            (0, 0.0, 0)
        };

    let progress_pct = if total_waiting > 0 {
        ((total_waiting - position + 1) as f64 / total_waiting as f64 * 100.0).min(99.0)
    } else {
        0.0
    };

    let eta_display = format_eta(eta_seconds);

    let mut env = minijinja::Environment::new();
    env.add_template("waiting", TEMPLATE_SRC).unwrap();
    let tmpl = env.get_template("waiting").unwrap();

    let html = tmpl
        .render(minijinja::context! {
            page_title => config.branding.page_title,
            logo_url => config.branding.logo_url,
            cookie_name => config.queue_cookie_name,
            position => position,
            progress_pct => format!("{:.0}", progress_pct),
            eta_display => eta_display,
        })
        .unwrap_or_else(|e| format!("Template error: {}", e));

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(html))
        .unwrap()
}

pub async fn sse_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> impl IntoResponse {
    let cookie_name = state.config.read().queue_cookie_name.clone();
    let session_id = match extract_session_from_cookie(&state, &req, &cookie_name) {
        Some(id) => id,
        None => {
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    let origin_url = state.config.read().origin_url.clone();
    let redirect_url = Some(origin_url);

    // Send initial state immediately on connect
    let initial_event = if state.queue.is_active(&session_id).await {
        let data = SseData {
            position: 0,
            total_waiting: 0,
            eta_seconds: 0.0,
            progress_pct: 100.0,
            action: Some("admit".to_string()),
            redirect_url: redirect_url.clone(),
        };
        Some(Ok::<_, Infallible>(
            Event::default().data(serde_json::to_string(&data).unwrap()),
        ))
    } else if let Some(pos) = state.queue.get_position(&session_id).await {
        let total = pos.total_waiting;
        let progress = if total > 0 {
            ((total - pos.position + 1) as f64 / total as f64 * 100.0).min(99.0)
        } else {
            0.0
        };
        let data = SseData {
            position: pos.position,
            total_waiting: pos.total_waiting,
            eta_seconds: pos.eta_seconds,
            progress_pct: progress,
            action: None,
            redirect_url: None,
        };
        Some(Ok(Event::default().data(serde_json::to_string(&data).unwrap())))
    } else {
        None
    };

    let initial_stream = tokio_stream::iter(initial_event.into_iter());

    let rx = state.sse_tx.subscribe();
    let stream = BroadcastStream::new(rx);

    let state_clone = state.clone();
    let update_stream = stream.then(move |_| {
        let state = state_clone.clone();
        let redirect_url = redirect_url.clone();
        async move {
            if state.queue.is_active(&session_id).await {
                let data = SseData {
                    position: 0,
                    total_waiting: 0,
                    eta_seconds: 0.0,
                    progress_pct: 100.0,
                    action: Some("admit".to_string()),
                    redirect_url,
                };
                Some(Ok::<_, Infallible>(
                    Event::default().data(serde_json::to_string(&data).unwrap()),
                ))
            } else if let Some(pos) = state.queue.get_position(&session_id).await {
                let total = pos.total_waiting;
                let progress = if total > 0 {
                    ((total - pos.position + 1) as f64 / total as f64 * 100.0).min(99.0)
                } else {
                    0.0
                };
                let data = SseData {
                    position: pos.position,
                    total_waiting: pos.total_waiting,
                    eta_seconds: pos.eta_seconds,
                    progress_pct: progress,
                    action: None,
                    redirect_url: None,
                };
                Some(Ok(Event::default().data(serde_json::to_string(&data).unwrap())))
            } else if !state.is_enabled() {
                // Waiting room disabled (schedule ended) — notify client
                let data = SseData {
                    position: 0,
                    total_waiting: 0,
                    eta_seconds: 0.0,
                    progress_pct: 0.0,
                    action: Some("closed".to_string()),
                    redirect_url: None,
                };
                Some(Ok(Event::default().data(serde_json::to_string(&data).unwrap())))
            } else {
                None
            }
        }
    }).filter_map(|x| x);

    let event_stream = initial_stream.chain(update_stream);

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::default()))
}

pub async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let enabled = state.is_enabled();
    let stats = state.queue.stats().await;
    axum::Json(serde_json::json!({
        "enabled": enabled,
        "active_users": stats.active_count,
        "queue_length": stats.waiting_count,
        "total_admitted": stats.total_admitted,
    }))
}

fn extract_session_from_cookie(
    state: &Arc<AppState>,
    req: &axum::extract::Request,
    cookie_name: &str,
) -> Option<SessionId> {
    let cookie_header: &str = req.headers().get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let trimmed: &str = part.trim();
        if let Some(value) = trimmed.strip_prefix(&format!("{}=", cookie_name)) {
            return state.session_mgr.read().verify_token(value.trim());
        }
    }
    None
}

fn format_eta(secs: f64) -> String {
    let secs = secs as u64;
    if secs < 60 {
        "1분 미만".to_string()
    } else if secs < 3600 {
        format!("약 {}분", (secs + 59) / 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600 + 59) / 60;
        format!("약 {}시간 {}분", h, m)
    }
}
