use clap::Parser;
use futures_util::StreamExt;
use reqwest::header;
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 순수 SSE 동시 연결 수 테스트
/// 서버가 실제로 몇 개의 SSE 연결을 동시에 유지할 수 있는지 확인
#[derive(Parser)]
#[command(name = "conn_test", about = "SSE max connection test")]
struct Args {
    /// 목표 동시 연결 수
    #[arg(short, long, default_value_t = 1000)]
    target: u64,

    /// 연결 생성 속도 (초당 연결 수)
    #[arg(short, long, default_value_t = 200)]
    rate: u64,

    /// 연결 유지 시간 (초)
    #[arg(long, default_value_t = 30)]
    hold: u64,

    /// Waiting room URL
    #[arg(long, default_value = "http://localhost:8080")]
    url: String,
}

struct Counters {
    connected: AtomicU64,     // 현재 SSE 연결 유지 중
    total_opened: AtomicU64,  // 총 연결 성공
    peak: AtomicU64,          // 최대 동시 연결
    connect_err: AtomicU64,   // GET / 실패
    sse_err: AtomicU64,       // SSE 연결 실패
    dropped: AtomicU64,       // SSE 연결 끊김
}

impl Counters {
    fn new() -> Self {
        Self {
            connected: AtomicU64::new(0),
            total_opened: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            connect_err: AtomicU64::new(0),
            sse_err: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    fn update_peak(&self) {
        let current = self.connected.load(Ordering::Relaxed);
        let mut peak = self.peak.load(Ordering::Relaxed);
        while current > peak {
            match self.peak.compare_exchange_weak(peak, current, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(p) => peak = p,
            }
        }
    }
}

async fn open_sse_connection(
    id: u64,
    args: &Args,
    counters: &Counters,
    client: &reqwest::Client,
    stop: &AtomicBool,
) {
    // Step 1: GET / → 쿠키 획득
    let resp = match client.get(&format!("{}/", args.url)).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[conn {}] GET / failed: {}", id, e);
            counters.connect_err.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    let status = resp.status().as_u16();
    if status != 200 && status != 302 {
        eprintln!("[conn {}] GET / status: {}", id, status);
        counters.connect_err.fetch_add(1, Ordering::Relaxed);
        return;
    }

    // 쿠키 추출 (302인 경우에도 Set-Cookie가 올 수 있음)
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
            // 302로 바로 입장한 경우 등
            counters.connect_err.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    // Step 2: SSE 연결
    let sse_resp = match client
        .get(&format!("{}/__wr/events", args.url))
        .header(header::COOKIE, format!("__wr_token={}", token))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[conn {}] SSE failed: {}", id, e);
            counters.sse_err.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    // 연결 성공
    counters.connected.fetch_add(1, Ordering::Relaxed);
    counters.total_opened.fetch_add(1, Ordering::Relaxed);
    counters.update_peak();

    // 연결 유지: 스트림을 읽으면서 hold 시간 또는 stop 시그널까지 대기
    let mut stream = sse_resp.bytes_stream();
    let deadline = Instant::now() + Duration::from_secs(args.hold);

    while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
        match tokio::time::timeout(Duration::from_secs(1), stream.next()).await {
            Ok(Some(Ok(_))) => {} // 데이터 수신, 계속 유지
            Ok(Some(Err(_))) => {
                // 연결 끊김
                counters.dropped.fetch_add(1, Ordering::Relaxed);
                break;
            }
            Ok(None) => {
                // 스트림 종료
                counters.dropped.fetch_add(1, Ordering::Relaxed);
                break;
            }
            Err(_) => {} // 1초 타임아웃, 계속 유지
        }
    }

    counters.connected.fetch_sub(1, Ordering::Relaxed);
}

#[tokio::main]
async fn main() {
    let args = Arc::new(Args::parse());
    let counters = Arc::new(Counters::new());
    let stop = Arc::new(AtomicBool::new(false));

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
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

    println!("=== SSE Max Connection Test ===");
    println!("Target:    {} connections", args.target);
    println!("Rate:      {}/sec", args.rate);
    println!("Hold:      {}s", args.hold);
    println!();

    let start = Instant::now();

    // 진행률 출력
    let counters_p = counters.clone();
    let stop_p = stop.clone();
    let progress = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let connected = counters_p.connected.load(Ordering::Relaxed);
            let peak = counters_p.peak.load(Ordering::Relaxed);
            let opened = counters_p.total_opened.load(Ordering::Relaxed);
            let c_err = counters_p.connect_err.load(Ordering::Relaxed);
            let s_err = counters_p.sse_err.load(Ordering::Relaxed);
            let dropped = counters_p.dropped.load(Ordering::Relaxed);
            println!(
                "  active:{:>6}  peak:{:>6}  opened:{:>6}  err:{}/{}  dropped:{}",
                connected, peak, opened, c_err, s_err, dropped
            );
            if stop_p.load(Ordering::Relaxed) && connected == 0 {
                break;
            }
        }
    });

    // rate 제어하면서 연결 생성
    let delay = Duration::from_micros(1_000_000 / args.rate);
    let mut handles = Vec::with_capacity(args.target as usize);

    for id in 1..=args.target {
        let args = args.clone();
        let counters = counters.clone();
        let client = client.clone();
        let stop = stop.clone();

        handles.push(tokio::spawn(async move {
            open_sse_connection(id, &args, &counters, &client, &stop).await;
        }));

        tokio::time::sleep(delay).await;
    }

    println!();
    println!("All {} connections requested. Holding for {}s...", args.target, args.hold);

    // hold 시간 대기
    tokio::time::sleep(Duration::from_secs(args.hold)).await;
    stop.store(true, Ordering::Relaxed);

    // 모든 태스크 종료 대기
    for h in handles {
        let _ = h.await;
    }

    let elapsed = start.elapsed();
    let _ = progress.await;

    // 결과
    let peak = counters.peak.load(Ordering::Relaxed);
    let opened = counters.total_opened.load(Ordering::Relaxed);
    let c_err = counters.connect_err.load(Ordering::Relaxed);
    let s_err = counters.sse_err.load(Ordering::Relaxed);
    let dropped = counters.dropped.load(Ordering::Relaxed);

    println!();
    println!("=== Results ({:.1}s) ===", elapsed.as_secs_f64());
    println!("  Peak concurrent SSE:  {}", peak);
    println!("  Total opened:         {}", opened);
    println!("  Connect errors:       {}", c_err);
    println!("  SSE errors:           {}", s_err);
    println!("  Dropped during hold:  {}", dropped);
    println!();

    if peak >= args.target {
        println!("  PASS: reached target {} connections", args.target);
    } else {
        println!("  FAIL: peak {} < target {}", peak, args.target);
    }

    println!();
    println!("=== Test Complete ===");
}
