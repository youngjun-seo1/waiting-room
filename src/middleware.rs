use axum::body::Body;
use axum::extract::State;
use axum::http::{Response, header};
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::backend::GateResult;
use crate::proxy::{forward_request, should_redirect};
use crate::queue::SessionId;
use crate::state::AppState;
use crate::waiting;

pub async fn gate_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response<Body> {
    let enabled = state.is_enabled();
    if !enabled {
        // __wr 경로는 admin/SSE 등 내부 경로이므로 통과
        let path = req.uri().path();
        if path.starts_with("/__wr") {
            return match forward_request(&state, req).await {
                Ok(resp) => resp,
                Err(status) => status.into_response().into(),
            };
        }
        return serve_closed_page();
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

    let origin_url = state.config.read().origin_url.clone();
    let req_host = req.headers().get(header::HOST)
        .and_then(|v| v.to_str().ok());
    let use_redirect = should_redirect(&origin_url, req_host);

    match result {
        GateResult::Active | GateResult::Admitted => {
            if use_redirect {
                let path_and_query = req.uri().path_and_query()
                    .map(|pq| pq.as_str())
                    .unwrap_or("/");
                let target = format!("{}{}", origin_url, path_and_query);
                let mut resp = Response::builder()
                    .status(302)
                    .header(header::LOCATION, &target)
                    .body(Body::empty())
                    .unwrap();
                if needs_cookie {
                    let token = state.session_mgr.create_token(used_id);
                    set_cookie(&mut resp, &cookie_name, &token);
                }
                resp
            } else {
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

fn serve_closed_page() -> Response<Body> {
    let html = r#"<!DOCTYPE html>
<html lang="ko">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>이벤트 안내</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .card {
    background: white;
    border-radius: 20px;
    padding: 48px 40px;
    max-width: 440px;
    width: 90%;
    text-align: center;
    box-shadow: 0 20px 60px rgba(0,0,0,0.15);
  }
  .icon {
    font-size: 56px;
    margin-bottom: 20px;
  }
  h1 {
    color: #1a1a2e;
    font-size: 22px;
    font-weight: 700;
    margin-bottom: 12px;
  }
  p {
    color: #6b7280;
    font-size: 15px;
    line-height: 1.6;
  }
</style>
</head>
<body>
  <div class="card">
    <div class="icon">&#128337;</div>
    <h1>현재 이벤트 참여 시간이 아닙니다</h1>
    <p>이벤트 시작 시간에 다시 방문해 주세요.<br>감사합니다.</p>
  </div>
</body>
</html>"#;

    Response::builder()
        .status(200)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap()
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
