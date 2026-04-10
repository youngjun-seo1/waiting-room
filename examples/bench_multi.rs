use reqwest::Client;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
struct ServerMetrics {
    samples: Vec<MetricSample>,
}

#[derive(Clone)]
struct MetricSample {
    elapsed_ms: u128,
    cpu_pct: f64,
    rss_mb: u64,
    threads: u32,
}

struct MetricsSummary {
    avg_cpu: f64,
    peak_cpu: f64,
    avg_rss_mb: u64,
    peak_rss_mb: u64,
    peak_threads: u32,
}

impl ServerMetrics {
    fn summary(&self) -> MetricsSummary {
        if self.samples.is_empty() {
            return MetricsSummary {
                avg_cpu: 0.0, peak_cpu: 0.0,
                avg_rss_mb: 0, peak_rss_mb: 0,
                peak_threads: 0,
            };
        }
        let n = self.samples.len() as f64;
        let avg_cpu = self.samples.iter().map(|s| s.cpu_pct).sum::<f64>() / n;
        let peak_cpu = self.samples.iter().map(|s| s.cpu_pct).fold(0.0f64, f64::max);
        let avg_rss = self.samples.iter().map(|s| s.rss_mb).sum::<u64>() / self.samples.len() as u64;
        let peak_rss = self.samples.iter().map(|s| s.rss_mb).max().unwrap_or(0);
        let peak_threads = self.samples.iter().map(|s| s.threads).max().unwrap_or(0);
        MetricsSummary { avg_cpu, peak_cpu, avg_rss_mb: avg_rss, peak_rss_mb: peak_rss, peak_threads }
    }
}

