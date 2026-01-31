//! DPDK TCP Echo Server using smoltcp (Async Version) - Multi-Queue
//!
//! This example starts an async TCP server on eth1 using DPDK+smoltcp.
//! It detects the number of hardware queues and spawns one tokio runtime
//! per queue for maximum performance.
//!
//! Usage:
//!   sudo -E cargo run --example dpdk_tcp_server
//!
//! Then from another machine on the same network:
//!   nc 10.0.0.5 8080
//!   # Type messages and see them echoed back

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net_test::app::dpdk_server_runner::DpdkServerRunner;
use dpdk_net_test::app::echo_server::{EchoServer, ServerStats};
use dpdk_net_test::manual::tcp::get_pci_addr;
use dpdk_net_test::util::ensure_hugepages;
use std::sync::Arc;
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

    // Setup hugepages (user's responsibility before using the runner)
    ensure_hugepages().expect("Failed to ensure hugepages");

    // Initialize DPDK EAL (user's responsibility before using the runner)
    let pci_addr = get_pci_addr(INTERFACE).expect("Failed to get PCI address");
    let _eal = EalBuilder::new()
        .allow(&pci_addr)
        .init()
        .expect("Failed to initialize EAL");

    // Shared statistics across all queues
    let stats = Arc::new(ServerStats::default());
    let start_time = std::time::Instant::now();

    // Clone for the closure
    let stats_for_runner = stats.clone();

    DpdkServerRunner::new(INTERFACE)
        .with_default_network_config()
        .with_default_hw_queues()
        .port(SERVER_PORT)
        .run(move |ctx| {
            let stats = stats_for_runner.clone();
            async move {
                // Create and run echo server for this queue
                EchoServer::new(ctx.listener, ctx.cancel, stats, ctx.queue_id, ctx.port)
                    .run()
                    .await
            }
        });

    // Print final statistics
    let runtime_secs = start_time.elapsed().as_secs();
    stats.print_summary(runtime_secs);
}
