//! DPDK Test Harness
//!
//! Provides reusable components for DPDK-based tests including:
//! - `DpdkTestContext` - RAII struct holding EAL, EthDev
//! - Builder for configuring test scenarios
//!
//! Uses `DpdkDevice` from `dpdk_net::tcp` for the smoltcp Device implementation.

use dpdk_net::api::rte::eal::{Eal, EalBuilder};
use dpdk_net::api::rte::eth::EthDev;

use crate::eth_dev_config::EthDevConfig;

// Re-export DpdkDevice for convenience
pub use dpdk_net::tcp::DpdkDevice;

// Re-export constants from eth_dev_config for backward compatibility
pub use crate::eth_dev_config::{
    DEFAULT_MBUF_DATA_ROOM_SIZE, DEFAULT_MBUF_HEADROOM, DEFAULT_MTU, DEFAULT_NB_DESC,
    DEFAULT_NUM_MBUFS,
};

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
    eth_dev_config: EthDevConfig,
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
            eth_dev_config: EthDevConfig::new().mempool_name("test_mempool"),
        }
    }

    /// Add a virtual device (e.g., "net_ring0").
    pub fn vdev(mut self, vdev: impl Into<String>) -> Self {
        self.vdev = Some(vdev.into());
        self
    }

    /// Set the mempool name.
    pub fn mempool_name(mut self, name: impl Into<String>) -> Self {
        self.eth_dev_config = self.eth_dev_config.mempool_name(name);
        self
    }

    /// Set the number of mbufs in the pool.
    pub fn num_mbufs(mut self, n: u32) -> Self {
        self.eth_dev_config = self.eth_dev_config.num_mbufs(n);
        self
    }

    /// Set the data room size for mbufs.
    pub fn data_room_size(mut self, size: u16) -> Self {
        self.eth_dev_config = self.eth_dev_config.data_room_size(size);
        self
    }

    /// Set the number of RX queues.
    pub fn nb_rx_queues(mut self, n: u16) -> Self {
        self.eth_dev_config = self.eth_dev_config.nb_rx_queues(n);
        self
    }

    /// Set the number of TX queues.
    pub fn nb_tx_queues(mut self, n: u16) -> Self {
        self.eth_dev_config = self.eth_dev_config.nb_tx_queues(n);
        self
    }

    /// Set the number of descriptors per queue.
    pub fn nb_desc(mut self, n: u16) -> Self {
        self.eth_dev_config = self.eth_dev_config.nb_desc(n);
        self
    }

    /// Set the MTU.
    pub fn mtu(mut self, mtu: usize) -> Self {
        self.eth_dev_config = self.eth_dev_config.mtu(mtu);
        self
    }

    /// Set the port ID.
    pub fn port_id(mut self, id: u16) -> Self {
        self.eth_dev_config = self.eth_dev_config.port_id(id);
        self
    }

    /// Build the test context and DpdkDevice.
    ///
    /// Returns the context (which must be kept alive) and the device for smoltcp/Reactor.
    pub fn build(self) -> Result<(DpdkTestContext, DpdkDevice), dpdk_net::api::Errno> {
        // Initialize EAL (test mode: no hugepages, no PCI)
        let mut eal_builder = EalBuilder::new().no_huge().no_pci();

        if let Some(vdev) = &self.vdev {
            eal_builder = eal_builder.vdev(vdev);
        }

        let eal = eal_builder.init()?;

        // Build mempool and eth device using shared config
        let (mempool, eth_dev) = self.eth_dev_config.clone().build()?;

        // Create device for queue 0
        let device = self.eth_dev_config.create_device(mempool, 0);

        let context = DpdkTestContext { _eal: eal, eth_dev };

        Ok((context, device))
    }

    /// Build only the EthDev and DpdkDevice, assuming EAL is already initialized.
    ///
    /// Use this when you have a global EAL and want to recreate devices per test.
    /// Returns the EthDev and DpdkDevice (caller is responsible for cleanup).
    pub fn build_device_only(self) -> Result<(EthDev, DpdkDevice), dpdk_net::api::Errno> {
        // Build mempool and eth device using shared config
        let (mempool, eth_dev) = self.eth_dev_config.clone().build()?;

        // Create device for queue 0
        let device = self.eth_dev_config.create_device(mempool, 0);

        Ok((eth_dev, device))
    }
}

/// Convenience function to create a simple loopback test setup.
///
/// Creates a DPDK context with a virtual ring device for loopback testing.
pub fn create_loopback_test_setup(
    mempool_name: &str,
) -> Result<(DpdkTestContext, DpdkDevice), dpdk_net::api::Errno> {
    DpdkTestContextBuilder::new()
        .vdev("net_ring0")
        .mempool_name(mempool_name)
        .build()
}
