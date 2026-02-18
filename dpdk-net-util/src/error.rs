use std::fmt;

/// Error type for dpdk-net-util operations.
#[derive(Debug)]
pub enum Error {
    /// TCP connection failed.
    Connect(smoltcp::socket::tcp::ConnectError),
    /// The TCP connection was refused or timed out.
    ConnectionFailed,
    /// HTTP handshake failed.
    Handshake(hyper::Error),
    /// Sending a request failed.
    Request(hyper::Error),
    /// Missing host in request URI.
    MissingHost,
    /// The connection is closed or not ready.
    ConnectionNotReady,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Connect(e) => write!(f, "TCP connect error: {e}"),
            Error::ConnectionFailed => write!(f, "TCP connection failed"),
            Error::Handshake(e) => write!(f, "HTTP handshake error: {e}"),
            Error::Request(e) => write!(f, "HTTP request error: {e}"),
            Error::MissingHost => write!(f, "missing host in request URI"),
            Error::ConnectionNotReady => write!(f, "connection is closed or not ready"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Connect(e) => Some(e),
            Error::Handshake(e) | Error::Request(e) => Some(e),
            _ => None,
        }
    }
}

impl From<smoltcp::socket::tcp::ConnectError> for Error {
    fn from(e: smoltcp::socket::tcp::ConnectError) -> Self {
        Error::Connect(e)
    }
}
