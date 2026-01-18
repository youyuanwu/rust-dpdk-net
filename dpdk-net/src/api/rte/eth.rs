// Ethernet Device API
// See /usr/local/include/rte_ethdev.h

use std::mem::MaybeUninit;

use dpdk_net_sys::ffi;

use super::pktmbuf::MemPool;
use crate::api::{Result, check_rte_success};

/// Ethernet device port ID
pub type PortId = u16;

/// Queue ID for RX/TX queues
pub type QueueId = u16;

// Re-export the raw types for advanced usage
pub use ffi::rte_eth_conf;
pub use ffi::rte_eth_dev_info;
pub use ffi::rte_eth_rxconf;
pub use ffi::rte_eth_stats;
pub use ffi::rte_eth_txconf;
pub use ffi::rte_ether_addr;

/// RX queue multi-queue mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum RxMqMode {
    /// No multi-queue mode
    #[default]
    None = 0,
    /// Receive Side Scaling
    Rss = 1,
    /// Data Center Bridging
    Dcb = 2,
    /// DCB + RSS
    DcbRss = 3,
    /// VMDq only
    Vmdq = 4,
    /// VMDq + DCB
    VmdqDcb = 5,
    /// VMDq + RSS
    VmdqRss = 6,
    /// VMDq + DCB + RSS
    VmdqDcbRss = 7,
}

/// TX queue multi-queue mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum TxMqMode {
    /// No multi-queue mode
    #[default]
    None = 0,
    /// Data Center Bridging
    Dcb = 1,
    /// VMDq only
    Vmdq = 2,
    /// VMDq + DCB
    VmdqDcb = 3,
}

/// RX mode configuration
#[derive(Debug, Clone)]
pub struct RxMode {
    /// Multi-queue mode
    pub mq_mode: RxMqMode,
    /// Maximum Transfer Unit
    pub mtu: u32,
    /// RX offload flags (RTE_ETH_RX_OFFLOAD_*)
    pub offloads: u64,
    /// Max LRO aggregated packet size
    pub max_lro_pkt_size: u32,
}

impl Default for RxMode {
    fn default() -> Self {
        Self {
            mq_mode: RxMqMode::None,
            mtu: 1500,
            offloads: 0,
            max_lro_pkt_size: 0,
        }
    }
}

/// TX mode configuration  
#[derive(Debug, Clone)]
pub struct TxMode {
    /// Multi-queue mode
    pub mq_mode: TxMqMode,
    /// TX offload flags (RTE_ETH_TX_OFFLOAD_*)
    pub offloads: u64,
}

impl Default for TxMode {
    fn default() -> Self {
        Self {
            mq_mode: TxMqMode::None,
            offloads: 0,
        }
    }
}

/// Ethernet device configuration
#[derive(Debug, Clone, Default)]
pub struct EthConf {
    /// Link speed bitmap (0 for autoneg)
    pub link_speeds: u32,
    /// RX mode configuration
    pub rx_mode: RxMode,
    /// TX mode configuration
    pub tx_mode: TxMode,
    /// Loopback mode (0 = disabled)
    pub loopback_mode: u32,
}

impl EthConf {
    /// Create a simple configuration with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Set MTU
    pub fn mtu(mut self, mtu: u32) -> Self {
        self.rx_mode.mtu = mtu;
        self
    }

    /// Set RX offloads
    pub fn rx_offloads(mut self, offloads: u64) -> Self {
        self.rx_mode.offloads = offloads;
        self
    }

    /// Set TX offloads
    pub fn tx_offloads(mut self, offloads: u64) -> Self {
        self.tx_mode.offloads = offloads;
        self
    }

    /// Enable RSS mode
    pub fn rss(mut self) -> Self {
        self.rx_mode.mq_mode = RxMqMode::Rss;
        self
    }

    /// Enable loopback mode
    pub fn loopback(mut self) -> Self {
        self.loopback_mode = 1;
        self
    }

    /// Convert to raw rte_eth_conf
    fn to_raw(&self) -> ffi::rte_eth_conf {
        let mut conf: ffi::rte_eth_conf = unsafe { std::mem::zeroed() };
        conf.link_speeds = self.link_speeds;
        conf.rxmode.mq_mode = self.rx_mode.mq_mode as u32;
        conf.rxmode.mtu = self.rx_mode.mtu;
        conf.rxmode.offloads = self.rx_mode.offloads;
        conf.rxmode.max_lro_pkt_size = self.rx_mode.max_lro_pkt_size;
        conf.txmode.mq_mode = self.tx_mode.mq_mode as u32;
        conf.txmode.offloads = self.tx_mode.offloads;
        conf.lpbk_mode = self.loopback_mode;
        conf
    }
}

/// RX queue configuration
#[derive(Debug, Clone)]
pub struct RxQueueConf {
    /// Number of descriptors
    pub nb_desc: u16,
    /// NUMA socket ID (-1 for any)
    pub socket_id: i32,
    /// Optional RX conf (None uses device defaults)
    pub conf: Option<ffi::rte_eth_rxconf>,
}

