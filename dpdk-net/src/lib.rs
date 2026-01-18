pub mod tcp;
/// A boxed error type for dpdk-net operations.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A Result type using BoxError.
pub type Result<T> = std::result::Result<T, BoxError>;
