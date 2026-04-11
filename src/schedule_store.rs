use chrono::Utc;
use deadpool_redis::redis::cmd;
use tracing::warn;

use crate::scheduler::{Schedule, SchedulePhase};
use crate::state::AppState;

/// Recompute phase from timestamps to avoid spurious log transitions after Redis load
fn recompute_phase(schedule: &mut Schedule) {
    let now = Utc::now();
    if now >= schedule.end_at {
        schedule.phase = SchedulePhase::Ended;
    } else if now >= schedule.start_at {
        schedule.phase = SchedulePhase::Active;
    }
    // else stays Pending (the default)
}

/// Load all schedules. Redis if available, otherwise in-memory.
pub async fn load_schedules(state: &AppState) -> Vec<Schedule> {
    if let Some(pool) = &state.redis_pool {
        match pool.get().await {
            Ok(mut conn) => {
                let result: Result<Vec<(String, String)>, _> = cmd("HGETALL")
                    .arg("wr:schedules")
                    .query_async(&mut *conn)
                    .await;
                match result {
                    Ok(pairs) => {
                        let mut schedules: Vec<Schedule> = pairs
                            .into_iter()
                            .filter_map(|(_, json)| serde_json::from_str(&json).ok())
                            .collect();
                        for s in &mut schedules {
                            recompute_phase(s);
                        }
                        // Update local cache
                        *state.schedules.write() = schedules.clone();
                        return schedules;
                    }
                    Err(e) => {
                        warn!("Redis HGETALL wr:schedules failed: {e}, using local cache");
                    }
                }
            }
            Err(e) => {
                warn!("Redis connection failed for schedules: {e}, using local cache");
            }
        }
    }
    state.schedules.read().clone()
}

/// Save a schedule. Redis if available, always updates in-memory.
pub async fn save_schedule(state: &AppState, schedule: &Schedule) {
    if let Some(pool) = &state.redis_pool {
        if let Ok(json) = serde_json::to_string(schedule) {
            match pool.get().await {
                Ok(mut conn) => {
                    let result: Result<(), _> = cmd("HSET")
                        .arg("wr:schedules")
                        .arg(&schedule.id)
                        .arg(&json)
                        .query_async(&mut *conn)
                        .await;
                    if let Err(e) = result {
                        warn!("Redis HSET wr:schedules failed: {e}");
                    }
                }
                Err(e) => {
                    warn!("Redis connection failed for save_schedule: {e}");
                }
            }
        }
    }
    state.schedules.write().push(schedule.clone());
}

/// Remove a schedule by ID. Returns true if found.
pub async fn remove_schedule(state: &AppState, id: &str) -> bool {
    let mut removed = false;

    if let Some(pool) = &state.redis_pool {
        match pool.get().await {
            Ok(mut conn) => {
                let result: Result<i64, _> = cmd("HDEL")
                    .arg("wr:schedules")
                    .arg(id)
                    .query_async(&mut *conn)
                    .await;
                match result {
                    Ok(n) if n > 0 => removed = true,
                    Err(e) => {
                        warn!("Redis HDEL wr:schedules failed: {e}");
                    }
                    _ => {}
                }
            }
            Err(e) => {
                warn!("Redis connection failed for remove_schedule: {e}");
            }
        }
    }

    let mut schedules = state.schedules.write();
    let before = schedules.len();
    schedules.retain(|s| s.id != id);
    if schedules.len() < before {
        removed = true;
    }

    removed
}
