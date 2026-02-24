//! DpdkApp builder and runner.

use crate::context::WorkerContext;

use dpdk_net::api::rte::eth::{EthConf, EthDev, EthDevBuilder, RxQueueConf, TxQueueConf, rss_hf};
use dpdk_net::api::rte::lcore::Lcore;
use dpdk_net::api::rte::pktmbuf::{MemPool, MemPoolConfig};
use dpdk_net::api::rte::queue::{RxQueue, TxQueue};
use dpdk_net::device::{DpdkDevice, SharedArpCache};
use dpdk_net::runtime::Reactor;

use smoltcp::iface::{Config, Interface};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};

use std::cell::Cell;
use std::future::Future;
use std::net::Ipv4Addr;
use std::rc::Rc;
use std::sync::Arc;

use tokio::runtime::Builder;
use tracing::{debug, info, warn};

/// Default headroom reserved at the front of each mbuf
const DEFAULT_MBUF_HEADROOM: usize = 128;

/// Default data room size for mbufs
const DEFAULT_MBUF_DATA_ROOM_SIZE: u16 = 2048 + DEFAULT_MBUF_HEADROOM as u16;

/// Default MTU
const DEFAULT_MTU: usize = 1500;

/// Builder for configuring and running a DPDK application.
///
/// `DpdkApp` uses DPDK's native lcore threading model, where:
/// - EAL creates lcore threads during `rte_eal_init()`
/// - Each lcore gets its own RX/TX queue
/// - Queue count equals lcore count
///
/// # Example
///
/// ```ignore
/// use dpdk_net::api::rte::eal::EalBuilder;
/// use dpdk_net_util::DpdkApp;
/// use dpdk_net::socket::TcpListener;
/// use smoltcp::wire::Ipv4Address;
///
/// fn main() {
///     let _eal = EalBuilder::new()
///         .core_list("0-3")
///         .allow("0000:00:04.0")
///         .init()
///         .expect("EAL init failed");
///     
///     DpdkApp::new()
///         .eth_dev(0)
///         .ip(Ipv4Address::new(10, 0, 0, 10))
///         .gateway(Ipv4Address::new(10, 0, 0, 1))
///         .run(|ctx| async move {
///             let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
///             // ... serve until done, then return
///         });
/// }
/// ```
pub struct DpdkApp {
    port_id: u16,
    ip_addr: Option<Ipv4Address>,
    gateway: Option<Ipv4Address>,
    mbufs_per_queue: u32,
    rx_desc: u16,
    tx_desc: u16,
    subnet_prefix: u8,
}

/// Errors returned by [`DpdkApp::try_run`].
#[derive(Debug)]
pub enum DpdkAppError {
    /// IP address was not configured.
    MissingIpAddress,
    /// Gateway was not configured.
    MissingGateway,
    /// No lcores available.
    NoLcores,
    /// Failed to query or configure the Ethernet device.
    DeviceError(String),
    /// Failed to create mempool.
    MempoolError(String),
}

impl std::fmt::Display for DpdkAppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingIpAddress => write!(f, "IP address not set. Call ip() before run()"),
            Self::MissingGateway => write!(f, "Gateway not set. Call gateway() before run()"),
            Self::NoLcores => write!(f, "No lcores available. Ensure EAL is initialized with -l flag"),
            Self::DeviceError(e) => write!(f, "Device error: {e}"),
            Self::MempoolError(e) => write!(f, "Mempool error: {e}"),
        }
    }
}

impl std::error::Error for DpdkAppError {}

impl Default for DpdkApp {
    fn default() -> Self {
        Self::new()
    }
}

impl DpdkApp {
    /// Create a new DpdkApp builder.
    pub fn new() -> Self {
        Self {
            port_id: 0,
            ip_addr: None,
            gateway: None,
            mbufs_per_queue: 8192,
            rx_desc: 1024,
            tx_desc: 1024,
            subnet_prefix: 24,
        }
    }

    /// Set the DPDK port ID (default: 0).
    pub fn eth_dev(mut self, port_id: u16) -> Self {
        self.port_id = port_id;
        self
    }

    /// Set the IP address.
    pub fn ip(mut self, addr: Ipv4Address) -> Self {
        self.ip_addr = Some(addr);
        self
    }

    /// Set the gateway address.
    pub fn gateway(mut self, addr: Ipv4Address) -> Self {
        self.gateway = Some(addr);
        self
    }

