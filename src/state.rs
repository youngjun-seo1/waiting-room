use deadpool_redis::{Pool, redis::cmd};
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::broadcast;

use crate::backend::QueueBackend;
use crate::config::Config;
use crate::proxy::{HttpClient, create_http_client};
use crate::scheduler::Schedule;
use crate::session::SessionManager;

pub struct AppState {
    pub config: RwLock<Config>,
    pub queue: Arc<dyn QueueBackend>,
    pub session_mgr: RwLock<SessionManager>,
    pub sse_tx: broadcast::Sender<()>,
    pub http_client: HttpClient,
    pub redis_pool: Option<Pool>,
    pub schedules: RwLock<Vec<Schedule>>,
    pub archives: RwLock<Vec<Schedule>>,
    pub enabled: AtomicBool,
}

impl AppState {
    pub fn new(config: Config, queue: Arc<dyn QueueBackend>, redis_pool: Option<Pool>) -> Self {
        let secret = generate_hmac_secret();
        let (sse_tx, _) = broadcast::channel(1024);
        Self {
            config: RwLock::new(config),
            queue,
            session_mgr: RwLock::new(SessionManager::new(&secret)),
            sse_tx,
            http_client: create_http_client(),
            redis_pool,
            schedules: RwLock::new(Vec::new()),
            archives: RwLock::new(Vec::new()),
            enabled: AtomicBool::new(false),
        }
    }

    /// Redis 모드에서 HMAC secret을 서버 간 공유.
    /// 첫 번째 서버가 secret을 Redis에 저장하고, 이후 서버들은 Redis에서 가져옴.
    pub async fn sync_hmac_secret(&self) {
        let Some(pool) = &self.redis_pool else { return };
        let Ok(mut conn) = pool.get().await else { return };

        // Redis에서 기존 secret 가져오기 시도
        let existing: Option<Vec<u8>> = cmd("GET")
            .arg("wr:hmac_secret")
            .query_async(&mut *conn)
            .await
            .ok()
            .flatten();

        if let Some(secret) = existing {
            *self.session_mgr.write() = crate::session::SessionManager::new(&secret);
            tracing::info!("HMAC secret loaded from Redis");
        } else {
            // SET NX로 원자적 생성 (동시 시작 시 하나만 성공)
            let secret = generate_hmac_secret();
            let set_result: Option<String> = cmd("SET")
                .arg("wr:hmac_secret")
                .arg(&secret)
                .arg("NX")
                .query_async(&mut *conn)
                .await
                .ok()
                .flatten();

            if set_result.is_some() {
                // 내가 첫 번째 → 내 secret 사용
                *self.session_mgr.write() = crate::session::SessionManager::new(&secret);
                tracing::info!("HMAC secret created and stored in Redis");
            } else {
                // 다른 서버가 먼저 저장함 → Redis에서 로드
                let shared: Vec<u8> = cmd("GET")
                    .arg("wr:hmac_secret")
                    .query_async(&mut *conn)
                    .await
                    .unwrap();
                *self.session_mgr.write() = crate::session::SessionManager::new(&shared);
                tracing::info!("HMAC secret loaded from Redis (race resolved)");
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Set enabled state and persist to Redis (if Redis mode).
    /// Also publishes the change via `wr:notify` so other instances sync.
    pub async fn set_enabled_sync(&self, val: bool) {
        self.enabled.store(val, Ordering::Relaxed);
        if let Some(pool) = &self.redis_pool {
            if let Ok(mut conn) = pool.get().await {
                let _: Result<(), _> = cmd("SET")
                    .arg("wr:enabled")
                    .arg(if val { "1" } else { "0" })
                    .query_async(&mut *conn)
                    .await;
            }
        }
    }

    /// Load enabled state from Redis into the local AtomicBool cache.
    /// Called on startup and by pubsub listener to stay in sync.
    pub async fn load_enabled_from_redis(&self) {
        if let Some(pool) = &self.redis_pool {
            if let Ok(mut conn) = pool.get().await {
                let val: Option<String> = cmd("GET")
                    .arg("wr:enabled")
                    .query_async(&mut *conn)
                    .await
                    .ok();
                let enabled = val.as_deref() == Some("1");
                self.enabled.store(enabled, Ordering::Relaxed);
            }
        }
    }

    pub fn notify_queue_update(&self) {
        // Local broadcast
        let _ = self.sse_tx.send(());

        // Redis publish (if Redis mode)
        if let Some(pool) = &self.redis_pool {
            let pool = pool.clone();
            tokio::spawn(async move {
                if let Ok(mut conn) = pool.get().await {
                    let _: Result<(), _> = cmd("PUBLISH")
                        .arg("wr:notify")
                        .arg("update")
                        .query_async(&mut *conn)
                        .await;
                }
            });
        }
    }
}

fn generate_hmac_secret() -> Vec<u8> {
    use rand::Rng;
    let mut rng = rand::rng();
    let mut secret = vec![0u8; 32];
    rng.fill(&mut secret[..]);
    secret
}
