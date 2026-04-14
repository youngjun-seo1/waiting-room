use clap::Parser;
use futures_util::StreamExt;
use rand::Rng;
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

    /// Waiting room URLs (comma-separated for multi-server, e.g. http://localhost:8080,http://localhost:8081)
    #[arg(long, default_value = "http://localhost:8080")]
    url: String,

    /// Ramp-up time in seconds (0 = all users at once)
    #[arg(long, default_value_t = 0)]
    ramp_up: u64,
}

struct Counters {
    get_count: AtomicU64,
    sse_count: AtomicU64,
    admitted_direct: AtomicU64,
    admitted_sse: AtomicU64,
    closed: AtomicU64,
    timeout: AtomicU64,
    error: AtomicU64,
}

impl Counters {
    fn new() -> Self {
        Self {
            get_count: AtomicU64::new(0),
            sse_count: AtomicU64::new(0),
            admitted_direct: AtomicU64::new(0),
            admitted_sse: AtomicU64::new(0),
            closed: AtomicU64::new(0),
            timeout: AtomicU64::new(0),
            error: AtomicU64::new(0),
        }
    }

    fn total(&self) -> u64 {
        self.admitted_direct.load(Ordering::Relaxed)
            + self.admitted_sse.load(Ordering::Relaxed)
            + self.closed.load(Ordering::Relaxed)
            + self.timeout.load(Ordering::Relaxed)
            + self.error.load(Ordering::Relaxed)
    }
}

async fn send_request(
    client: &reqwest::Client,
    url: &str,
    cookie: Option<&str>,
    timeout_secs: u64,
) -> Result<reqwest::Response, String> {
    let mut req = client.get(url);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    if timeout_secs > 0 {
        req = req.timeout(Duration::from_secs(timeout_secs));
    }
    req.send().await.map_err(|e| format!("{}", e))
}

