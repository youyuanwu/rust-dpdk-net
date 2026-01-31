pub mod app;
pub mod eth_dev_config;
pub mod manual;
pub mod udp;

pub mod dpdk_test;

pub mod util {

    use std::process::Command;

    pub const TEST_MBUF_COUNT: u32 = 8192;
    pub const TEST_MBUF_CACHE_SIZE: u32 = 256;

    /// Channel information from ethtool
    #[derive(Debug, Clone, Copy, Default)]
    pub struct EthtoolChannels {
        /// Maximum supported RX-only channels
        pub max_rx: u32,
        /// Maximum supported TX-only channels
        pub max_tx: u32,
        /// Maximum supported other channels
        pub max_other: u32,
        /// Maximum supported combined channels (RX+TX on same queue)
        pub max_combined: u32,
        /// Current RX-only channels
        pub rx_count: u32,
        /// Current TX-only channels
        pub tx_count: u32,
        /// Current other channels
        pub other_count: u32,
        /// Current combined channels
        pub combined_count: u32,
    }

    /// Get ethtool channel information for a network interface.
    ///
    /// This uses the SIOCETHTOOL ioctl to query channel counts,
    /// equivalent to running `ethtool -l <interface>`.
    ///
    /// # Example
    /// ```no_run
    /// use dpdk_net_test::util::get_ethtool_channels;
    ///
    /// let channels = get_ethtool_channels("eth1").unwrap();
    /// println!("Max combined queues: {}", channels.max_combined);
    /// println!("Current combined queues: {}", channels.combined_count);
    /// ```
    pub fn get_ethtool_channels(interface: &str) -> Result<EthtoolChannels, String> {
        use nix::libc;
        use std::ffi::CString;

        // ethtool command constants
        const ETHTOOL_GCHANNELS: u32 = 0x0000003c;
        const SIOCETHTOOL: libc::c_ulong = 0x8946;

        // struct ethtool_channels from linux/ethtool.h
        #[repr(C)]
        struct EthtoolChannelsRaw {
            cmd: u32,
            max_rx: u32,
            max_tx: u32,
            max_other: u32,
            max_combined: u32,
            rx_count: u32,
            tx_count: u32,
            other_count: u32,
            combined_count: u32,
        }

        // Create a socket for the ioctl
        let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if sock < 0 {
            return Err("Failed to create socket".to_string());
        }

        // Ensure socket is closed when we're done
        struct SocketGuard(i32);
        impl Drop for SocketGuard {
            fn drop(&mut self) {
                unsafe { libc::close(self.0) };
            }
        }
        let _guard = SocketGuard(sock);

        // Prepare ethtool_channels struct
        let mut channels = EthtoolChannelsRaw {
            cmd: ETHTOOL_GCHANNELS,
            max_rx: 0,
            max_tx: 0,
            max_other: 0,
            max_combined: 0,
            rx_count: 0,
            tx_count: 0,
            other_count: 0,
            combined_count: 0,
        };

        // Prepare ifreq struct
        let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };

