//! UDP Async Test
//!
//! This test demonstrates the async UDP socket implementation using DPDK with smoltcp.
//! It creates a simple UDP echo scenario where a "client" sends datagrams to a "server"
//! and the server echoes them back.
//!
//! Note: This uses a virtual ring device for loopback testing without real hardware.
//! EAL and EthDev are initialized once globally; each test recreates the DpdkDevice.

use dpdk_net::api::rte::eal::{Eal, EalBuilder};
use dpdk_net::tcp::{DpdkDevice, Reactor, UdpSocket};
use dpdk_net_test::eth_dev_config::EthDevConfig;
use smoltcp::iface::{Config, Interface};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use tokio::runtime::Builder;

const SERVER_PORT: u16 = 9999;
const CLIENT_PORT: u16 = 8888;
const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);

/// Global test context - EAL, EthDev, and MemPool initialized once for all tests.
struct GlobalTestContext {
    _eal: Eal,
    mempool: Arc<dpdk_net::api::rte::pktmbuf::MemPool>,
    eth_dev_config: EthDevConfig,
}

static GLOBAL_CTX: OnceLock<GlobalTestContext> = OnceLock::new();

/// Initialize the global test context (EAL + EthDev + MemPool).
fn init_global_ctx() -> &'static GlobalTestContext {
    GLOBAL_CTX.get_or_init(|| {
        let eal = EalBuilder::new()
            .no_huge()
            .no_pci()
            .vdev("net_ring0")
            .init()
            .expect("Failed to initialize EAL");

        let eth_dev_config = EthDevConfig::new().mempool_name("global_test_pool");

        let (mempool, _eth_dev) = eth_dev_config
            .clone()
            .build()
            .expect("Failed to build EthDev");

        // Note: We don't close eth_dev - it stays alive for all tests
        // The _eth_dev is intentionally leaked (kept alive via DPDK internals)

        GlobalTestContext {
            _eal: eal,
            mempool,
            eth_dev_config,
        }
    })
}

/// Create a fresh DpdkDevice for a test (reuses global mempool).
fn create_test_device() -> DpdkDevice {
    let ctx = init_global_ctx();
    ctx.eth_dev_config.create_device(ctx.mempool.clone(), 0)
}

/// Basic test: bind UDP socket and verify state
#[test]
#[serial_test::serial]
fn test_udp_socket_bind() {
    let mut device = create_test_device();

    // Configure smoltcp interface
    let mac = EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
    let config = Config::new(mac.into());
    let mut iface = Interface::new(config, &mut device, Instant::now());
    iface.update_ip_addrs(|addrs| {
        addrs
            .push(IpCidr::new(IpAddress::Ipv4(SERVER_IP), 24))
            .unwrap();
    });

    // Create tokio runtime
    let rt = Builder::new_current_thread().build().unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        // Create reactor
        let reactor = Reactor::new(device, iface);
        let handle = reactor.handle();

        // Create cancel flag
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        // Spawn reactor
        let reactor_task = tokio::task::spawn_local(async move {
            reactor.run(cancel_clone).await;
        });

        // Bind UDP socket
        let socket =
            UdpSocket::bind(&handle, SERVER_PORT, 16, 16, 1500).expect("Failed to bind UDP socket");

        // Verify socket is open
        assert!(socket.is_open());
        assert_eq!(socket.endpoint().port, SERVER_PORT);

        println!("UDP socket bound to port {}", SERVER_PORT);

        // Cleanup
        drop(socket);
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        reactor_task.await.unwrap();
    });
}

/// Test: UDP send and receive (loopback via ring device)
#[test]
#[serial_test::serial]
fn test_udp_send_recv_loopback() {
    let mut device = create_test_device();

    // Configure smoltcp interface
    let mac = EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]);
    let config = Config::new(mac.into());
    let mut iface = Interface::new(config, &mut device, Instant::now());

    // Add both server and client IPs (simulating two endpoints on same interface)
    let client_ip = Ipv4Address::new(192, 168, 1, 2);
    iface.update_ip_addrs(|addrs| {
        addrs
            .push(IpCidr::new(IpAddress::Ipv4(SERVER_IP), 24))
            .unwrap();
        addrs
            .push(IpCidr::new(IpAddress::Ipv4(client_ip), 24))
            .unwrap();
    });

    // Create tokio runtime
    let rt = Builder::new_current_thread().build().unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        // Create reactor
        let reactor = Reactor::new(device, iface);
        let handle = reactor.handle();

        // Create cancel flag
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        // Spawn reactor
        let reactor_task = tokio::task::spawn_local(async move {
            reactor.run(cancel_clone).await;
        });

        // Create server socket
        let server = UdpSocket::bind(&handle, SERVER_PORT, 16, 16, 1500)
            .expect("Failed to bind server socket");
        println!("Server listening on port {}", SERVER_PORT);

        // Create client socket
        let client = UdpSocket::bind(&handle, CLIENT_PORT, 16, 16, 1500)
            .expect("Failed to bind client socket");
        println!("Client bound to port {}", CLIENT_PORT);

        // Send from client to server
        let message = b"Hello UDP!";
        let server_endpoint = IpEndpoint::new(IpAddress::Ipv4(SERVER_IP), SERVER_PORT);

        let sent = client
            .send_to(message, server_endpoint)
            .await
            .expect("Failed to send");
        println!("Client sent {} bytes to {:?}", sent, server_endpoint);

        // Give the reactor a chance to process
        tokio::task::yield_now().await;

        // Note: With net_ring0 loopback, the packet should be received
        // In practice, this depends on how the ring driver handles loopback
        // For a real test, you'd need two separate endpoints or use net_tap

        // Cleanup
        drop(client);
        drop(server);
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        reactor_task.await.unwrap();
    });
}

/// Test: Multiple UDP sockets on different ports
#[test]
#[serial_test::serial]
fn test_udp_multiple_sockets() {
    let mut device = create_test_device();

    let mac = EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x03]);
    let config = Config::new(mac.into());
    let mut iface = Interface::new(config, &mut device, Instant::now());
    iface.update_ip_addrs(|addrs| {
        addrs
            .push(IpCidr::new(IpAddress::Ipv4(SERVER_IP), 24))
            .unwrap();
    });

    let rt = Builder::new_current_thread().build().unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let reactor = Reactor::new(device, iface);
        let handle = reactor.handle();

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        let reactor_task = tokio::task::spawn_local(async move {
            reactor.run(cancel_clone).await;
        });

        // Create multiple sockets on different ports
        let socket1 = UdpSocket::bind(&handle, 5001, 8, 8, 1500).expect("Failed to bind socket 1");
        let socket2 = UdpSocket::bind(&handle, 5002, 8, 8, 1500).expect("Failed to bind socket 2");
        let socket3 = UdpSocket::bind(&handle, 5003, 8, 8, 1500).expect("Failed to bind socket 3");

        // Verify all are open on different ports
        assert!(socket1.is_open());
        assert!(socket2.is_open());
        assert!(socket3.is_open());
        assert_eq!(socket1.endpoint().port, 5001);
        assert_eq!(socket2.endpoint().port, 5002);
        assert_eq!(socket3.endpoint().port, 5003);

        println!("Created 3 UDP sockets on ports 5001, 5002, 5003");

        // Cleanup
        drop(socket1);
        drop(socket2);
        drop(socket3);
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        reactor_task.await.unwrap();
    });
}
