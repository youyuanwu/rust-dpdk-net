//! Async UDP socket implementation

use super::super::DpdkDevice;
use super::{ReactorHandle, ReactorInner};
use smoltcp::iface::SocketHandle;
use smoltcp::socket::udp::{self, BindError, RecvError, SendError, UdpMetadata};
use smoltcp::wire::IpEndpoint;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

/// An async UDP socket.
///
/// Similar to `std::net::UdpSocket`, this represents a UDP socket that can
/// send and receive datagrams asynchronously.
///
/// Unlike TCP, UDP is connectionless. You can send to and receive from
/// any endpoint without establishing a connection first.
pub struct UdpSocket {
    handle: SocketHandle,
    reactor: Rc<RefCell<ReactorInner<DpdkDevice>>>,
}

impl UdpSocket {
    /// Creates a new UDP socket bound to the specified port.
    ///
    /// # Arguments
    /// * `handle` - The reactor handle
    /// * `port` - The local port to bind to
    /// * `rx_buffer_packets` - Number of packets the receive buffer can hold
    /// * `tx_buffer_packets` - Number of packets the transmit buffer can hold
    /// * `max_packet_size` - Maximum size of a single packet
    pub fn bind(
        handle: &ReactorHandle,
        port: u16,
        rx_buffer_packets: usize,
        tx_buffer_packets: usize,
        max_packet_size: usize,
    ) -> Result<Self, BindError> {
        let mut inner = handle.inner.borrow_mut();

        // Create packet buffers for UDP
        let rx_meta = vec![udp::PacketMetadata::EMPTY; rx_buffer_packets];
        let rx_payload = vec![0u8; rx_buffer_packets * max_packet_size];
        let tx_meta = vec![udp::PacketMetadata::EMPTY; tx_buffer_packets];
        let tx_payload = vec![0u8; tx_buffer_packets * max_packet_size];

        let rx_buffer = udp::PacketBuffer::new(rx_meta, rx_payload);
        let tx_buffer = udp::PacketBuffer::new(tx_meta, tx_payload);

        let mut socket = udp::Socket::new(rx_buffer, tx_buffer);
        socket.bind(port)?;

        let socket_handle = inner.sockets.add(socket);

        Ok(UdpSocket {
            handle: socket_handle,
            reactor: handle.inner.clone(),
        })
    }

    /// Get the underlying socket handle
    pub fn socket_handle(&self) -> SocketHandle {
        self.handle
    }

    /// Check whether the socket is open (bound to a port)
    pub fn is_open(&self) -> bool {
        let inner = self.reactor.borrow();
        let socket = inner.sockets.get::<udp::Socket>(self.handle);
        socket.is_open()
    }

    /// Get the local endpoint this socket is bound to
    pub fn endpoint(&self) -> smoltcp::wire::IpListenEndpoint {
        let inner = self.reactor.borrow();
        let socket = inner.sockets.get::<udp::Socket>(self.handle);
        socket.endpoint()
    }

    /// Send a datagram to the specified endpoint asynchronously.
    ///
    /// Returns the number of bytes sent when the operation completes.
    pub fn send_to<'a>(&'a self, data: &'a [u8], endpoint: IpEndpoint) -> UdpSendFuture<'a> {
        UdpSendFuture {
            socket: self,
            data,
            endpoint,
        }
    }

    /// Receive a datagram asynchronously.
    ///
    /// Returns the number of bytes received and the source endpoint.
    pub fn recv_from<'a>(&'a self, buf: &'a mut [u8]) -> UdpRecvFuture<'a> {
        UdpRecvFuture { socket: self, buf }
    }

    /// Close the socket.
    pub fn close(&self) {
        let mut inner = self.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<udp::Socket>(self.handle);
        socket.close();
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        let mut inner = self.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<udp::Socket>(self.handle);
        socket.close();
        inner.sockets.remove(self.handle);
    }
}

/// Future for sending UDP data
pub struct UdpSendFuture<'a> {
    socket: &'a UdpSocket,
    data: &'a [u8],
    endpoint: IpEndpoint,
}

impl Future for UdpSendFuture<'_> {
    type Output = Result<usize, SendError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.socket.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<udp::Socket>(self.socket.handle);

        match socket.send_slice(self.data, self.endpoint) {
            Ok(()) => Poll::Ready(Ok(self.data.len())),
            Err(SendError::BufferFull) => {
                // Register waker and wait
                socket.register_send_waker(cx.waker());
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

/// Future for receiving UDP data
pub struct UdpRecvFuture<'a> {
    socket: &'a UdpSocket,
    buf: &'a mut [u8],
}

impl Future for UdpRecvFuture<'_> {
    type Output = Result<(usize, UdpMetadata), RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.socket.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<udp::Socket>(self.socket.handle);

        match socket.recv_slice(self.buf) {
            Ok((len, metadata)) => Poll::Ready(Ok((len, metadata))),
            Err(RecvError::Exhausted) => {
                // No data available, register waker and wait
                socket.register_recv_waker(cx.waker());
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}