        // Copy interface name
        let ifname = CString::new(interface).map_err(|_| "Invalid interface name")?;
        let ifname_bytes = ifname.as_bytes_with_nul();
        if ifname_bytes.len() > libc::IFNAMSIZ {
            return Err("Interface name too long".to_string());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                ifname_bytes.as_ptr(),
                ifr.ifr_name.as_mut_ptr() as *mut u8,
                ifname_bytes.len(),
            );
        }

        // Set ifr_data to point to our ethtool_channels struct
        ifr.ifr_ifru.ifru_data = &mut channels as *mut _ as *mut libc::c_char;

        // Make the ioctl call
        let ret = unsafe { libc::ioctl(sock, SIOCETHTOOL, &mut ifr) };
        if ret < 0 {
            let errno = std::io::Error::last_os_error();
            return Err(format!("ioctl SIOCETHTOOL failed: {}", errno));
        }

        Ok(EthtoolChannels {
            max_rx: channels.max_rx,
            max_tx: channels.max_tx,
            max_other: channels.max_other,
            max_combined: channels.max_combined,
            rx_count: channels.rx_count,
            tx_count: channels.tx_count,
            other_count: channels.other_count,
            combined_count: channels.combined_count,
        })
    }

    /// Ensure that hugepages are set up correctly
    /// nr_hugepages: number of hugepages to allocate
    pub fn ensure_hugepages() -> Result<(), String> {
        use std::path::Path;
        let nr_hugepages = 1024;
        // Check if hugepages directory already exists
        let hugepages_path = Path::new("/dev/hugepages");
        if !hugepages_path.exists() {
            // Create hugepages directory
            let status = Command::new("sudo")
                .args(["mkdir", "-p", "/dev/hugepages"])
                .status()
                .map_err(|e| format!("Failed to create hugepages directory: {}", e))?;

            if !status.success() {
                return Err("Failed to create hugepages directory".to_string());
            }
        }

        // Check if hugetlbfs is already mounted by checking if /proc/mounts contains it
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        let already_mounted = mounts
            .lines()
            .any(|line| line.contains("hugetlbfs") && line.contains("/dev/hugepages"));

        if !already_mounted {
            // Mount hugetlbfs
            let status = Command::new("sudo")
                .args(["mount", "-t", "hugetlbfs", "none", "/dev/hugepages"])
                .status()
                .map_err(|e| format!("Failed to mount hugetlbfs: {}", e))?;

            if !status.success() {
                return Err("Failed to mount hugetlbfs".to_string());
            }
        }

        // Set number of hugepages
        let hugepages_str = nr_hugepages.to_string();
        let mut child = Command::new("sudo")
            .args([
                "tee",
                "/sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages",
            ])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn tee command: {}", e))?;

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin
                .write_all(hugepages_str.as_bytes())
                .map_err(|e| format!("Failed to write to tee: {}", e))?;
        }

        let status = child
            .wait()
            .map_err(|e| format!("Failed to wait for tee command: {}", e))?;

        if !status.success() {
            return Err("Failed to set number of hugepages".to_string());
        }

        Ok(())
    }

    /// Get the MAC address of a neighbor (gateway) from the kernel's neighbor cache.
    ///
    /// This is equivalent to running `ip neigh show <ip_address>`.
    /// Returns the MAC address as a 6-byte array if found.
    ///
    /// # Example
    /// ```no_run
    /// use dpdk_net_test::util::get_neighbor_mac;
    ///
    /// if let Some(mac) = get_neighbor_mac("10.0.0.1") {
    ///     println!("Gateway MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
    ///         mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
    /// }
    /// ```
    pub fn get_neighbor_mac(ip_address: &str) -> Option<[u8; 6]> {
        let output = Command::new("ip")
            .args(["neigh", "show", ip_address])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse output like: "10.0.0.1 dev eth0 lladdr 12:34:56:78:9a:bc REACHABLE"
        for line in stdout.lines() {
            if let Some(lladdr_pos) = line.find("lladdr ") {
                let mac_start = lladdr_pos + 7;
                let mac_str: String = line[mac_start..].chars().take(17).collect();
                return parse_mac_address(&mac_str);
            }
        }
        None
    }

    /// Parse a MAC address string like "12:34:56:78:9a:bc" into bytes
    fn parse_mac_address(mac_str: &str) -> Option<[u8; 6]> {
        let parts: Vec<&str> = mac_str.split(':').collect();
        if parts.len() != 6 {
            return None;
        }

        let mut mac = [0u8; 6];
        for (i, part) in parts.iter().enumerate() {
            mac[i] = u8::from_str_radix(part, 16).ok()?;
        }
        Some(mac)
    }

    /// Build an ARP reply packet that teaches smoltcp the gateway MAC.
    ///
    /// This creates a fake ARP reply from the gateway, which smoltcp will
    /// process and add to its neighbor cache.
    ///
    /// # Arguments
    /// * `our_mac` - Our interface's MAC address
    /// * `our_ip` - Our interface's IP address
    /// * `gateway_mac` - The gateway's MAC address (from kernel neighbor cache)
    /// * `gateway_ip` - The gateway's IP address
    ///
    /// # Returns
    /// A Vec<u8> containing the complete Ethernet frame with ARP reply
    pub fn build_arp_reply(
        our_mac: [u8; 6],
        our_ip: [u8; 4],
        gateway_mac: [u8; 6],
        gateway_ip: [u8; 4],
    ) -> Vec<u8> {
        let mut packet = vec![0u8; 42]; // Ethernet (14) + ARP (28)

        // Ethernet header
        packet[0..6].copy_from_slice(&our_mac); // Destination MAC (us)
        packet[6..12].copy_from_slice(&gateway_mac); // Source MAC (gateway)
        packet[12..14].copy_from_slice(&[0x08, 0x06]); // EtherType: ARP

        // ARP header
        packet[14..16].copy_from_slice(&[0x00, 0x01]); // Hardware type: Ethernet
        packet[16..18].copy_from_slice(&[0x08, 0x00]); // Protocol type: IPv4
        packet[18] = 6; // Hardware address length
        packet[19] = 4; // Protocol address length
        packet[20..22].copy_from_slice(&[0x00, 0x02]); // Operation: ARP Reply

        // Sender hardware address (gateway MAC)
        packet[22..28].copy_from_slice(&gateway_mac);
        // Sender protocol address (gateway IP)
        packet[28..32].copy_from_slice(&gateway_ip);
        // Target hardware address (our MAC)
        packet[32..38].copy_from_slice(&our_mac);
        // Target protocol address (our IP)
        packet[38..42].copy_from_slice(&our_ip);

        packet
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_get_ethtool_channels_eth1() {
            match get_ethtool_channels("eth1") {
                Ok(channels) => {
                    println!("eth1 channel info:");
                    println!("  max_rx: {}", channels.max_rx);
                    println!("  max_tx: {}", channels.max_tx);
                    println!("  max_combined: {}", channels.max_combined);
                    println!("  rx_count: {}", channels.rx_count);
                    println!("  tx_count: {}", channels.tx_count);
                    println!("  combined_count: {}", channels.combined_count);

                    // On Azure VMs with accelerated networking, combined_count should be > 0
                    assert!(
                        channels.combined_count > 0 || channels.rx_count > 0,
                        "Expected at least one queue"
                    );
                }
                Err(e) => {
                    // eth1 might not exist on all systems
                    println!("Skipping test: {}", e);
                }
            }
        }

        #[test]
        fn test_get_ethtool_channels_invalid_interface() {
            let result = get_ethtool_channels("nonexistent_interface_xyz");
            assert!(result.is_err(), "Expected error for non-existent interface");
        }
    }
}

