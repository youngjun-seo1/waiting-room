use async_trait::async_trait;
use parking_lot::RwLock;

use crate::queue::{QueuePosition, QueueStats, SessionId, WaitingQueue};

#[allow(dead_code)]
pub enum GateResult {
    Active,
    Waiting { position: usize, total: usize },
    Admitted,
    Enqueued { position: usize, total: usize },
}

#[async_trait]
pub trait QueueBackend: Send + Sync + 'static {
    /// Combined gate check: is_active/touch, is_waiting, admit/enqueue.
    /// `id` is Some if the request has a valid session cookie.
    /// `new_id` is used if a new session needs to be created.
    async fn gate_check(
        &self,
        id: Option<SessionId>,
        new_id: SessionId,
        max_active: u32,
        ttl_secs: u64,
    ) -> GateResult;

    async fn get_position(&self, id: &SessionId) -> Option<QueuePosition>;
    async fn is_active(&self, id: &SessionId) -> bool;
    async fn stats(&self) -> QueueStats;
    async fn flush(&self);

    /// Reaper cycle: expire stale sessions, admit from queue.
    /// Returns (expired_count, admitted_count).
    async fn reaper_cycle(&self, ttl_secs: u64, max_active: u32) -> (usize, usize);
}

// --- In-memory backend ---

pub struct MemoryBackend {
    queue: RwLock<WaitingQueue>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self {
            queue: RwLock::new(WaitingQueue::new()),
        }
    }
}

#[async_trait]
impl QueueBackend for MemoryBackend {
    async fn gate_check(
        &self,
        id: Option<SessionId>,
        new_id: SessionId,
        max_active: u32,
        _ttl_secs: u64,
    ) -> GateResult {
        if let Some(id) = id {
            // Check active (read lock)
            {
                let q = self.queue.read();
                if q.is_active(&id) {
                    q.touch(&id);
                    return GateResult::Active;
                }
                if q.is_waiting(&id) {
                    let pos = q.get_position(&id).unwrap();
                    return GateResult::Waiting {
                        position: pos.position,
                        total: pos.total_waiting,
                    };
                }
            }
            // Not found — try admit or enqueue (write lock)
            let mut q = self.queue.write();
            if (q.active_count() as u32) < max_active {
                q.admit(id);
                GateResult::Admitted
            } else {
                q.enqueue(id);
                let pos = q.get_position(&id).unwrap();
                GateResult::Enqueued {
                    position: pos.position,
                    total: pos.total_waiting,
                }
            }
        } else {
            // New session
            let mut q = self.queue.write();
            if (q.active_count() as u32) < max_active {
                q.admit(new_id);
                GateResult::Admitted
            } else {
                q.enqueue(new_id);
                let pos = q.get_position(&new_id).unwrap();
                GateResult::Enqueued {
                    position: pos.position,
                    total: pos.total_waiting,
                }
            }
        }
    }

    async fn get_position(&self, id: &SessionId) -> Option<QueuePosition> {
        self.queue.read().get_position(id)
    }

    async fn is_active(&self, id: &SessionId) -> bool {
        self.queue.read().is_active(id)
    }

    async fn stats(&self) -> QueueStats {
        self.queue.read().stats()
    }

    async fn flush(&self) {
        self.queue.write().flush();
    }

    async fn reaper_cycle(&self, ttl_secs: u64, max_active: u32) -> (usize, usize) {
        let mut q = self.queue.write();
        let expired = q.expire_stale(ttl_secs);
        let slots = (max_active as usize).saturating_sub(q.active_count());
        let admitted = q.admit_from_queue(slots);
        (expired, admitted.len())
    }
}
