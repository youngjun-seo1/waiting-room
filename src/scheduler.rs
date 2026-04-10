use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;
use uuid::Uuid;

use crate::state::AppState;

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
    /// Current phase
    #[serde(skip_deserializing)]
    pub phase: SchedulePhase,
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
}

impl Schedule {
    pub fn new(req: CreateScheduleRequest) -> Self {
        Self {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            name: req.name,
            start_at: req.start_at,
            end_at: req.end_at,
            max_active_users: req.max_active_users,
            phase: SchedulePhase::Pending,
        }
    }
}

/// Returns the current effective schedule state
pub struct ScheduleState {
    pub enabled: bool,
    pub max_active_override: Option<u32>,
    pub active_schedule: Option<String>, // schedule name
    /// A schedule just transitioned from Active to Ended
    pub just_ended: bool,
}

pub fn evaluate_schedules(schedules: &mut Vec<Schedule>) -> ScheduleState {
    let now = Utc::now();
    let mut result = ScheduleState {
        enabled: false,
        max_active_override: None,
        active_schedule: None,
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
            }
            result.enabled = true;
            result.max_active_override = schedule.max_active_users;
            result.active_schedule = Some(schedule.name.clone());
            return result;
        }

        // Future schedule, still pending
    }

    result
}

pub fn spawn_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;

            let schedule_state = {
                let mut schedules = state.schedules.write();
                evaluate_schedules(&mut schedules)
            };

            // Apply schedule state to config
            let mut config = state.config.write();

            if let Some(_name) = &schedule_state.active_schedule {
                // A schedule is active — override config
                config.enabled = true;
                if let Some(max) = schedule_state.max_active_override {
                    config.max_active_users = max;
                }
            } else if schedule_state.just_ended {
                // Schedule just ended with no other active schedule — disable
                config.enabled = false;
            }
        }
    });
}
