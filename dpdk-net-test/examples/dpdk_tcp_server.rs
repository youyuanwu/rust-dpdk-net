//! DPDK TCP Echo Server using smoltcp
//!
//! This example starts a TCP server on eth1 using DPDK+smoltcp.
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
use dpdk_net::tcp::DpdkDeviceWithPool;
use dpdk_net_test::dpdk_test::{DEFAULT_MBUF_DATA_ROOM_SIZE, DEFAULT_MBUF_HEADROOM, DEFAULT_MTU};
use dpdk_net_test::echo_server::{EchoServerConfig, run_echo_server, setup_ctrlc_handler};
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};

fn main() {
    // Setup hugepages
    dpdk_net_test::util::ensure_hugepages().unwrap();

    // Get network configuration for eth1
    let interface = "eth1";
    let ip_addr = dpdk_net_test::tcp::get_interface_ipv4(interface)
        .expect("Failed to get IP address for eth1");
    let gateway =
        dpdk_net_test::tcp::get_default_gateway().unwrap_or(Ipv4Address::new(10, 0, 0, 1));

    println!("\n========================================");
    println!("DPDK TCP Echo Server");
    println!("========================================");
    println!("IP Address: {}:8080", ip_addr);
    println!("Gateway: {}", gateway);
    println!("\nServer is starting...\n");

    // Get PCI address for eth1
    let pci_addr =
        dpdk_net_test::tcp::get_pci_addr(interface).expect("Failed to get PCI address for eth1");

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

    // Create socket set
    let mut sockets = SocketSet::new(vec![]);

    let port = 8080;
    println!("\n✓ Server will listen on {}:{}", ip_addr, port);
    println!("\nConnect from another machine:");
    println!("  nc {} {}", ip_addr, port);
    println!("\nPress Ctrl+C to stop the server\n");
    println!("========================================\n");

    // Setup Ctrl+C handler
    let running = setup_ctrlc_handler();

    // Run the echo server loop (creates and manages the socket internally)
    let result = run_echo_server(
        &mut device,
        &mut iface,
        &mut sockets,
        port,
        running,
        EchoServerConfig::default(),
    );

    // Print final statistics
    println!("\n========================================");
    println!("Server Statistics:");
    println!("  Runtime: {} seconds", result.runtime_secs);
    println!("  Connections: {}", result.stats.connections);
    println!("  Bytes received: {}", result.stats.bytes_received);
    println!("  Bytes sent: {}", result.stats.bytes_sent);
    println!("  Bytes dropped: {}", result.stats.bytes_dropped);
    println!("  Send errors: {}", result.stats.send_errors);
    println!("========================================\n");

    // Cleanup
    println!("Cleaning up...");
    drop(device);
    drop(sockets);
    drop(iface);

    // Stop and close eth device
    let _ = eth_dev.stop();
    let _ = eth_dev.close();

    println!("Server stopped.");
}
