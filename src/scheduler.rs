use chrono::{DateTime, Utc};

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
    /// Current phase
    #[serde(skip_deserializing)]
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
    pub active_schedule: Option<String>, // schedule name
    pub active_schedule_id: Option<String>,
    /// A schedule just transitioned to Active
    pub just_started: bool,
    /// A schedule just transitioned from Active to Ended
    pub just_ended: bool,
}

pub fn evaluate_schedules(schedules: &mut Vec<Schedule>) -> ScheduleState {
    let now = Utc::now();
    let mut result = ScheduleState {
        enabled: false,
        max_active_override: None,
        origin_url_override: None,
        active_schedule: None,
        active_schedule_id: None,
        just_started: false,
        just_ended: false,
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
            result.active_schedule = Some(schedule.name.clone());
            result.active_schedule_id = Some(schedule.id.clone());
            return result;
        }

        // Future schedule, still pending
    }

    result
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

            // Apply schedule state to config (lock scoped to block)
            {
                let mut config = state.config.write();
                if let Some(_name) = &schedule_state.active_schedule {
                    config.enabled = true;
                    if let Some(max) = schedule_state.max_active_override {
                        config.max_active_users = max;
                    }
                    if let Some(url) = &schedule_state.origin_url_override {
                        config.origin_url = url.clone();
                    }
                } else if schedule_state.just_ended {
                    config.enabled = false;
                }
            }

            // Flush queue on schedule start (clean slate)
            if schedule_state.just_started {
                state.queue.flush().await;
                info!("schedule started: queue flushed for clean start");
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
            if schedule_state.just_ended && schedule_state.active_schedule.is_none() {
                // Final stats snapshot before flush
                let queue_stats = state.queue.stats().await;
                {
                    let mut schedules = state.schedules.write();
                    for schedule in schedules.iter_mut() {
                        if schedule.phase == SchedulePhase::Ended {
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
                }
                // Persist final stats to Redis
                crate::schedule_store::save_all_schedules(&state).await;

                state.queue.flush().await;
                state.notify_queue_update();
                info!("schedule ended: queue flushed, clients notified");
            }

            // Periodically persist stats to Redis (every 10 seconds)
            if schedule_state.active_schedule_id.is_some() && tick % 10 == 0 {
                crate::schedule_store::save_all_schedules(&state).await;
            }
        }
    });
}
