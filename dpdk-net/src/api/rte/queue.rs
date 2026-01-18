// RX/TX Queue API
// See: /usr/local/include/rte_ethdev.h

use dpdk_net_sys::ffi;

use super::eth::{PortId, QueueId};
use super::mbuf::Mbuf;

/// Maximum burst size for RX/TX operations
pub const MAX_BURST_SIZE: usize = 64;

/// RX Queue handle for receiving packets
#[derive(Debug, Clone, Copy)]
pub struct RxQueue {
    port_id: PortId,
    queue_id: QueueId,
}

impl RxQueue {
    /// Create a new RX queue handle.
    ///
    /// Note: The queue must already be set up via `EthDev::rx_queue_setup()`.
    #[inline]
    pub fn new(port_id: PortId, queue_id: QueueId) -> Self {
        Self { port_id, queue_id }
    }

    /// Get the port ID
    #[inline]
    pub fn port_id(&self) -> PortId {
        self.port_id
    }

    /// Get the queue ID
    #[inline]
    pub fn queue_id(&self) -> QueueId {
        self.queue_id
    }

    /// Receive a burst of packets into the provided buffer.
    ///
    /// Returns the number of packets received.
    /// Packets are appended to the `mbufs` vector (up to its remaining capacity).
    #[inline]
    pub fn rx<const N: usize>(&self, mbufs: &mut arrayvec::ArrayVec<Mbuf, N>) -> usize {
        let capacity = mbufs.capacity() - mbufs.len();
        if capacity == 0 {
            return 0;
        }

        let max_pkts = capacity.min(u16::MAX as usize) as u16;

        // Allocate temporary buffer for raw pointers
        let mut raw_mbufs: [*mut ffi::rte_mbuf; MAX_BURST_SIZE] =
            [std::ptr::null_mut(); MAX_BURST_SIZE];

        let nb_pkts = max_pkts.min(MAX_BURST_SIZE as u16);

        let received = unsafe {
            ffi::rust_eth_rx_burst(self.port_id, self.queue_id, raw_mbufs.as_mut_ptr(), nb_pkts)
        };

        // Convert raw pointers to Mbuf and push to the vector
        for raw_mbuf in raw_mbufs.iter().take(received as usize) {
            if let Some(mbuf) = unsafe { Mbuf::from_raw(*raw_mbuf) } {
                // Safety: ArrayVec has capacity (checked above)
                let _ = mbufs.try_push(mbuf);
            }
        }

        received as usize
    }

    /// Receive a burst of packets, returning them as a new ArrayVec.
    #[inline]
    pub fn rx_burst<const N: usize>(&self) -> arrayvec::ArrayVec<Mbuf, N> {
        let mut mbufs = arrayvec::ArrayVec::new();
        self.rx(&mut mbufs);
        mbufs
    }
}

/// TX Queue handle for transmitting packets
#[derive(Debug, Clone, Copy)]
pub struct TxQueue {
    port_id: PortId,
    queue_id: QueueId,
}

impl TxQueue {
    /// Create a new TX queue handle.
    ///
    /// Note: The queue must already be set up via `EthDev::tx_queue_setup()`.
    #[inline]
    pub fn new(port_id: PortId, queue_id: QueueId) -> Self {
        Self { port_id, queue_id }
    }

    /// Get the port ID
    #[inline]
    pub fn port_id(&self) -> PortId {
        self.port_id
    }

    /// Get the queue ID
    #[inline]
    pub fn queue_id(&self) -> QueueId {
        self.queue_id
    }

    /// Transmit a burst of packets from the provided buffer.
    ///
    /// Successfully transmitted packets are removed from the front of `mbufs`.
    /// Returns the number of packets transmitted.
    ///
    /// Note: Packets that are successfully transmitted are freed by DPDK.
    /// Packets that fail to transmit remain in the buffer (caller must handle).
    #[inline]
    pub fn tx<const N: usize>(&self, mbufs: &mut arrayvec::ArrayVec<Mbuf, N>) -> usize {
        if mbufs.is_empty() {
            return 0;
        }

        let nb_pkts = mbufs.len().min(MAX_BURST_SIZE).min(u16::MAX as usize) as u16;

        // Build array of raw pointers (without consuming the Mbufs yet)
        let mut raw_mbufs: [*mut ffi::rte_mbuf; MAX_BURST_SIZE] =
            [std::ptr::null_mut(); MAX_BURST_SIZE];

        for (i, mbuf) in mbufs.iter().take(nb_pkts as usize).enumerate() {
            raw_mbufs[i] = mbuf.as_ptr();
        }

        let sent = unsafe {
            ffi::rust_eth_tx_burst(self.port_id, self.queue_id, raw_mbufs.as_mut_ptr(), nb_pkts)
        };

        // Remove sent packets from the buffer.
        // We need to forget them since DPDK has taken ownership and will free them.
        for _ in 0..sent {
            let mbuf = mbufs.remove(0);
            // Don't drop - DPDK owns the mbuf now
            std::mem::forget(mbuf);
        }

        sent as usize
    }

    /// Transmit a single packet.
    ///
    /// Returns `true` if the packet was transmitted, `false` otherwise.
    /// On success, the mbuf is consumed by DPDK.
    /// On failure, the mbuf is returned via the Option.
    #[inline]
    pub fn tx_one(&self, mbuf: Mbuf) -> Option<Mbuf> {
        let mut raw_mbuf = mbuf.as_ptr();

        let sent = unsafe { ffi::rust_eth_tx_burst(self.port_id, self.queue_id, &mut raw_mbuf, 1) };

        if sent == 1 {
            // DPDK took ownership, don't drop
            std::mem::forget(mbuf);
            None
        } else {
            // Failed to send, return the mbuf
            Some(mbuf)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_creation() {
        let rxq = RxQueue::new(0, 1);
        assert_eq!(rxq.port_id(), 0);
        assert_eq!(rxq.queue_id(), 1);

        let txq = TxQueue::new(2, 3);
        assert_eq!(txq.port_id(), 2);
        assert_eq!(txq.queue_id(), 3);
    }
}