async fn simulate_user(
    _id: u64,
    urls: &[String],
    args: &Args,
    counters: &Counters,
    client: &reqwest::Client,
) {
    let url_count = urls.len();
    let get_url = &urls[rand::rng().random_range(0..url_count)];

    // Step 1: GET / → 쿠키 획득 (redirect 따라가지 않음)
    counters.get_count.fetch_add(1, Ordering::Relaxed);
    let resp = match send_request(client, &format!("{}/", get_url), None, 0).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[user {}] GET / failed: {}", _id, e);
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

    // Step 2: SSE 연결 → admit/closed 대기
    let sse_url = &urls[rand::rng().random_range(0..url_count)];
    counters.sse_count.fetch_add(1, Ordering::Relaxed);
    let cookie = format!("__wr_token={}", token);
    let sse_resp = match send_request(
        client,
        &format!("{}/__wr/events", sse_url),
        Some(&cookie),
        args.sse_timeout,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[user {}] SSE connect failed: {}", _id, e);
            counters.error.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    let sse_status = sse_resp.status().as_u16();
    if sse_status != 200 {
        eprintln!("[user {}] SSE unexpected status: {}", _id, sse_status);
        counters.error.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let mut stream = sse_resp.bytes_stream();
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
                buf.push_str(&String::from_utf8_lossy(&bytes));

                if buf.contains("\"admit\"") {
                    counters.admitted_sse.fetch_add(1, Ordering::Relaxed);
                    return;
                }

                if buf.contains("\"closed\"") {
                    counters.closed.fetch_add(1, Ordering::Relaxed);
                    return;
                }

                if buf.len() > 4096 {
                    let keep = buf.len() - 1024;
                    buf.drain(..keep);
                }
            }
            Ok(Some(Err(e))) => {
                eprintln!("[user {}] SSE stream error: {}", _id, e);
                counters.error.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Ok(None) => {
                // 서버가 스트림을 닫음 (세션 만료 등)
                eprintln!("[user {}] SSE stream ended without admit/closed", _id);
                counters.error.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(_) => {
                // SSE 타임아웃
                counters.timeout.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
    }

    counters.timeout.fetch_add(1, Ordering::Relaxed);
}

#[tokio::main]
async fn main() {
    let args = Arc::new(Args::parse());
    let counters = Arc::new(Counters::new());
    let semaphore = Arc::new(Semaphore::new(args.concurrency));

    // URL 파싱 (콤마 구분)
    let urls: Arc<Vec<String>> = Arc::new(
        args.url.split(',').map(|s| s.trim().to_string()).collect()
    );

    // redirect를 따라가지 않는 클라이언트
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)  // 연결 재사용 방지 (각 유저 = 독립 연결)
        .build()
        .unwrap();

    // 각 서버의 Waiting room 활성화 확인
    for url in urls.iter() {
        let status_resp = reqwest::get(&format!("{}/__wr/status", url))
            .await
            .unwrap_or_else(|e| panic!("Failed to connect to {}: {}", url, e));
        let status_json: serde_json::Value = status_resp.json().await.unwrap();
        if status_json["enabled"] != true {
            eprintln!("Error: Waiting room is disabled on {}. Create a schedule first.", url);
            std::process::exit(1);
        }
    }

    println!("=== Waiting Room SSE Load Test ===");
    println!("Servers:     {} ({})", urls.len(), urls.join(", "));
    println!("Total users: {}", args.total);
    println!("Concurrency: {}", args.concurrency);
    println!("SSE timeout: {}s", args.sse_timeout);
    if args.ramp_up > 0 {
        println!("Ramp-up:     {}s ({:.0} users/sec)", args.ramp_up, args.total as f64 / args.ramp_up as f64);
    }
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
                let gets = counters_progress.get_count.load(Ordering::Relaxed);
                let sses = counters_progress.sse_count.load(Ordering::Relaxed);
                let direct = counters_progress.admitted_direct.load(Ordering::Relaxed);
                let sse = counters_progress.admitted_sse.load(Ordering::Relaxed);
                let closed = counters_progress.closed.load(Ordering::Relaxed);
                let timeouts = counters_progress.timeout.load(Ordering::Relaxed);
                let errors = counters_progress.error.load(Ordering::Relaxed);
                println!(
                    "  [{}/{}] GET:{} SSE:{} | direct:{} sse:{} closed:{} timeout:{} error:{}",
                    current, total, gets, sses, direct, sse, closed, timeouts, errors
                );
                last_total = current;
            }
            if current >= total {
                break;
            }
        }
    });

    // 유저 태스크 생성 (ramp-up 적용)
    let ramp_delay = if args.ramp_up > 0 {
        Duration::from_micros(args.ramp_up * 1_000_000 / args.total)
    } else {
        Duration::ZERO
    };

    let mut handles = Vec::with_capacity(args.total as usize);
    for id in 1..=args.total {
        let args = args.clone();
        let urls = urls.clone();
        let counters = counters.clone();
        let sem = semaphore.clone();
        let client = client.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            simulate_user(id, &urls, &args, &counters, &client).await;
        }));

        if ramp_delay > Duration::ZERO {
            tokio::time::sleep(ramp_delay).await;
        }
    }

    // 모든 유저 완료 대기
    for h in handles {
        let _ = h.await;
    }

    let elapsed = start.elapsed();
    let _ = progress_handle.await;

    // 결과 출력
    let gets = counters.get_count.load(Ordering::Relaxed);
    let sses = counters.sse_count.load(Ordering::Relaxed);
    let direct = counters.admitted_direct.load(Ordering::Relaxed);
    let sse = counters.admitted_sse.load(Ordering::Relaxed);
    let closed = counters.closed.load(Ordering::Relaxed);
    let timeouts = counters.timeout.load(Ordering::Relaxed);
    let errors = counters.error.load(Ordering::Relaxed);
    let total_done = direct + sse + closed + timeouts + errors;

    println!();
    println!("[Results] {:.1}s elapsed", elapsed.as_secs_f64());
    println!("  GET / calls:        {}", gets);
    println!("  SSE connections:    {}", sses);
    println!("  Admitted (direct):  {}", direct);
    println!("  Admitted (SSE):     {}", sse);
    println!("  Closed:             {}", closed);
    println!("  Timeout:            {}", timeouts);
    println!("  Errors:             {}", errors);
    println!("  Total:              {} / {}", total_done, args.total);

    // 큐 상태 (각 서버)
    println!();
    println!("[Queue Status]");
    for url in urls.iter() {
        if let Ok(resp) = reqwest::get(&format!("{}/__wr/status", url)).await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                println!("  {}: {}", url, serde_json::to_string(&json).unwrap());
            }
        }
    }

    println!();
    println!("=== Test Complete ===");
}
