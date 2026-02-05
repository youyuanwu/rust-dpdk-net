use nix::ifaddrs::getifaddrs;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};
use std::fs;
use std::io::{BufRead, BufReader};
use std::time::Duration;

use super::tcp_echo::{EchoClient, EchoServer, SocketConfig, run_echo_test};
use crate::dpdk_test::{
    DEFAULT_MBUF_DATA_ROOM_SIZE, DEFAULT_MBUF_HEADROOM, DEFAULT_MTU, DpdkDevice, DpdkTestContext,
    DpdkTestContextBuilder,
};

/// Get PCI address for a network interface.
///
/// This function supports multiple scenarios:
/// 1. Direct PCI device (e.g., mlx5 interface) - reads from sysfs
/// 2. Virtio device (QEMU/KVM) - follows parent to find PCI address
/// 3. Azure hv_netvsc - follows lower_ links to find VF
/// 4. **Fallback**: If interface doesn't exist (unbound for DPDK), scans for
///    virtio-net devices bound to vfio-pci
pub fn get_pci_addr(interface: &str) -> Option<String> {
    // First, try the normal path via interface
    if let Some(addr) = get_pci_addr_from_interface(interface) {
        return Some(addr);
    }

    // Fallback: Interface doesn't exist (likely unbound for DPDK use)
    // Scan for virtio-net devices bound to vfio-pci
    tracing::warn!(
        interface,
        "Interface not found, scanning for vfio-pci bound virtio-net devices"
    );
    if let Some(addr) = find_vfio_virtio_net() {
        return Some(addr);
    }

    tracing::warn!(interface, "Could not find PCI address");
    None
}

/// Find a virtio-net device bound to vfio-pci.
///
/// Scans /sys/bus/pci/devices/ for devices with:
/// - vendor = 0x1af4 (Red Hat / Virtio)
/// - device = 0x1000 (virtio-net)
/// - driver = vfio-pci
fn find_vfio_virtio_net() -> Option<String> {
    let devices_dir = "/sys/bus/pci/devices";
    let entries = fs::read_dir(devices_dir).ok()?;

    for entry in entries.flatten() {
        let pci_addr = entry.file_name().to_str()?.to_string();
        let device_path = entry.path();

        // Check vendor (0x1af4 = Red Hat / Virtio)
        let vendor_path = device_path.join("vendor");
        let vendor = fs::read_to_string(&vendor_path).ok()?;
        if vendor.trim() != "0x1af4" {
            continue;
        }

        // Check device ID (0x1000 = virtio-net)
        let device_id_path = device_path.join("device");
        let device_id = fs::read_to_string(&device_id_path).ok()?;
        if device_id.trim() != "0x1000" {
            continue;
        }

        // Check if bound to vfio-pci
        let driver_path = device_path.join("driver");
        if let Ok(driver_link) = fs::read_link(&driver_path)
            && let Some(driver_name) = driver_link.file_name()
            && driver_name.to_str() == Some("vfio-pci")
        {
            tracing::info!(pci_addr, "Found virtio-net device bound to vfio-pci");
            return Some(pci_addr);
        }
    }

    None
}

