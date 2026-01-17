pub mod tcp;
pub mod udp;

pub mod echo_server;
pub mod tcp_echo;

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
    use rpkt_dpdk::*;
    use smoltcp::wire;

    use dpdk_net::tcp::DEFAULT_MBUF_DATA_ROOM_SIZE;

    /// port_id is the device port id to send packets
    /// VM might have only port 0.
    pub fn udp_gen(mem_pool_name: &str, port_id: u16) {
        DpdkOption::new()
            .args(["--no-huge", "--no-pci", "--vdev=net_null0"])
            .init()
            .unwrap();
        let nb_qs = 2;

        // Use small mempool for testing
        service()
            .mempool_alloc(
                mem_pool_name,
                4096,
                256,
                DEFAULT_MBUF_DATA_ROOM_SIZE as u16,
                0,
            )
            .unwrap();

        let eth_conf = EthConf::new();
        let mut rxq_confs = Vec::new();
        let mut txq_confs = Vec::new();
        for _ in 0..nb_qs {
            rxq_confs.push(RxqConf::new(1024, 0, mem_pool_name));
            txq_confs.push(TxqConf::new(1024, 0));
        }

        service()
            .dev_configure_and_start(port_id, &eth_conf, &rxq_confs, &txq_confs)
            .unwrap();

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
                service().thread_bind_to(i).unwrap();
                let mut txq = service().tx_queue(0, i as u16).unwrap();
                let mp = service().mempool(&mem_pool_name).unwrap();
                let mut batch = ArrayVec::<_, 64>::new();

                while run.load(Ordering::Acquire) {
                    mp.fill_up_batch(&mut batch);
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
                        assert!(_sent > 0);
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

        let mut old_stats = service().stats_query(0).unwrap().query();
        while run_curr.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let curr_stats = service().stats_query(0).unwrap().query();
            println!(
                "pkts per sec: {}, bytes per sec: {}, errors per sec: {}",
                curr_stats.opackets() - old_stats.opackets(),
                (curr_stats.obytes() - old_stats.obytes()) as f64 * 8.0 / 1000000000.0,
                curr_stats.oerrors() - old_stats.oerrors(),
            );

            old_stats = curr_stats;
        }

        for jh in jhs {
            jh.join().unwrap();
        }

        service().dev_stop_and_close(port_id).unwrap();
        println!("port {} closed", port_id);

        service().mempool_free(mem_pool_name).unwrap();
        println!("mempool {} freed", mem_pool_name);

        service().graceful_cleanup().unwrap();
        println!("dpdk service shutdown gracefully");
    }
}