    /// Set mbufs per queue (default: 8192).
    pub fn mbufs_per_queue(mut self, count: u32) -> Self {
        self.mbufs_per_queue = count;
        self
    }

    /// Set RX/TX descriptors (default: 1024).
    pub fn descriptors(mut self, rx: u16, tx: u16) -> Self {
        self.rx_desc = rx;
        self.tx_desc = tx;
        self
    }

    /// Set subnet prefix length (default: 24, i.e., /24).
    pub fn subnet_prefix(mut self, prefix: u8) -> Self {
        self.subnet_prefix = prefix;
        self
    }

    /// Run the application.
    ///
    /// Launches work on all worker lcores and runs queue 0 on the main lcore.
    /// Blocks until all worker closures return.
    ///
    /// # Arguments
    ///
    /// * `server` - Closure that creates the async server/client for each lcore.
    ///   The closure receives a [`WorkerContext`] and should return when done.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - IP address is not set
    /// - Gateway is not set
    /// - No lcores are available
    /// - Ethernet device configuration fails
    pub fn run<F, Fut>(self, server: F)
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        if let Err(e) = self.try_run(server) {
            panic!("{e}");
        }
    }

    /// Run the application, returning errors instead of panicking.
    ///
    /// This is the fallible version of [`run`](Self::run). See its documentation
    /// for details on behavior.
    pub fn try_run<F, Fut>(self, server: F) -> Result<(), DpdkAppError>
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let ip_addr = self.ip_addr.ok_or(DpdkAppError::MissingIpAddress)?;
        let gateway = self.gateway.ok_or(DpdkAppError::MissingGateway)?;
        let subnet_prefix = self.subnet_prefix;

        // Collect lcores
        let lcores: Vec<Lcore> = Lcore::all().collect();
        let num_queues = lcores.len();

        if num_queues == 0 {
            return Err(DpdkAppError::NoLcores);
        }

        info!(
            num_lcores = num_queues,
            port_id = self.port_id,
            ip = %ip_addr,
            %gateway,
            subnet_prefix,
            "DpdkApp starting"
        );

        // Query device info
        let dev_info = EthDev::new(self.port_id)
            .info()
            .map_err(|e| DpdkAppError::DeviceError(format!("{e:?}")))?;
        let reta_size = dev_info.reta_size as usize;

        // Create mempool
        let total_mbufs = self.mbufs_per_queue * num_queues as u32;
        let mempool_config = MemPoolConfig::new()
            .num_mbufs(total_mbufs)
            .data_room_size(DEFAULT_MBUF_DATA_ROOM_SIZE);

        let mempool = Arc::new(
            MemPool::create("dpdk_app_pool", &mempool_config)
                .map_err(|e| DpdkAppError::MempoolError(format!("{e:?}")))?,
        );

        // Configure ethernet device with RSS for both TCP and UDP if supported
        let eth_conf = if reta_size > 0 && num_queues > 1 {
            info!(reta_size, "Enabling RSS for multi-queue (TCP + UDP)");
            EthConf::new().rss_with_hash(rss_hf::TCP | rss_hf::UDP)
        } else {
            if num_queues > 1 {
                warn!(
                    "Device does not support RSS (reta_size=0), multi-queue may not work properly"
                );
            }
            EthConf::new()
        };

        let eth_dev = EthDevBuilder::new(self.port_id)
            .eth_conf(eth_conf)
            .nb_rx_queues(num_queues as u16)
            .nb_tx_queues(num_queues as u16)
            .rx_queue_conf(RxQueueConf::new().nb_desc(self.rx_desc))
            .tx_queue_conf(TxQueueConf::new().nb_desc(self.tx_desc))
            .build(&mempool)
            .map_err(|e| DpdkAppError::DeviceError(format!("{e:?}")))?;

        // Get MAC address
        let mac = eth_dev
            .mac_addr()
            .map_err(|e| DpdkAppError::DeviceError(format!("{e:?}")))?;
        let mac_addr = EthernetAddress(mac.addr_bytes);

        info!(
            mac = ?mac_addr,
            ip = %ip_addr,
            %gateway,
            queues = num_queues,
            "Ethernet device configured"
        );

        // Create shared ARP cache for multi-queue setups
        let shared_arp_cache = if num_queues > 1 {
            info!("Multi-queue mode: using shared ARP cache");
            Some(SharedArpCache::new())
        } else {
            None
        };

        // Wrap server in Arc for sharing
        let server = Arc::new(server);

        // Launch on worker lcores (all except main)
        let _main_lcore = Lcore::main();
        let mut main_queue_id = 0u16;

        for (queue_id, lcore) in lcores.iter().enumerate() {
            if lcore.is_main() {
                main_queue_id = queue_id as u16;
                continue; // Run on main thread after launching workers
            }

            let mempool = mempool.clone();
            let shared_arp_cache = shared_arp_cache.clone();
            let server = server.clone();
            let queue_id = queue_id as u16;
            let port_id = self.port_id;

            lcore
                .launch(move || {
                    Self::run_worker(
                        queue_id,
                        port_id,
                        mempool,
                        mac_addr,
                        ip_addr,
                        gateway,
                        subnet_prefix,
                        shared_arp_cache,
                        server,
                    );
                    0
                })
                .map_err(|e| DpdkAppError::DeviceError(format!("Failed to launch lcore: {e:?}")))?;
        }

        // Run main queue on main lcore
        Self::run_worker(
            main_queue_id,
            self.port_id,
            mempool.clone(),
            mac_addr,
            ip_addr,
            gateway,
            subnet_prefix,
            shared_arp_cache,
            server,
        );

        // Wait for all workers to finish
        Lcore::wait_all_workers();

        info!("All workers finished, cleaning up");

        // Cleanup
        let _ = eth_dev.stop();
        let _ = eth_dev.close();
        drop(mempool);

        info!("DpdkApp shutdown complete");
        Ok(())
    }

    /// Run a single worker on the current lcore.
    #[allow(clippy::too_many_arguments)]
    fn run_worker<F, Fut>(
        queue_id: u16,
        port_id: u16,
        mempool: Arc<MemPool>,
        mac_addr: EthernetAddress,
        ip_addr: Ipv4Address,
        gateway: Ipv4Address,
        subnet_prefix: u8,
        shared_arp_cache: Option<SharedArpCache>,
        server: Arc<F>,
    ) where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let lcore = Lcore::current().expect("Not running on an lcore");
        debug!(
            queue_id,
            lcore_id = lcore.id(),
            socket_id = lcore.socket_id(),
            "Worker starting"
        );

        // Create DPDK device for this queue
        let rxq = RxQueue::new(port_id, queue_id);
        let txq = TxQueue::new(port_id, queue_id);
        let mbuf_capacity = DEFAULT_MBUF_DATA_ROOM_SIZE as usize - DEFAULT_MBUF_HEADROOM;
        let mut device = DpdkDevice::new(rxq, txq, mempool, DEFAULT_MTU, mbuf_capacity);

        // Configure shared ARP cache if multi-queue
        if let Some(cache) = shared_arp_cache {
            let octets = ip_addr.octets();
            device = device.with_shared_arp_cache(
                queue_id,
                cache,
                mac_addr.0,
                Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]),
            );
            if queue_id == 0 {
                debug!("Queue 0: ARP cache producer");
            }
        }

        // Configure smoltcp interface
        let config = Config::new(mac_addr.into());
        let mut iface = Interface::new(config, &mut device, Instant::now());

        iface.update_ip_addrs(|ip_addrs| {
            ip_addrs
                .push(IpCidr::new(IpAddress::Ipv4(ip_addr), subnet_prefix))
                .unwrap();
        });
        iface.routes_mut().add_default_ipv4_route(gateway).unwrap();

        // Create tokio runtime
        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        let local = tokio::task::LocalSet::new();

        local.block_on(&rt, async {
            // Create reactor
            let reactor = Reactor::new(device, iface);
            let handle = reactor.handle();

            // Reactor cancel flag
            let reactor_cancel = Rc::new(Cell::new(false));
            let reactor_cancel_clone = reactor_cancel.clone();

            // Spawn reactor
            let reactor_task = tokio::task::spawn_local(async move {
                reactor.run(reactor_cancel_clone).await;
            });

            // Create worker context
            let ctx = WorkerContext::new(lcore, queue_id, lcore.socket_id(), handle);

            // Run user's server/client
            server(ctx).await;

            // Signal reactor to stop
            reactor_cancel.set(true);
            let _ = reactor_task.await;
        });

        debug!(queue_id, "Worker finished");
    }
}