/// Get PCI address from a network interface via sysfs.
///
/// This handles:
/// - Direct PCI devices (e.g., mlx5) - device symlink points to PCI address
/// - Azure hv_netvsc - follows lower_ links to find the VF's PCI address
///
/// Note: Does NOT handle virtio devices. For virtio, the interface exists only
/// when bound to kernel driver. Once bound to vfio-pci for DPDK use, the interface
/// disappears and we must use `find_vfio_virtio_net()` instead.
fn get_pci_addr_from_interface(interface: &str) -> Option<String> {
    let path = format!("/sys/class/net/{}/device", interface);
    let link = fs::read_link(&path).ok()?;
    let filename = link.file_name()?.to_str()?;

    // Check if this is a PCI device directly (e.g., mlx5 interface)
    if filename.contains(':') && filename.contains('.') {
        tracing::debug!(interface, pci_addr = filename, "Found PCI address directly");
        return Some(filename.to_string());
    }

    // Skip virtio devices - if interface exists, device is kernel-bound and unusable by DPDK
    // The vfio-pci fallback in get_pci_addr() will find it after binding
    if filename.starts_with("virtio") {
        tracing::warn!(
            interface,
            "Virtio device still bound to kernel driver, trying vfio-pci fallback"
        );
        return None;
    }

    tracing::debug!(
        interface,
        device = filename,
        "Device is not PCI, checking for lower_ links"
    );

    // If not a PCI device (e.g., hv_netvsc on Azure), look for lower_ links to find the slave VF
    let net_dir = format!("/sys/class/net/{}", interface);
    if let Ok(entries) = fs::read_dir(&net_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(name_str) = name.to_str()
                && name_str.starts_with("lower_")
            {
                tracing::debug!(interface, lower_link = name_str, "Found lower link");
                // Follow the lower link to the actual device
                if let Ok(lower_link) = fs::read_link(entry.path()) {
                    // Extract PCI address from path like: ../../../.../c167:00:02.0/net/enP49511s2
                    let path_str = lower_link.to_str()?;
                    tracing::debug!(interface, path = path_str, "Lower link target");
                    for component in path_str.split('/') {
                        // Match PCI address pattern: XXXX:XX:XX.X (domain:bus:device.function)
                        if component.contains(':') && component.contains('.') {
                            // Verify it looks like a PCI address (not a GUID)
                            let parts: Vec<&str> = component.split(':').collect();
                            if parts.len() >= 2
                                && parts.last().map(|s| s.contains('.')).unwrap_or(false)
                            {
                                tracing::info!(
                                    interface,
                                    pci_addr = component,
                                    "Found PCI address via lower link"
                                );
                                return Some(component.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Get IPv4 address for a network interface
pub fn get_interface_ipv4(interface: &str) -> Option<Ipv4Address> {
    let ifaddrs = getifaddrs().ok()?;

    for ifaddr in ifaddrs {
        if ifaddr.interface_name == interface
            && let Some(address) = ifaddr.address
            && let Some(sockaddr_in) = address.as_sockaddr_in()
        {
            let ip = sockaddr_in.ip();
            let octets = ip.octets();
            return Some(Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]));
        }
    }

    None
}

/// Parse hex gateway string (little-endian format) to IPv4 address
/// Example: "0100000A" -> 10.0.0.1
pub fn parse_hex_gateway(hex_str: &str) -> Option<Ipv4Address> {
    let gateway_hex = u32::from_str_radix(hex_str, 16).ok()?;
    // Convert from little-endian hex to IP octets
    let octets = gateway_hex.to_le_bytes();
    Some(Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]))
}

/// Get default gateway IPv4 address from routing table
pub fn get_default_gateway() -> Option<Ipv4Address> {
    let file = fs::File::open("/proc/net/route").ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().skip(1) {
        // Skip header line
        let line = line.ok()?;
        let fields: Vec<&str> = line.split_whitespace().collect();

        if fields.len() < 3 {
            continue;
        }

        // Check if this is the default route (Destination == 00000000)
        if fields[1] == "00000000" {
            return parse_hex_gateway(fields[2]);
        }
    }

    None
}

/// Run TCP echo test using new wrappers
fn run_tcp_echo_with_device(
    device: &mut DpdkDevice,
    ip_addr: Ipv4Address,
    gateway: Ipv4Address,
    mac_addr: EthernetAddress,
) {
    // Configure smoltcp interface with the device's MAC address
    let config = Config::new(mac_addr.into());
    let mut iface = Interface::new(config, device, Instant::now());

    // Set IP address and enable loopback
    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs
            .push(IpCidr::new(IpAddress::Ipv4(ip_addr), 24))
            .unwrap();
    });

    // Add a route for loopback traffic
    iface.routes_mut().add_default_ipv4_route(gateway).unwrap();

    // Create socket set
    let mut sockets = SocketSet::new(vec![]);

    // Create server and client using the new API
    let mut server = EchoServer::new(&mut sockets, 8080, SocketConfig::default());

    let mut client = EchoClient::new(
        &mut sockets,
        &mut iface,
        ip_addr,
        8080,
        49152,
        SocketConfig::default(),
    );
    client.send(b"Hello, TCP server!");

    println!("Smoltcp interface initialized on DPDK");
    println!("IP: {}/24", ip_addr);
    println!("MAC: {:?}", mac_addr);
    println!("Server listening on port 8080");

    // Run the echo test
    let result = run_echo_test(
        device,
        &mut iface,
        &mut sockets,
        &mut server,
        &mut client,
        Duration::from_secs(5),
    );

    // Print results
    println!("\n=== Test Results ===");
    println!("  Connected: {}", result.connected);
    println!("  Bytes sent: {}", result.bytes_sent);
    println!("  Bytes received: {}", result.bytes_received);
    println!("  Echo verified: {}", result.echo_verified);
    println!("  Server stats: {:?}", server.stats());

    // Assert that the full echo cycle completed
    assert!(result.connected, "Client failed to connect");
    assert!(result.echo_verified, "Echo verification failed");
    assert_eq!(result.bytes_sent, 18, "Wrong number of bytes sent");
    assert_eq!(result.bytes_received, 18, "Wrong number of bytes received");

    println!("Echo test completed successfully!");
}

pub fn tcp_echo_test(use_hardware: bool) {
    // Get IP address BEFORE DPDK takes over the interface
    let interface = "eth1";
    let ip_addr = if use_hardware {
        let addr = get_interface_ipv4(interface).expect("Failed to get IP address for eth1");
        println!("[Debug] Detected IP address for {}: {:?}", interface, addr);
        addr
    } else {
        Ipv4Address::new(192, 168, 1, 100) // Use test IP for virtual device
    };

    let gateway = get_default_gateway().unwrap_or(Ipv4Address::new(10, 0, 0, 1)); // Fallback to Azure default
    println!("[Debug] Detected gateway: {:?}", gateway);

    // Build DPDK context using new wrappers
    let (_ctx, mut device): (DpdkTestContext, DpdkDevice) = if use_hardware {
        // Dynamically get PCI address for eth1
        let pci_addr = get_pci_addr(interface).expect("Failed to get PCI address for eth1");

        // For hardware, we need to use EalBuilder directly with PCI device
        use dpdk_net::api::rte::eal::EalBuilder;
        use dpdk_net::api::rte::eth::{EthConf, EthDevBuilder, RxQueueConf, TxQueueConf};
        use dpdk_net::api::rte::pktmbuf::{MemPool, MemPoolConfig};
        use dpdk_net::api::rte::queue::{RxQueue, TxQueue};

        let eal = EalBuilder::new()
            .arg(format!("-a {}", pci_addr))
            .init()
            .expect("Failed to initialize EAL");

        let mempool_config = MemPoolConfig::new()
            .num_mbufs(8191)
            .data_room_size(DEFAULT_MBUF_DATA_ROOM_SIZE as u16);
        let mempool =
            MemPool::create("tcp_pool", &mempool_config).expect("Failed to create mempool");

        let eth_dev = EthDevBuilder::new(0)
            .eth_conf(EthConf::new())
            .nb_rx_queues(1)
            .nb_tx_queues(1)
            .rx_queue_conf(RxQueueConf::new().nb_desc(1024))
            .tx_queue_conf(TxQueueConf::new().nb_desc(1024))
            .build(&mempool)
            .expect("Failed to configure eth device");

        let rxq = RxQueue::new(0, 0);
        let txq = TxQueue::new(0, 0);
        let mbuf_capacity = DEFAULT_MBUF_DATA_ROOM_SIZE - DEFAULT_MBUF_HEADROOM;
        let device = DpdkDevice::new(
            rxq,
            txq,
            std::sync::Arc::new(mempool),
            DEFAULT_MTU,
            mbuf_capacity,
        );

        let ctx = DpdkTestContext::from_parts(eal, eth_dev);
        (ctx, device)
    } else {
        DpdkTestContextBuilder::new()
            .vdev("net_ring0")
            .mempool_name("tcp_pool")
            .build()
            .expect("Failed to create DPDK test context")
    };

    // Get the actual MAC address from DPDK device
    let mac = _ctx
        .eth_dev()
        .mac_addr()
        .expect("Failed to get MAC address");
    let mac_addr = EthernetAddress(mac.addr_bytes);

    run_tcp_echo_with_device(&mut device, ip_addr, gateway, mac_addr);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_gateway() {
        // Test Azure default gateway: 10.0.0.1
        // Hex representation (little-endian): 0x0100000A
        let result = parse_hex_gateway("0100000A");
        assert_eq!(result, Some(Ipv4Address::new(10, 0, 0, 1)));
    }

    #[test]
    fn test_parse_hex_gateway_different_addresses() {
        // Test 192.168.1.1: 0x0101A8C0
        assert_eq!(
            parse_hex_gateway("0101A8C0"),
            Some(Ipv4Address::new(192, 168, 1, 1))
        );

        // Test 172.16.0.1: 0x010010AC
        assert_eq!(
            parse_hex_gateway("010010AC"),
            Some(Ipv4Address::new(172, 16, 0, 1))
        );
    }

    #[test]
    fn test_parse_hex_gateway_invalid() {
        assert_eq!(parse_hex_gateway("ZZZZZZZZ"), None);
        assert_eq!(parse_hex_gateway(""), None);
    }
}
