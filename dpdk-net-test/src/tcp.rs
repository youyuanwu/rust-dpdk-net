use dpdk_net::tcp::{DEFAULT_MBUF_DATA_ROOM_SIZE, DEFAULT_MBUF_HEADROOM, DpdkDeviceWithPool};
use nix::ifaddrs::getifaddrs;
use rpkt_dpdk::*;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};
use std::fs;
use std::io::{BufRead, BufReader};
use std::time::Duration;

use crate::tcp_echo::{EchoClient, EchoServer, SocketConfig, run_echo_test};
use crate::util::{TEST_MBUF_CACHE_SIZE, TEST_MBUF_COUNT};

/// Get PCI address for a network interface
pub fn get_pci_addr(interface: &str) -> Option<String> {
    let path = format!("/sys/class/net/{}/device", interface);
    let link = fs::read_link(&path).ok()?;
    let filename = link.file_name()?.to_str()?;

    // Check if this is a PCI device directly
    if filename.contains(':') && filename.contains('.') {
        return Some(filename.to_string());
    }

    // If not a PCI device, look for lower_ links (bonded/virtual interfaces)
    let net_dir = format!("/sys/class/net/{}", interface);
    if let Ok(entries) = fs::read_dir(&net_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(name_str) = name.to_str()
                && name_str.starts_with("lower_")
            {
                // Follow the lower link to the actual device
                if let Ok(lower_link) = fs::read_link(entry.path()) {
                    // Extract PCI address from path like: ../../../pci79ba:00/79ba:00:02.0/net/enP31162s2
                    let path_str = lower_link.to_str()?;
                    for component in path_str.split('/') {
                        // 79ba:00:02.0
                        if component.contains(':') && component.contains('.') {
                            return Some(component.to_string());
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

    let args = if use_hardware {
        // Dynamically get PCI address for eth1
        let pci_addr = get_pci_addr(interface).expect("Failed to get PCI address for eth1");
        format!("-a {}", pci_addr)
    } else {
        "--no-huge --no-pci --vdev=net_ring0".to_string() // virtual ring device only
    };
    // Initialize DPDK with specified device
    DpdkOption::new()
        .args(args.split(" ").collect::<Vec<_>>())
        .init()
        .unwrap();
    // Create mempool
    service()
        .mempool_alloc(
            "tcp_pool",
            TEST_MBUF_COUNT,
            TEST_MBUF_CACHE_SIZE,
            DEFAULT_MBUF_DATA_ROOM_SIZE as u16,
            0,
        )
        .unwrap();

    // Configure port
    let eth_conf = EthConf::new();
    let rxq_confs = vec![RxqConf::new(1024, 0, "tcp_pool")];
    let txq_confs = vec![TxqConf::new(1024, 0)];

    service()
        .dev_configure_and_start(0, &eth_conf, &rxq_confs, &txq_confs)
        .unwrap();

    // Get queues and mempool
    let rxq = service().rx_queue(0, 0).unwrap();
    let txq = service().tx_queue(0, 0).unwrap();
    let mempool = service().mempool("tcp_pool").unwrap();

    // Create DPDK device for smoltcp
    let mbuf_capacity = DEFAULT_MBUF_DATA_ROOM_SIZE - DEFAULT_MBUF_HEADROOM;
    let mut device = DpdkDeviceWithPool::new(rxq, txq, mempool, 1500, mbuf_capacity);

    // Enable software loopback for self-addressed packets
    // Gateway won't help because it only routes to different networks
    // Physical NICs don't loopback packets to themselves at hardware level
    if use_hardware {
        // software loopback removed
        // device.enable_loopback();
    }

    // Get the actual MAC address from DPDK device
    let dev_info = service().dev_info(0).unwrap();
    let mac_addr = EthernetAddress(dev_info.mac_addr);

    // Configure smoltcp interface with the device's MAC address
    let config = Config::new(mac_addr.into());
    let mut iface = Interface::new(config, &mut device, Instant::now());

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
        &mut device,
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

    // Cleanup
    drop(device);
    drop(sockets);
    drop(iface);

    service().dev_stop_and_close(0).unwrap();
    service().mempool_free("tcp_pool").unwrap();
    service().graceful_cleanup().unwrap();
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
