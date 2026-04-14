use async_trait::async_trait;
use deadpool_redis::{Config as RedisConfig, Pool, Runtime, redis::cmd};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::{GateResult, QueueBackend};
use crate::queue::{QueuePosition, QueueStats, SessionId};

pub struct RedisBackend {
    pool: Pool,
}

impl RedisBackend {
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    pub async fn new(redis_url: &str) -> anyhow::Result<Self> {
        Self::with_pool_size(redis_url, 64).await
    }

    pub async fn with_pool_size(redis_url: &str, pool_size: usize) -> anyhow::Result<Self> {
        let mut cfg = RedisConfig::from_url(redis_url);
        cfg.pool = Some(deadpool_redis::PoolConfig {
            max_size: pool_size,
            ..Default::default()
        });
        let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
        tracing::info!("Redis pool size: {}", pool_size);

        // Test connection
        let mut conn = pool.get().await?;
        let _: String = cmd("PING").query_async(&mut *conn).await?;
        tracing::info!("Redis connection OK");

        Ok(Self { pool })
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn session_id_str(id: &SessionId) -> String {
    id.0.to_string()
}

// Lua scripts
const GATE_CHECK_LUA: &str = r#"
local id = ARGV[1]
local max_active = tonumber(ARGV[2])
local ttl_secs = tonumber(ARGV[3])
local now_ms = ARGV[4]
local new_id = ARGV[5]

-- If we have an existing session, check it
if id ~= '' then
    -- Check active
    if redis.call('EXISTS', 'wr:active:' .. id .. ':ls') == 1 then
        redis.call('EXPIRE', 'wr:active:' .. id .. ':ls', ttl_secs)
        return {'active', '0', '0'}
    end
    -- Check waiting
    local rank = redis.call('ZRANK', 'wr:waiting', id)
    if rank then
        local total = redis.call('ZCARD', 'wr:waiting')
        return {'waiting', tostring(rank + 1), tostring(total)}
    end
end

-- Use new_id if no existing id, otherwise use existing id
local use_id = id
if id == '' then
    use_id = new_id
end

-- Try admit or enqueue
redis.call('HINCRBY', 'wr:stats', 'total_visitors', 1)
local count = redis.call('HLEN', 'wr:active')
if count < max_active then
    redis.call('HSET', 'wr:active', use_id, now_ms)
    redis.call('SET', 'wr:active:' .. use_id .. ':ls', '1', 'EX', ttl_secs)
    redis.call('HINCRBY', 'wr:stats', 'total_admitted', 1)
    return {'admitted', '0', '0'}
else
    redis.call('ZADD', 'wr:waiting', now_ms, use_id)
    local rank = redis.call('ZRANK', 'wr:waiting', use_id)
    local total = redis.call('ZCARD', 'wr:waiting')
    return {'enqueued', tostring(rank + 1), tostring(total)}
end
"#;

const REAPER_LUA: &str = r#"
local now_ms = tonumber(ARGV[1])
local max_active = tonumber(ARGV[2])
local ttl_secs = tonumber(ARGV[3])
local expired = 0

local active = redis.call('HGETALL', 'wr:active')
for i = 1, #active, 2 do
    local id = active[i]
    local admitted_ms = tonumber(active[i+1])
    if redis.call('EXISTS', 'wr:active:' .. id .. ':ls') == 0 then
        redis.call('HDEL', 'wr:active', id)
        local duration_ms = now_ms - admitted_ms
        redis.call('HINCRBYFLOAT', 'wr:stats', 'total_active_duration_ms', duration_ms)
        redis.call('HINCRBY', 'wr:stats', 'completed_sessions', 1)
        expired = expired + 1
    end
end

local current_active = redis.call('HLEN', 'wr:active')
local slots = max_active - current_active
local admitted = 0
if slots > 0 then
    local entries = redis.call('ZPOPMIN', 'wr:waiting', slots)
    for i = 1, #entries, 2 do
        local id = entries[i]
        redis.call('HSET', 'wr:active', id, now_ms)
        redis.call('SET', 'wr:active:' .. id .. ':ls', '1', 'EX', ttl_secs)
        admitted = admitted + 1
    end
end

if admitted > 0 then
    redis.call('HINCRBY', 'wr:stats', 'total_admitted', admitted)
end

return {expired, admitted}
"#;

const FLUSH_LUA: &str = r#"
local active = redis.call('HKEYS', 'wr:active')
for i = 1, #active do
    redis.call('DEL', 'wr:active:' .. active[i] .. ':ls')
end
redis.call('DEL', 'wr:active', 'wr:waiting', 'wr:stats')
return 1
"#;

#[async_trait]
impl QueueBackend for RedisBackend {
    async fn gate_check(
        &self,
        id: Option<SessionId>,
        new_id: SessionId,
        max_active: u32,
        ttl_secs: u64,
    ) -> GateResult {
        let mut conn = match self.pool.get().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("redis connection failed in gate_check: {}", e);
                return GateResult::Enqueued { position: 0, total: 0 };
            }
        };
        let id_str = id.map(|i| session_id_str(&i)).unwrap_or_default();
        let new_id_str = session_id_str(&new_id);

        let result: Vec<String> = cmd("EVAL")
            .arg(GATE_CHECK_LUA)
            .arg(0) // no KEYS
            .arg(&id_str)
            .arg(max_active)
            .arg(ttl_secs)
            .arg(now_ms().to_string())
            .arg(&new_id_str)
            .query_async(&mut *conn)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Redis gate_check error: {}", e);
                vec!["error".to_string(), "0".to_string(), "0".to_string()]
            });

        if result.len() < 3 {
            return GateResult::Enqueued { position: 1, total: 1 };
        }

        let position: usize = result[1].parse().unwrap_or(0);
        let total: usize = result[2].parse().unwrap_or(0);

        match result[0].as_str() {
            "active" => GateResult::Active,
            "waiting" => GateResult::Waiting { position, total },
            "admitted" => GateResult::Admitted,
            "enqueued" => GateResult::Enqueued { position, total },
            _ => GateResult::Enqueued { position: 1, total: 1 },
        }
    }

    async fn get_position(&self, id: &SessionId) -> Option<QueuePosition> {
        let mut conn = self.pool.get().await.ok()?;
        let id_str = session_id_str(id);

        let rank: Option<usize> = cmd("ZRANK")
            .arg("wr:waiting")
            .arg(&id_str)
            .query_async(&mut *conn)
            .await
            .ok()?;

        let rank = rank?;
        let total: usize = cmd("ZCARD")
            .arg("wr:waiting")
            .query_async(&mut *conn)
            .await
            .unwrap_or(0);

        // Get ETA stats
        let stats_raw: Vec<String> = cmd("HGETALL")
            .arg("wr:stats")
            .query_async(&mut *conn)
            .await
            .unwrap_or_default();

        let mut total_duration_ms: f64 = 0.0;
        let mut completed: u64 = 0;
        for i in (0..stats_raw.len()).step_by(2) {
            if i + 1 < stats_raw.len() {
                match stats_raw[i].as_str() {
                    "total_active_duration_ms" => {
                        total_duration_ms = stats_raw[i + 1].parse().unwrap_or(0.0);
                    }
                    "completed_sessions" => {
                        completed = stats_raw[i + 1].parse().unwrap_or(0);
                    }
                    _ => {}
                }
            }
        }

        let active_count: usize = cmd("HLEN")
            .arg("wr:active")
            .query_async(&mut *conn)
            .await
            .unwrap_or(1);

        let avg_duration_secs = if completed > 0 {
            total_duration_ms / completed as f64 / 1000.0
        } else {
            300.0
        };
        let position = rank + 1;
        let eta = (position as f64 / active_count.max(1) as f64) * avg_duration_secs;

        Some(QueuePosition {
            position,
            total_waiting: total,
            eta_seconds: eta,
        })
    }

    async fn is_active(&self, id: &SessionId) -> bool {
        let mut conn = match self.pool.get().await {
            Ok(c) => c,
            Err(_) => return false,
        };
        let id_str = session_id_str(id);
        let exists: bool = cmd("EXISTS")
            .arg(format!("wr:active:{}:ls", id_str))
            .query_async(&mut *conn)
            .await
            .unwrap_or(false);
        exists
    }

    async fn stats(&self) -> QueueStats {
        let mut conn = match self.pool.get().await {
            Ok(c) => c,
            Err(_) => {
                return QueueStats {
                    active_count: 0,
                    waiting_count: 0,
                    avg_active_duration_secs: 0.0,
                    total_admitted: 0,
                    total_visitors: 0,
                };
            }
        };

        let active: usize = cmd("HLEN")
            .arg("wr:active")
            .query_async(&mut *conn)
            .await
            .unwrap_or(0);
        let waiting: usize = cmd("ZCARD")
            .arg("wr:waiting")
            .query_async(&mut *conn)
            .await
            .unwrap_or(0);

        let stats_raw: Vec<String> = cmd("HGETALL")
            .arg("wr:stats")
            .query_async(&mut *conn)
            .await
            .unwrap_or_default();

        let mut total_duration_ms: f64 = 0.0;
        let mut completed: u64 = 0;
        let mut total_admitted: u64 = 0;
        let mut total_visitors: u64 = 0;
        for i in (0..stats_raw.len()).step_by(2) {
            if i + 1 < stats_raw.len() {
                match stats_raw[i].as_str() {
                    "total_active_duration_ms" => {
                        total_duration_ms = stats_raw[i + 1].parse().unwrap_or(0.0);
                    }
                    "completed_sessions" => {
                        completed = stats_raw[i + 1].parse().unwrap_or(0);
                    }
                    "total_admitted" => {
                        total_admitted = stats_raw[i + 1].parse().unwrap_or(0);
                    }
                    "total_visitors" => {
                        total_visitors = stats_raw[i + 1].parse().unwrap_or(0);
                    }
                    _ => {}
                }
            }
        }

        QueueStats {
            active_count: active,
            waiting_count: waiting,
            avg_active_duration_secs: if completed > 0 {
                total_duration_ms / completed as f64 / 1000.0
            } else {
                0.0
            },
            total_admitted,
            total_visitors,
        }
    }

    async fn flush(&self) {
        let mut conn = match self.pool.get().await {
            Ok(c) => c,
            Err(_) => return,
        };
        let _: Option<i64> = cmd("EVAL")
            .arg(FLUSH_LUA)
            .arg(0)
            .query_async(&mut *conn)
            .await
            .ok();
    }

    async fn reaper_cycle(&self, ttl_secs: u64, max_active: u32) -> (usize, usize) {
        let mut conn = match self.pool.get().await {
            Ok(c) => c,
            Err(_) => return (0, 0),
        };

        let result: Vec<i64> = cmd("EVAL")
            .arg(REAPER_LUA)
            .arg(0)
            .arg(now_ms().to_string())
            .arg(max_active)
            .arg(ttl_secs)
            .query_async(&mut *conn)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Redis reaper error: {}", e);
                vec![0, 0]
            });

        let expired = result.first().copied().unwrap_or(0) as usize;
        let admitted = result.get(1).copied().unwrap_or(0) as usize;
        (expired, admitted)
    }
}
