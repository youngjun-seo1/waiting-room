use chrono::{DateTime, Utc};
use deadpool_redis::redis::cmd;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduleStats {
    pub peak_active_users: usize,
    pub peak_queue_length: usize,
    pub total_admitted: u64,
    pub total_visitors: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    /// When the waiting room activates (queuing + admission begins)
    pub start_at: DateTime<Utc>,
    /// When the waiting room closes (all traffic passes through)
    pub end_at: DateTime<Utc>,
    /// Override max_active_users for this schedule
    #[serde(default)]
    pub max_active_users: Option<u32>,
    /// Override origin_url for this schedule
    #[serde(default)]
    pub origin_url: Option<String>,
    /// Override session_ttl_secs for this schedule (defaults to server config)
    #[serde(default)]
    pub session_ttl_secs: Option<u64>,
    /// Current phase
    #[serde(default)]
    pub phase: SchedulePhase,
    /// Per-schedule statistics
    #[serde(default)]
    pub stats: ScheduleStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SchedulePhase {
    #[default]
    Pending,
    Active,   // start_at reached: waiting room on, admitting from queue
    Ended,
}

#[derive(Deserialize)]
pub struct CreateScheduleRequest {
    pub name: String,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub max_active_users: Option<u32>,
    pub origin_url: Option<String>,
    pub session_ttl_secs: Option<u64>,
}

impl Schedule {
    pub fn new(req: CreateScheduleRequest) -> Self {
        Self {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            name: req.name,
            start_at: req.start_at,
            end_at: req.end_at,
            max_active_users: req.max_active_users,
            origin_url: req.origin_url,
            session_ttl_secs: req.session_ttl_secs,
            phase: SchedulePhase::Pending,
            stats: ScheduleStats::default(),
        }
    }
}

/// Returns the current effective schedule state
pub struct ScheduleState {
    pub enabled: bool,
    pub max_active_override: Option<u32>,
    pub origin_url_override: Option<String>,
    pub session_ttl_override: Option<u64>,
    pub active_schedule: Option<String>, // schedule name
    pub active_schedule_id: Option<String>,
    /// A schedule just transitioned to Active
    pub just_started: bool,
    /// A schedule just transitioned from Active to Ended
    pub just_ended: bool,
    /// IDs of schedules that have ended (for one-time cleanup)
    pub ended_schedule_ids: Vec<String>,
}

pub fn evaluate_schedules(schedules: &mut Vec<Schedule>) -> ScheduleState {
    let now = Utc::now();
    let mut result = ScheduleState {
        enabled: false,
        max_active_override: None,
        origin_url_override: None,
        session_ttl_override: None,
        active_schedule: None,
        active_schedule_id: None,
        just_started: false,
        just_ended: false,
        ended_schedule_ids: Vec::new(),
    };

    for schedule in schedules.iter_mut() {
        if now >= schedule.end_at {
            if schedule.phase != SchedulePhase::Ended {
                let was_active = schedule.phase == SchedulePhase::Active;
                info!(name = %schedule.name, "schedule ended");
                schedule.phase = SchedulePhase::Ended;
                if was_active {
                    result.just_ended = true;
                }
            }
            result.ended_schedule_ids.push(schedule.id.clone());
            continue;
        }

        if now >= schedule.start_at {
            if schedule.phase != SchedulePhase::Active {
                info!(name = %schedule.name, "schedule active");
                schedule.phase = SchedulePhase::Active;
                result.just_started = true;
            }
            result.enabled = true;
            result.max_active_override = schedule.max_active_users;
            result.origin_url_override = schedule.origin_url.clone();
            result.session_ttl_override = schedule.session_ttl_secs;
            result.active_schedule = Some(schedule.name.clone());
            result.active_schedule_id = Some(schedule.id.clone());
            return result;
        }

        // Future schedule, still pending
    }

    result
}

/// Attempt to claim a one-time event for a schedule via Redis SETNX.
/// Returns true only for the first caller (across all instances).
/// For memory-only mode, always returns true (single instance).
async fn try_claim_once(state: &AppState, key: &str) -> bool {
    if let Some(pool) = &state.redis_pool {
        if let Ok(mut conn) = pool.get().await {
            let acquired: Option<String> = cmd("SET")
                .arg(key)
                .arg("1")
                .arg("NX")
                .arg("EX")
                .arg(86400) // expire after 24h
                .query_async(&mut *conn)
                .await
                .ok()
                .flatten();
            return acquired.is_some();
        }
        return false;
    }
    true
}

pub fn spawn_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick: u64 = 0;

        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            tick += 1;

            let schedule_state = if state.redis_pool.is_some() {
                let mut schedules = crate::schedule_store::load_schedules(&state).await;
                let result = evaluate_schedules(&mut schedules);
                *state.schedules.write() = schedules;
                result
            } else {
                let mut schedules = state.schedules.write();
                evaluate_schedules(&mut schedules)
            };

            // Apply schedule state (persist to Redis if available)
            if let Some(_name) = &schedule_state.active_schedule {
                state.set_enabled_sync(true).await;
                let mut config = state.config.write();
                if let Some(max) = schedule_state.max_active_override {
                    config.max_active_users = max;
                }
                if let Some(url) = &schedule_state.origin_url_override {
                    config.origin_url = url.clone();
                }
                if let Some(ttl) = schedule_state.session_ttl_override {
                    config.session_ttl_secs = ttl;
                }
            } else {
                state.set_enabled_sync(false).await;
            }

            // Flush queue on schedule start (clean slate)
            if let Some(ref schedule_id) = schedule_state.active_schedule_id {
                // Memory mode: flush only on transition event
                // Redis mode: use SETNX to ensure flush happens exactly once per schedule,
                // even across multiple instances or server restarts.
                let should_flush = if state.redis_pool.is_some() {
                    try_claim_once(&state, &format!("wr:flushed:{}", schedule_id)).await
                } else {
                    schedule_state.just_started
                };
                if should_flush {
                    state.queue.flush().await;
                    state.notify_queue_update();
                    info!("schedule started: queue flushed for clean start");
                }
            }

            // Update stats for active schedule
            if let Some(ref schedule_id) = schedule_state.active_schedule_id {
                let queue_stats = state.queue.stats().await;
                let mut schedules = state.schedules.write();
                if let Some(schedule) = schedules.iter_mut().find(|s| s.id == *schedule_id) {
                    if queue_stats.active_count > schedule.stats.peak_active_users {
                        schedule.stats.peak_active_users = queue_stats.active_count;
                    }
                    if queue_stats.waiting_count > schedule.stats.peak_queue_length {
                        schedule.stats.peak_queue_length = queue_stats.waiting_count;
                    }
                    schedule.stats.total_admitted = queue_stats.total_admitted;
                    schedule.stats.total_visitors = queue_stats.total_visitors;
                }
            }

            // Capture final stats and flush queue on schedule end
            // Use Redis SETNX to ensure cleanup happens exactly once per schedule,
            // even across multiple instances or after server restarts.
            if schedule_state.active_schedule.is_none() {
                for ended_id in &schedule_state.ended_schedule_ids {
                    let key = format!("wr:ended:{}", ended_id);
                    let should_cleanup = if state.redis_pool.is_some() {
                        try_claim_once(&state, &key).await
                    } else {
                        schedule_state.just_ended
                    };
                    if should_cleanup {
                        // Final stats snapshot before flush
                        let queue_stats = state.queue.stats().await;
                        {
                            let mut schedules = state.schedules.write();
                            if let Some(schedule) = schedules.iter_mut().find(|s| s.id == *ended_id) {
                                if queue_stats.active_count > schedule.stats.peak_active_users {
                                    schedule.stats.peak_active_users = queue_stats.active_count;
                                }
                                if queue_stats.waiting_count > schedule.stats.peak_queue_length {
                                    schedule.stats.peak_queue_length = queue_stats.waiting_count;
                                }
                                schedule.stats.total_admitted = queue_stats.total_admitted;
                                schedule.stats.total_visitors = queue_stats.total_visitors;
                            }
                        }
                        // Persist final stats to Redis
                        crate::schedule_store::save_all_schedules(&state).await;

                        state.queue.flush().await;
                        state.notify_queue_update();
                        info!(schedule_id = %ended_id, "schedule ended: queue flushed, clients notified");
                        break; // only flush once per tick
                    }
                }
            }

            // Periodically persist stats to Redis (every 10 seconds)
            if schedule_state.active_schedule_id.is_some() && tick % 10 == 0 {
                crate::schedule_store::save_all_schedules(&state).await;
            }

            // Archive and remove expired ended schedules (every 60 seconds)
            if tick % 60 == 0 {
                let retention_secs = state.config.read().advanced.schedule_retention_secs;
                let cutoff = Utc::now() - chrono::Duration::seconds(retention_secs as i64);
                let expired: Vec<Schedule> = {
                    let schedules = state.schedules.read();
                    schedules.iter()
                        .filter(|s| s.phase == SchedulePhase::Ended && s.end_at < cutoff)
                        .cloned()
                        .collect()
                };
                for schedule in &expired {
                    crate::archive_store::archive_schedule(&state, schedule).await;
                    crate::schedule_store::remove_schedule(&state, &schedule.id).await;
                    info!(schedule_id = %schedule.id, name = %schedule.name, "schedule archived and removed");
                }
            }
        }
    });
}
