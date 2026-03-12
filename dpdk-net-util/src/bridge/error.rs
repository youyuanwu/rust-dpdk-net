use std::fmt;
use std::io;

/// Error type for bridge operations.
#[derive(Debug)]
pub enum BridgeError {
    /// DPDK lcore shut down or channel closed.
    Disconnected,
    /// TCP connection failed (RST, timeout, etc.).
    ConnectionFailed,
    /// IO error from the underlying stream.
    Io(io::Error),
    /// TCP connect error from smoltcp.
    Connect(smoltcp::socket::tcp::ConnectError),
    /// TCP listen error from smoltcp.
    Listen(smoltcp::socket::tcp::ListenError),
    /// UDP bind error from smoltcp.
    UdpBind(smoltcp::socket::udp::BindError),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeError::Disconnected => write!(f, "DPDK lcore disconnected"),
            BridgeError::ConnectionFailed => write!(f, "TCP connection failed"),
            BridgeError::Io(e) => write!(f, "IO error: {e}"),
            BridgeError::Connect(e) => write!(f, "TCP connect error: {e}"),
            BridgeError::Listen(e) => write!(f, "TCP listen error: {e}"),
            BridgeError::UdpBind(e) => write!(f, "UDP bind error: {e}"),
        }
    }
}

impl std::error::Error for BridgeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BridgeError::Io(e) => Some(e),
            BridgeError::Connect(e) => Some(e),
            BridgeError::Listen(e) => Some(e),
            BridgeError::UdpBind(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for BridgeError {
    fn from(e: io::Error) -> Self {
        BridgeError::Io(e)
    }
}

impl From<smoltcp::socket::tcp::ConnectError> for BridgeError {
    fn from(e: smoltcp::socket::tcp::ConnectError) -> Self {
        BridgeError::Connect(e)
    }
}

impl From<smoltcp::socket::tcp::ListenError> for BridgeError {
    fn from(e: smoltcp::socket::tcp::ListenError) -> Self {
        BridgeError::Listen(e)
    }
}

impl From<smoltcp::socket::udp::BindError> for BridgeError {
    fn from(e: smoltcp::socket::udp::BindError) -> Self {
        BridgeError::UdpBind(e)
    }
}

impl From<BridgeError> for io::Error {
    fn from(e: BridgeError) -> Self {
        match e {
            BridgeError::Io(e) => e,
            BridgeError::Disconnected => {
                io::Error::new(io::ErrorKind::BrokenPipe, "DPDK lcore disconnected")
            }
            BridgeError::ConnectionFailed => {
                io::Error::new(io::ErrorKind::ConnectionRefused, "TCP connection failed")
            }
            BridgeError::Connect(e) => {
                io::Error::new(io::ErrorKind::ConnectionRefused, e.to_string())
            }
            BridgeError::Listen(e) => io::Error::new(io::ErrorKind::AddrInUse, e.to_string()),
            BridgeError::UdpBind(e) => io::Error::new(io::ErrorKind::AddrInUse, e.to_string()),
        }
    }
}
