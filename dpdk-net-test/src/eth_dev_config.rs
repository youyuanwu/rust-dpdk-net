//! Shared Ethernet device configuration for DPDK.
//!
//! This module provides `EthDevConfig`, a reusable configuration struct
//! for creating DPDK memory pools and ethernet devices. It's used by both
//! `create_test_context()` (for testing) and `DpdkApp` (for production).

use std::sync::Arc;

use dpdk_net::api::Errno;
use dpdk_net::api::rte::eth::{EthConf, EthDev, EthDevBuilder, RxQueueConf, TxQueueConf};
use dpdk_net::api::rte::pktmbuf::{MemPool, MemPoolConfig};
use dpdk_net::api::rte::queue::{RxQueue, TxQueue};
use dpdk_net::device::DpdkDevice;

/// Default headroom reserved at the front of each mbuf (matches RTE_PKTMBUF_HEADROOM)
pub const DEFAULT_MBUF_HEADROOM: usize = 128;

/// Default data room size for mbufs (2048 bytes of usable space + headroom)
pub const DEFAULT_MBUF_DATA_ROOM_SIZE: usize = 2048 + DEFAULT_MBUF_HEADROOM;

/// Default MTU for devices
pub const DEFAULT_MTU: usize = 1500;

/// Default number of mbufs in the pool
pub const DEFAULT_NUM_MBUFS: u32 = 8191;

/// Default number of descriptors per queue
pub const DEFAULT_NB_DESC: u16 = 1024;

/// Configuration for creating DPDK ethernet devices.
///
/// This struct holds all the common configuration needed to create
/// a memory pool and ethernet device. Use the builder methods to
/// customize, then call `build()` to create the resources.
///
/// # Example
/// ```no_run
/// use dpdk_net_test::eth_dev_config::EthDevConfig;
///
/// let config = EthDevConfig::new()
///     .mempool_name("my_pool")
///     .num_mbufs(4096)
///     .nb_queues(2);
///
/// let (mempool, eth_dev) = config.build().expect("Failed to build");
/// ```
#[derive(Clone)]
pub struct EthDevConfig {
    pub(crate) mempool_name: String,
    pub(crate) num_mbufs: u32,
    pub(crate) data_room_size: u16,
    pub(crate) nb_rx_queues: u16,
    pub(crate) nb_tx_queues: u16,
    pub(crate) rx_desc: u16,
    pub(crate) tx_desc: u16,
    pub(crate) mtu: usize,
    pub(crate) port_id: u16,
    pub(crate) eth_conf: Option<EthConf>,
}

impl Default for EthDevConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl EthDevConfig {
    /// Create a new configuration with default settings.
    pub fn new() -> Self {
        Self {
            mempool_name: "dpdk_mempool".to_string(),
            num_mbufs: DEFAULT_NUM_MBUFS,
            data_room_size: DEFAULT_MBUF_DATA_ROOM_SIZE as u16,
            nb_rx_queues: 1,
            nb_tx_queues: 1,
            rx_desc: DEFAULT_NB_DESC,
            tx_desc: DEFAULT_NB_DESC,
            mtu: DEFAULT_MTU,
            port_id: 0,
            eth_conf: None,
        }
    }

    /// Set the mempool name.
    pub fn mempool_name(mut self, name: impl Into<String>) -> Self {
        self.mempool_name = name.into();
        self
    }

    /// Set the number of mbufs in the pool.
    pub fn num_mbufs(mut self, n: u32) -> Self {
        self.num_mbufs = n;
        self
    }

    /// Set the data room size for mbufs.
    pub fn data_room_size(mut self, size: u16) -> Self {
        self.data_room_size = size;
        self
    }

    /// Set the number of RX and TX queues (symmetric).
    pub fn nb_queues(mut self, n: u16) -> Self {
        self.nb_rx_queues = n;
        self.nb_tx_queues = n;
        self
    }

    /// Set the number of RX queues.
    pub fn nb_rx_queues(mut self, n: u16) -> Self {
        self.nb_rx_queues = n;
        self
    }

    /// Set the number of TX queues.
    pub fn nb_tx_queues(mut self, n: u16) -> Self {
        self.nb_tx_queues = n;
        self
    }

    /// Set the number of RX and TX descriptors per queue (symmetric).
    pub fn nb_desc(mut self, n: u16) -> Self {
        self.rx_desc = n;
        self.tx_desc = n;
        self
    }

    /// Set the number of RX descriptors per queue.
    pub fn rx_desc(mut self, n: u16) -> Self {
        self.rx_desc = n;
        self
    }

    /// Set the number of TX descriptors per queue.
    pub fn tx_desc(mut self, n: u16) -> Self {
        self.tx_desc = n;
        self
    }

    /// Set the MTU.
    pub fn mtu(mut self, mtu: usize) -> Self {
        self.mtu = mtu;
        self
    }

    /// Set the port ID.
    pub fn port_id(mut self, id: u16) -> Self {
        self.port_id = id;
        self
    }

    /// Set custom ethernet configuration.
    ///
    /// If not set, uses `EthConf::new()` (default configuration).
    pub fn eth_conf(mut self, conf: EthConf) -> Self {
        self.eth_conf = Some(conf);
        self
    }

    /// Get the mbuf capacity (data room size minus headroom).
    pub fn mbuf_capacity(&self) -> usize {
        self.data_room_size as usize - DEFAULT_MBUF_HEADROOM
    }

    /// Build the memory pool and ethernet device.
    ///
    /// Returns an Arc-wrapped mempool and the configured EthDev.
    pub fn build(self) -> Result<(Arc<MemPool>, EthDev), Errno> {
        // Create mempool
        let mempool_config = MemPoolConfig::new()
            .num_mbufs(self.num_mbufs)
            .data_room_size(self.data_room_size);

        let mempool = Arc::new(MemPool::create(self.mempool_name.clone(), &mempool_config)?);

        // Configure and start ethernet device
        let eth_conf = self.eth_conf.unwrap_or_default();

        let eth_dev = EthDevBuilder::new(self.port_id)
            .eth_conf(eth_conf)
            .nb_rx_queues(self.nb_rx_queues)
            .nb_tx_queues(self.nb_tx_queues)
            .rx_queue_conf(RxQueueConf::new().nb_desc(self.rx_desc))
            .tx_queue_conf(TxQueueConf::new().nb_desc(self.tx_desc))
            .build(&mempool)?;

        Ok((mempool, eth_dev))
    }

    /// Create a DpdkDevice for the specified queue.
    ///
    /// The mempool should be the one returned from `build()`.
    pub fn create_device(&self, mempool: Arc<MemPool>, queue_id: u16) -> DpdkDevice {
        let rxq = RxQueue::new(self.port_id, queue_id);
        let txq = TxQueue::new(self.port_id, queue_id);
        DpdkDevice::new(rxq, txq, mempool, self.mtu, self.mbuf_capacity())
    }
}
