//! [`BridgeIncoming`] — concrete `Stream` adapter for tonic's
//! `serve_with_incoming_shutdown`.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;

use crate::{BridgeError, BridgeTcpListener};

use super::io::BridgeIo;

/// Stream of incoming bridge connections, adapted for tonic transport.
///
/// Wraps [`BridgeTcpListener`] and implements
/// `Stream<Item = Result<BridgeIo, BridgeError>>`.
///
/// Pass to `tonic::transport::Server::builder().add_service(...).serve_with_incoming_shutdown()`.
pub struct BridgeIncoming {
    listener: BridgeTcpListener,
}

impl BridgeIncoming {
    /// Create a new incoming stream from a bridge listener.
    pub fn new(listener: BridgeTcpListener) -> Self {
        Self { listener }
    }
}

impl Stream for BridgeIncoming {
    type Item = Result<BridgeIo, BridgeError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.listener.accept_rx.poll_recv(cx) {
            Poll::Ready(Some(result)) => Poll::Ready(Some(result.map(BridgeIo::new))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
