mod admin;
mod backend;
mod config;
mod middleware;
mod proxy;
mod pubsub;
mod queue;
mod reaper;
mod redis_backend;
mod scheduler;
mod session;
mod state;
mod waiting;

use axum::Router;
use axum::http::{Method, header};
use axum::routing::get;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "waiting_room=info".into()),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".to_string());

    let config = config::Config::load(&config_path)?;
    let listen_addr = config.listen_addr;

    info!("Starting waiting room on {}", listen_addr);
    info!("Origin: {}", config.origin_url);
    info!("Max active users: {}", config.max_active_users);

    // Select backend based on config
    let (queue_backend, redis_pool): (Arc<dyn backend::QueueBackend>, _) =
        if config.redis_url.is_empty() {
            info!("Backend: in-memory");
            (Arc::new(backend::MemoryBackend::new()), None)
        } else {
            info!("Backend: Redis ({})", config.redis_url);
            let rb = redis_backend::RedisBackend::new(&config.redis_url).await?;
            let pool = rb.pool().clone();
            (Arc::new(rb), Some(pool))
        };

    let app_state = Arc::new(state::AppState::new(config.clone(), queue_backend, redis_pool));

    // Spawn reaper
    reaper::spawn_reaper(app_state.clone());

    // Spawn scheduler
    scheduler::spawn_scheduler(app_state.clone());

    // Spawn Redis Pub/Sub listener if Redis mode
    if !config.redis_url.is_empty() {
        pubsub::spawn_pubsub_listener(config.redis_url.clone(), app_state.sse_tx.clone());
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::PUT, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::HeaderName::from_static("x-api-key")]);

    let app = Router::new()
        .route("/__wr/events", get(waiting::sse_handler))
        .route("/__wr/status", get(waiting::status_handler))
        .nest("/__wr/admin", admin::admin_router(app_state.clone()))
        .fallback(middleware::gate_handler)
        .layer(cors)
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    info!("Listening on {}", listen_addr);
    axum::serve(listener, app).await?;

    Ok(())
}
