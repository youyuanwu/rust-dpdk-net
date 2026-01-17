use arrayvec::ArrayVec;
use rpkt_dpdk::*;
use smoltcp::phy::{self, Device, DeviceCapabilities, Medium};
use smoltcp::time::Instant;

/// A smoltcp Device implementation backed by DPDK rx/tx queues
pub struct DpdkDevice {
    rxq: RxQueue,
    txq: TxQueue,
    rx_batch: ArrayVec<Mbuf, 64>,
    tx_batch: ArrayVec<Mbuf, 64>,
    mtu: usize,
}

impl DpdkDevice {
    pub fn new(rxq: RxQueue, txq: TxQueue, mtu: usize) -> Self {
        Self {
            rxq,
            txq,
            rx_batch: ArrayVec::new(),
            tx_batch: ArrayVec::new(),
            mtu,
        }
    }

    /// Receive packets from DPDK into internal buffer
    fn poll_rx(&mut self) {
        if self.rx_batch.is_empty() {
            self.rxq.rx(&mut self.rx_batch);
        }
    }

    /// Flush pending tx packets to DPDK
    fn flush_tx(&mut self) {
        while !self.tx_batch.is_empty() {
            self.txq.tx(&mut self.tx_batch);
        }
    }
}

impl Device for DpdkDevice {
    type RxToken<'a>
        = DpdkRxToken
    where
        Self: 'a;
    type TxToken<'a>
        = DpdkTxToken<'a>
    where
        Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.poll_rx();

        if let Some(mbuf) = self.rx_batch.pop() {
            let rx_token = DpdkRxToken { mbuf };
            let tx_token = DpdkTxToken {
                tx_batch: &mut self.tx_batch,
                txq: &mut self.txq,
            };
            Some((rx_token, tx_token))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.tx_batch.len() < self.tx_batch.capacity() {
            Some(DpdkTxToken {
                tx_batch: &mut self.tx_batch,
                txq: &mut self.txq,
            })
        } else {
            // Batch is full, flush first
            self.flush_tx();
            if self.tx_batch.len() < self.tx_batch.capacity() {
                Some(DpdkTxToken {
                    tx_batch: &mut self.tx_batch,
                    txq: &mut self.txq,
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

pub struct DpdkTxToken<'a> {
    tx_batch: &'a mut ArrayVec<Mbuf, 64>,
    txq: &'a mut TxQueue,
}

impl<'a> phy::TxToken for DpdkTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // Need to allocate an mbuf for transmission
        // This is a limitation - we'd need access to a mempool here
        // In a real implementation, you'd pass the mempool to DpdkDevice

        // For now, use a stack buffer as a workaround
        let mut buffer = vec![0u8; len];
        let result = f(&mut buffer);

        // TODO: Allocate mbuf, copy buffer data to mbuf, add to tx_batch
        // This is where the copy overhead happens

        // Flush immediately to make progress
        while !self.tx_batch.is_empty() {
            self.txq.tx(self.tx_batch);
        }

        result
    }
}

/// More complete implementation with mempool access
pub struct DpdkDeviceWithPool {
    rxq: RxQueue,
    txq: TxQueue,
    mempool: Mempool,
    rx_batch: ArrayVec<Mbuf, 64>,
    tx_batch: ArrayVec<Mbuf, 64>,
    mtu: usize,
}

impl DpdkDeviceWithPool {
    pub fn new(rxq: RxQueue, txq: TxQueue, mempool: Mempool, mtu: usize) -> Self {
        Self {
            rxq,
            txq,
            mempool,
            rx_batch: ArrayVec::new(),
            tx_batch: ArrayVec::new(),
            mtu,
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
    mempool: &'a Mempool,
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
            let _ = self.tx_batch.try_push(mbuf);

            result
        } else {
            // Fallback if allocation fails
            let mut buffer = vec![0u8; len];
            f(&mut buffer)
        }
    }
}
