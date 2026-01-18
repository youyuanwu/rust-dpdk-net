//! Runtime abstraction for async executors.
//!
//! This module provides the [`Runtime`] trait for abstracting runtime-specific
//! operations, allowing the reactor to work with different async runtimes.

use std::future::Future;

/// Trait for abstracting async runtime operations.
///
/// This allows the reactor to work with different async runtimes (tokio, async-std, etc.)
/// by abstracting runtime-specific operations like yielding.
///
/// # Example
///
/// ```rust
/// use dpdk_net::tcp::async_net::Runtime;
///
/// struct MyRuntime;
///
/// impl Runtime for MyRuntime {
///     async fn yield_now() {
///         // Your runtime's yield implementation
///     }
/// }
/// ```
pub trait Runtime {
    /// Yield control back to the runtime scheduler.
    ///
    /// This allows other tasks to run. The reactor calls this periodically
    /// to avoid monopolizing the executor.
    fn yield_now() -> impl Future<Output = ()>;
}
