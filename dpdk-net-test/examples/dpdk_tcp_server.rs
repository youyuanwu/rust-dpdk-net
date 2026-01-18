//! DPDK TCP Echo Server using smoltcp (Async Version)
//!
//! This example starts an async TCP server on eth1 using DPDK+smoltcp.
//! It listens on port 8080 and echoes back any data received.
//!
//! Usage:
//!   sudo -E cargo run --example dpdk_tcp_server
//!
//! Then from another machine on the same network:
//!   nc 10.0.0.5 8080
//!   # Type messages and see them echoed back

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::api::rte::eth::{EthConf, EthDevBuilder, RxQueueConf, TxQueueConf};
use dpdk_net::api::rte::pktmbuf::{MemPool, MemPoolConfig};
use dpdk_net::api::rte::queue::{RxQueue, TxQueue};
use dpdk_net::tcp::{DpdkDeviceWithPool, Reactor, TcpListener, TcpStream};
use dpdk_net_test::dpdk_test::{DEFAULT_MBUF_DATA_ROOM_SIZE, DEFAULT_MBUF_HEADROOM, DEFAULT_MTU};
use smoltcp::iface::{Config, Interface};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::runtime::Builder;

const SERVER_PORT: u16 = 8080;

/// Statistics for the echo server
#[derive(Default)]
struct ServerStats {
    connections: AtomicU64,
    bytes_received: AtomicU64,
    bytes_sent: AtomicU64,
    send_errors: AtomicU64,
}

/// Handle a single client connection: receive and echo data until closed
async fn handle_connection(stream: TcpStream, conn_id: u64, stats: Arc<ServerStats>) {
    let mut buf = [0u8; 4096];

    loop {
        // Receive data
        let len = match stream.recv(&mut buf).await {
            Ok(0) => {
                println!("Connection {}: client closed", conn_id);
                break;
            }
            Ok(len) => len,
            Err(e) => {
                eprintln!("Connection {}: recv error: {:?}", conn_id, e);
                break;
            }
        };

        stats
            .bytes_received
            .fetch_add(len as u64, Ordering::Relaxed);

        // Echo it back
        match stream.send(&buf[..len]).await {
            Ok(len) => {
                stats.bytes_sent.fetch_add(len as u64, Ordering::Relaxed);
            }
            Err(e) => {
                eprintln!("Connection {}: send error: {:?}", conn_id, e);
                stats.send_errors.fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
    }

    // Close gracefully
    stream.close().await;
}

/// Run the async echo server
async fn run_server(mut listener: TcpListener, running: Arc<AtomicBool>, stats: Arc<ServerStats>) {
    println!("Server: listening on port {}", SERVER_PORT);

    let mut conn_id = 0u64;

    while running.load(Ordering::Relaxed) {
        // Use tokio::select! to check for shutdown while accepting
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok(stream) => {
                        let id = conn_id;
                        conn_id += 1;
                        stats.connections.fetch_add(1, Ordering::Relaxed);
                        println!("Connection {}: accepted", id);

                        // Spawn handler as background task
                        let stats_clone = stats.clone();
                        tokio::task::spawn_local(async move {
                            handle_connection(stream, id, stats_clone).await;
                        });
                    }
                    Err(e) => {
                        eprintln!("Server: accept error: {:?}", e);
                    }
                }
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                // Periodically check the running flag
            }
        }
    }

    println!("Server: shutting down...");
}

