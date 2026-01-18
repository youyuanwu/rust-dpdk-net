use arrayvec::ArrayVec;
use dpdk_net::api::rte::mbuf::Mbuf;
use dpdk_net::api::rte::pktmbuf::MemPool;
use dpdk_net::api::rte::queue::{RxQueue, TxQueue};
use smoltcp::wire;
use std::net::Ipv4Addr;

/// Error returned by socket operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketError {
    /// Socket is not bound to a port
    NotBound,
    /// Invalid endpoint (zero port or unspecified address)
    InvalidEndpoint,
    /// Buffer is full, cannot send more packets
    BufferFull,
    /// No packets available to receive
    NoPackets,
    /// RX/TX queue error
    QueueError,
}

/// UDP endpoint (IP address + port)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub addr: Ipv4Addr,
    pub port: u16,
}

impl Endpoint {
    pub fn new(addr: Ipv4Addr, port: u16) -> Self {
        Self { addr, port }
    }

    pub fn is_specified(&self) -> bool {
        !self.addr.is_unspecified() && self.port != 0
    }
}

/// A DPDK-based UDP socket
///
/// This socket works directly with DPDK mbufs, avoiding copies.
/// It maintains separate RX and TX queues and uses a mempool for allocation.
pub struct UdpSocket {
    /// Local endpoint this socket is bound to
    local_endpoint: Option<Endpoint>,
    /// Local MAC address
    local_mac: [u8; 6],
    /// RX queue for receiving packets
    rxq: Option<RxQueue>,
    /// TX queue for sending packets
    txq: Option<TxQueue>,
    /// Mempool for allocating mbufs
    mempool: Option<MemPool>,
    /// TX batch buffer
    tx_batch: ArrayVec<Mbuf, 64>,
    /// RX batch buffer
    rx_batch: ArrayVec<Mbuf, 64>,
}

impl Default for UdpSocket {
    fn default() -> Self {
        Self::new()
    }
}

impl UdpSocket {
    /// Create a new UDP socket
    pub fn new() -> Self {
        Self {
            local_endpoint: None,
            local_mac: [0; 6],
            rxq: None,
            txq: None,
            mempool: None,
            tx_batch: ArrayVec::new(),
            rx_batch: ArrayVec::new(),
        }
    }

    /// Bind the socket to a local endpoint
    pub fn bind(&mut self, endpoint: Endpoint) -> Result<(), SocketError> {
        if endpoint.port == 0 {
            return Err(SocketError::InvalidEndpoint);
        }
        self.local_endpoint = Some(endpoint);
        Ok(())
    }

    /// Set the local MAC address for outgoing packets
    pub fn set_local_mac(&mut self, mac: [u8; 6]) {
        self.local_mac = mac;
    }

    /// Attach RX and TX queues and mempool to this socket
    pub fn attach_queues(
        &mut self,
        rxq: RxQueue,
        txq: TxQueue,
        mempool: MemPool,
    ) -> Result<(), SocketError> {
        self.rxq = Some(rxq);
        self.txq = Some(txq);
        self.mempool = Some(mempool);
        Ok(())
    }

    /// Check if the socket is bound
    pub fn is_bound(&self) -> bool {
        self.local_endpoint.is_some()
    }

    /// Get the local endpoint
    pub fn local_endpoint(&self) -> Option<Endpoint> {
        self.local_endpoint
    }

    /// Send a UDP packet to the specified remote endpoint
    ///
    /// This allocates an mbuf, constructs the packet headers, and queues it for transmission.
    /// The actual transmission happens when `flush()` is called.
    pub fn send_to(
        &mut self,
        data: &[u8],
        remote_endpoint: Endpoint,
        remote_mac: [u8; 6],
    ) -> Result<(), SocketError> {
        let local_endpoint = self.local_endpoint.ok_or(SocketError::NotBound)?;

        if !remote_endpoint.is_specified() {
            return Err(SocketError::InvalidEndpoint);
        }

        let mempool = self.mempool.as_ref().ok_or(SocketError::QueueError)?;

        // Allocate mbuf
        let mut mbuf = mempool.try_alloc().ok_or(SocketError::BufferFull)?;

        // Calculate total packet size
        let eth_header_len = 14;
        let ip_header_len = 20;
        let udp_header_len = 8;
        let total_len = eth_header_len + ip_header_len + udp_header_len + data.len();

        // Extend mbuf to fit all headers and payload
        unsafe {
            mbuf.extend(total_len);
        }

        // Build Ethernet header
        let mut eth_frame = wire::EthernetFrame::new_unchecked(mbuf.data_mut());
        eth_frame.set_dst_addr(wire::EthernetAddress(remote_mac));
        eth_frame.set_src_addr(wire::EthernetAddress(self.local_mac));
        eth_frame.set_ethertype(wire::EthernetProtocol::Ipv4);

        // Build IPv4 header
        let mut ipv4_pkt = wire::Ipv4Packet::new_unchecked(eth_frame.payload_mut());
        ipv4_pkt.set_version(4);
        ipv4_pkt.set_header_len(5); // 20 bytes
        ipv4_pkt.set_dscp(0);
        ipv4_pkt.set_ecn(0);
        ipv4_pkt.set_total_len((ip_header_len + udp_header_len + data.len()) as u16);
        ipv4_pkt.set_ident(0);
        ipv4_pkt.clear_flags();
        ipv4_pkt.set_frag_offset(0);
        ipv4_pkt.set_hop_limit(64);
        ipv4_pkt.set_next_header(wire::IpProtocol::Udp);
        ipv4_pkt.set_src_addr(local_endpoint.addr);
        ipv4_pkt.set_dst_addr(remote_endpoint.addr);
        ipv4_pkt.set_checksum(0);
        ipv4_pkt.fill_checksum();

        // Build UDP header
        let mut udp_pkt = wire::UdpPacket::new_unchecked(ipv4_pkt.payload_mut());
        udp_pkt.set_src_port(local_endpoint.port);
        udp_pkt.set_dst_port(remote_endpoint.port);
        udp_pkt.set_len((udp_header_len + data.len()) as u16);
        udp_pkt.set_checksum(0); // Optional for IPv4

        // Copy payload
        udp_pkt.payload_mut()[..data.len()].copy_from_slice(data);

        // Add to TX batch
        if self.tx_batch.try_push(mbuf).is_err() {
            return Err(SocketError::BufferFull);
        }

        Ok(())
    }

