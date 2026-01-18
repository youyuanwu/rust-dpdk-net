// Mbuf API
// See: /usr/local/include/rte_mbuf_core.h
// and /usr/local/include/rte_mbuf.h

use std::ptr::NonNull;
use std::slice;

use dpdk_net_sys::ffi;

use super::pktmbuf::MemPool;

/// A wrapper around DPDK's rte_mbuf.
///
/// This provides a safe, buffer-like interface for packet data.
/// The mbuf is automatically freed when dropped.
pub struct Mbuf {
    inner: NonNull<ffi::rte_mbuf>,
}

// Mbufs themselves are not thread-safe - only one thread should access at a time
// But they can be sent between threads
unsafe impl Send for Mbuf {}

impl Mbuf {
    /// Create an Mbuf from a raw pointer.
    ///
    /// # Safety
    /// The pointer must be a valid, non-null rte_mbuf that the caller owns.
    #[inline]
    pub unsafe fn from_raw(ptr: *mut ffi::rte_mbuf) -> Option<Self> {
        NonNull::new(ptr).map(|inner| Mbuf { inner })
    }

    /// Allocate a new mbuf from the given mempool.
    ///
    /// Returns `None` if allocation fails (pool exhausted).
    #[inline]
    pub fn alloc(mempool: &MemPool) -> Option<Self> {
        let ptr = unsafe { ffi::rust_pktmbuf_alloc(mempool.as_ptr()) };
        NonNull::new(ptr).map(|inner| Mbuf { inner })
    }

    /// Get the raw pointer to the underlying rte_mbuf.
    #[inline]
    pub fn as_ptr(&self) -> *mut ffi::rte_mbuf {
        self.inner.as_ptr()
    }

    /// Consume the Mbuf and return the raw pointer without freeing.
    ///
    /// The caller is responsible for freeing the mbuf.
    #[inline]
    pub fn into_raw(self) -> *mut ffi::rte_mbuf {
        let ptr = self.inner.as_ptr();
        std::mem::forget(self);
        ptr
    }

    /// Get the current data length (bytes of valid data).
    #[inline]
    pub fn data_len(&self) -> usize {
        unsafe { ffi::rust_pktmbuf_data_len(self.inner.as_ptr()) as usize }
    }

    /// Get the total packet length (for chained mbufs).
    #[inline]
    pub fn pkt_len(&self) -> usize {
        unsafe { ffi::rust_pktmbuf_pkt_len(self.inner.as_ptr()) as usize }
    }

    /// Get the headroom (unused space at the front of the buffer).
    #[inline]
    pub fn headroom(&self) -> usize {
        unsafe { ffi::rust_pktmbuf_headroom(self.inner.as_ptr()) as usize }
    }

    /// Get the tailroom (unused space at the end of the buffer).
    #[inline]
    pub fn tailroom(&self) -> usize {
        unsafe { ffi::rust_pktmbuf_tailroom(self.inner.as_ptr()) as usize }
    }

    /// Get the total capacity (data_len + tailroom).
    #[inline]
    pub fn capacity(&self) -> usize {
        self.data_len() + self.tailroom()
    }

