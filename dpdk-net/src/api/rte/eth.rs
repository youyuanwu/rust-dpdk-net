// Ethernet Device API
// See /usr/local/include/rte_ethdev.h

use std::fmt;
use std::mem::MaybeUninit;

use dpdk_net_sys::ffi;
use tracing::{debug, warn};

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

/// RSS hash function flags for TCP/IP packet distribution
/// Re-exported from generated bindings (from wrapper.h static consts)
pub mod rss_hf {
    use dpdk_net_sys::ffi;

    /// IPv4 (hashes on src/dst IP)
    pub const IPV4: u64 = ffi::RUST_RTE_ETH_RSS_IPV4;
    /// IPv4 fragmented packets
    pub const FRAG_IPV4: u64 = ffi::RUST_RTE_ETH_RSS_FRAG_IPV4;
    /// IPv4 TCP (hashes on src/dst IP + src/dst port)
    pub const NONFRAG_IPV4_TCP: u64 = ffi::RUST_RTE_ETH_RSS_NONFRAG_IPV4_TCP;
    /// IPv4 UDP
    pub const NONFRAG_IPV4_UDP: u64 = ffi::RUST_RTE_ETH_RSS_NONFRAG_IPV4_UDP;
    /// IPv4 SCTP
    pub const NONFRAG_IPV4_SCTP: u64 = ffi::RUST_RTE_ETH_RSS_NONFRAG_IPV4_SCTP;
    /// IPv4 other
    pub const NONFRAG_IPV4_OTHER: u64 = ffi::RUST_RTE_ETH_RSS_NONFRAG_IPV4_OTHER;
    /// IPv6
    pub const IPV6: u64 = ffi::RUST_RTE_ETH_RSS_IPV6;
    /// IPv6 fragmented
    pub const FRAG_IPV6: u64 = ffi::RUST_RTE_ETH_RSS_FRAG_IPV6;
    /// IPv6 TCP
    pub const NONFRAG_IPV6_TCP: u64 = ffi::RUST_RTE_ETH_RSS_NONFRAG_IPV6_TCP;
    /// IPv6 UDP
    pub const NONFRAG_IPV6_UDP: u64 = ffi::RUST_RTE_ETH_RSS_NONFRAG_IPV6_UDP;
    /// IPv6 SCTP
    pub const NONFRAG_IPV6_SCTP: u64 = ffi::RUST_RTE_ETH_RSS_NONFRAG_IPV6_SCTP;
    /// IPv6 other
    pub const NONFRAG_IPV6_OTHER: u64 = ffi::RUST_RTE_ETH_RSS_NONFRAG_IPV6_OTHER;
    /// IPv6 extended header
    pub const IPV6_EX: u64 = ffi::RUST_RTE_ETH_RSS_IPV6_EX;
    /// IPv6 TCP extended
    pub const IPV6_TCP_EX: u64 = ffi::RUST_RTE_ETH_RSS_IPV6_TCP_EX;
    /// IPv6 UDP extended
    pub const IPV6_UDP_EX: u64 = ffi::RUST_RTE_ETH_RSS_IPV6_UDP_EX;

    /// Combined: All IP (IPv4 + IPv6)
    pub const IP: u64 = ffi::RUST_RTE_ETH_RSS_IP;
    /// Combined: All TCP (IPv4 + IPv6)
    pub const TCP: u64 = ffi::RUST_RTE_ETH_RSS_TCP;
    /// Combined: All UDP (IPv4 + IPv6)
    pub const UDP: u64 = ffi::RUST_RTE_ETH_RSS_UDP;
}

