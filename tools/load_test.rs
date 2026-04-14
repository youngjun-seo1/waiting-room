use clap::Parser;
use futures_util::StreamExt;
use reqwest::header;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Parser)]
#[command(name = "load_test", about = "Waiting Room SSE load test")]
struct Args {
    /// Total number of simulated users
    #[arg(short, long, default_value_t = 1000)]
    total: u64,

    /// Max concurrent connections
    #[arg(short, long, default_value_t = 500)]
    concurrency: usize,

    /// SSE timeout in seconds
    #[arg(long, default_value_t = 1800)]
    sse_timeout: u64,

    /// Waiting room URL
    #[arg(long, default_value = "http://localhost:8080")]
    url: String,

    /// Max retries when SSE returns no data (session expired)
    #[arg(long, default_value_t = 3)]
    retries: u32,
}

struct Counters {
    admitted_direct: AtomicU64,
    admitted_sse: AtomicU64,
    timeout: AtomicU64,
    error: AtomicU64,
}

impl Counters {
    fn new() -> Self {
        Self {
            admitted_direct: AtomicU64::new(0),
            admitted_sse: AtomicU64::new(0),
            timeout: AtomicU64::new(0),
            error: AtomicU64::new(0),
        }
    }

    fn total(&self) -> u64 {
        self.admitted_direct.load(Ordering::Relaxed)
            + self.admitted_sse.load(Ordering::Relaxed)
            + self.timeout.load(Ordering::Relaxed)
            + self.error.load(Ordering::Relaxed)
    }
}

const MAX_CONNECT_RETRIES: u32 = 10;
const RETRY_BASE_MS: u64 = 200;

async fn request_with_retry(
    client: &reqwest::Client,
    url: &str,
    cookie: Option<&str>,
    timeout_secs: u64,
) -> Result<reqwest::Response, String> {
    for attempt in 0..MAX_CONNECT_RETRIES {
        let mut req = client.get(url);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        if timeout_secs > 0 {
            req = req.timeout(Duration::from_secs(timeout_secs));
        }
        match req.send().await {
            Ok(r) => return Ok(r),
            Err(_) if attempt + 1 < MAX_CONNECT_RETRIES => {
                let delay = (RETRY_BASE_MS * 2u64.pow(attempt)).min(5000);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                continue;
            }
            Err(e) => return Err(format!("{}", e)),
        }
    }
    unreachable!()
}

