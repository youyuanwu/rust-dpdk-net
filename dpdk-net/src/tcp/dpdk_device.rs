use arrayvec::ArrayVec;
use smoltcp::phy::{self, Device, DeviceCapabilities, Medium};
use smoltcp::time::Instant;
use std::net::Ipv4Addr;
use std::sync::Arc;

use crate::api::rte::mbuf::Mbuf;
use crate::api::rte::pktmbuf::MemPool;
use crate::api::rte::queue::{RxQueue, TxQueue};

use super::arp_cache::{SharedArpCache, parse_arp_reply};

/// Default headroom reserved at the front of each mbuf (matches RTE_PKTMBUF_HEADROOM)
pub const DEFAULT_MBUF_HEADROOM: usize = 128;

/// Default data room size for mbufs (2048 bytes of usable space + headroom)
pub const DEFAULT_MBUF_DATA_ROOM_SIZE: usize = 2048 + DEFAULT_MBUF_HEADROOM;

/// Maximum packet overhead: Ethernet (14) + IP (20) + TCP with options (60)
const MAX_PACKET_OVERHEAD: usize = 14 + 20 + 60;

pub struct DpdkRxToken {
    mbuf: Mbuf,
}

impl phy::RxToken for DpdkRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        // Smoltcp reads the received packet data (immutable reference)
        f(self.mbuf.data())
    }
}

/// More complete implementation with mempool access
pub struct DpdkDevice {
    rxq: RxQueue,
    txq: TxQueue,
    mempool: Arc<MemPool>,
    rx_batch: ArrayVec<Mbuf, 64>,
    tx_batch: ArrayVec<Mbuf, 256>,
    mtu: usize,
    #[allow(dead_code)] // Validated in constructor, stored for debugging/future use
    mbuf_capacity: usize,
    /// Queue ID (0 = producer for shared ARP cache)
    queue_id: u16,
    /// Shared ARP cache for multi-queue setups (optional)
    shared_arp_cache: Option<SharedArpCache>,
    /// Our MAC address (for building ARP injection packets)
    our_mac: Option<[u8; 6]>,
    /// Our IP address (for building ARP injection packets)  
    our_ip: Option<Ipv4Addr>,
    /// Last seen cache version (skip injection if unchanged)
    last_cache_version: usize,
}

impl DpdkDevice {
    /// Create a new DPDK device for smoltcp.
    ///
    /// # Arguments
    /// * `rxq` - DPDK receive queue
    /// * `txq` - DPDK transmit queue  
    /// * `mempool` - Memory pool for mbuf allocation (wrapped in Arc for sharing)
    /// * `mtu` - Maximum transmission unit (payload size, typically 1500)
    /// * `mbuf_capacity` - Usable capacity of mbufs (data_room_size - headroom)
    ///
    /// # Panics
    /// Panics if MTU + maximum packet overhead exceeds mbuf capacity.
    pub fn new(
        rxq: RxQueue,
        txq: TxQueue,
        mempool: Arc<MemPool>,
        mtu: usize,
        mbuf_capacity: usize,
    ) -> Self {
        assert!(
            mtu + MAX_PACKET_OVERHEAD <= mbuf_capacity,
            "MTU ({}) + max overhead ({}) = {} exceeds mbuf capacity ({})",
            mtu,
            MAX_PACKET_OVERHEAD,
            mtu + MAX_PACKET_OVERHEAD,
            mbuf_capacity
        );
        Self {
            rxq,
            txq,
            mempool,
            rx_batch: ArrayVec::new(),
            tx_batch: ArrayVec::new(),
            mtu,
            mbuf_capacity,
            queue_id: 0,
            shared_arp_cache: None,
            our_mac: None,
            our_ip: None,
            last_cache_version: 0,
        }
    }

    /// Configure shared ARP cache for multi-queue support.
    ///
    /// # Arguments
    /// * `queue_id` - This queue's ID (queue 0 is the ARP producer)
    /// * `cache` - Shared ARP cache
    /// * `our_mac` - Our interface MAC address
    /// * `our_ip` - Our interface IP address
    ///
    /// Queue 0 will update the cache when it receives ARP replies.
    /// Other queues will check the cache and inject ARP packets into smoltcp.
    pub fn with_shared_arp_cache(
        mut self,
        queue_id: u16,
        cache: SharedArpCache,
        our_mac: [u8; 6],
        our_ip: Ipv4Addr,
    ) -> Self {
        self.queue_id = queue_id;
        self.shared_arp_cache = Some(cache);
        self.our_mac = Some(our_mac);
        self.our_ip = Some(our_ip);
        self
    }

    fn poll_rx(&mut self) {
        // First flush any pending TX packets
        self.flush_tx();

        // Poll from network only when rx_batch is empty (drain-then-refill pattern).
        // This minimizes DPDK API calls and improves cache locality.
        if self.rx_batch.is_empty() {
            self.rxq.rx(&mut self.rx_batch);

            // If we have a shared ARP cache, process received packets
            if let Some(ref cache) = self.shared_arp_cache {
                // Queue 0: scan for ARP replies and update shared cache
                if self.queue_id == 0 {
                    for mbuf in &self.rx_batch {
                        if let Some((ip, mac)) = parse_arp_reply(mbuf.data()) {
                            cache.insert(ip, mac);
                        }
                    }
                }
            }
        }
    }