/// Standard Microsoft RSS key (40 bytes) for Toeplitz hash
/// This key provides good distribution for TCP/IP traffic
pub const RSS_KEY_40: [u8; 40] = [
    0x6d, 0x5a, 0x56, 0xda, 0x25, 0x5b, 0x0e, 0xc2, 0x41, 0x67, 0x25, 0x3d, 0x43, 0xa3, 0x8f, 0xb0,
    0xd0, 0xca, 0x2b, 0xcb, 0xae, 0x7b, 0x30, 0xb4, 0x77, 0xcb, 0x2d, 0xa3, 0x80, 0x30, 0xf2, 0x0c,
    0x6a, 0x42, 0xb7, 0x3b, 0xbe, 0xac, 0x01, 0xfa,
];

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
    /// RSS hash function flags (only used when rx_mode.mq_mode == Rss)
    pub rss_hf: u64,
    /// RSS key (None = use driver default, Some = use this key)
    pub rss_key: Option<Vec<u8>>,
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

    /// Enable RSS mode with default TCP/IP hash function and standard key
    pub fn rss(mut self) -> Self {
        self.rx_mode.mq_mode = RxMqMode::Rss;
        // Use driver defaults for RSS hash function (don't override)
        // Setting rss_hf to 0 tells DPDK to use driver defaults
        self.rss_hf = 0;
        self.rss_key = None;
        self
    }

    /// Enable RSS mode with explicit hash types
    pub fn rss_with_hash(mut self, hf: u64) -> Self {
        self.rx_mode.mq_mode = RxMqMode::Rss;
        self.rss_hf = hf;
        self.rss_key = None;
        self
    }

    /// Enable RSS mode with explicit Microsoft RSS key
    pub fn rss_with_key(mut self) -> Self {
        self.rx_mode.mq_mode = RxMqMode::Rss;
        self.rss_hf = rss_hf::IP | rss_hf::TCP;
        self.rss_key = Some(RSS_KEY_40.to_vec());
        self
    }

    /// Set custom RSS hash function flags
    pub fn rss_hf(mut self, hf: u64) -> Self {
        self.rss_hf = hf;
        self
    }

    /// Enable loopback mode
    pub fn loopback(mut self) -> Self {
        self.loopback_mode = 1;
        self
    }

    /// Convert to raw rte_eth_conf
    /// Returns the config and an optional key buffer that must be kept alive
    fn to_raw(&self) -> (ffi::rte_eth_conf, Option<Vec<u8>>) {
        let mut conf: ffi::rte_eth_conf = unsafe { std::mem::zeroed() };
        conf.link_speeds = self.link_speeds;
        conf.rxmode.mq_mode = self.rx_mode.mq_mode as u32;
        conf.rxmode.mtu = self.rx_mode.mtu;
        conf.rxmode.offloads = self.rx_mode.offloads;
        conf.rxmode.max_lro_pkt_size = self.rx_mode.max_lro_pkt_size;
        conf.txmode.mq_mode = self.tx_mode.mq_mode as u32;
        conf.txmode.offloads = self.tx_mode.offloads;
        conf.lpbk_mode = self.loopback_mode;

        let mut key_buffer: Option<Vec<u8>> = None;

        // Configure RSS hash function if RSS mode is enabled
        if self.rx_mode.mq_mode == RxMqMode::Rss && self.rss_hf != 0 {
            conf.rx_adv_conf.rss_conf.rss_hf = self.rss_hf;

            if let Some(ref key) = self.rss_key {
                // Clone the key and store it first
                key_buffer = Some(key.clone());
                // Now get pointer from the stored buffer (after it's in its final location)
                let key_ref = key_buffer.as_mut().unwrap();
                conf.rx_adv_conf.rss_conf.rss_key = key_ref.as_mut_ptr();
                conf.rx_adv_conf.rss_conf.rss_key_len = key_ref.len() as u8;
            } else {
                // Use driver default key
                conf.rx_adv_conf.rss_conf.rss_key = std::ptr::null_mut();
                conf.rx_adv_conf.rss_conf.rss_key_len = 0;
            }
        }

        (conf, key_buffer)
    }
}

/// RX queue configuration
#[derive(Clone)]
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

