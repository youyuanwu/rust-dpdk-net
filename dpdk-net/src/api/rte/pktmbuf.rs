// pktmbuf API
// See: /usr/local/include/rte_mbuf.h

use std::ffi::CString;
use std::ptr::NonNull;

use dpdk_net_sys::ffi;

/// Wrapper for DPDK rte_mempool for packet mbufs (owning)
pub struct MemPool {
    inner: NonNull<ffi::rte_mempool>,
}

// DPDK mempools are thread-safe
unsafe impl Send for MemPool {}
unsafe impl Sync for MemPool {}

/// Non-owning reference to a DPDK mempool.
///
/// This is returned by `MemPool::lookup()` and does NOT free the pool when dropped.
/// Use this when you need to share a mempool across threads.
#[derive(Clone, Copy)]
pub struct MemPoolRef {
    inner: NonNull<ffi::rte_mempool>,
}

// DPDK mempools are thread-safe
unsafe impl Send for MemPoolRef {}
unsafe impl Sync for MemPoolRef {}

impl MemPoolRef {
    /// Get the raw pointer to the underlying rte_mempool
    #[inline]
    pub fn as_ptr(&self) -> *mut ffi::rte_mempool {
        self.inner.as_ptr()
    }

    /// Try to allocate an mbuf from this pool.
    ///
    /// Returns `None` if the pool is exhausted.
    #[inline]
    pub fn try_alloc(&self) -> Option<super::mbuf::Mbuf> {
        let ptr = unsafe { ffi::rust_pktmbuf_alloc(self.inner.as_ptr()) };
        unsafe { super::mbuf::Mbuf::from_raw(ptr) }
    }

    /// Fill a batch of mbufs up to the remaining capacity of the ArrayVec.
    #[inline]
    pub fn fill_batch<const N: usize>(
        &self,
        batch: &mut arrayvec::ArrayVec<super::mbuf::Mbuf, N>,
    ) -> usize {
        let mut count = 0;
        while batch.len() < batch.capacity() {
            if let Some(mbuf) = self.try_alloc() {
                batch.push(mbuf);
                count += 1;
            } else {
                break;
            }
        }
        count
    }
}

/// Configuration for creating a MemPool
#[derive(Debug, Clone)]
pub struct MemPoolConfig {
    /// Number of mbufs in the pool (optimum: 2^q - 1)
    pub num_mbufs: u32,
    /// Per-core cache size (0 to disable caching)
    pub cache_size: u32,
    /// Private area size between rte_mbuf struct and data buffer
    pub priv_size: u16,
    /// Data room size including RTE_PKTMBUF_HEADROOM
    pub data_room_size: u16,
    /// NUMA socket ID (-1 for SOCKET_ID_ANY)
    pub socket_id: i32,
}

impl Default for MemPoolConfig {
    fn default() -> Self {
        Self {
            num_mbufs: 8191, // 2^13 - 1
            cache_size: 256,
            priv_size: 0,
            data_room_size: ffi::RTE_MBUF_DEFAULT_DATAROOM as u16
                + ffi::RTE_PKTMBUF_HEADROOM as u16,
            socket_id: -1, // SOCKET_ID_ANY
        }
    }
}

impl MemPoolConfig {
    /// Create a new MemPoolConfig with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the number of mbufs in the pool.
    ///
    /// Optimum value is 2^q - 1 (e.g., 8191, 16383, 32767).
    pub fn num_mbufs(mut self, n: u32) -> Self {
        self.num_mbufs = n;
        self
    }

    /// Set the per-core cache size.
    ///
    /// Set to 0 to disable caching. Should be less than num_mbufs / 1.5.
    pub fn cache_size(mut self, size: u32) -> Self {
        self.cache_size = size;
        self
    }

    /// Set the private area size between rte_mbuf struct and data buffer.
    pub fn priv_size(mut self, size: u16) -> Self {
        self.priv_size = size;
        self
    }

