use deadpool_redis::redis;
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info};

use crate::state::AppState;

/// Spawns a background task that subscribes to Redis Pub/Sub channel `wr:notify`
/// and forwards each message to the local SSE broadcast channel.
/// Also syncs the enabled state from Redis on each notification.
pub fn spawn_pubsub_listener(redis_url: String, state: Arc<AppState>) {
    let sse_tx = state.sse_tx.clone();
    tokio::spawn(async move {
        loop {
            match run_subscriber(&redis_url, &sse_tx, &state).await {
                Ok(()) => {
                    info!("Redis Pub/Sub subscriber ended, reconnecting...");
                }
                Err(e) => {
                    error!("Redis Pub/Sub error: {}, reconnecting in 1s...", e);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

async fn run_subscriber(
    redis_url: &str,
    sse_tx: &broadcast::Sender<()>,
    state: &Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = redis::Client::open(redis_url)?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe("wr:notify").await?;
    info!("Subscribed to wr:notify");

    let mut stream = pubsub.on_message();
    while let Some(_msg) = stream.next().await {
        // Sync enabled state from Redis so all instances stay consistent
        state.load_enabled_from_redis().await;
        let _ = sse_tx.send(());
    }

    Ok(())
}
