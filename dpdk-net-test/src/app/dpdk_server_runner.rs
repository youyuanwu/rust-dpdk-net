//! Reusable DPDK multi-queue server runner.
//!
//! This module provides `DpdkServerRunner` which handles all the boilerplate
//! for setting up a multi-queue DPDK server:
//! - Ethernet device configuration
//! - Per-queue worker threads with tokio runtimes
//! - Graceful shutdown with CancellationToken
//!
//! You provide a factory function that creates your server given a `TcpListener`.
//!
//! # Prerequisites
//!
//! Before using this runner, you must:
//! 1. Setup hugepages (e.g., via `crate::util::ensure_hugepages()`)
//! 2. Initialize DPDK EAL (e.g., via `EalBuilder::new().allow(&pci_addr).init()`)
//!
//! # Example
//!
//! ```no_run
//! use dpdk_net::api::rte::eal::EalBuilder;
//! use dpdk_net_test::app::dpdk_server_runner::DpdkServerRunner;
//! use dpdk_net_test::app::echo_server::{EchoServer, ServerStats};
//! use dpdk_net_test::manual::tcp::get_pci_addr;
//! use dpdk_net_test::util::ensure_hugepages;
//! use std::sync::Arc;
//!
//! // Setup hugepages
//! ensure_hugepages().expect("Failed to ensure hugepages");
//!
//! // Initialize DPDK EAL
//! let pci_addr = get_pci_addr("eth1").expect("Failed to get PCI address");
//! let _eal = EalBuilder::new()
//!     .allow(&pci_addr)
//!     .init()
//!     .expect("Failed to initialize EAL");
//!
//! let stats = Arc::new(ServerStats::default());
//! DpdkServerRunner::new("eth1")
//!     .with_default_network_config()
//!     .with_default_hw_queues()
//!     .port(8080)
//!     .run(move |ctx| {
//!         let stats = stats.clone();
//!         async move {
//!             EchoServer::new(ctx.listener, ctx.cancel, stats, ctx.queue_id, ctx.port)
//!                 .run()
//!                 .await
//!         }
//!     });
//! ```

use std::sync::Arc;
use std::thread;

use dpdk_net::api::rte::eth::{EthConf, EthDev, rss_hf};
use dpdk_net::api::rte::thread::{ThreadRegistration, set_cpu_affinity};
use dpdk_net::device::SharedArpCache;
use dpdk_net::runtime::Reactor;
use dpdk_net::socket::TcpListener;

use smoltcp::iface::{Config, Interface};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};

use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::eth_dev_config::EthDevConfig;

/// Context passed to the server factory function.
///
/// Contains everything needed to create a server for a specific queue.
pub struct ServerContext {
    /// The TCP listener bound to the server port
    pub listener: TcpListener,
    /// Cancellation token for graceful shutdown
    pub cancel: CancellationToken,
    /// Queue ID (0-based)
    pub queue_id: usize,
    /// Server port number
    pub port: u16,
}

/// Builder for configuring and running a multi-queue DPDK server.
pub struct DpdkServerRunner {
    interface: String,
    port: u16,
    ip_addr: Option<Ipv4Address>,
    gateway: Option<Ipv4Address>,
    hw_queues: Option<usize>,
    max_queues: Option<usize>,
    mbufs_per_queue: u32,
    rx_desc: u16,
    tx_desc: u16,
    tcp_rx_buffer: usize,
    tcp_tx_buffer: usize,
    backlog: usize,
}

impl DpdkServerRunner {
    /// Create a new server runner for the specified network interface.
    ///
    /// **Note on multi-queue TCP**: Each queue has an independent TCP stack.
    /// RSS may distribute packets from the same connection to different queues,
    /// causing connection failures. Use `max_queues(1)` for reliable single-client
    /// TCP, or ensure clients come from different IPs for multi-queue scaling.
    pub fn new(interface: &str) -> Self {
        Self {
            interface: interface.to_string(),
            port: 8080,
            ip_addr: None,
            gateway: None,
            hw_queues: None,
            max_queues: None,
            mbufs_per_queue: 8192,
            rx_desc: 1024,
            tx_desc: 1024,
            tcp_rx_buffer: 4096,
            tcp_tx_buffer: 4096,
            backlog: 16,
        }
    }

