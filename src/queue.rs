use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[allow(dead_code)]
pub struct ActiveSession {
    pub session_id: SessionId,
    pub admitted_at: Instant,
    pub last_seen: AtomicU64, // nanos since queue creation (base_instant)
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct QueueEntry {
    pub session_id: SessionId,
    pub joined_at: Instant,
}

pub struct WaitingQueue {
    base_instant: Instant,
    active: HashMap<SessionId, ActiveSession>,
    waiting: VecDeque<QueueEntry>,
    waiting_index: HashMap<SessionId, usize>,
    generation: u64,
    // ETA tracking
    total_active_duration_secs: f64,
    completed_sessions: u64,
    total_admitted: u64,
    total_visitors: u64,
}

pub struct QueuePosition {
    pub position: usize, // 1-based
    pub total_waiting: usize,
    pub eta_seconds: f64,
}

pub struct QueueStats {
    pub active_count: usize,
    pub waiting_count: usize,
    pub avg_active_duration_secs: f64,
    pub total_admitted: u64,
    pub total_visitors: u64,
}

impl WaitingQueue {
    pub fn new() -> Self {
        Self {
            base_instant: Instant::now(),
            active: HashMap::new(),
            waiting: VecDeque::new(),
            waiting_index: HashMap::new(),
            generation: 0,
            total_active_duration_secs: 0.0,
            completed_sessions: 0,
            total_admitted: 0,
            total_visitors: 0,
        }
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    #[cfg(test)]
    pub fn waiting_count(&self) -> usize {
        self.waiting.len()
    }

    pub fn is_active(&self, id: &SessionId) -> bool {
        self.active.contains_key(id)
    }

    pub fn is_waiting(&self, id: &SessionId) -> bool {
        self.waiting_index.contains_key(id)
    }

    pub fn touch(&self, id: &SessionId) {
        if let Some(session) = self.active.get(id) {
            let now_nanos = self.base_instant.elapsed().as_nanos() as u64;
            session.last_seen.store(now_nanos, Ordering::Relaxed);
        }
    }

    pub fn admit(&mut self, id: SessionId) {
        let now_nanos = self.base_instant.elapsed().as_nanos() as u64;
        self.active.insert(
            id,
            ActiveSession {
                session_id: id,
                admitted_at: Instant::now(),
                last_seen: AtomicU64::new(now_nanos),
            },
        );
        self.total_admitted += 1;
    }

    /// Record a new visitor (call when a new session first enters the system)
    pub fn record_visitor(&mut self) {
        self.total_visitors += 1;
    }

    pub fn enqueue(&mut self, id: SessionId) {
        let pos = self.waiting.len();
        self.waiting.push_back(QueueEntry {
            session_id: id,
            joined_at: Instant::now(),
        });
        self.waiting_index.insert(id, pos);
    }

    pub fn get_position(&self, id: &SessionId) -> Option<QueuePosition> {
        let pos = self.waiting_index.get(id)?;
        let position = pos + 1; // 1-based
        let eta = self.estimate_wait(position);
        Some(QueuePosition {
            position,
            total_waiting: self.waiting.len(),
            eta_seconds: eta,
        })
    }

    fn estimate_wait(&self, position: usize) -> f64 {
        let avg_duration = if self.completed_sessions > 0 {
            self.total_active_duration_secs / self.completed_sessions as f64
        } else {
            300.0 // default 5 min if no data yet
        };
        let max_active = self.active.len().max(1) as f64;
        (position as f64 / max_active) * avg_duration
    }

    /// Expire sessions not seen within ttl_secs. Returns number of expired sessions.
    pub fn expire_stale(&mut self, ttl_secs: u64) -> usize {
        let now_nanos = self.base_instant.elapsed().as_nanos() as u64;
        let ttl_nanos = ttl_secs * 1_000_000_000;

        let expired: Vec<SessionId> = self
            .active
            .iter()
            .filter(|(_, s)| {
                let last = s.last_seen.load(Ordering::Relaxed);
                now_nanos.saturating_sub(last) > ttl_nanos
            })
            .map(|(id, _)| *id)
            .collect();

        let count = expired.len();
        for id in &expired {
            if let Some(session) = self.active.remove(id) {
                let duration = session.admitted_at.elapsed().as_secs_f64();
                self.total_active_duration_secs += duration;
                self.completed_sessions += 1;
            }
        }
        count
    }

    /// Admit up to `slots` users from the waiting queue. Returns admitted session IDs.
    pub fn admit_from_queue(&mut self, slots: usize) -> Vec<SessionId> {
        let mut admitted = Vec::with_capacity(slots);
        for _ in 0..slots {
            if let Some(entry) = self.waiting.pop_front() {
                self.waiting_index.remove(&entry.session_id);
                self.admit(entry.session_id);
                admitted.push(entry.session_id);
            } else {
                break;
            }
        }
        if !admitted.is_empty() {
            self.rebuild_index();
        }
        admitted
    }

    fn rebuild_index(&mut self) {
        self.waiting_index.clear();
        for (i, entry) in self.waiting.iter().enumerate() {
            self.waiting_index.insert(entry.session_id, i);
        }
        self.generation += 1;
    }

    pub fn stats(&self) -> QueueStats {
        QueueStats {
            active_count: self.active.len(),
            waiting_count: self.waiting.len(),
            avg_active_duration_secs: if self.completed_sessions > 0 {
                self.total_active_duration_secs / self.completed_sessions as f64
            } else {
                0.0
            },
            total_admitted: self.total_admitted,
            total_visitors: self.total_visitors,
        }
    }

    pub fn flush(&mut self) {
        self.active.clear();
        self.waiting.clear();
        self.waiting_index.clear();
        self.generation += 1;
        self.total_active_duration_secs = 0.0;
        self.completed_sessions = 0;
        self.total_admitted = 0;
        self.total_visitors = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fifo_ordering() {
        let mut q = WaitingQueue::new();
        let ids: Vec<SessionId> = (0..5).map(|_| SessionId::new()).collect();

        for id in &ids {
            q.enqueue(*id);
        }

        assert_eq!(q.waiting_count(), 5);

        // Check positions are 1-based and ordered
        for (i, id) in ids.iter().enumerate() {
            let pos = q.get_position(id).unwrap();
            assert_eq!(pos.position, i + 1);
        }

        // Admit 2 from queue
        let admitted = q.admit_from_queue(2);
        assert_eq!(admitted.len(), 2);
        assert_eq!(admitted[0], ids[0]);
        assert_eq!(admitted[1], ids[1]);
        assert_eq!(q.active_count(), 2);
        assert_eq!(q.waiting_count(), 3);

        // Remaining positions should be updated
        let pos = q.get_position(&ids[2]).unwrap();
        assert_eq!(pos.position, 1);
    }

    #[test]
    fn test_admit_direct() {
        let mut q = WaitingQueue::new();
        let id = SessionId::new();
        q.admit(id);
        assert!(q.is_active(&id));
        assert!(!q.is_waiting(&id));
        assert_eq!(q.active_count(), 1);
    }

    #[test]
    fn test_touch_updates_last_seen() {
        let mut q = WaitingQueue::new();
        let id = SessionId::new();
        q.admit(id);

        let before = q.active.get(&id).unwrap().last_seen.load(Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(10));
        q.touch(&id);
        let after = q.active.get(&id).unwrap().last_seen.load(Ordering::Relaxed);
        assert!(after > before);
    }
}