    /// Flush pending TX packets
    ///
    /// This sends all queued packets on the TX queue.
    /// Returns the number of packets successfully sent.
    pub fn flush(&mut self) -> Result<usize, SocketError> {
        if self.tx_batch.is_empty() {
            return Ok(0);
        }

        let txq = self.txq.as_ref().ok_or(SocketError::QueueError)?;
        let mut sent = 0;

        while !self.tx_batch.is_empty() {
            let count = txq.tx(&mut self.tx_batch);
            sent += count;
            // tx() removes successfully sent packets from the batch
        }

        Ok(sent)
    }

    /// Receive UDP packets
    ///
    /// This fills the internal RX batch by receiving from the RX queue.
    /// Returns the number of packets received.
    pub fn poll(&mut self) -> Result<usize, SocketError> {
        let rxq = self.rxq.as_ref().ok_or(SocketError::QueueError)?;

        // Receive packets into RX batch
        let received = rxq.rx(&mut self.rx_batch);
        Ok(received)
    }

    /// Try to receive a UDP packet
    ///
    /// Returns the payload data, remote endpoint, and consumes the mbuf.
    /// You should call `poll()` first to fill the RX buffer.
    pub fn recv_from(&mut self) -> Result<(Mbuf, Endpoint), SocketError> {
        let local_endpoint = self.local_endpoint.ok_or(SocketError::NotBound)?;

        // Pop a packet from the RX batch
        let mbuf = self.rx_batch.pop().ok_or(SocketError::NoPackets)?;

        // Parse packet headers
        let eth_frame =
            wire::EthernetFrame::new_checked(mbuf.data()).map_err(|_| SocketError::QueueError)?;

        // Check if it's IPv4
        if eth_frame.ethertype() != wire::EthernetProtocol::Ipv4 {
            return Err(SocketError::QueueError);
        }

        let ipv4_pkt = wire::Ipv4Packet::new_checked(eth_frame.payload())
            .map_err(|_| SocketError::QueueError)?;

        // Check if it's UDP
        if ipv4_pkt.next_header() != wire::IpProtocol::Udp {
            return Err(SocketError::QueueError);
        }

        let udp_pkt = wire::UdpPacket::new_checked(ipv4_pkt.payload())
            .map_err(|_| SocketError::QueueError)?;

        // Check if it's for our port
        if udp_pkt.dst_port() != local_endpoint.port {
            return Err(SocketError::QueueError);
        }

        let remote_endpoint = Endpoint::new(ipv4_pkt.src_addr(), udp_pkt.src_port());

        Ok((mbuf, remote_endpoint))
    }

    /// Get the number of packets pending in RX batch
    pub fn rx_pending(&self) -> usize {
        self.rx_batch.len()
    }

    /// Get the number of packets pending in TX batch
    pub fn tx_pending(&self) -> usize {
        self.tx_batch.len()
    }

    /// Check if we can send more packets
    pub fn can_send(&self) -> bool {
        self.tx_batch.len() < 64
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        // Flush any pending TX packets
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint() {
        let ep = Endpoint::new(Ipv4Addr::new(192, 168, 1, 1), 8080);
        assert!(ep.is_specified());

        let unspec = Endpoint::new(Ipv4Addr::UNSPECIFIED, 8080);
        assert!(!unspec.is_specified());

        let no_port = Endpoint::new(Ipv4Addr::new(192, 168, 1, 1), 0);
        assert!(!no_port.is_specified());
    }

    #[test]
    fn test_socket_bind() {
        let mut socket = UdpSocket::new();
        assert!(!socket.is_bound());

        let ep = Endpoint::new(Ipv4Addr::new(192, 168, 1, 1), 8080);
        assert!(socket.bind(ep).is_ok());
        assert!(socket.is_bound());
        assert_eq!(socket.local_endpoint(), Some(ep));

        // Can't bind to port 0
        let invalid = Endpoint::new(Ipv4Addr::new(192, 168, 1, 1), 0);
        let mut socket2 = UdpSocket::new();
        assert_eq!(socket2.bind(invalid), Err(SocketError::InvalidEndpoint));
    }
}
