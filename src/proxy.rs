use axum::body::Body;
use axum::http::{Uri, header};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::sync::Arc;

use crate::state::AppState;

pub type HttpClient = Client<hyper_util::client::legacy::connect::HttpConnector, Body>;

pub fn create_http_client() -> HttpClient {
    Client::builder(TokioExecutor::new()).build_http()
}

/// Returns true if the origin host differs from the request host.
/// Same domain → reverse proxy, different domain → redirect.
pub fn should_redirect(origin_url: &str, req_host: Option<&str>) -> bool {
    let origin_host = origin_url
        .split("://")
        .nth(1)
        .unwrap_or(origin_url)
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    let req_host = req_host
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    if origin_host.is_empty() || req_host.is_empty() {
        return false;
    }

    origin_host != req_host
}

pub async fn forward_request(
    state: &Arc<AppState>,
    req: axum::extract::Request,
) -> Result<axum::http::Response<Body>, axum::http::StatusCode> {
    let origin_url = state.config.read().origin_url.clone();

    let (mut parts, body) = req.into_parts();

    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let uri = format!("{}{}", origin_url, path_and_query);
    parts.uri = uri
        .parse::<Uri>()
        .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;

    parts.headers.remove(header::HOST);

    let req = axum::http::Request::from_parts(parts, body);

    let resp = state
        .http_client
        .request(req)
        .await
        .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;

    let (parts, body) = resp.into_parts();
    let body = Body::new(body);
    Ok(axum::http::Response::from_parts(parts, body))
}
