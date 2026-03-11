//! `tonic_tls::Incoming` impl for [`BridgeIncoming`].

use super::super::incoming::BridgeIncoming;
use super::super::io::BridgeIo;
use crate::BridgeError;

impl tonic_tls::Incoming for BridgeIncoming {
    type Io = BridgeIo;
    type Error = BridgeError;
}