impl Default for RxQueueConf {
    fn default() -> Self {
        Self {
            nb_desc: 1024,
            socket_id: -1,
            conf: None,
        }
    }
}

impl RxQueueConf {
    /// Create a new RxQueueConf with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the number of descriptors for the RX ring.
    pub fn nb_desc(mut self, n: u16) -> Self {
        self.nb_desc = n;
        self
    }

    /// Set the NUMA socket ID.
    ///
    /// Use -1 for automatic detection based on the device.
    pub fn socket_id(mut self, id: i32) -> Self {
        self.socket_id = id;
        self
    }

    /// Set the raw RX configuration.
    ///
    /// Use `None` to use device defaults.
    pub fn conf(mut self, conf: ffi::rte_eth_rxconf) -> Self {
        self.conf = Some(conf);
        self
    }
}

/// TX queue configuration
#[derive(Debug, Clone)]
pub struct TxQueueConf {
    /// Number of descriptors
    pub nb_desc: u16,
    /// NUMA socket ID (-1 for any)
    pub socket_id: i32,
    /// Optional TX conf (None uses device defaults)
    pub conf: Option<ffi::rte_eth_txconf>,
}

impl Default for TxQueueConf {
    fn default() -> Self {
        Self {
            nb_desc: 1024,
            socket_id: -1,
            conf: None,
        }
    }
}

impl TxQueueConf {
    /// Create a new TxQueueConf with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the number of descriptors for the TX ring.
    pub fn nb_desc(mut self, n: u16) -> Self {
        self.nb_desc = n;
        self
    }

    /// Set the NUMA socket ID.
    ///
    /// Use -1 for automatic detection based on the device.
    pub fn socket_id(mut self, id: i32) -> Self {
        self.socket_id = id;
        self
    }

    /// Set the raw TX configuration.
    ///
    /// Use `None` to use device defaults.
    pub fn conf(mut self, conf: ffi::rte_eth_txconf) -> Self {
        self.conf = Some(conf);
        self
    }
}

/// Ethernet device wrapper
pub struct EthDev {
    port_id: PortId,
}

impl EthDev {
    /// Create a handle for an existing port
    ///
    /// Does not configure or start the device.
    pub fn new(port_id: PortId) -> Self {
        Self { port_id }
    }

    /// Get the port ID
    #[inline]
    pub fn port_id(&self) -> PortId {
        self.port_id
    }

    /// Get the number of available Ethernet devices
    pub fn count_avail() -> u16 {
        unsafe { ffi::rte_eth_dev_count_avail() }
    }

    /// Get device info
    pub fn info(&self) -> Result<ffi::rte_eth_dev_info> {
        let mut info = MaybeUninit::<ffi::rte_eth_dev_info>::uninit();
        let ret = unsafe { ffi::rte_eth_dev_info_get(self.port_id, info.as_mut_ptr()) };
        check_rte_success(ret)?;
        Ok(unsafe { info.assume_init() })
    }

    /// Get the NUMA socket ID of the device
    pub fn socket_id(&self) -> i32 {
        unsafe { ffi::rte_eth_dev_socket_id(self.port_id) }
    }

    /// Get the MAC address
    pub fn mac_addr(&self) -> Result<ffi::rte_ether_addr> {
        let mut addr = MaybeUninit::<ffi::rte_ether_addr>::uninit();
        let ret = unsafe { ffi::rte_eth_macaddr_get(self.port_id, addr.as_mut_ptr()) };
        check_rte_success(ret)?;
        Ok(unsafe { addr.assume_init() })
    }

    /// Get device statistics
    pub fn stats(&self) -> Result<ffi::rte_eth_stats> {
        let mut stats = MaybeUninit::<ffi::rte_eth_stats>::uninit();
        let ret = unsafe { ffi::rte_eth_stats_get(self.port_id, stats.as_mut_ptr()) };
        check_rte_success(ret)?;
        Ok(unsafe { stats.assume_init() })
    }

    /// Configure the device
    pub fn configure(&self, nb_rx_queues: u16, nb_tx_queues: u16, conf: &EthConf) -> Result<()> {
        let raw_conf = conf.to_raw();
        let ret = unsafe {
            ffi::rte_eth_dev_configure(self.port_id, nb_rx_queues, nb_tx_queues, &raw_conf)
        };
        check_rte_success(ret)
    }

    /// Setup an RX queue
    pub fn rx_queue_setup(
        &self,
        queue_id: QueueId,
        mempool: &MemPool,
        conf: &RxQueueConf,
    ) -> Result<()> {
        let conf_ptr = conf
            .conf
            .as_ref()
            .map_or(std::ptr::null(), |c| c as *const _);
        let socket_id = if conf.socket_id < 0 {
            self.socket_id() as u32
        } else {
            conf.socket_id as u32
        };
        let ret = unsafe {
            ffi::rte_eth_rx_queue_setup(
                self.port_id,
                queue_id,
                conf.nb_desc,
                socket_id,
                conf_ptr,
                mempool.as_ptr(),
            )
        };
        check_rte_success(ret)
    }