fn get_pid(port: u16) -> Option<u32> {
    let output = std::process::Command::new("lsof")
        .args(["-ti", &format!(":{port}")])
        .output()
        .ok()?;
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

fn sample_process(pid: u32) -> Option<MetricSample> {
    // ps -o %cpu,rss,th -p <pid>
    let output = std::process::Command::new("ps")
        .args(["-o", "%cpu=,rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let line = String::from_utf8(output.stdout).ok()?;
    let parts: Vec<&str> = line.trim().split_whitespace().collect();
    if parts.len() < 2 { return None; }
    let cpu_pct: f64 = parts[0].parse().unwrap_or(0.0);
    let rss_kb: u64 = parts[1].parse().unwrap_or(0);

    // Thread count via proc_info (macOS doesn't have /proc)
    let th_output = std::process::Command::new("ps")
        .args(["-M", "-p", &pid.to_string()])
        .output()
        .ok();
    let threads = th_output
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().count().saturating_sub(1) as u32)
        .unwrap_or(0);

    Some(MetricSample {
        elapsed_ms: 0,
        cpu_pct,
        rss_mb: rss_kb / 1024,
        threads,
    })
}

fn get_redis_info() -> String {
    let output = std::process::Command::new("redis-cli")
        .args(["INFO", "stats"])
        .output()
        .ok();
    output
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
}

fn get_redis_ops_per_sec() -> u64 {
    let info = get_redis_info();
    for line in info.lines() {
        if line.starts_with("instantaneous_ops_per_sec:") {
            return line.split(':').nth(1).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        }
    }
    0
}

fn get_redis_memory_mb() -> u64 {
    let output = std::process::Command::new("redis-cli")
        .args(["INFO", "memory"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    for line in output.lines() {
        if line.starts_with("used_memory_rss:") {
            return line.split(':').nth(1)
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(0) / 1024 / 1024;
        }
    }
    0
}

#[tokio::main]
async fn main() {
    let servers = vec![
        "http://127.0.0.1:8080",
        "http://127.0.0.1:8081",
        "http://127.0.0.1:8082",
        "http://127.0.0.1:8083",
    ];
    let ports: Vec<u16> = vec![8080, 8081, 8082, 8083];
    let admin_key = "change-me-in-production";
    let levels = [1_000, 5_000, 10_000, 20_000, 50_000, 100_000];
    let batch_size = 2_000;

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║   Multi-Server Benchmark (Redis, {} servers, batch={})       ║", servers.len(), batch_size);
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();

    let client = Client::builder()
        .pool_max_idle_per_host(100)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    // Resolve PIDs
    let pids: Vec<Option<u32>> = ports.iter().map(|p| get_pid(*p)).collect();
    for (i, pid) in pids.iter().enumerate() {
        println!("  Server {} (:{}) PID: {}", i, ports[i],
            pid.map(|p| p.to_string()).unwrap_or("N/A".into()));
    }
    println!();

    for &n in &levels {
        // Flush
        let _ = client
            .post(format!("{}/__wr/admin/flush", servers[0]))
            .header("X-Api-Key", admin_key)
            .send()
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;

        let ok = Arc::new(AtomicUsize::new(0));
        let admitted = Arc::new(AtomicUsize::new(0));
        let queued = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(AtomicUsize::new(0));
        let server_count = servers.len();

        // Start metrics collection
        let metrics: Vec<Arc<Mutex<ServerMetrics>>> = (0..server_count)
            .map(|_| Arc::new(Mutex::new(ServerMetrics::default())))
            .collect();
        let collecting = Arc::new(AtomicBool::new(true));

        let monitor_handle = {
            let pids = pids.clone();
            let metrics = metrics.clone();
            let collecting = collecting.clone();
            tokio::spawn(async move {
                let start = Instant::now();
                while collecting.load(Ordering::Relaxed) {
                    let elapsed = start.elapsed().as_millis();
                    for (i, pid) in pids.iter().enumerate() {
                        if let Some(pid) = pid {
                            if let Some(mut sample) = sample_process(*pid) {
                                sample.elapsed_ms = elapsed;
                                metrics[i].lock().await.samples.push(sample);
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
        };

        let redis_ops_before = get_redis_ops_per_sec();
        let start = Instant::now();

        // Send requests in batches
        let mut sent = 0usize;
        while sent < n {
            let this_batch = batch_size.min(n - sent);
            let mut handles = Vec::with_capacity(this_batch);

            for i in 0..this_batch {
                let client = client.clone();
                let server = servers[(sent + i) % server_count].to_string();
                let ok = ok.clone();
                let admitted = admitted.clone();
                let queued = queued.clone();
                let errors = errors.clone();

                handles.push(tokio::spawn(async move {
                    let url = format!("{}/", server);
                    match client.get(&url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            ok.fetch_add(1, Ordering::Relaxed);
                            let body = resp.text().await.unwrap_or_default();
                            if body.contains("티켓 구매") {
                                admitted.fetch_add(1, Ordering::Relaxed);
                            } else if body.contains("Please wait") {
                                queued.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        _ => {
                            errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }));
            }
            for h in handles { let _ = h.await; }
            sent += this_batch;
        }

        let elapsed = start.elapsed();
        let elapsed_ms = elapsed.as_millis();
        let rps = if elapsed_ms > 0 { n as u128 * 1000 / elapsed_ms } else { 0 };

        // Stop monitoring
        collecting.store(false, Ordering::Relaxed);
        let _ = monitor_handle.await;

        let redis_ops_after = get_redis_ops_per_sec();
        let redis_mem = get_redis_memory_mb();

        let ok_n = ok.load(Ordering::Relaxed);
        let err_n = errors.load(Ordering::Relaxed);
        let adm_n = admitted.load(Ordering::Relaxed);
        let que_n = queued.load(Ordering::Relaxed);

        // Queue status
        let status: serde_json::Value = client
            .get(format!("{}/__wr/status", servers[0]))
            .send()
            .await
            .ok()
            .and_then(|r| {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(r.json()).ok()
                })
            })
            .unwrap_or_default();

        println!("┌─ {:>7} users ─────────────────────────────────────────────────", n);
        println!("│  Time: {}ms  Throughput: ~{} req/s", elapsed_ms, rps);
        println!("│  OK: {}  Errors: {}  (admitted={}, queued={})", ok_n, err_n, adm_n, que_n);
        println!("│  Queue: active={}, waiting={}", status["active_users"], status["queue_length"]);
        println!("│");
        println!("│  {:^8} {:>8} {:>8} {:>8} {:>8} {:>8}", "Server", "Avg CPU", "Peak CPU", "Avg RSS", "Peak RSS", "Threads");
        println!("│  {:─<8} {:─>8} {:─>8} {:─>8} {:─>8} {:─>8}", "", "", "", "", "", "");

        for (i, m) in metrics.iter().enumerate() {
            let s = m.lock().await.summary();
            println!("│  S{:<7} {:>7.1}% {:>7.1}% {:>6}MB {:>6}MB {:>8}",
                i, s.avg_cpu, s.peak_cpu, s.avg_rss_mb, s.peak_rss_mb, s.peak_threads);
        }

        // Sum
        let mut total_avg_cpu = 0.0f64;
        let mut total_peak_cpu = 0.0f64;
        let mut total_peak_rss = 0u64;
        for m in &metrics {
            let s = m.lock().await.summary();
            total_avg_cpu += s.avg_cpu;
            total_peak_cpu += s.peak_cpu;
            total_peak_rss += s.peak_rss_mb;
        }
        println!("│  {:─<8} {:─>8} {:─>8} {:─>8} {:─>8}", "", "", "", "", "");
        println!("│  {:<8} {:>7.1}% {:>7.1}%           {:>6}MB",
            "Total", total_avg_cpu, total_peak_cpu, total_peak_rss);

        println!("│");
        println!("│  Redis: ops/s={}, mem={}MB", redis_ops_after, redis_mem);
        println!("└──────────────────────────────────────────────────────────────────");
        println!();

        // Check servers alive
        let mut alive = true;
        for server in &servers {
            if client.get(format!("{}/__wr/status", server)).send().await.is_err() {
                println!("  ⚠ {} DOWN!", server);
                alive = false;
            }
        }
        if !alive { break; }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    println!("=== Done ===");
}