    /// Check shared ARP cache and inject any new entries into our rx path.
    ///
    /// This allows other queues to learn MACs that queue 0 discovered.
    /// Optimization: use version counter to detect any changes (including updates).
    /// Queue 0 skips injection - it receives ARP replies directly from network.
    ///
    /// Called from receive() after poll_rx to ensure injected packets get
    /// high priority processing (pushed to back, popped first with FIFO).
    #[inline(always)]
    fn inject_from_shared_cache(&mut self) {
        use super::arp_cache::build_arp_reply_for_injection;

        // Queue 0 receives ARP replies directly, no injection needed
        if self.queue_id == 0 {
            return;
        }

        let (Some(cache), Some(our_mac), Some(our_ip)) =
            (&self.shared_arp_cache, self.our_mac, self.our_ip)
        else {
            return;
        };

        // Fast path: skip if cache version unchanged
        // Version increments on any insert/update, so this catches MAC refreshes too
        let current_version = cache.version();
        if current_version == self.last_cache_version {
            return;
        }

        // Load the current cache snapshot (lock-free)
        let cache_snapshot = cache.snapshot();

        // Inject all entries (we only get here when there are new/updated ones)
        // Re-injecting already-known entries is harmless - smoltcp deduplicates
        for (&ip, &mac) in cache_snapshot.iter() {
            let arp_packet = build_arp_reply_for_injection(our_mac, our_ip, mac, ip);

            if self.rx_batch.len() < self.rx_batch.capacity()
                && let Some(mut mbuf) = self.mempool.try_alloc()
                && mbuf.copy_from_slice(&arp_packet)
            {
                self.rx_batch.push(mbuf);
            } else {
                // Injection failed (batch full, alloc failed, or copy failed).
                // Return without updating cache version so we retry next iteration.
                tracing::warn!("Failed to inject ARP entry for {}, will retry", ip);
                return;
            }
        }

        self.last_cache_version = current_version;
    }

    /// Flush pending TX packets to the hardware.
    ///
    /// This tries to send packets from tx_batch but doesn't spin if the TX ring is full.
    /// Remaining packets stay in tx_batch and will be retried on next call.
    pub(crate) fn flush_tx(&mut self) {
        if !self.tx_batch.is_empty() {
            self.txq.tx(&mut self.tx_batch);
        }
    }

    /// Inject a packet into the receive path.
    ///
    /// This is useful for pre-populating the ARP cache by injecting
    /// a fake ARP reply before the device starts processing real traffic.
    ///
    /// # Arguments
    /// * `data` - The raw Ethernet frame to inject
    ///
    /// # Returns
    /// `true` if the packet was injected successfully, `false` if there's no space
    pub fn inject_rx_packet(&mut self, data: &[u8]) -> bool {
        if self.rx_batch.len() >= self.rx_batch.capacity() {
            return false;
        }

        // Allocate mbuf from mempool and copy data
        if let Some(mut mbuf) = self.mempool.try_alloc()
            && mbuf.copy_from_slice(data)
        {
            self.rx_batch.push(mbuf);
            return true;
        }
        false
    }
}

impl Device for DpdkDevice {
    type RxToken<'a>
        = DpdkRxToken
    where
        Self: 'a;
    type TxToken<'a>
        = DpdkTxTokenWithPool<'a>
    where
        Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.poll_rx();

        // Inject ARP entries after poll_rx (which may have reversed the batch).
        // This ensures injected ARPs are at the back, processed first by pop() = high priority.
        // Critical for queue 1+ to resolve gateway MAC quickly for SYN-ACKs.
        self.inject_from_shared_cache();

        if let Some(mbuf) = self.rx_batch.pop() {
            let rx_token = DpdkRxToken { mbuf };
            let tx_token = DpdkTxTokenWithPool {
                mempool: &self.mempool,
                tx_batch: &mut self.tx_batch,
            };
            Some((rx_token, tx_token))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.tx_batch.len() < self.tx_batch.capacity() {
            Some(DpdkTxTokenWithPool {
                mempool: &self.mempool,
                tx_batch: &mut self.tx_batch,
            })
        } else {
            // TX batch is full - try to flush to hardware.
            // With a 256-packet batch and 1024-descriptor TX ring, this should
            // rarely fail unless under extreme load.
            self.flush_tx();
            if self.tx_batch.len() < self.tx_batch.capacity() {
                Some(DpdkTxTokenWithPool {
                    mempool: &self.mempool,
                    tx_batch: &mut self.tx_batch,
                })
            } else {
                // Hardware TX ring is full - caller will have to wait
                None
            }
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = self.mtu;
        caps.medium = Medium::Ethernet;
        caps
    }
}

pub struct DpdkTxTokenWithPool<'a> {
    mempool: &'a MemPool,
    tx_batch: &'a mut ArrayVec<Mbuf, 256>,
}

impl<'a> phy::TxToken for DpdkTxTokenWithPool<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // Allocate mbuf from mempool
        if let Some(mut mbuf) = self.mempool.try_alloc() {
            unsafe {
                mbuf.extend(len);
            }

            // Let smoltcp write directly to the mbuf
            let result = f(mbuf.data_mut());

            // Add to tx batch (will be flushed later)
            // Safety: transmit() only returns a token when tx_batch has space
            self.tx_batch
                .try_push(mbuf)
                .expect("tx_batch should have space (checked in transmit())");

            result
        } else {
            // Fallback if allocation fails - packet data is lost
            let mut buffer = vec![0u8; len];
            f(&mut buffer)
        }
    }
}
