//! DPDK Test Harness
//!
//! Provides reusable components for DPDK-based manual (non-async) tests:
//! - `DpdkTestContext` - RAII struct holding EAL, EthDev
//! - `create_test_context()` - creates a virtual ring loopback setup
//!
//! For async tests, prefer `DpdkApp` from `dpdk-net-util`.

use dpdk_net::api::rte::eal::{Eal, EalBuilder};
use dpdk_net::api::rte::eth::EthDev;

use crate::eth_dev_config::EthDevConfig;

// Re-export DpdkDevice for convenience
pub use dpdk_net::device::DpdkDevice;

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

/// Create a DPDK test context with a virtual ring device for loopback testing.
///
/// Initializes EAL (no hugepages, no PCI), creates a mempool and eth device,
/// and returns the context (must be kept alive) and the `DpdkDevice` for smoltcp.
///
/// # Example
/// ```no_run
/// use dpdk_net_test::dpdk_test::create_test_context;
///
/// let (ctx, device) = create_test_context()
///     .expect("Failed to create DPDK test context");
/// ```
pub fn create_test_context() -> Result<(DpdkTestContext, DpdkDevice), dpdk_net::api::Errno> {
    let eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .vdev("net_ring0")
        .init()?;

    let eth_dev_config = EthDevConfig::new().mempool_name("test_mempool");
    let (mempool, eth_dev) = eth_dev_config.clone().build()?;
    let device = eth_dev_config.create_device(mempool, 0);

    let context = DpdkTestContext { _eal: eal, eth_dev };
    Ok((context, device))
}