    /// Set the server port (default: 8080).
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set the IP address explicitly.
    pub fn ip_addr(mut self, ip: Ipv4Address) -> Self {
        self.ip_addr = Some(ip);
        self
    }

    /// Set the gateway address explicitly.
    pub fn gateway(mut self, gateway: Ipv4Address) -> Self {
        self.gateway = Some(gateway);
        self
    }

    /// Auto-detect IP address and gateway from the configured interface.
    ///
    /// This queries the interface's IPv4 address and the system's default gateway.
    /// If no gateway is found, defaults to 10.0.0.1.
    ///
    /// # Panics
    /// Panics if the interface IP address cannot be determined.
    pub fn with_default_network_config(mut self) -> Self {
        let ip_addr = crate::manual::tcp::get_interface_ipv4(&self.interface)
            .expect("Failed to get IP address for interface");
        let gateway =
            crate::manual::tcp::get_default_gateway().unwrap_or(Ipv4Address::new(10, 0, 0, 1));
        self.ip_addr = Some(ip_addr);
        self.gateway = Some(gateway);
        self
    }

    /// Set the number of hardware queues explicitly.
    ///
    /// This overrides auto-detection. Use this when you know the exact
    /// number of queues to use.
    pub fn hw_queues(mut self, queues: usize) -> Self {
        self.hw_queues = Some(queues);
        self
    }

    /// Auto-detect hardware queues via ethtool for the configured interface.
    ///
    /// This queries the NIC's combined channel count using ethtool.
    /// Call this after `new()` to enable auto-detection, or use `hw_queues()`
    /// to set an explicit value.
    ///
    /// # Panics
    /// Panics if ethtool query fails.
    pub fn with_default_hw_queues(mut self) -> Self {
        let hw_queues = crate::util::get_ethtool_channels(&self.interface)
            .map(|ch| ch.combined_count as usize)
            .expect("Failed to get hardware queues via ethtool");
        self.hw_queues = Some(hw_queues);
        self
    }

    /// Set the maximum number of queues to use (default: 1).
    ///
    /// **Warning**: With multiple queues, each has an independent TCP stack.
    /// This only works reliably when:
    /// - Traffic comes from multiple client IPs (each IP hashes to one queue)
    /// - Using UDP (stateless)
    ///
    /// For single-client TCP benchmarks, keep this at 1.
    pub fn max_queues(mut self, max: usize) -> Self {
        self.max_queues = Some(max);
        self
    }

    /// Set the number of mbufs per queue (default: 8192).
    pub fn mbufs_per_queue(mut self, count: u32) -> Self {
        self.mbufs_per_queue = count;
        self
    }

    /// Set the TCP buffer sizes (default: 4096).
    pub fn tcp_buffers(mut self, rx: usize, tx: usize) -> Self {
        self.tcp_rx_buffer = rx;
        self.tcp_tx_buffer = tx;
        self
    }

    /// Set the listen backlog (default: 16).
    pub fn backlog(mut self, backlog: usize) -> Self {
        self.backlog = backlog;
        self
    }