async fn simulate_user(
    _id: u64,
    args: &Args,
    counters: &Counters,
    client: &reqwest::Client,
) {
    for attempt in 1..=args.retries {
        // Step 1: GET / → 쿠키 획득 (redirect 따라가지 않음)
        let resp = match request_with_retry(client, &format!("{}/", args.url), None, 0).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[user {}] GET / failed after retries: {}", _id, e);
                counters.error.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        let status = resp.status().as_u16();

        // 302 = 바로 입장 (origin redirect)
        if status == 302 {
            counters.admitted_direct.fetch_add(1, Ordering::Relaxed);
            return;
        }

        if status != 200 {
            eprintln!("[user {}] GET / unexpected status: {}", _id, status);
            counters.error.fetch_add(1, Ordering::Relaxed);
            return;
        }

        // Set-Cookie에서 토큰 추출
        let token = resp
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|s| s.starts_with("__wr_token="))
            .and_then(|s| s.strip_prefix("__wr_token="))
            .and_then(|s| s.split(';').next())
            .map(|s| s.to_string());

        let token = match token {
            Some(t) => t,
            None => {
                eprintln!("[user {}] no __wr_token cookie in 200 response", _id);
                counters.error.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        // Step 2: SSE 연결 (재시도 포함)
        let cookie = format!("__wr_token={}", token);
        let sse_resp = match request_with_retry(
            client,
            &format!("{}/__wr/events", args.url),
            Some(&cookie),
            args.sse_timeout,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[user {}] SSE connect failed after retries: {}", _id, e);
                counters.error.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        let mut stream = sse_resp.bytes_stream();
        let mut admitted = false;
        let mut _got_any = false;
        let mut buf = String::new();

        let deadline = Instant::now() + Duration::from_secs(args.sse_timeout);

        while Instant::now() < deadline {
            let chunk = tokio::time::timeout(
                deadline.saturating_duration_since(Instant::now()),
                stream.next(),
            )
            .await;

            match chunk {
                Ok(Some(Ok(bytes))) => {
                    _got_any = true;
                    buf.push_str(&String::from_utf8_lossy(&bytes));

                    if buf.contains("\"admit\"") {
                        admitted = true;
                        break;
                    }

                    // 버퍼가 너무 커지면 마지막 1KB만 유지
                    if buf.len() > 4096 {
                        let keep = buf.len() - 1024;
                        buf.drain(..keep);
                    }
                }
                Ok(Some(Err(e))) => {
                    eprintln!("[user {}] SSE stream error: {}", _id, e);
                    break;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        if admitted {
            counters.admitted_sse.fetch_add(1, Ordering::Relaxed);
            return;
        }

        // admit 못 받음 → GET /부터 재시도 (세션 만료 or 연결 끊김)
        if attempt < args.retries {
            continue;
        }
    }

    counters.timeout.fetch_add(1, Ordering::Relaxed);
}

#[tokio::main]
async fn main() {
    let args = Arc::new(Args::parse());
    let counters = Arc::new(Counters::new());
    let semaphore = Arc::new(Semaphore::new(args.concurrency));

    // redirect를 따라가지 않는 클라이언트
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)  // 연결 재사용 방지 (각 유저 = 독립 연결)
        .build()
        .unwrap();

    // Waiting room 활성화 확인
    let status_resp = reqwest::get(&format!("{}/__wr/status", args.url))
        .await
        .expect("Failed to connect to server");
    let status_json: serde_json::Value = status_resp.json().await.unwrap();
    if status_json["enabled"] != true {
        eprintln!("Error: Waiting room is disabled. Create a schedule first.");
        std::process::exit(1);
    }

    println!("=== Waiting Room SSE Load Test ===");
    println!("Total users: {}", args.total);
    println!("Concurrency: {}", args.concurrency);
    println!("SSE timeout: {}s", args.sse_timeout);
    println!();

    let start = Instant::now();

    // 진행률 출력 태스크
    let counters_progress = counters.clone();
    let total = args.total;
    let progress_handle = tokio::spawn(async move {
        let mut last_total = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let current = counters_progress.total();
            if current != last_total {
                let direct = counters_progress.admitted_direct.load(Ordering::Relaxed);
                let sse = counters_progress.admitted_sse.load(Ordering::Relaxed);
                let timeouts = counters_progress.timeout.load(Ordering::Relaxed);
                let errors = counters_progress.error.load(Ordering::Relaxed);
                println!(
                    "  [{}/{}] direct:{} sse:{} timeout:{} error:{}",
                    current, total, direct, sse, timeouts, errors
                );
                last_total = current;
            }
            if current >= total {
                break;
            }
        }
    });

    // 유저 태스크 생성
    let mut handles = Vec::with_capacity(args.total as usize);
    for id in 1..=args.total {
        let args = args.clone();
        let counters = counters.clone();
        let sem = semaphore.clone();
        let client = client.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            simulate_user(id, &args, &counters, &client).await;
        }));
    }

    // 모든 유저 완료 대기
    for h in handles {
        let _ = h.await;
    }

    let elapsed = start.elapsed();
    let _ = progress_handle.await;

    // 결과 출력
    let direct = counters.admitted_direct.load(Ordering::Relaxed);
    let sse = counters.admitted_sse.load(Ordering::Relaxed);
    let timeouts = counters.timeout.load(Ordering::Relaxed);
    let errors = counters.error.load(Ordering::Relaxed);
    let total_done = direct + sse + timeouts + errors;

    println!();
    println!("[Results] {:.1}s elapsed", elapsed.as_secs_f64());
    println!("  Admitted (direct):  {}", direct);
    println!("  Admitted (SSE):     {}", sse);
    println!("  Timeout:            {}", timeouts);
    println!("  Errors:             {}", errors);
    println!("  Total:              {} / {}", total_done, args.total);

    // 큐 상태
    if let Ok(resp) = reqwest::get(&format!("{}/__wr/status", args.url)).await {
        if let Ok(json) = resp.json::<serde_json::Value>().await {
            println!();
            println!("[Queue Status]");
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }

    println!();
    println!("=== Test Complete ===");
}
