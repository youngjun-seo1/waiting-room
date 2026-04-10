use axum::body::Body;
use axum::extract::State;
use axum::http::{Response, header};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::backend::GateResult;
use crate::proxy::forward_request;
use crate::queue::SessionId;
use crate::state::AppState;
use crate::waiting;

pub async fn gate_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response<Body> {
    let enabled = state.config.read().enabled;
    if !enabled {
        return match forward_request(&state, req).await {
            Ok(resp) => resp,
            Err(status) => status.into_response().into(),
        };
    }

    let cookie_name = state.config.read().queue_cookie_name.clone();
    let max_active = state.config.read().max_active_users;
    let ttl_secs = state.config.read().session_ttl_secs;

    // Extract existing session from cookie
    let existing_id = extract_session(&state, &req, &cookie_name);
    let new_id = SessionId::new();

    let result = state
        .queue
        .gate_check(existing_id, new_id, max_active, ttl_secs)
        .await;

    // Determine which session ID was used
    let used_id = existing_id.unwrap_or(new_id);
    let needs_cookie = existing_id.is_none();

    match result {
        GateResult::Active | GateResult::Admitted => {
            let mut resp = match forward_request(&state, req).await {
                Ok(resp) => resp,
                Err(status) => return status.into_response().into(),
            };
            if needs_cookie {
                let token = state.session_mgr.create_token(used_id);
                set_cookie(&mut resp, &cookie_name, &token);
            }
            resp
        }
        GateResult::Waiting { .. } => {
            waiting::serve_waiting_page(&state, used_id).await
        }
        GateResult::Enqueued { .. } => {
            let token = state.session_mgr.create_token(used_id);
            let mut resp = waiting::serve_waiting_page(&state, used_id).await;
            set_cookie(&mut resp, &cookie_name, &token);
            resp
        }
    }
}

fn extract_session(
    state: &Arc<AppState>,
    req: &axum::extract::Request,
    cookie_name: &str,
) -> Option<SessionId> {
    let cookie_header: &str = req.headers().get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let trimmed: &str = part.trim();
        if let Some(value) = trimmed.strip_prefix(&format!("{}=", cookie_name)) {
            return state.session_mgr.verify_token(value.trim());
        }
    }
    None
}

fn set_cookie(resp: &mut Response<Body>, name: &str, value: &str) {
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400",
        name, value
    );
    resp.headers_mut().insert(
        header::SET_COOKIE,
        cookie.parse().expect("valid cookie header"),
    );
}
