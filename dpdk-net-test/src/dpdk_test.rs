//! DPDK Test Harness
//!
//! Provides reusable components for DPDK-based tests including:
//! - `DpdkTestContext` - RAII struct holding EAL, EthDev
//! - Builder for configuring test scenarios
//!
//! Uses `DpdkDeviceWithPool` from `dpdk_net::tcp` for the smoltcp Device implementation.

use dpdk_net::api::rte::eal::{Eal, EalBuilder};
use dpdk_net::api::rte::eth::{EthConf, EthDev, EthDevBuilder, RxQueueConf, TxQueueConf};
use dpdk_net::api::rte::pktmbuf::{MemPool, MemPoolConfig};
use dpdk_net::api::rte::queue::{RxQueue, TxQueue};

// Re-export DpdkDeviceWithPool for convenience
pub use dpdk_net::tcp::DpdkDeviceWithPool;

/// Default headroom reserved at the front of each mbuf (matches RTE_PKTMBUF_HEADROOM)
pub const DEFAULT_MBUF_HEADROOM: usize = 128;

/// Default data room size for mbufs (2048 bytes of usable space + headroom)
pub const DEFAULT_MBUF_DATA_ROOM_SIZE: usize = 2048 + DEFAULT_MBUF_HEADROOM;

/// Default MTU for test devices
pub const DEFAULT_MTU: usize = 1500;

/// Default number of mbufs in the pool
pub const DEFAULT_NUM_MBUFS: u32 = 8191;

/// Default number of descriptors per queue
pub const DEFAULT_NB_DESC: u16 = 1024;

/// DPDK test context holding all resources needed for a test.
///
/// This is an RAII struct that cleans up all DPDK resources when dropped.
/// Resources are dropped in reverse order: EthDev -> EAL.
pub struct DpdkTestContext {
    /// The EAL instance (must be dropped last)
    _eal: Eal,
    /// The ethernet device
    eth_dev: EthDev,
}

impl DpdkTestContext {
    /// Create a context from pre-initialized components.
    ///
    /// This is useful when you need custom EAL initialization (e.g., for hardware devices).
    pub fn from_parts(eal: Eal, eth_dev: EthDev) -> Self {
        Self { _eal: eal, eth_dev }
    }

    /// Get a reference to the ethernet device.
    pub fn eth_dev(&self) -> &EthDev {
        &self.eth_dev
    }
}

impl Drop for DpdkTestContext {
    fn drop(&mut self) {
        // Stop and close the eth device before EAL cleanup
        if let Err(e) = self.eth_dev.stop() {
            eprintln!("Warning: Failed to stop eth device: {:?}", e);
        }
        if let Err(e) = self.eth_dev.close() {
            eprintln!("Warning: Failed to close eth device: {:?}", e);
        }
    }
}

/// Builder for creating a DPDK test context.
///
/// # Example
/// ```no_run
/// use dpdk_net_test::dpdk_test::DpdkTestContextBuilder;
///
/// let (ctx, device) = DpdkTestContextBuilder::new()
///     .vdev("net_ring0")
///     .mempool_name("test_pool")
///     .build()
///     .expect("Failed to create DPDK test context");
/// ```
pub struct DpdkTestContextBuilder {
    vdev: Option<String>,
    mempool_name: String,
    num_mbufs: u32,
    data_room_size: u16,
    nb_rx_queues: u16,
    nb_tx_queues: u16,
    nb_desc: u16,
    mtu: usize,
    port_id: u16,
}

impl Default for DpdkTestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DpdkTestContextBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self {
            vdev: None,
            mempool_name: "test_mempool".to_string(),
            num_mbufs: DEFAULT_NUM_MBUFS,
            data_room_size: DEFAULT_MBUF_DATA_ROOM_SIZE as u16,
            nb_rx_queues: 1,
            nb_tx_queues: 1,
            nb_desc: DEFAULT_NB_DESC,
            mtu: DEFAULT_MTU,
            port_id: 0,
        }
    }

    /// Add a virtual device (e.g., "net_ring0").
    pub fn vdev(mut self, vdev: impl Into<String>) -> Self {
        self.vdev = Some(vdev.into());
        self
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

    /// Set the number of descriptors per queue.
    pub fn nb_desc(mut self, n: u16) -> Self {
        self.nb_desc = n;
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

    /// Build the test context and DpdkDeviceWithPool.
    ///
    /// Returns the context (which must be kept alive) and the device for smoltcp/Reactor.
    pub fn build(self) -> Result<(DpdkTestContext, DpdkDeviceWithPool), dpdk_net::api::Errno> {
        // Initialize EAL
        let mut eal_builder = EalBuilder::new().no_huge().no_pci();

        if let Some(vdev) = &self.vdev {
            eal_builder = eal_builder.vdev(vdev);
        }

        let eal = eal_builder.init()?;

        // Create mempool
        let mempool_config = MemPoolConfig::new()
            .num_mbufs(self.num_mbufs)
            .data_room_size(self.data_room_size);

        let mempool = MemPool::create(self.mempool_name.clone(), &mempool_config)?;

        // Configure and start ethernet device
        let eth_dev = EthDevBuilder::new(self.port_id)
            .eth_conf(EthConf::new())
            .nb_rx_queues(self.nb_rx_queues)
            .nb_tx_queues(self.nb_tx_queues)
            .rx_queue_conf(RxQueueConf::new().nb_desc(self.nb_desc))
            .tx_queue_conf(TxQueueConf::new().nb_desc(self.nb_desc))
            .build(&mempool)?;

        // Create queue handles (using queue 0)
        let rxq = RxQueue::new(self.port_id, 0);
        let txq = TxQueue::new(self.port_id, 0);

        // Calculate mbuf capacity
        let mbuf_capacity = self.data_room_size as usize - DEFAULT_MBUF_HEADROOM;

        // Create the device
        let device = DpdkDeviceWithPool::new(rxq, txq, mempool, self.mtu, mbuf_capacity);

        let context = DpdkTestContext { _eal: eal, eth_dev };

        Ok((context, device))
    }
}

/// Convenience function to create a simple loopback test setup.
///
/// Creates a DPDK context with a virtual ring device for loopback testing.
pub fn create_loopback_test_setup(
    mempool_name: &str,
) -> Result<(DpdkTestContext, DpdkDeviceWithPool), dpdk_net::api::Errno> {
    DpdkTestContextBuilder::new()
        .vdev("net_ring0")
        .mempool_name(mempool_name)
        .build()
}
