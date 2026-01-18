//! Async TCP socket implementation

use super::super::DpdkDeviceWithPool;
use super::{ReactorHandle, ReactorInner};
use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::{self, ConnectError, ListenError, RecvError, SendError, State};
use smoltcp::wire::IpAddress;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

/// A TCP stream between a local and a remote socket.
///
/// Similar to `std::net::TcpStream`, this represents a connected TCP socket
/// that can send and receive data asynchronously.
///
/// A `TcpStream` is created by either connecting to a remote endpoint via
/// [`TcpStream::connect`], or by accepting a connection from a [`TcpListener`].
pub struct TcpStream {
    pub(crate) handle: SocketHandle,
    pub(crate) reactor: Rc<RefCell<ReactorInner<DpdkDeviceWithPool>>>,
}

impl TcpStream {
    /// Opens a TCP connection to a remote host.
    ///
    /// Returns an error if the connection cannot be initiated (e.g., invalid
    /// state, unspecified local/remote addresses, or port already in use).
    pub fn connect(
        handle: &ReactorHandle,
        remote_addr: IpAddress,
        remote_port: u16,
        local_port: u16,
        rx_buffer_size: usize,
        tx_buffer_size: usize,
    ) -> Result<Self, ConnectError> {
        let mut inner = handle.inner.borrow_mut();

        let rx_buffer = tcp::SocketBuffer::new(vec![0; rx_buffer_size]);
        let tx_buffer = tcp::SocketBuffer::new(vec![0; tx_buffer_size]);
        let mut socket = tcp::Socket::new(rx_buffer, tx_buffer);

        // Connect before adding to socket set
        socket.connect(
            inner.iface.context(),
            (remote_addr, remote_port),
            local_port,
        )?;

        let socket_handle = inner.sockets.add(socket);

        Ok(TcpStream {
            handle: socket_handle,
            reactor: handle.inner.clone(),
        })
    }

    /// Create a TcpStream from an already-connected socket handle.
    ///
    /// This is used internally by TcpListener::accept().
    pub(crate) fn from_handle(
        handle: SocketHandle,
        reactor: Rc<RefCell<ReactorInner<DpdkDeviceWithPool>>>,
    ) -> Self {
        TcpStream { handle, reactor }
    }

    /// Get the underlying socket handle
    pub fn socket_handle(&self) -> SocketHandle {
        self.handle
    }

    /// Check if the stream is connected (in Established state)
    pub fn is_connected(&self) -> bool {
        let inner = self.reactor.borrow();
        let socket = inner.sockets.get::<tcp::Socket>(self.handle);
        socket.state() == State::Established
    }

    /// Check if the stream is active (exchanging data)
    pub fn is_active(&self) -> bool {
        let inner = self.reactor.borrow();
        let socket = inner.sockets.get::<tcp::Socket>(self.handle);
        socket.is_active()
    }

    /// Get the current socket state
    pub fn state(&self) -> State {
        let inner = self.reactor.borrow();
        let socket = inner.sockets.get::<tcp::Socket>(self.handle);
        socket.state()
    }

    /// Send data asynchronously
    ///
    /// Returns the number of bytes sent when the operation completes.
    pub fn send<'a>(&'a self, data: &'a [u8]) -> TcpSendFuture<'a> {
        TcpSendFuture {
            socket: self,
            data,
            offset: 0,
        }
    }

    /// Receive data asynchronously
    ///
    /// Returns the number of bytes received when the operation completes.
    /// Returns 0 if the connection was closed gracefully.
    pub fn recv<'a>(&'a self, buf: &'a mut [u8]) -> TcpRecvFuture<'a> {
        TcpRecvFuture { socket: self, buf }
    }

    /// Wait for the connection to be fully established
    ///
    /// This is useful after `connect()` to wait for the TCP handshake to complete.
    pub fn wait_connected(&self) -> WaitConnectedFuture<'_> {
        WaitConnectedFuture { socket: self }
    }

    /// Close the stream gracefully and wait for shutdown to complete
    ///
    /// This initiates a graceful shutdown (FIN) and returns a future that
    /// completes when the connection is fully closed. The socket remains
    /// in the socket set until the future completes, allowing the TCP
    /// state machine to process the FIN handshake.
    pub fn close(&self) -> CloseFuture<'_> {
        {
            let mut inner = self.reactor.borrow_mut();
            let socket = inner.sockets.get_mut::<tcp::Socket>(self.handle);
            socket.close();
        }
        CloseFuture { socket: self }
    }

    /// Abort the connection immediately
    ///
    /// This sends a RST and terminates the connection.
    pub fn abort(&self) {
        let mut inner = self.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<tcp::Socket>(self.handle);
        socket.abort();
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        let mut inner = self.reactor.borrow_mut();

        // Abort only if the socket is still active and not in a closing state.
        // If close() was called, the socket will be in FinWait1/FinWait2/Closing/TimeWait,
        // and we should let the graceful shutdown complete.
        let socket = inner.sockets.get_mut::<tcp::Socket>(self.handle);
        match socket.state() {
            // Already closed or in graceful shutdown - don't abort
            State::Closed
            | State::FinWait1
            | State::FinWait2
            | State::Closing
            | State::TimeWait
            | State::LastAck => {}
            // Still active - abort to notify peer
            _ => socket.abort(),
        }

        // Remove from socket set
        inner.sockets.remove(self.handle);
    }
}

