use deadpool_redis::redis::cmd;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::state::AppState;

pub fn spawn_reaper(state: Arc<AppState>) {
    let server_id = uuid::Uuid::new_v4().to_string();

    tokio::spawn(async move {
        loop {
            let interval_secs = state.config.read().advanced.reaper_interval_secs;
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;

            let enabled = state.is_enabled();
            if !enabled {
                continue;
            }

            // Leader election: only one server runs the reaper when using Redis
            if let Some(pool) = &state.redis_pool {
                match pool.get().await {
                    Ok(mut conn) => {
                        let acquired: Option<String> = cmd("SET")
                            .arg("wr:reaper:lock")
                            .arg(&server_id)
                            .arg("NX")
                            .arg("EX")
                            .arg(10)
                            .query_async(&mut *conn)
                            .await
                            .ok()
                            .flatten();
                        if acquired.is_none() {
                            continue; // Another server holds the lock
                        }
                    }
                    Err(_) => continue,
                }
            }

            let ttl_secs = state.config.read().session_ttl_secs;
            let max_active = state.config.read().max_active_users;

            let (expired, admitted) = state.queue.reaper_cycle(ttl_secs, max_active).await;

            if expired > 0 || admitted > 0 {
                let stats = state.queue.stats().await;
                info!(
                    expired = expired,
                    admitted = admitted,
                    active = stats.active_count,
                    waiting = stats.waiting_count,
                    "reaper cycle"
                );
            }

            state.notify_queue_update();
        }
    });
}