pub mod send {
    use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};

    use arrayvec::ArrayVec;
    use ctrlc;
    use dpdk_net::api::rte::eal::EalBuilder;
    use dpdk_net::api::rte::eth::{EthConf, EthDevBuilder, RxQueueConf, TxQueueConf};
    use dpdk_net::api::rte::pktmbuf::{MemPool, MemPoolConfig};
    use dpdk_net::api::rte::queue::TxQueue;
    use smoltcp::wire;

    use crate::dpdk_test::DEFAULT_MBUF_DATA_ROOM_SIZE;

    /// port_id is the device port id to send packets
    /// VM might have only port 0.
    pub fn udp_gen(mem_pool_name: &str, port_id: u16) {
        let _eal = EalBuilder::new()
            .no_huge()
            .no_pci()
            .vdev("net_null0")
            .file_prefix("udp_gen")
            .init()
            .expect("Failed to initialize EAL");

        let nb_qs = 2;

        // Create mempool
        let mempool_config = MemPoolConfig::new()
            .num_mbufs(4096)
            .data_room_size(DEFAULT_MBUF_DATA_ROOM_SIZE as u16);
        let mempool =
            MemPool::create(mem_pool_name, &mempool_config).expect("Failed to create mempool");

        // Configure and start ethernet device with multiple queues
        let eth_dev = EthDevBuilder::new(port_id)
            .eth_conf(EthConf::new())
            .nb_rx_queues(nb_qs)
            .nb_tx_queues(nb_qs)
            .rx_queue_conf(RxQueueConf::new().nb_desc(1024))
            .tx_queue_conf(TxQueueConf::new().nb_desc(1024))
            .build(&mempool)
            .expect("Failed to configure eth device");

        let run = Arc::new(AtomicBool::new(true));
        let run_curr = run.clone();
        let run_clone = run.clone();
        ctrlc::set_handler(move || {
            run_clone.store(false, Ordering::Release);
        })
        .unwrap();

        let total_header_len = 42;
        let payload_len = 18;

        let mut jhs = Vec::new();
        for i in 0..nb_qs {
            let run = run.clone();
            let mem_pool_name = mem_pool_name.to_string();
            let jh = std::thread::spawn(move || {
                // Note: thread_bind_to is not available in our wrapper yet
                // For now, we proceed without CPU pinning
                let txq = TxQueue::new(port_id, i);
                // Re-get mempool reference in thread via lookup
                let mp = MemPool::lookup(mem_pool_name.clone()).expect("Failed to lookup mempool");
                let mut batch = ArrayVec::<_, 64>::new();

                while run.load(Ordering::Acquire) {
                    mp.fill_batch(&mut batch);
                    for mbuf in batch.iter_mut() {
                        unsafe { mbuf.extend(total_header_len + payload_len) };

                        let mut frame = wire::EthernetFrame::new_unchecked(mbuf.data_mut());
                        frame.set_src_addr(wire::EthernetAddress([
                            0x00, 0x50, 0x56, 0xae, 0x76, 0xf5,
                        ]));
                        frame.set_dst_addr(wire::EthernetAddress([
                            0x00, 0x0b, 0x86, 0x64, 0x8b, 0xa0,
                        ]));
                        frame.set_ethertype(wire::EthernetProtocol::Ipv4);

                        let mut ipv4_pkt = wire::Ipv4Packet::new_unchecked(frame.payload_mut());
                        ipv4_pkt.set_version(4);
                        ipv4_pkt.set_header_len(20);
                        ipv4_pkt.set_dscp(0);
                        ipv4_pkt.set_ecn(0);
                        ipv4_pkt.set_total_len((28 + payload_len) as u16);
                        ipv4_pkt.set_ident(0x5c65);
                        ipv4_pkt.clear_flags();
                        ipv4_pkt.set_frag_offset(0);
                        ipv4_pkt.set_hop_limit(128);
                        ipv4_pkt.set_next_header(wire::IpProtocol::Udp);
                        ipv4_pkt.set_src_addr(wire::Ipv4Address::new(192, 168, 29, 58));
                        ipv4_pkt.set_dst_addr(wire::Ipv4Address::new(192, 168, 29, 160));
                        ipv4_pkt.set_checksum(0);

                        let mut udp_pkt = wire::UdpPacket::new_unchecked(ipv4_pkt.payload_mut());
                        udp_pkt.set_src_port(60376);
                        udp_pkt.set_dst_port(161);
                        udp_pkt.set_len((8 + payload_len) as u16);
                        udp_pkt.set_checksum(0xbc86);
                    }

                    while !batch.is_empty() {
                        let _sent = txq.tx(&mut batch);
                        // net_null0 might not accept all packets, just continue
                        if _sent == 0 {
                            // Drain remaining to avoid infinite loop
                            batch.clear();
                            break;
                        }
                    }
                }
            });
            jhs.push(jh);
        }

        // stop the loop after 2 seconds
        let run_stop = run_curr.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            run_stop.store(false, Ordering::Release);
        });

        // Wait for stop signal
        while run_curr.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        for jh in jhs {
            jh.join().unwrap();
        }

        let _ = eth_dev.stop();
        let _ = eth_dev.close();
        println!("port {} closed", port_id);

        println!("dpdk service shutdown gracefully");
    }
}