    /// Setup a TX queue
    pub fn tx_queue_setup(&self, queue_id: QueueId, conf: &TxQueueConf) -> Result<()> {
        let conf_ptr = conf
            .conf
            .as_ref()
            .map_or(std::ptr::null(), |c| c as *const _);
        let socket_id = if conf.socket_id < 0 {
            self.socket_id() as u32
        } else {
            conf.socket_id as u32
        };
        let ret = unsafe {
            ffi::rte_eth_tx_queue_setup(self.port_id, queue_id, conf.nb_desc, socket_id, conf_ptr)
        };
        check_rte_success(ret)
    }

    /// Start the device
    pub fn start(&self) -> Result<()> {
        let ret = unsafe { ffi::rte_eth_dev_start(self.port_id) };
        check_rte_success(ret)
    }

    /// Stop the device
    pub fn stop(&self) -> Result<()> {
        let ret = unsafe { ffi::rte_eth_dev_stop(self.port_id) };
        check_rte_success(ret)
    }

    /// Close the device
    pub fn close(&self) -> Result<()> {
        let ret = unsafe { ffi::rte_eth_dev_close(self.port_id) };
        check_rte_success(ret)
    }

    /// Enable promiscuous mode
    pub fn promiscuous_enable(&self) -> Result<()> {
        let ret = unsafe { ffi::rte_eth_promiscuous_enable(self.port_id) };
        check_rte_success(ret)
    }

    /// Disable promiscuous mode
    pub fn promiscuous_disable(&self) -> Result<()> {
        let ret = unsafe { ffi::rte_eth_promiscuous_disable(self.port_id) };
        check_rte_success(ret)
    }
}

/// Builder for configuring and starting an Ethernet device
pub struct EthDevBuilder {
    port_id: PortId,
    eth_conf: EthConf,
    nb_rx_queues: u16,
    nb_tx_queues: u16,
    rx_queue_conf: RxQueueConf,
    tx_queue_conf: TxQueueConf,
    promiscuous: bool,
}

impl EthDevBuilder {
    /// Create a new builder for the given port
    pub fn new(port_id: PortId) -> Self {
        Self {
            port_id,
            eth_conf: EthConf::default(),
            nb_rx_queues: 1,
            nb_tx_queues: 1,
            rx_queue_conf: RxQueueConf::default(),
            tx_queue_conf: TxQueueConf::default(),
            promiscuous: false,
        }
    }

    /// Set device configuration
    pub fn eth_conf(mut self, conf: EthConf) -> Self {
        self.eth_conf = conf;
        self
    }

    /// Set number of RX queues
    pub fn nb_rx_queues(mut self, n: u16) -> Self {
        self.nb_rx_queues = n;
        self
    }

    /// Set number of TX queues
    pub fn nb_tx_queues(mut self, n: u16) -> Self {
        self.nb_tx_queues = n;
        self
    }

    /// Set RX queue configuration (applied to all queues)
    pub fn rx_queue_conf(mut self, conf: RxQueueConf) -> Self {
        self.rx_queue_conf = conf;
        self
    }

    /// Set TX queue configuration (applied to all queues)
    pub fn tx_queue_conf(mut self, conf: TxQueueConf) -> Self {
        self.tx_queue_conf = conf;
        self
    }

    /// Enable promiscuous mode
    pub fn promiscuous(mut self) -> Self {
        self.promiscuous = true;
        self
    }

    /// Build and start the device
    ///
    /// This will:
    /// 1. Configure the device
    /// 2. Setup all RX queues
    /// 3. Setup all TX queues
    /// 4. Enable promiscuous mode (if set)
    /// 5. Start the device
    pub fn build(self, mempool: &MemPool) -> Result<EthDev> {
        let dev = EthDev::new(self.port_id);

        // Configure device
        dev.configure(self.nb_rx_queues, self.nb_tx_queues, &self.eth_conf)?;

        // Setup RX queues
        for q in 0..self.nb_rx_queues {
            dev.rx_queue_setup(q, mempool, &self.rx_queue_conf)?;
        }

        // Setup TX queues
        for q in 0..self.nb_tx_queues {
            dev.tx_queue_setup(q, &self.tx_queue_conf)?;
        }

        // Enable promiscuous mode if requested
        if self.promiscuous {
            dev.promiscuous_enable()?;
        }

        // Start the device
        dev.start()?;

        Ok(dev)
    }
}

/// Iterate over available port IDs
pub fn iter_ports() -> impl Iterator<Item = PortId> {
    0..EthDev::count_avail()
}

/// Format MAC address as string
pub fn format_mac_addr(addr: &ffi::rte_ether_addr) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        addr.addr_bytes[0],
        addr.addr_bytes[1],
        addr.addr_bytes[2],
        addr.addr_bytes[3],
        addr.addr_bytes[4],
        addr.addr_bytes[5]
    )
}
