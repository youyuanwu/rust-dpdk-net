use arrayvec::ArrayVec;
use smoltcp::phy::{self, Device, DeviceCapabilities, Medium};
use smoltcp::time::Instant;

use crate::api::rte::mbuf::Mbuf;
use crate::api::rte::pktmbuf::MemPool;
use crate::api::rte::queue::{RxQueue, TxQueue};

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
pub struct DpdkDeviceWithPool {
    rxq: RxQueue,
    txq: TxQueue,
    mempool: MemPool,
    rx_batch: ArrayVec<Mbuf, 64>,
    tx_batch: ArrayVec<Mbuf, 64>,
    mtu: usize,
    #[allow(dead_code)] // Validated in constructor, stored for debugging/future use
    mbuf_capacity: usize,
}

impl DpdkDeviceWithPool {
    /// Create a new DPDK device for smoltcp.
    ///
    /// # Arguments
    /// * `rxq` - DPDK receive queue
    /// * `txq` - DPDK transmit queue  
    /// * `mempool` - Memory pool for mbuf allocation
    /// * `mtu` - Maximum transmission unit (payload size, typically 1500)
    /// * `mbuf_capacity` - Usable capacity of mbufs (data_room_size - headroom)
    ///
    /// # Panics
    /// Panics if MTU + maximum packet overhead exceeds mbuf capacity.
    pub fn new(
        rxq: RxQueue,
        txq: TxQueue,
        mempool: MemPool,
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
        }
    }

    fn poll_rx(&mut self) {
        // First flush any pending TX packets
        self.flush_tx();

        // Then poll from network if rx_batch has space
        if self.rx_batch.is_empty() {
            self.rxq.rx(&mut self.rx_batch);
        }
    }

    fn flush_tx(&mut self) {
        // Normal mode - send to network
        while !self.tx_batch.is_empty() {
            self.txq.tx(&mut self.tx_batch);
        }
    }
}

impl Device for DpdkDeviceWithPool {
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
            self.flush_tx();
            if self.tx_batch.len() < self.tx_batch.capacity() {
                Some(DpdkTxTokenWithPool {
                    mempool: &self.mempool,
                    tx_batch: &mut self.tx_batch,
                })
            } else {
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
    tx_batch: &'a mut ArrayVec<Mbuf, 64>,
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