/// A TCP socket server, listening for connections.
///
/// Similar to `std::net::TcpListener`, this listens for incoming TCP connections.
/// Use [`TcpListener::accept`] to accept new connections.
///
/// Internally maintains multiple listening sockets (based on backlog) to handle
/// concurrent connection attempts. This ensures there's always at least one socket
/// ready to receive incoming SYN packets.
pub struct TcpListener {
    /// Pool of sockets for handling concurrent connections
    handles: Vec<SocketHandle>,
    reactor: Rc<RefCell<ReactorInner<DpdkDeviceWithPool>>>,
    port: u16,
    rx_buffer_size: usize,
    tx_buffer_size: usize,
}

impl TcpListener {
    /// Creates a new TcpListener bound to the specified port with default backlog of 2.
    ///
    /// Similar to `std::net::TcpListener::bind()`.
    pub fn bind(
        handle: &ReactorHandle,
        port: u16,
        rx_buffer_size: usize,
        tx_buffer_size: usize,
    ) -> Result<Self, ListenError> {
        Self::bind_with_backlog(handle, port, rx_buffer_size, tx_buffer_size, 2)
    }

    /// Creates a new TcpListener with a specified backlog size.
    ///
    /// The backlog determines how many simultaneous connection attempts can be
    /// handled before `accept()` is called. For a single-threaded server, set
    /// this to the maximum expected burst of concurrent connections.
    pub fn bind_with_backlog(
        handle: &ReactorHandle,
        port: u16,
        rx_buffer_size: usize,
        tx_buffer_size: usize,
        backlog: usize,
    ) -> Result<Self, ListenError> {
        let backlog = backlog.max(1); // At least 1 socket
        let mut inner = handle.inner.borrow_mut();
        let mut handles = Vec::with_capacity(backlog);

        for _ in 0..backlog {
            let h =
                Self::create_listening_socket(&mut inner, port, rx_buffer_size, tx_buffer_size)?;
            handles.push(h);
        }

        Ok(TcpListener {
            handles,
            reactor: handle.inner.clone(),
            port,
            rx_buffer_size,
            tx_buffer_size,
        })
    }

    /// Create a new listening socket and add it to the reactor
    fn create_listening_socket(
        inner: &mut ReactorInner<DpdkDeviceWithPool>,
        port: u16,
        rx_buffer_size: usize,
        tx_buffer_size: usize,
    ) -> Result<SocketHandle, ListenError> {
        let rx_buffer = tcp::SocketBuffer::new(vec![0; rx_buffer_size]);
        let tx_buffer = tcp::SocketBuffer::new(vec![0; tx_buffer_size]);
        let mut socket = tcp::Socket::new(rx_buffer, tx_buffer);
        socket.listen(port)?;
        let handle = inner.sockets.add(socket);
        Ok(handle)
    }

    /// Get the port this listener is bound to
    pub fn local_port(&self) -> u16 {
        self.port
    }

    /// Accept a new incoming connection.
    ///
    /// This waits for a client to connect and returns a `TcpStream` for the
    /// accepted connection. The listener remains valid and can accept more
    /// connections, similar to `std::net::TcpListener::accept()`.
    pub fn accept(&mut self) -> AcceptFuture<'_> {
        AcceptFuture { listener: self }
    }

    /// Check if a connection is pending (ready to be accepted)
    pub fn is_pending(&self) -> bool {
        let inner = self.reactor.borrow();
        self.handles.iter().any(|&h| {
            let socket = inner.sockets.get::<tcp::Socket>(h);
            matches!(socket.state(), State::SynReceived | State::Established)
        })
    }

    /// Get the states of the internal sockets (for debugging)
    pub fn states(&self) -> Vec<State> {
        let inner = self.reactor.borrow();
        self.handles
            .iter()
            .map(|&h| inner.sockets.get::<tcp::Socket>(h).state())
            .collect()
    }

    /// Get the backlog size (number of listening sockets)
    pub fn backlog(&self) -> usize {
        self.handles.len()
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        let mut inner = self.reactor.borrow_mut();

        // Close all listening sockets
        for &handle in &self.handles {
            let socket = inner.sockets.get_mut::<tcp::Socket>(handle);
            if socket.state() != State::Closed {
                socket.abort();
            }
            inner.sockets.remove(handle);
        }
    }
}

/// Future for accepting a connection on a TcpListener
///
/// When a connection is established, this future:
/// 1. Finds a socket that has reached Established state
/// 2. Takes it and wraps it in a `TcpStream`
/// 3. Creates a new listening socket to replace it
/// 4. Returns the `TcpStream`, leaving the listener ready for more connections
pub struct AcceptFuture<'a> {
    listener: &'a mut TcpListener,
}

