use crate::dpdk_device::DpdkDeviceWithPool;
use nix::ifaddrs::getifaddrs;
use rpkt_dpdk::*;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};
use std::fs;
use std::io::{BufRead, BufReader};

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
        .mempool_alloc("tcp_pool", 8192, 256, 2048 + 128, 0)
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
    let mut device = DpdkDeviceWithPool::new(rxq, txq, mempool, 1500);

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

    // Create socket set with two TCP sockets
    let mut sockets = SocketSet::new(vec![]);

    // Server socket - listens on port 8080
    let server_rx_buffer = tcp::SocketBuffer::new(vec![0; 4096]);
    let server_tx_buffer = tcp::SocketBuffer::new(vec![0; 4096]);
    let mut server_socket = tcp::Socket::new(server_rx_buffer, server_tx_buffer);
    server_socket.listen(8080).unwrap();
    let server_handle = sockets.add(server_socket);

    // Client socket - will connect to the server
    let client_rx_buffer = tcp::SocketBuffer::new(vec![0; 4096]);
    let client_tx_buffer = tcp::SocketBuffer::new(vec![0; 4096]);
    let client_socket = tcp::Socket::new(client_rx_buffer, client_tx_buffer);
    let client_handle = sockets.add(client_socket);

    println!("Smoltcp interface initialized on DPDK");
    println!("IP: {}/24", ip_addr);
    println!("MAC: {:?}", mac_addr);
    println!("Server listening on port 8080");

    let mut client_connected = false;
    let mut client_sent = false;
    let mut server_echoed = false;
    let mut client_received = false;

    // Poll loop - run for up to 200 iterations to allow TCP handshake
    for iteration in 0..200 {
        let timestamp = Instant::now();
        let poll_result = iface.poll(timestamp, &mut device, &mut sockets);

        // Debug output every 20 iterations
        if iteration % 20 == 0 && iteration > 0 {
            println!(
                "[Debug] Iteration {}: poll_result={:?}",
                iteration, poll_result
            );
        }

        // Client socket logic
        {
            let client = sockets.get_mut::<tcp::Socket>(client_handle);

            if !client_connected && !client.is_open() {
                // Connect to local server
                println!("[Client] Connecting to {}:8080", ip_addr);
                let remote_endpoint = (ip_addr, 8080);
                client
                    .connect(iface.context(), remote_endpoint, 49152)
                    .unwrap();
                client_connected = true;
            } else if client.is_active() && client.may_send() && !client_sent {
                // Send data once connected and ready
                let data = b"Hello, TCP server!";
                if client.send_slice(data).is_ok() {
                    println!("[Client] Sent: {:?}", std::str::from_utf8(data).unwrap());
                    client_sent = true;
                }
            } else if client.may_recv() {
                // Read response from server
                client
                    .recv(|data| {
                        if !data.is_empty() {
                            println!(
                                "[Client] Received: {:?}",
                                std::str::from_utf8(data).unwrap()
                            );
                            client_received = true;
                        }
                        (data.len(), ())
                    })
                    .ok();
            }

            // Debug output every 20 iterations
            if iteration % 20 == 0 && iteration > 0 {
                let state = client.state();
                println!(
                    "[Debug] Iteration {}: Client state={:?}, may_send={}, may_recv={}",
                    iteration,
                    state,
                    client.may_send(),
                    client.may_recv()
                );
            }
        }

        // Server socket logic
        {
            let server = sockets.get_mut::<tcp::Socket>(server_handle);

            // Debug output every 20 iterations
            if iteration % 20 == 0 && iteration > 0 {
                let state = server.state();
                println!(
                    "[Debug] Iteration {}: Server state={:?}, is_listening={}, is_active={}, may_recv={}",
                    iteration,
                    state,
                    server.is_listening(),
                    server.is_active(),
                    server.may_recv()
                );
            }

            if server.may_recv() {
                // Read data from client and collect it
                let mut echo_data = Vec::new();
                let received = server.recv(|data| {
                    if !data.is_empty() {
                        println!(
                            "[Server] Received: {:?}",
                            std::str::from_utf8(data).unwrap()
                        );
                        echo_data.extend_from_slice(data);
                    }
                    (data.len(), ())
                });

                // Echo back the data if we received any
                if received.is_ok()
                    && !echo_data.is_empty()
                    && server.may_send()
                    && server.send_slice(&echo_data).is_ok()
                {
                    println!("[Server] Echoed back {} bytes", echo_data.len());
                    server_echoed = true;
                }
            }
        }

        // Exit early if we've completed the echo cycle
        if server_echoed && client_received && iteration > 10 {
            println!("Echo test completed successfully!");
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Assert that the full echo cycle completed
    assert!(client_sent, "Client failed to send data");
    assert!(server_echoed, "Server failed to echo data back");
    assert!(client_received, "Client failed to receive echoed data");

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