impl fmt::Debug for RxQueueConf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RxQueueConf")
            .field("nb_desc", &self.nb_desc)
            .field("socket_id", &self.socket_id)
            .field("conf", &self.conf.as_ref().map(|_| "<rte_eth_rxconf>"))
            .finish()
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
#[derive(Clone)]
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

impl fmt::Debug for TxQueueConf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TxQueueConf")
            .field("nb_desc", &self.nb_desc)
            .field("socket_id", &self.socket_id)
            .field("conf", &self.conf.as_ref().map(|_| "<rte_eth_txconf>"))
            .finish()
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

    /// Get device info.
    ///
    /// **Note**: The `max_rx_queues` and `max_tx_queues` fields in the returned
    /// `rte_eth_dev_info` represent the driver's theoretical maximum, which may
    /// be much higher than the actual hardware capability. For example, the MANA
    /// driver (Azure accelerated networking) reports 1024, but the actual hardware
    /// limit is typically equal to the number of vCPUs.
    ///
    /// To get the actual hardware queue count, use the ethtool GCHANNELS ioctl
    /// (equivalent to `ethtool -l <interface>`) before initializing DPDK.
    /// See [`dpdk_net_test::util::get_ethtool_channels`] for a helper function.
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
        let (raw_conf, _key_buffer) = conf.to_raw();
        // Note: _key_buffer is kept alive until after rte_eth_dev_configure returns
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

    /// Query the actual RSS hash configuration from the device.
    ///
    /// Returns the RSS hash functions that are actually enabled (not just advertised).
    /// This should be called after the device is started.
    ///
    /// Returns (rss_hf, rss_key) where rss_hf is the bitmask of enabled hash functions.
    pub fn rss_hash_conf(&self) -> Result<(u64, Vec<u8>)> {
        let mut rss_conf: ffi::rte_eth_rss_conf = unsafe { std::mem::zeroed() };

        // Allocate buffer for RSS key (max 52 bytes for some NICs)
        let mut key_buffer = vec![0u8; 52];
        rss_conf.rss_key = key_buffer.as_mut_ptr();
        rss_conf.rss_key_len = key_buffer.len() as u8;

        let ret = unsafe { ffi::rte_eth_dev_rss_hash_conf_get(self.port_id, &mut rss_conf) };
        check_rte_success(ret)?;

        // Truncate key to actual length
        key_buffer.truncate(rss_conf.rss_key_len as usize);

        Ok((rss_conf.rss_hf, key_buffer))
    }

    /// Check if TCP RSS hashing is actually enabled on this device.
    ///
    /// Returns true if the device is hashing on TCP ports (4-tuple),
    /// false if only IP-based hashing is active.
    pub fn has_tcp_rss(&self) -> Result<bool> {
        let (rss_hf, _) = self.rss_hash_conf()?;
        // Check for IPv4-TCP or IPv6-TCP hash functions
        let tcp_flags = rss_hf::NONFRAG_IPV4_TCP | rss_hf::NONFRAG_IPV6_TCP;
        Ok((rss_hf & tcp_flags) != 0)
    }

    /// Configure RSS Redirection Table (RETA) to distribute packets across queues.
    ///
    /// This sets up the RETA to evenly distribute traffic across the specified
    /// number of RX queues using round-robin assignment.
    pub fn configure_rss_reta(&self, nb_rx_queues: u16) -> Result<()> {
        // Get device info to find RETA size
        let info = self.info()?;
        let reta_size = info.reta_size;

        if reta_size == 0 {
            // Device doesn't support RSS RETA
            return Ok(());
        }

        // Each rte_eth_rss_reta_entry64 covers 64 entries
        let num_groups = (reta_size as usize).div_ceil(64);

        // Allocate RETA configuration
        let mut reta_conf: Vec<ffi::rte_eth_rss_reta_entry64> =
            vec![unsafe { std::mem::zeroed() }; num_groups];

        // Configure each entry to map to queues in round-robin
        for (group_idx, group) in reta_conf.iter_mut().enumerate() {
            group.mask = u64::MAX; // Update all entries in this group
            for i in 0..64 {
                let entry_idx = group_idx * 64 + i;
                if entry_idx < reta_size as usize {
                    group.reta[i] = (entry_idx % nb_rx_queues as usize) as u16;
                }
            }
        }

        let ret = unsafe {
            ffi::rte_eth_dev_rss_reta_update(self.port_id, reta_conf.as_mut_ptr(), reta_size)
        };
        check_rte_success(ret)
    }

    /// Update the RSS hash configuration on the device.
    ///
    /// This should be called after the device is configured to ensure the
    /// RSS hash function is properly applied.
    pub fn update_rss_hash(&self, rss_hf: u64, key: Option<&[u8]>) -> Result<()> {
        let mut rss_conf: ffi::rte_eth_rss_conf = unsafe { std::mem::zeroed() };
        rss_conf.rss_hf = rss_hf;

        // Use provided key or null for driver default
        let mut key_buffer: Vec<u8>;
        if let Some(k) = key {
            key_buffer = k.to_vec();
            rss_conf.rss_key = key_buffer.as_mut_ptr();
            rss_conf.rss_key_len = key_buffer.len() as u8;
        } else {
            rss_conf.rss_key = std::ptr::null_mut();
            rss_conf.rss_key_len = 0;
        }

        let ret = unsafe { ffi::rte_eth_dev_rss_hash_update(self.port_id, &mut rss_conf) };
        check_rte_success(ret)
    }

    /// Query the current RSS RETA configuration
    pub fn query_rss_reta(&self) -> Result<Vec<u16>> {
        let info = self.info()?;
        let reta_size = info.reta_size;

        if reta_size == 0 {
            return Ok(vec![]);
        }

        let num_groups = (reta_size as usize).div_ceil(64);
        let mut reta_conf: Vec<ffi::rte_eth_rss_reta_entry64> =
            vec![unsafe { std::mem::zeroed() }; num_groups];

        // Set mask to query all entries
        for group in reta_conf.iter_mut() {
            group.mask = u64::MAX;
        }

        let ret = unsafe {
            ffi::rte_eth_dev_rss_reta_query(self.port_id, reta_conf.as_mut_ptr(), reta_size)
        };
        check_rte_success(ret)?;

        // Collect all RETA entries
        let mut result = Vec::with_capacity(reta_size as usize);
        for (group_idx, group) in reta_conf.iter().enumerate() {
            for i in 0..64 {
                let entry_idx = group_idx * 64 + i;
                if entry_idx < reta_size as usize {
                    result.push(group.reta[i]);
                }
            }
        }

        Ok(result)
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
    /// 4. Configure RSS RETA (if multi-queue)
    /// 5. Update RSS hash configuration (if multi-queue)
    /// 6. Enable promiscuous mode (if set)
    /// 7. Start the device
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

        // Configure RSS for multi-queue
        if self.nb_rx_queues > 1 {
            // Configure RETA before starting
            match dev.configure_rss_reta(self.nb_rx_queues) {
                Ok(()) => debug!(nb_rx_queues = self.nb_rx_queues, "RSS RETA configured"),
                Err(e) => {
                    warn!(error = %e, "Failed to configure RSS RETA (driver may not support it)")
                }
            }

            // Explicitly update RSS hash configuration to ensure it's applied
            // Use the same rss_hf from eth_conf, or default to IP-based hashing
            let rss_hf = self.eth_conf.rss_hf;
            if rss_hf != 0 {
                // Use Microsoft RSS key for Azure NICs
                match dev.update_rss_hash(rss_hf, Some(&RSS_KEY_40)) {
                    Ok(()) => debug!(rss_hf = format!("{:#x}", rss_hf), "RSS hash updated"),
                    Err(e) => {
                        warn!(error = %e, rss_hf = format!("{:#x}", rss_hf), "Failed to update RSS hash")
                    }
                }
            }
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
