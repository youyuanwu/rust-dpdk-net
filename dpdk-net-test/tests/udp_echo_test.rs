// Test: UDP socket binding and basic operations
//
// This demonstrates how to use the UdpSocket with DPDK.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::api::rte::eth::{EthConf, EthDevBuilder, RxQueueConf, TxQueueConf};
use dpdk_net::api::rte::pktmbuf::{MemPool, MemPoolConfig};
use dpdk_net::api::rte::queue::{RxQueue, TxQueue};
use dpdk_net_test::dpdk_test::DEFAULT_MBUF_DATA_ROOM_SIZE;
use dpdk_net_test::udp::{Endpoint, UdpSocket};
use std::net::Ipv4Addr;

#[test]
fn test_udp_socket_basic() {
    // Initialize DPDK EAL
    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    // Create mempool
    let mempool_config = MemPoolConfig::new()
        .num_mbufs(8192)
        .data_room_size(DEFAULT_MBUF_DATA_ROOM_SIZE as u16);
    let mempool = MemPool::create("udp_pool", &mempool_config).expect("Failed to create mempool");

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

    // Create and configure UDP socket
    let mut socket = UdpSocket::new();
    socket.set_local_mac([0x00, 0x50, 0x56, 0xae, 0x76, 0xf5]);

    // Test binding
    assert!(
        socket
            .bind(Endpoint::new(Ipv4Addr::new(192, 168, 1, 100), 8080))
            .is_ok()
    );

    assert!(socket.is_bound());
    assert_eq!(
        socket.local_endpoint(),
        Some(Endpoint::new(Ipv4Addr::new(192, 168, 1, 100), 8080))
    );

    // Attach queues
    socket.attach_queues(rxq, txq, mempool).unwrap();

    // Test send capability
    assert!(socket.can_send());

    // Test sending a packet
    let remote_mac = [0x00, 0x0b, 0x86, 0x64, 0x8b, 0xa0];
    let remote_endpoint = Endpoint::new(Ipv4Addr::new(192, 168, 1, 200), 9000);
    let test_data = b"Hello, UDP!";

    assert!(
        socket
            .send_to(test_data, remote_endpoint, remote_mac)
            .is_ok()
    );
    assert_eq!(socket.tx_pending(), 1);

    // Flush and verify
    let sent = socket.flush().unwrap();
    assert!(sent > 0);

    // Poll for packets (won't receive any in test environment, but should not error)
    let result = socket.poll();
    assert!(result.is_ok());

    // Cleanup
    drop(socket);
    let _ = eth_dev.stop();
    let _ = eth_dev.close();
}
