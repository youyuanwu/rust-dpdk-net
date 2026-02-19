//! HTTP Benchmark Server - DPDK, Tokio, or Kimojio
//!
//! A high-performance HTTP server for benchmarking, supporting DPDK, Tokio, and Kimojio backends.
//!
//! Supports four modes:
//! - **dpdk**: Multi-queue DPDK + smoltcp + hyper (requires root, hardware NIC)
//! - **tokio**: Standard tokio + hyper with multi-threaded runtime (works anywhere)
//! - **tokio-local**: Thread-per-core tokio + hyper with CPU pinning (works anywhere)
//! - **kimojio**: Thread-per-core io_uring + custom HTTP parser (Linux 5.15+)
//!
//! # Usage
//!
//! ```bash
//! # DPDK mode (requires sudo)
//! sudo -E dpdk-bench-server --mode dpdk
//!
//! # Tokio mode (no sudo needed)
//! dpdk-bench-server --mode tokio
//!
//! # Tokio thread-per-core mode
//! dpdk-bench-server --mode tokio-local
//!
//! # Kimojio io_uring mode (Linux 5.15+)
//! dpdk-bench-server --mode kimojio
//!
//! # Custom address and port
//! dpdk-bench-server --mode tokio --addr 127.0.0.1:3000
//! ```
//!
//! # Testing
//!
//! ```bash
//! curl http://localhost:8080/
//! dpdk-bench-client -c 10 -d 10s http://localhost:8080/
//! ```

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::{Parser, ValueEnum};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Request, Response, StatusCode};
use tracing::info;
use tracing_subscriber::EnvFilter;

/// Global request counter shared across all connections
static REQUEST_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ServerMode {
    /// DPDK + smoltcp + hyper (requires root and hardware NIC)
    Dpdk,
    /// Standard tokio + hyper with multi-threaded runtime
    Tokio,
    /// Thread-per-core tokio + hyper with CPU pinning
    TokioLocal,
    /// Thread-per-core kimojio + io_uring (Linux 5.15+)
    Kimojio,
    /// Thread-per-core kimojio + io_uring with busy polling (Linux 5.15+)
    KimojioPoll,
}

#[derive(Parser, Debug)]
#[command(name = "dpdk-bench-server")]
#[command(about = "HTTP benchmark server with DPDK, Tokio, or Kimojio backend")]
struct Args {
    /// Server mode: dpdk, tokio, tokio-local, or kimojio
    #[arg(short, long, value_enum, default_value = "dpdk")]
    mode: ServerMode,

    /// Listen address for tokio mode (ignored in dpdk mode)
    #[arg(short, long, default_value = "0.0.0.0:8080")]
    addr: SocketAddr,

    /// Network interface for DPDK mode (ignored in tokio mode)
    #[arg(short, long, default_value = "eth1")]
    interface: String,

    /// Server port (used in DPDK mode)
    #[arg(short, long, default_value = "8080")]
    port: u16,

    /// IP address for DPDK mode (required when interface is unbound for vfio-pci)
    #[arg(long)]
    ip_addr: Option<String>,

    /// Gateway address for DPDK mode (defaults to 10.0.0.1)
    #[arg(long)]
    gateway: Option<String>,

    /// Number of hardware queues (required when interface is unbound for vfio-pci)
    #[arg(long)]
    hw_queues: Option<usize>,

    /// Maximum number of queues for DPDK mode
    #[arg(long)]
    max_queues: Option<usize>,

    /// Listen backlog for DPDK mode (number of pending connections)
    #[arg(long, default_value = "64")]
    backlog: usize,
}

/// Generate the HTML response body for the counter page.
fn generate_counter_html() -> Bytes {
    let count = REQUEST_COUNT.fetch_add(1, Ordering::Relaxed);

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>HTTP Benchmark Server</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
        }}
        .container {{
            text-align: center;
            padding: 2rem;
            background: rgba(255, 255, 255, 0.1);
            border-radius: 20px;
            backdrop-filter: blur(10px);
        }}
        h1 {{ font-size: 3rem; margin-bottom: 0.5rem; }}
        .count {{ font-size: 6rem; font-weight: bold; }}
        .label {{ font-size: 1.5rem; opacity: 0.8; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>🚀 HTTP Benchmark Server</h1>
        <div class="count">{}</div>
        <div class="label">requests received</div>
    </div>
</body>
</html>"#,
        count
    );

    Bytes::from(html)
}

/// HTTP handler for hyper-based servers (tokio, dpdk).
/// Returns Result<Response<Full<Bytes>>, hyper::Error> for compatibility with hyper.
async fn counter_handler(_req: Request<Bytes>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Full::new(generate_counter_html()))
        .unwrap())
}

/// HTTP handler for kimojio-based server.
/// Returns Response<Bytes> directly for use with kimojio's HTTP server.
async fn counter_handler_kimojio(_req: Request<Bytes>) -> Response<Bytes> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(generate_counter_html())
        .unwrap()
}