impl<'a> Future for AcceptFuture<'a> {
    type Output = Result<TcpStream, ListenError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Find a socket that has an established connection
        let established_idx = {
            let inner = this.listener.reactor.borrow();

            // Find first established socket
            this.listener
                .handles
                .iter()
                .enumerate()
                .find_map(|(i, &h)| {
                    let socket = inner.sockets.get::<tcp::Socket>(h);
                    if socket.state() == State::Established {
                        Some(i)
                    } else {
                        None
                    }
                })
        };

        match established_idx {
            Some(idx) => {
                // Found an established connection
                let mut inner = this.listener.reactor.borrow_mut();

                // Get the connected socket handle
                let connected_handle = this.listener.handles[idx];

                // Create a new listening socket to replace it
                let new_handle = TcpListener::create_listening_socket(
                    &mut inner,
                    this.listener.port,
                    this.listener.rx_buffer_size,
                    this.listener.tx_buffer_size,
                )?;

                // Replace the connected handle with the new listening one
                this.listener.handles[idx] = new_handle;

                drop(inner);

                // Create a TcpStream from the connected socket
                let stream =
                    TcpStream::from_handle(connected_handle, this.listener.reactor.clone());

                Poll::Ready(Ok(stream))
            }
            None => {
                // No established connection yet
                let mut inner = this.listener.reactor.borrow_mut();

                // Check if all sockets are dead
                let all_dead = this.listener.handles.iter().all(|&h| {
                    let socket = inner.sockets.get::<tcp::Socket>(h);
                    matches!(socket.state(), State::Closed | State::TimeWait)
                });

                if all_dead {
                    return Poll::Ready(Err(ListenError::Unaddressable));
                }

                // Register wakers on all listening sockets and wait
                for &handle in &this.listener.handles {
                    let socket = inner.sockets.get_mut::<tcp::Socket>(handle);
                    socket.register_send_waker(cx.waker());
                }

                Poll::Pending
            }
        }
    }
}

/// Future for sending data on a TCP stream
pub struct TcpSendFuture<'a> {
    socket: &'a TcpStream,
    data: &'a [u8],
    offset: usize,
}

impl<'a> Future for TcpSendFuture<'a> {
    type Output = Result<usize, SendError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.socket.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<tcp::Socket>(self.socket.handle);

        // Try to send remaining data
        let remaining = &self.data[self.offset..];
        match socket.send_slice(remaining) {
            // No space in send buffer - register waker and wait
            Ok(0) => {
                socket.register_send_waker(cx.waker());
                Poll::Pending
            }
            // Some data sent
            Ok(sent) => {
                self.offset += sent;
                if self.offset >= self.data.len() {
                    // All data sent
                    Poll::Ready(Ok(self.data.len()))
                } else {
                    // More data to send - register waker for next poll
                    socket.register_send_waker(cx.waker());
                    Poll::Pending
                }
            }
            // Connection reset or invalid state
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

/// Future for receiving data from a TCP stream
pub struct TcpRecvFuture<'a> {
    socket: &'a TcpStream,
    buf: &'a mut [u8],
}

impl<'a> Future for TcpRecvFuture<'a> {
    type Output = Result<usize, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut inner = this.socket.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<tcp::Socket>(this.socket.handle);

        // Try to receive data directly
        match socket.recv_slice(this.buf) {
            // No data available - register waker and wait
            Ok(0) if this.buf.is_empty() => {
                // Empty buffer - return immediately per async Read contract
                Poll::Ready(Ok(0))
            }
            Ok(0) => {
                // No data ready yet
                socket.register_recv_waker(cx.waker());
                Poll::Pending
            }
            // Data received
            Ok(len) => Poll::Ready(Ok(len)),
            // EOF - connection closed gracefully
            Err(RecvError::Finished) => Poll::Ready(Ok(0)),
            // Connection reset or invalid state
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

/// Future for waiting until a stream is connected
pub struct WaitConnectedFuture<'a> {
    socket: &'a TcpStream,
}

impl<'a> Future for WaitConnectedFuture<'a> {
    type Output = Result<(), ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.socket.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<tcp::Socket>(self.socket.handle);

        // Check connection state
        match socket.state() {
            // Connected!
            State::Established => Poll::Ready(Ok(())),
            // Connection failed
            State::Closed | State::TimeWait => Poll::Ready(Err(())),
            // Still connecting - register waker and wait
            State::SynSent | State::SynReceived => {
                socket.register_send_waker(cx.waker());
                Poll::Pending
            }
            // Other states - keep waiting
            _ => {
                socket.register_send_waker(cx.waker());
                Poll::Pending
            }
        }
    }
}
/// Future for waiting until a stream is closed
pub struct CloseFuture<'a> {
    socket: &'a TcpStream,
}

impl<'a> Future for CloseFuture<'a> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.socket.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<tcp::Socket>(self.socket.handle);

        // Check if we've reached a terminal state
        match socket.state() {
            // Fully closed - done!
            State::Closed | State::TimeWait => Poll::Ready(()),
            // Still closing - register waker and wait
            _ => {
                socket.register_send_waker(cx.waker());
                Poll::Pending
            }
        }
    }
}
