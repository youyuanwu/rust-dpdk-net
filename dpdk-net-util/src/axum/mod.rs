//! Axum web framework integration for dpdk-net.
//!
//! Re-exports the [`serve`] function for running an axum `Router`
//! on a dpdk-net `TcpListener`.

mod serve;

pub use serve::serve;