/// Run the DPDK-based HTTP server
fn run_dpdk_server(
    interface: &str,
    port: u16,
    max_queues: Option<usize>,
    backlog: usize,
    ip_addr: Option<&str>,
    gateway: Option<&str>,
    hw_queues: Option<usize>,
) {
    use dpdk_net::api::rte::eal::EalBuilder;
    use dpdk_net::socket::TcpListener;
    use dpdk_net_test::app::http_server::Http1Server;
    use dpdk_net_test::manual::tcp::{get_default_gateway, get_interface_ipv4, get_pci_addr};
    use dpdk_net_util::{DpdkApp, WorkerContext};
    use smoltcp::wire::Ipv4Address;
    use tokio_util::sync::CancellationToken;
    use tracing::warn;

    // Detect network config: use explicit IP/gateway if provided, otherwise auto-detect
    let ip = if let Some(ip_str) = ip_addr {
        Ipv4Address::from_str(ip_str).expect("Invalid IP address")
    } else {
        get_interface_ipv4(interface).expect("Failed to get IP address for interface")
    };
    let gw = if let Some(gw_str) = gateway {
        Ipv4Address::from_str(gw_str).expect("Invalid gateway")
    } else {
        get_default_gateway().unwrap_or(Ipv4Address::new(10, 0, 0, 1))
    };

    // Detect hardware queues: use explicit value if provided, otherwise auto-detect
    let num_queues = if let Some(queues) = hw_queues {
        queues
    } else {
        dpdk_net_test::util::get_ethtool_channels(interface)
            .map(|ch| ch.combined_count as usize)
            .expect("Failed to get hardware queues via ethtool")
    };

    // Apply max_queues cap
    let num_queues = if let Some(max) = max_queues {
        num_queues.min(max)
    } else {
        num_queues
    };
    let core_list = format!("0-{}", num_queues.saturating_sub(1));

    // Initialize DPDK EAL with core list matching queue count
    let pci_addr = get_pci_addr(interface).expect("Failed to get PCI address");
    let _eal = EalBuilder::new()
        .allow(&pci_addr)
        .core_list(&core_list)
        .init()
        .expect("Failed to initialize EAL");

    // Setup Ctrl+C handler
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    ctrlc::set_handler(move || {
        warn!("Received Ctrl+C, shutting down");
        cancel_clone.cancel();
    })
    .expect("Failed to set Ctrl+C handler");

    DpdkApp::new()
        .eth_dev(0)
        .ip(ip)
        .gateway(gw)
        .run(move |ctx: WorkerContext| {
            let cancel = cancel.clone();
            async move {
                let listener =
                    TcpListener::bind_with_backlog(&ctx.reactor, port, 16384, 16384, backlog)
                        .expect("Failed to bind listener");
                Http1Server::new(
                    listener,
                    cancel,
                    counter_handler,
                    ctx.queue_id as usize,
                    port,
                )
                .run()
                .await
            }
        });
}

fn main() {
    // Initialize tracing - respects RUST_LOG, defaults to info if not set
    // Disable ANSI colors for clean log output
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .init();

    let args = Args::parse();

    match args.mode {
        ServerMode::Tokio => {
            use dpdk_net_test::app::tokio_server::run_tokio_multi_thread_server;

            info!(mode = "tokio", addr = %args.addr, "Starting HTTP benchmark server");
            run_tokio_multi_thread_server(args.addr, counter_handler);
        }
        ServerMode::TokioLocal => {
            use dpdk_net_test::app::tokio_server::run_tokio_thread_per_core_server;

            info!(mode = "tokio-local", addr = %args.addr, "Starting HTTP benchmark server");
            run_tokio_thread_per_core_server(args.addr, counter_handler);
        }
        ServerMode::Dpdk => {
            info!(
                mode = "dpdk",
                interface = %args.interface,
                port = args.port,
                ip_addr = ?args.ip_addr,
                gateway = ?args.gateway,
                hw_queues = ?args.hw_queues,
                max_queues = args.max_queues,
                backlog = args.backlog,
                "Starting HTTP benchmark server"
            );
            run_dpdk_server(
                &args.interface,
                args.port,
                args.max_queues,
                args.backlog,
                args.ip_addr.as_deref(),
                args.gateway.as_deref(),
                args.hw_queues,
            );
        }
        ServerMode::Kimojio => {
            use dpdk_net_test::app::kimojio_server::run_kimojio_thread_per_core_server;

            info!(
                mode = "kimojio",
                port = args.port,
                "Starting HTTP benchmark server"
            );
            run_kimojio_thread_per_core_server(args.port, counter_handler_kimojio, false);
        }
        ServerMode::KimojioPoll => {
            use dpdk_net_test::app::kimojio_server::run_kimojio_thread_per_core_server;

            info!(
                mode = "kimojio-poll",
                port = args.port,
                "Starting HTTP benchmark server with busy polling"
            );
            run_kimojio_thread_per_core_server(args.port, counter_handler_kimojio, true);
        }
    }
}