fn main() {
    // Setup hugepages
    dpdk_net_test::util::ensure_hugepages().unwrap();

    // Get network configuration for eth1
    let interface = "eth1";
    let ip_addr = dpdk_net_test::manual::tcp::get_interface_ipv4(interface)
        .expect("Failed to get IP address for eth1");
    let gateway =
        dpdk_net_test::manual::tcp::get_default_gateway().unwrap_or(Ipv4Address::new(10, 0, 0, 1));

    println!("\n========================================");
    println!("DPDK TCP Echo Server (Async)");
    println!("========================================");
    println!("IP Address: {}:{}", ip_addr, SERVER_PORT);
    println!("Gateway: {}", gateway);
    println!("\nServer is starting...\n");

    // Get PCI address for eth1
    let pci_addr = dpdk_net_test::manual::tcp::get_pci_addr(interface)
        .expect("Failed to get PCI address for eth1");

    // Initialize DPDK EAL with PCI device
    let _eal = EalBuilder::new()
        .arg(format!("-a {}", pci_addr))
        .init()
        .expect("Failed to initialize EAL");

    // Create mempool
    let mempool_config = MemPoolConfig::new()
        .num_mbufs(8192)
        .data_room_size(DEFAULT_MBUF_DATA_ROOM_SIZE as u16);
    let mempool =
        MemPool::create("server_pool", &mempool_config).expect("Failed to create mempool");

    // Configure and start ethernet device
    let eth_dev = EthDevBuilder::new(0)
        .eth_conf(EthConf::new())
        .nb_rx_queues(1)
        .nb_tx_queues(1)
        .rx_queue_conf(RxQueueConf::new().nb_desc(1024))
        .tx_queue_conf(TxQueueConf::new().nb_desc(1024))
        .build(&mempool)
        .expect("Failed to configure eth device");

    // Get queues
    let rxq = RxQueue::new(0, 0);
    let txq = TxQueue::new(0, 0);

    // Create DPDK device for smoltcp
    let mbuf_capacity = DEFAULT_MBUF_DATA_ROOM_SIZE - DEFAULT_MBUF_HEADROOM;
    let mut device = DpdkDeviceWithPool::new(rxq, txq, mempool, DEFAULT_MTU, mbuf_capacity);

    // Get MAC address from DPDK
    let mac = eth_dev.mac_addr().expect("Failed to get MAC address");
    let mac_addr = EthernetAddress(mac.addr_bytes);

    // Configure smoltcp interface
    let config = Config::new(mac_addr.into());
    let mut iface = Interface::new(config, &mut device, Instant::now());

    // Set IP address
    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs
            .push(IpCidr::new(IpAddress::Ipv4(ip_addr), 24))
            .unwrap();
    });

    // Add default route
    iface.routes_mut().add_default_ipv4_route(gateway).unwrap();

    println!("Interface configured:");
    println!("  IP: {}/24", ip_addr);
    println!("  MAC: {:?}", mac_addr);
    println!("  Gateway: {}", gateway);

    println!("\n✓ Server will listen on {}:{}", ip_addr, SERVER_PORT);
    println!("\nConnect from another machine:");
    println!("  nc {} {}", ip_addr, SERVER_PORT);
    println!("\nPress Ctrl+C to stop the server\n");
    println!("========================================\n");

    // Setup Ctrl+C handler
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl+C, shutting down...");
        running_clone.store(false, Ordering::Relaxed);
    })
    .expect("Failed to set Ctrl+C handler");

    // Create statistics tracker
    let stats = Arc::new(ServerStats::default());
    let stats_clone = stats.clone();

    // Record start time
    let start_time = std::time::Instant::now();

    // Create single-threaded tokio runtime
    // We use current_thread because DPDK and smoltcp are not thread-safe
    let rt = Builder::new_current_thread().enable_all().build().unwrap();

    // Create a LocalSet to run !Send futures (Rc-based reactor)
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        // Create the async reactor
        let reactor = Reactor::new(device, iface);
        let handle = reactor.handle();

        // Spawn the reactor polling task (runs in background)
        tokio::task::spawn_local(async move {
            reactor.run().await;
        });

        // Create server listener with reasonable backlog
        let listener = TcpListener::bind_with_backlog(&handle, SERVER_PORT, 4096, 4096, 16)
            .expect("Failed to bind listener");

        // Run the async server
        run_server(listener, running, stats_clone).await;
    });

    let runtime_secs = start_time.elapsed().as_secs();

    // Print final statistics
    println!("\n========================================");
    println!("Server Statistics:");
    println!("  Runtime: {} seconds", runtime_secs);
    println!(
        "  Connections: {}",
        stats.connections.load(Ordering::Relaxed)
    );
    println!(
        "  Bytes received: {}",
        stats.bytes_received.load(Ordering::Relaxed)
    );
    println!("  Bytes sent: {}", stats.bytes_sent.load(Ordering::Relaxed));
    println!(
        "  Send errors: {}",
        stats.send_errors.load(Ordering::Relaxed)
    );
    println!("========================================\n");

    // Cleanup
    println!("Cleaning up...");

    // Stop and close eth device
    let _ = eth_dev.stop();
    let _ = eth_dev.close();

    println!("Server stopped.");
}
