use reqwest::Client;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let levels = [1_000, 5_000, 10_000, 20_000, 50_000];
    let wr_url = "http://127.0.0.1:8080";
    let admin_key = "change-me-in-production";

    let config: serde_json::Value = Client::new()
        .get(format!("{wr_url}/__wr/admin/config"))
        .header("X-Api-Key", admin_key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap_or_default();

    let backend = if config.get("redis_url").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
        "in-memory"
    } else {
        "Redis"
    };

    println!("=== Waiting Room Benchmark ({}) ===", backend);
    println!();

    let client = Client::builder()
        .pool_max_idle_per_host(0)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    for &n in &levels {
        // flush
        let _ = client
            .post(format!("{wr_url}/__wr/admin/flush"))
            .header("X-Api-Key", admin_key)
            .send()
            .await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let ok = Arc::new(AtomicUsize::new(0));
        let admitted = Arc::new(AtomicUsize::new(0));
        let queued = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(AtomicUsize::new(0));

        let start = Instant::now();

        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let client = client.clone();
            let url = format!("{wr_url}/");
            let ok = ok.clone();
            let admitted = admitted.clone();
            let queued = queued.clone();
            let errors = errors.clone();

            handles.push(tokio::spawn(async move {
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

        for h in handles {
            let _ = h.await;
        }

        let elapsed = start.elapsed();
        let elapsed_ms = elapsed.as_millis();
        let rps = if elapsed_ms > 0 {
            n as u128 * 1000 / elapsed_ms
        } else {
            0
        };

        let status: serde_json::Value = client
            .get(format!("{wr_url}/__wr/status"))
            .send()
            .await
            .and_then(|r| Ok(r))
            .ok()
            .and_then(|r| {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(r.json()).ok()
                })
            })
            .unwrap_or_default();

        let rss = get_rss_mb(8080);

        println!(
            "[{:>6} users] {:>5}ms  ok={:<5} err={:<4} ~{} req/s  active={} queue={}  mem={}MB",
            n,
            elapsed_ms,
            ok.load(Ordering::Relaxed),
            errors.load(Ordering::Relaxed),
            rps,
            status["active_users"],
            status["queue_length"],
            rss,
        );

        if client
            .get(format!("{wr_url}/__wr/status"))
            .send()
            .await
            .is_err()
        {
            println!("SERVER DOWN at {} users!", n);
            break;
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    println!();
    println!("=== Done ===");
}

fn get_rss_mb(port: u16) -> u64 {
    let output = std::process::Command::new("lsof")
        .args(["-ti", &format!(":{port}")])
        .output()
        .ok();
    let pid = output
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
        .unwrap_or_default();
    if pid.is_empty() {
        return 0;
    }
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok();
    output
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
        / 1024
}