    /// Run the server with a factory function that creates servers for each queue.
    ///
    /// The factory receives a `ServerContext` and should return a future that
    /// runs until shutdown.
    ///
    /// # Prerequisites
    /// - Hugepages must be configured (e.g., via `crate::util::ensure_hugepages()`)
    /// - DPDK EAL must be initialized (e.g., via `EalBuilder::new().allow(&pci_addr).init()`)
    ///
    /// # Type Parameters
    /// * `F` - Factory function type
    /// * `Fut` - Future type returned by the factory
    pub fn run<F, Fut>(self, server_factory: F)
    where
        F: Fn(ServerContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + 'static,
    {
        // Get network configuration (must be set via ip_addr()/gateway() or with_default_network_config())
        let ip_addr = self.ip_addr.expect(
            "ip_addr not set. Call ip_addr() or with_default_network_config() before run()",
        );
        let gateway = self.gateway.expect(
            "gateway not set. Call gateway() or with_default_network_config() before run()",
        );

        // Get hardware queue count (must be set via hw_queues() or with_default_hw_queues())
        let hw_queues = self
            .hw_queues
            .expect("hw_queues not set. Call hw_queues() or with_default_hw_queues() before run()");
        info!("Hardware queue count: {}", hw_queues);

        // Query device info to get RETA size before configuring
        let dev_info = EthDev::new(0).info().expect("Failed to get device info");
        let reta_size = dev_info.reta_size as usize;
        let max_rx_queues = dev_info.max_rx_queues as usize;
        let max_tx_queues = dev_info.max_tx_queues as usize;

        // Get number of cpus
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        info!(
            hw_queues,
            num_cpus, reta_size, max_rx_queues, max_tx_queues, "Device info"
        );

        // Calculate number of queues - limited by RETA size for proper RSS distribution
        let mut num_queues = hw_queues;

        if num_cpus > num_queues {
            warn!(
                num_cpus,
                num_queues,
                "Queue count is less than CPU count. Scale up to cpu count for best performance."
            );
            num_queues = num_cpus;
        }

        if let Some(max_queue) = self.max_queues
            && max_queue < num_queues
        {
            warn!(
                max_queue,
                num_queues, "Limiting number of queues to user-specified maximum"
            );
            num_queues = max_queue;
        }

        self.print_banner(ip_addr, gateway, hw_queues, num_queues);

        // Build mempool and ethernet device using shared config
        let total_mbufs = self.mbufs_per_queue * num_queues as u32;

        // Only enable RSS if the device supports it (reta_size > 0)
        let eth_conf = if reta_size > 0 {
            EthConf::new().rss_with_hash(rss_hf::NONFRAG_IPV4_TCP | rss_hf::NONFRAG_IPV6_TCP)
        } else {
            info!("Device does not support RSS (reta_size=0), using simple queue mode");
            EthConf::new()
        };

        let eth_dev_config = EthDevConfig::new()
            .mempool_name("server_pool")
            .num_mbufs(total_mbufs)
            .nb_queues(num_queues as u16)
            .rx_desc(self.rx_desc)
            .tx_desc(self.tx_desc)
            .eth_conf(eth_conf);

        let (mempool, eth_dev) = eth_dev_config
            .clone()
            .build()
            .expect("Failed to configure eth device");

        // Log RSS configuration after device is started
        if num_queues > 1 {
            // Query and log RETA distribution
            if let Ok(reta) = eth_dev.query_rss_reta() {
                let mut queue_counts = std::collections::HashMap::new();
                for &q in &reta {
                    *queue_counts.entry(q).or_insert(0) += 1;
                }
                info!(
                    "RSS RETA: {} entries, distribution: {:?}",
                    reta.len(),
                    queue_counts
                );
            }

            // Log actual RSS hash configuration
            if let Ok((rss_hf, _)) = eth_dev.rss_hash_conf() {
                let has_tcp = eth_dev.has_tcp_rss().unwrap_or(false);
                info!(
                    rss_hf = format!("{:#x}", rss_hf),
                    tcp_rss = has_tcp,
                    "RSS hash config"
                );
                if !has_tcp {
                    warn!(
                        "TCP RSS hashing not enabled! All packets from same client will go to one queue."
                    );
                }
            }
        }

        // Get MAC address
        let mac = eth_dev.mac_addr().expect("Failed to get MAC address");
        let mac_addr = EthernetAddress(mac.addr_bytes);

        self.print_interface_info(ip_addr, mac_addr, gateway);

        // Setup Ctrl+C handler
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        ctrlc::set_handler(move || {
            warn!("Received Ctrl+C, shutting down");
            cancel_clone.cancel();
        })
        .expect("Failed to set Ctrl+C handler");

        let start_time = std::time::Instant::now();

        // Wrap factory in Arc for sharing across threads
        let factory = Arc::new(server_factory);

        // Create shared ARP cache for multi-queue setups
        // Queue 0 receives all ARP replies (not matched by TCP RSS) and updates the cache
        // Other queues read from the cache and inject ARP packets into their smoltcp instance
        let shared_arp_cache = if num_queues > 1 {
            info!("Multi-queue mode: using shared ARP cache (SPMC pattern)");
            Some(SharedArpCache::new())
        } else {
            None
        };

        // Keep a reference for logging after shutdown
        let arp_cache_for_stats = shared_arp_cache.clone();

        // Spawn worker threads for queues 1..num_queues
        // Queue 0 will run on the current thread to save one thread
        let handles = self.spawn_workers(
            num_queues,
            cancel.clone(),
            mempool.clone(),
            eth_dev_config.clone(),
            mac_addr,
            ip_addr,
            gateway,
            shared_arp_cache.clone(),
            factory.clone(),
        );

        // Run queue 0 on the current thread
        Self::run_worker(
            0,
            cancel,
            mempool.clone(),
            eth_dev_config,
            mac_addr,
            ip_addr,
            gateway,
            shared_arp_cache,
            factory,
            self.port,
            self.tcp_rx_buffer,
            self.tcp_tx_buffer,
            self.backlog,
        );

        // Wait for all other threads
        for handle in handles {
            let _ = handle.join();
        }

        let runtime_secs = start_time.elapsed().as_secs();

        // Log shared ARP cache stats
        if let Some(cache) = arp_cache_for_stats {
            info!(
                runtime_secs,
                arp_cache_version = cache.version(),
                "Server stopped"
            );
        } else {
            info!(runtime_secs, "Server stopped");
        }

        self.cleanup(eth_dev, num_queues);
        drop(mempool);
    }

    /// Run a single worker (queue). Can be called from any thread.
    #[allow(clippy::too_many_arguments)]
    fn run_worker<F, Fut>(
        queue_id: usize,
        cancel: CancellationToken,
        mempool: Arc<dpdk_net::api::rte::pktmbuf::MemPool>,
        eth_dev_config: EthDevConfig,
        mac_addr: EthernetAddress,
        ip_addr: Ipv4Address,
        gateway: Ipv4Address,
        shared_arp_cache: Option<SharedArpCache>,
        factory: Arc<F>,
        port: u16,
        tcp_rx: usize,
        tcp_tx: usize,
        backlog: usize,
    ) where
        F: Fn(ServerContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + 'static,
    {
        // Register thread with DPDK
        let _dpdk_registration =
            ThreadRegistration::new().expect("Failed to register thread with DPDK");

        // Pin this thread to CPU `queue_id` for optimal cache locality
        // This mimics what DPDK EAL lcores do with pthread_setaffinity_np
        if let Err(e) = set_cpu_affinity(queue_id) {
            warn!(queue_id, error = %e, "Failed to set CPU affinity, performance may be degraded");
        } else {
            debug!(queue_id, cpu = queue_id, "Thread pinned to CPU");
        }

        debug!(queue_id, "Starting worker");

        // Create DPDK device using shared config
        let mut device = eth_dev_config.create_device(mempool, queue_id as u16);

        // Enable shared ARP cache for multi-queue setups
        if let Some(cache) = shared_arp_cache {
            let octets = ip_addr.octets();
            device = device.with_shared_arp_cache(
                queue_id as u16,
                cache,
                mac_addr.0,
                std::net::Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]),
            );
            if queue_id == 0 {
                info!("Queue 0 will update shared ARP cache (SPMC producer)");
            } else {
                debug!(queue_id, "Using shared ARP cache (SPMC consumer)");
            }
        }

        // Configure smoltcp interface
        let config = Config::new(mac_addr.into());
        let mut iface = Interface::new(config, &mut device, Instant::now());

        // IMPORTANT: Set up IP address BEFORE processing ARP packets
        // smoltcp's process_arp() checks if target_protocol_addr matches our IP
        iface.update_ip_addrs(|ip_addrs| {
            ip_addrs
                .push(IpCidr::new(IpAddress::Ipv4(ip_addr), 24))
                .unwrap();
        });
        iface.routes_mut().add_default_ipv4_route(gateway).unwrap();

        // Create tokio runtime
        let rt = Builder::new_current_thread().build().unwrap();
        let local = tokio::task::LocalSet::new();

        local.block_on(&rt, async {
            // Create reactor
            let reactor = Reactor::new(device, iface);
            let handle = reactor.handle();

            // Create cancel flag for reactor - set when factory finishes
            let reactor_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let reactor_cancel_clone = reactor_cancel.clone();

            // Spawn reactor
            let reactor_task = tokio::task::spawn_local(async move {
                reactor.run(reactor_cancel_clone).await;
            });

            // Create listener
            let listener = TcpListener::bind_with_backlog(&handle, port, tcp_rx, tcp_tx, backlog)
                .expect("Failed to bind listener");

            // Create and run server
            let ctx = ServerContext {
                listener,
                cancel,
                queue_id,
                port,
            };
            factory(ctx).await;

            // Signal reactor to stop and wait for it to finish
            reactor_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = reactor_task.await;
        });

        debug!(queue_id, "Worker stopped");
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_workers<F, Fut>(
        &self,
        num_queues: usize,
        cancel: CancellationToken,
        mempool: Arc<dpdk_net::api::rte::pktmbuf::MemPool>,
        eth_dev_config: EthDevConfig,
        mac_addr: EthernetAddress,
        ip_addr: Ipv4Address,
        gateway: Ipv4Address,
        shared_arp_cache: Option<SharedArpCache>,
        factory: Arc<F>,
    ) -> Vec<thread::JoinHandle<()>>
    where
        F: Fn(ServerContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + 'static,
    {
        // Skip queue 0 - it runs on the main thread
        let spawned_queues = num_queues.saturating_sub(1);
        let mut handles = Vec::with_capacity(spawned_queues);

        // Start from queue 1 (queue 0 runs on main thread)
        for queue_id in 1..num_queues {
            let cancel = cancel.clone();
            let mempool = mempool.clone();
            let eth_dev_config = eth_dev_config.clone();
            let factory = factory.clone();
            let shared_arp_cache = shared_arp_cache.clone();
            let port = self.port;
            let tcp_rx = self.tcp_rx_buffer;
            let tcp_tx = self.tcp_tx_buffer;
            let backlog = self.backlog;

            let handle = thread::Builder::new()
                .name(format!("queue-{}", queue_id))
                .spawn(move || {
                    Self::run_worker(
                        queue_id,
                        cancel,
                        mempool,
                        eth_dev_config,
                        mac_addr,
                        ip_addr,
                        gateway,
                        shared_arp_cache,
                        factory,
                        port,
                        tcp_rx,
                        tcp_tx,
                        backlog,
                    );
                })
                .expect("Failed to spawn worker thread");

            handles.push(handle);
        }

        handles
    }

    fn print_banner(
        &self,
        ip_addr: Ipv4Address,
        gateway: Ipv4Address,
        hw_queues: usize,
        num_queues: usize,
    ) {
        info!(
            ip = %ip_addr,
            port = self.port,
            %gateway,
            hw_queues,
            max_queues = self.max_queues,
            using_queues = num_queues,
            "DPDK Server Runner starting"
        );
    }

    fn print_interface_info(
        &self,
        ip_addr: Ipv4Address,
        mac_addr: EthernetAddress,
        gateway: Ipv4Address,
    ) {
        info!(
            ip = %ip_addr,
            mac = ?mac_addr,
            %gateway,
            port = self.port,
            "Interface configured"
        );
    }

    fn cleanup(&self, eth_dev: EthDev, num_queues: usize) {
        // Print per-queue stats before cleanup
        if let Ok(stats) = eth_dev.stats() {
            info!(
                "Device stats: ipackets={}, opackets={}, ibytes={}, obytes={}",
                stats.ipackets, stats.opackets, stats.ibytes, stats.obytes
            );
            let max_queue_stats = stats.q_ipackets.len();
            for q in 0..std::cmp::min(num_queues, max_queue_stats) {
                info!(
                    "Queue {} - RX: {} packets, TX: {} packets",
                    q, stats.q_ipackets[q], stats.q_opackets[q]
                );
            }
        }

        debug!("Cleaning up");
        let _ = eth_dev.stop();
        let _ = eth_dev.close();
        info!("Server cleanup complete");
    }
}