    /// Set the data room size (including RTE_PKTMBUF_HEADROOM).
    ///
    /// The usable data capacity is `data_room_size - RTE_PKTMBUF_HEADROOM`.
    pub fn data_room_size(mut self, size: u16) -> Self {
        self.data_room_size = size;
        self
    }

    /// Set the NUMA socket ID.
    ///
    /// Use -1 for SOCKET_ID_ANY (allocate on any socket).
    pub fn socket_id(mut self, id: i32) -> Self {
        self.socket_id = id;
        self
    }
}

impl MemPool {
    /// Create a new pktmbuf mempool
    ///
    /// # Arguments
    /// * `name` - Pool name (anything convertible to CString)
    /// * `config` - Pool configuration
    pub fn create<S>(name: S, config: &MemPoolConfig) -> crate::api::Result<Self>
    where
        S: Into<Vec<u8>>,
    {
        let c_name = CString::new(name).map_err(|_| nix::errno::Errno::EINVAL)?;
        let ptr = unsafe {
            ffi::rte_pktmbuf_pool_create(
                c_name.as_ptr(),
                config.num_mbufs,
                config.cache_size,
                config.priv_size,
                config.data_room_size,
                config.socket_id,
            )
        };
        NonNull::new(ptr)
            .map(|inner| MemPool { inner })
            .ok_or_else(crate::api::rte_errno)
    }

    /// Create a mempool with default configuration
    pub fn create_default<S>(name: S, num_mbufs: u32) -> crate::api::Result<Self>
    where
        S: Into<Vec<u8>>,
    {
        let config = MemPoolConfig {
            num_mbufs,
            ..Default::default()
        };
        Self::create(name, &config)
    }

    /// Lookup an existing mempool by name.
    ///
    /// **Warning**: The returned MemPool is a non-owning reference.
    /// You must ensure the original pool outlives this reference.
    /// This is marked unsafe because dropping this handle will NOT free the pool.
    pub fn lookup<S>(name: S) -> crate::api::Result<MemPoolRef>
    where
        S: Into<Vec<u8>>,
    {
        let c_name = CString::new(name).map_err(|_| nix::errno::Errno::EINVAL)?;
        let ptr = unsafe { ffi::rte_mempool_lookup(c_name.as_ptr()) };
        NonNull::new(ptr)
            .map(|inner| MemPoolRef { inner })
            .ok_or_else(crate::api::rte_errno)
    }

    /// Get the raw pointer to the underlying rte_mempool
    #[inline]
    pub fn as_ptr(&self) -> *mut ffi::rte_mempool {
        self.inner.as_ptr()
    }

    /// Get the number of available (free) objects in the pool
    #[inline]
    pub fn avail_count(&self) -> u32 {
        unsafe { ffi::rte_mempool_avail_count(self.inner.as_ptr()) }
    }

    /// Try to allocate an mbuf from this pool.
    ///
    /// Returns `None` if the pool is exhausted.
    #[inline]
    pub fn try_alloc(&self) -> Option<super::mbuf::Mbuf> {
        super::mbuf::Mbuf::alloc(self)
    }

    /// Fill a batch of mbufs up to the remaining capacity of the ArrayVec.
    ///
    /// Allocates as many mbufs as possible until either:
    /// - The ArrayVec is full
    /// - The pool is exhausted
    ///
    /// Returns the number of mbufs allocated.
    #[inline]
    pub fn fill_batch<const N: usize>(
        &self,
        batch: &mut arrayvec::ArrayVec<super::mbuf::Mbuf, N>,
    ) -> usize {
        let mut count = 0;
        while batch.len() < batch.capacity() {
            if let Some(mbuf) = self.try_alloc() {
                batch.push(mbuf);
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    /// Get the data room size for mbufs in this pool.
    #[inline]
    pub fn data_room_size(&self) -> u16 {
        unsafe { ffi::rust_pktmbuf_data_room_size(self.inner.as_ptr()) }
    }
}

impl Drop for MemPool {
    fn drop(&mut self) {
        unsafe {
            ffi::rte_mempool_free(self.inner.as_ptr());
        }
    }
}
