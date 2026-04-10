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
