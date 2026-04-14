mod admin;
mod archive_store;
mod backend;
mod config;
mod middleware;
mod proxy;
mod pubsub;
mod queue;
mod reaper;
mod redis_backend;
mod schedule_store;
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
        // Sync HMAC secret across servers
        app_state.sync_hmac_secret().await;
        // Load enabled state from Redis on startup
        app_state.load_enabled_from_redis().await;
        pubsub::spawn_pubsub_listener(config.redis_url.clone(), app_state.clone());
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::PUT, Method::POST, Method::DELETE, Method::PATCH, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::HeaderName::from_static("x-api-key")]);

    let app = Router::new()
        .route("/__wr/events", get(waiting::sse_handler))
        .route("/__wr/status", get(waiting::status_handler))
        .nest("/__wr/admin", admin::admin_router(app_state.clone()))
        .fallback(middleware::gate_handler)
        .layer(cors)
        .with_state(app_state);

    // TCP backlog를 높게 설정 (OS 기본값 128은 동시 접속에 부족)
    let socket = socket2::Socket::new(
        if listen_addr.is_ipv6() { socket2::Domain::IPV6 } else { socket2::Domain::IPV4 },
        socket2::Type::STREAM,
        None,
    )?;
    socket.set_reuse_address(true)?;
    socket.bind(&listen_addr.into())?;
    socket.listen(8192)?;  // backlog: OS가 허용하는 최대값으로 clamp됨
    socket.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(socket.into())?;
    info!("Listening on {} (backlog: 8192)", listen_addr);
    axum::serve(listener, app).await?;

    Ok(())
}