    /// Get an immutable slice of the packet data.
    #[inline]
    pub fn data(&self) -> &[u8] {
        let ptr = unsafe { ffi::rust_pktmbuf_mtod(self.inner.as_ptr()) };
        let len = self.data_len();
        if ptr.is_null() || len == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(ptr as *const u8, len) }
        }
    }

    /// Get a mutable slice of the packet data.
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        let ptr = unsafe { ffi::rust_pktmbuf_mtod(self.inner.as_ptr()) };
        let len = self.data_len();
        if ptr.is_null() || len == 0 {
            &mut []
        } else {
            unsafe { slice::from_raw_parts_mut(ptr as *mut u8, len) }
        }
    }

    /// Append space to the end of the packet data.
    ///
    /// Returns a mutable slice to the newly appended region,
    /// or `None` if there's not enough tailroom.
    #[inline]
    pub fn append(&mut self, len: usize) -> Option<&mut [u8]> {
        if len > u16::MAX as usize {
            return None;
        }
        let ptr = unsafe { ffi::rust_pktmbuf_append(self.inner.as_ptr(), len as u16) };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { slice::from_raw_parts_mut(ptr as *mut u8, len) })
        }
    }

    /// Prepend space to the front of the packet data.
    ///
    /// Returns a mutable slice to the newly prepended region,
    /// or `None` if there's not enough headroom.
    #[inline]
    pub fn prepend(&mut self, len: usize) -> Option<&mut [u8]> {
        if len > u16::MAX as usize {
            return None;
        }
        let ptr = unsafe { ffi::rust_pktmbuf_prepend(self.inner.as_ptr(), len as u16) };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { slice::from_raw_parts_mut(ptr as *mut u8, len) })
        }
    }

    /// Remove bytes from the beginning of the packet data.
    ///
    /// Returns a pointer to the new data start, or `None` if len > data_len.
    #[inline]
    pub fn adj(&mut self, len: usize) -> bool {
        if len > u16::MAX as usize {
            return false;
        }
        let ptr = unsafe { ffi::rust_pktmbuf_adj(self.inner.as_ptr(), len as u16) };
        !ptr.is_null()
    }

    /// Remove bytes from the end of the packet data.
    ///
    /// Returns `true` on success, `false` if len > data_len.
    #[inline]
    pub fn trim(&mut self, len: usize) -> bool {
        if len > u16::MAX as usize {
            return false;
        }
        unsafe { ffi::rust_pktmbuf_trim(self.inner.as_ptr(), len as u16) == 0 }
    }

    /// Reset the mbuf to its initial state (empty, with default headroom).
    #[inline]
    pub fn reset(&mut self) {
        unsafe { ffi::rust_pktmbuf_reset(self.inner.as_ptr()) }
    }

    /// Extend the data length by `len` bytes (unsafe - doesn't check bounds).
    ///
    /// # Safety
    /// Caller must ensure there is sufficient tailroom.
    #[inline]
    pub unsafe fn extend(&mut self, len: usize) {
        let new_data_len = self.data_len() + len;
        let new_pkt_len = self.pkt_len() + len;
        unsafe {
            ffi::rust_pktmbuf_set_data_len(self.inner.as_ptr(), new_data_len as u16);
            ffi::rust_pktmbuf_set_pkt_len(self.inner.as_ptr(), new_pkt_len as u32);
        }
    }

    /// Shrink the data length by `len` bytes (unsafe - doesn't check bounds).
    ///
    /// # Safety
    /// Caller must ensure len <= data_len.
    #[inline]
    pub unsafe fn shrink(&mut self, len: usize) {
        let new_data_len = self.data_len().saturating_sub(len);
        let new_pkt_len = self.pkt_len().saturating_sub(len);
        unsafe {
            ffi::rust_pktmbuf_set_data_len(self.inner.as_ptr(), new_data_len as u16);
            ffi::rust_pktmbuf_set_pkt_len(self.inner.as_ptr(), new_pkt_len as u32);
        }
    }

    /// Copy data from a slice, resetting the mbuf first.
    pub fn copy_from_slice(&mut self, data: &[u8]) -> bool {
        self.reset();
        if let Some(buf) = self.append(data.len()) {
            buf.copy_from_slice(data);
            true
        } else {
            false
        }
    }
}

impl Drop for Mbuf {
    fn drop(&mut self) {
        unsafe {
            ffi::rust_pktmbuf_free(self.inner.as_ptr());
        }
    }
}

impl AsRef<[u8]> for Mbuf {
    fn as_ref(&self) -> &[u8] {
        self.data()
    }
}

impl AsMut<[u8]> for Mbuf {
    fn as_mut(&mut self) -> &mut [u8] {
        self.data_mut()
    }
}

impl std::fmt::Debug for Mbuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mbuf")
            .field("data_len", &self.data_len())
            .field("pkt_len", &self.pkt_len())
            .field("headroom", &self.headroom())
            .field("tailroom", &self.tailroom())
            .finish()
    }
}
