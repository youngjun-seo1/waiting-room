use deadpool_redis::redis::cmd;
use tracing::warn;

use crate::scheduler::Schedule;
use crate::state::AppState;

/// Archive a completed schedule. Redis hash `wr:archives` if available, also in-memory.
pub async fn archive_schedule(state: &AppState, schedule: &Schedule) {
    if let Some(pool) = &state.redis_pool {
        if let Ok(json) = serde_json::to_string(schedule) {
            match pool.get().await {
                Ok(mut conn) => {
                    let result: Result<(), _> = cmd("HSET")
                        .arg("wr:archives")
                        .arg(&schedule.id)
                        .arg(&json)
                        .query_async(&mut *conn)
                        .await;
                    if let Err(e) = result {
                        warn!("Redis HSET wr:archives failed: {e}");
                    }
                }
                Err(e) => {
                    warn!("Redis connection failed for archive_schedule: {e}");
                }
            }
        }
    }
    state.archives.write().push(schedule.clone());
}

/// Load all archived schedules.
pub async fn load_archives(state: &AppState) -> Vec<Schedule> {
    if let Some(pool) = &state.redis_pool {
        match pool.get().await {
            Ok(mut conn) => {
                let result: Result<Vec<(String, String)>, _> = cmd("HGETALL")
                    .arg("wr:archives")
                    .query_async(&mut *conn)
                    .await;
                match result {
                    Ok(pairs) => {
                        let mut archives: Vec<Schedule> = pairs
                            .into_iter()
                            .filter_map(|(_, json)| serde_json::from_str(&json).ok())
                            .collect();
                        archives.sort_by(|a, b| b.end_at.cmp(&a.end_at));
                        return archives;
                    }
                    Err(e) => {
                        warn!("Redis HGETALL wr:archives failed: {e}");
                    }
                }
            }
            Err(e) => {
                warn!("Redis connection failed for load_archives: {e}");
            }
        }
    }
    let mut archives = state.archives.read().clone();
    archives.sort_by(|a, b| b.end_at.cmp(&a.end_at));
    archives
}
