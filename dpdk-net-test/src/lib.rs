pub mod tcp;
pub mod udp;

pub mod echo_server;
pub mod tcp_echo;

pub mod dpdk_test;

pub mod util {

    use std::process::Command;

    pub const TEST_MBUF_COUNT: u32 = 8192;
    pub const TEST_MBUF_CACHE_SIZE: u32 = 256;

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
