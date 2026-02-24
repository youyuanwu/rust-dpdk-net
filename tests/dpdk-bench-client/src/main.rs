//! HTTP/UDP benchmark client similar to wrk
//!
//! A high-performance benchmark tool for testing DPDK-based servers.
//!
//! # Usage
//!
//! ```bash
//! # Raw TCP mode (fastest, HTTP/1.1 only)
//! dpdk-bench-client -c 100 -d 30s http://10.0.0.4:8080/
//!
//! # Hyper mode (supports HTTP/2)
//! dpdk-bench-client --mode hyper -c 100 -d 30s http://10.0.0.4:8080/
//! dpdk-bench-client --mode hyper --http2 -c 100 -d 30s http://10.0.0.4:8080/
//!
//! # UDP echo mode (measures PPS, RTT, loss)
//! dpdk-bench-client --mode udp -c 10 -d 10s 10.0.0.4:8080
//! dpdk-bench-client --mode udp -c 10 -d 10s --packet-size 1024 10.0.0.4:8080
//! ```

mod hyper_client;
mod raw_client;
mod stats;
mod udp_client;

use clap::{Parser, ValueEnum};
use serde::Serialize;
use stats::BenchStats;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, ValueEnum)]
enum ClientMode {
    /// Raw TCP client for maximum throughput (HTTP/1.1 only)
    Raw,
    /// Hyper-based client with HTTP/2 support
    Hyper,
    /// UDP echo client for measuring PPS, RTT, and loss
    Udp,
}

#[derive(Parser, Debug)]
#[command(name = "dpdk-bench-client")]
#[command(about = "HTTP benchmark client similar to wrk", long_about = None)]
struct Args {
    /// Target URL to benchmark
    #[arg(required = true)]
    url: String,

    /// Number of concurrent connections
    #[arg(short = 'c', long, default_value = "10")]
    connections: usize,

    /// Duration of the benchmark (e.g., 10s, 1m)
    #[arg(short = 'd', long, default_value = "10s")]
    duration: String,

    /// Client mode: raw (fastest) or hyper (HTTP/2 support)
    #[arg(short = 'm', long, value_enum, default_value = "raw")]
    mode: ClientMode,

    /// Use HTTP/2 instead of HTTP/1.1 (only with hyper mode)
    #[arg(long, default_value = "false")]
    http2: bool,

    /// Print latency statistics
    #[arg(long, default_value = "true")]
    latency: bool,

    /// Request timeout in milliseconds
    #[arg(long, default_value = "5000")]
    timeout: u64,

    /// UDP packet size in bytes (only for udp mode, min 16)
    #[arg(long, default_value = "64")]
    packet_size: usize,
}

/// Parse duration string like "10s", "1m", "500ms"
fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if let Some(stripped) = s.strip_suffix("ms") {
        let num: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_millis(num))
    } else if let Some(stripped) = s.strip_suffix('s') {
        let num: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_secs(num))
    } else if let Some(stripped) = s.strip_suffix('m') {
        let num: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_secs(num * 60))
    } else {
        let num: u64 = s.parse().map_err(|_| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_secs(num))
    }
}

/// Round to 2 decimal places
fn round_2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .init();

    let args = Args::parse();

    let duration = parse_duration(&args.duration).expect("Invalid duration format");
    let timeout = Duration::from_millis(args.timeout);

    // Warn if HTTP/2 requested with incompatible mode
    if args.http2 && !matches!(args.mode, ClientMode::Hyper) {
        eprintln!("Warning: --http2 is only supported with --mode hyper, ignoring");
    }

    let worker_threads = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);

    let mode_str = match args.mode {
        ClientMode::Raw => "raw",
        ClientMode::Hyper => {
            if args.http2 {
                "hyper (HTTP/2)"
            } else {
                "hyper (HTTP/1.1)"
            }
        }
        ClientMode::Udp => "udp",
    };

    let stats = Arc::new(BenchStats::new());

    // Run the appropriate benchmark
    match args.mode {
        ClientMode::Raw => {
            raw_client::run_benchmark(
                &args.url,
                args.connections,
                duration,
                timeout,
                stats.clone(),
            )
            .await;
        }
        ClientMode::Hyper => {
            hyper_client::run_benchmark(
                &args.url,
                args.connections,
                duration,
                timeout,
                args.http2,
                stats.clone(),
            )
            .await;
        }
        ClientMode::Udp => {
            // For UDP mode, url is used as "host:port" target address
            let target = args.url.strip_prefix("udp://").unwrap_or(&args.url);
            udp_client::run_benchmark(
                target,
                args.connections,
                duration,
                timeout,
                args.packet_size,
                stats.clone(),
            )
            .await;
        }
    }

    // Collect final statistics
    let total_requests = stats.get_requests();
    let total_errors = stats.get_errors();
    let total_bytes = stats.get_bytes();
    let actual_duration = duration.as_secs_f64();
    let error_samples: Vec<String> = stats.error_samples.lock().await.clone();

    let latency = if args.latency {
        let hist = stats.latency_histogram.lock().await;
        if !hist.is_empty() {
            Some(LatencyStats {
                p50_us: hist.value_at_percentile(50.0),
                p75_us: hist.value_at_percentile(75.0),
                p90_us: hist.value_at_percentile(90.0),
                p99_us: hist.value_at_percentile(99.0),
                avg_us: hist.mean() as u64,
                max_us: hist.max(),
                stdev_us: hist.stdev() as u64,
            })
        } else {
            None
        }
    } else {
        None
    };

    let result = BenchmarkResult {
        url: args.url,
        connections: args.connections,
        duration_secs: round_2(actual_duration),
        mode: mode_str.to_string(),
        worker_threads,
        timeout_ms: args.timeout,
        requests: total_requests,
        errors: total_errors,
        gb_read: round_2(total_bytes as f64 / (1024.0 * 1024.0 * 1024.0)),
        requests_per_sec: round_2(total_requests as f64 / actual_duration),
        mb_per_sec: round_2((total_bytes as f64 / actual_duration) / (1024.0 * 1024.0)),
        error_samples: if error_samples.is_empty() {
            None
        } else {
            Some(error_samples)
        },
        latency,
    };

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

#[derive(Serialize)]
struct BenchmarkResult {
    url: String,
    connections: usize,
    duration_secs: f64,
    mode: String,
    worker_threads: usize,
    timeout_ms: u64,
    requests: u64,
    errors: u64,
    gb_read: f64,
    requests_per_sec: f64,
    mb_per_sec: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_samples: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency: Option<LatencyStats>,
}

#[derive(Serialize)]
struct LatencyStats {
    p50_us: u64,
    p75_us: u64,
    p90_us: u64,
    p99_us: u64,
    avg_us: u64,
    max_us: u64,
    stdev_us: u64,
}
