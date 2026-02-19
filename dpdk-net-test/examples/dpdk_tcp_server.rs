//! DPDK TCP Echo Server using smoltcp (Async Version) - Multi-Queue
//!
//! This example starts an async TCP server on eth1 using DPDK+smoltcp.
//! It detects the number of hardware queues and spawns one tokio runtime
//! per lcore for maximum performance.
//!
//! Usage:
//!   sudo -E cargo run --example dpdk_tcp_server
//!
//! Then from another machine on the same network:
//!   nc 10.0.0.5 8080
//!   # Type messages and see them echoed back

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::TcpListener;
use dpdk_net_test::app::echo_server::{EchoServer, ServerStats};
use dpdk_net_test::manual::tcp::{get_default_gateway, get_interface_ipv4, get_pci_addr};
use dpdk_net_test::util::{ensure_hugepages, get_ethtool_channels};
use dpdk_net_util::{DpdkApp, WorkerContext};
use smoltcp::wire::Ipv4Address;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use tracing_subscriber::EnvFilter;

const SERVER_PORT: u16 = 8080;
const INTERFACE: &str = "eth1";

fn main() {
    // Initialize tracing subscriber with env filter
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("dpdk_net_test=info".parse().unwrap()),
        )
        .init();

    // Setup hugepages
    ensure_hugepages().expect("Failed to ensure hugepages");

    // Auto-detect network config from interface (before EAL init)
    let ip_addr = get_interface_ipv4(INTERFACE).expect("Failed to get IP address for interface");
    let gateway = get_default_gateway().unwrap_or(Ipv4Address::new(10, 0, 0, 1));

    // Auto-detect hardware queues (before EAL init)
    let hw_queues = get_ethtool_channels(INTERFACE)
        .map(|ch| ch.combined_count as usize)
        .expect("Failed to get hardware queues via ethtool");
    let core_list = format!("0-{}", hw_queues.saturating_sub(1));

    // Initialize DPDK EAL with core list matching hw queues
    let pci_addr = get_pci_addr(INTERFACE).expect("Failed to get PCI address");
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

    // Shared statistics across all queues
    let stats = Arc::new(ServerStats::default());
    let start_time = std::time::Instant::now();

    let stats_for_runner = stats.clone();

    DpdkApp::new()
        .eth_dev(0)
        .ip(ip_addr)
        .gateway(gateway)
        .run(move |ctx: WorkerContext| {
            let stats = stats_for_runner.clone();
            let cancel = cancel.clone();
            async move {
                let listener = TcpListener::bind(&ctx.reactor, SERVER_PORT, 4096, 4096).unwrap();
                EchoServer::new(listener, cancel, stats, ctx.queue_id as usize, SERVER_PORT)
                    .run()
                    .await
            }
        });

    // Print final statistics
    let runtime_secs = start_time.elapsed().as_secs();
    stats.print_summary(runtime_secs);
}
