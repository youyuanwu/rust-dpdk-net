//! Lcore (logical core) APIs.
//!
//! Provides a safe, ergonomic interface to DPDK's lcore functionality.
//!
//! # Overview
//!
//! DPDK lcores are EAL-managed threads pinned to specific CPU cores. This module
//! provides Rust-idiomatic wrappers for:
//! - Querying lcore information (ID, socket, role, state)
//! - Launching closures on worker lcores
//! - Waiting for lcore completion
//!
//! # Example
//!
//! ```no_run
//! use dpdk_net::api::rte::lcore::Lcore;
//!
//! // Get the main lcore
//! let main = Lcore::main();
//! println!("Main lcore {} on socket {}", main.id(), main.socket_id());
//!
//! // Launch work on all workers
//! for worker in Lcore::workers() {
//!     worker.launch(|| {
//!         println!("Hello from worker!");
//!         0
//!     }).unwrap();
//! }
//!
//! // Wait for all workers
//! Lcore::wait_all_workers();
//! ```

use dpdk_net_sys::ffi;
use std::ffi::c_void;
use std::sync::Arc;

use crate::Result;

/// Special value indicating "any lcore" or "not an lcore thread"
pub const LCORE_ID_ANY: u32 = u32::MAX;

/// Role of an lcore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Role {
    /// Regular EAL thread (RTE role)
    Rte = 0,
    /// Off - lcore is not used
    Off = 1,
    /// Service core
    Service = 2,
    /// Non-EAL thread (registered via rte_thread_register)
    NonEal = 3,
}

impl TryFrom<u32> for Role {
    type Error = ();

    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Rte),
            1 => Ok(Self::Off),
            2 => Ok(Self::Service),
            3 => Ok(Self::NonEal),
            _ => Err(()),
        }
    }
}

/// State of an lcore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum State {
    /// Lcore is waiting for new command
    Wait = 0,
    /// Lcore is running a function
    Running = 1,
    /// Lcore has finished, waiting for ack
    Finished = 2,
}

impl From<i32> for State {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Wait,
            1 => Self::Running,
            2 => Self::Finished,
            _ => Self::Wait,
        }
    }
}

/// A handle to a DPDK logical core (lcore).
///
/// Lcores are EAL-managed threads created during `rte_eal_init()`.
/// Each lcore is pinned to a specific CPU core and can execute
/// functions launched via [`Lcore::launch()`].
///
/// This type is `Copy`, `Send`, and `Sync` - it's just a lightweight
/// handle to an lcore, not the lcore itself.
///
/// # Example
///
/// ```no_run
/// use dpdk_net::api::rte::lcore::Lcore;
///
/// // Get the main lcore
/// let main = Lcore::main();
/// println!("Main lcore {} on socket {}", main.id(), main.socket_id());
///
/// // Launch work on all workers
/// for worker in Lcore::workers() {
///     worker.launch(|| {
///         println!("Hello from worker!");
///         0
///     }).unwrap();
/// }
///
/// // Wait for all workers
/// Lcore::wait_all_workers();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lcore {
    id: u32,
}

// Lcore is just an ID handle - safe to send/share
unsafe impl Send for Lcore {}
unsafe impl Sync for Lcore {}

impl Lcore {
    // ==================== Constructors ====================

    /// Create an Lcore handle from an ID.
    ///
    /// Returns `None` if the lcore ID is invalid or not enabled.
    pub fn from_id(id: u32) -> Option<Self> {
        if id < ffi::RTE_MAX_LCORE && unsafe { ffi::rte_lcore_is_enabled(id) != 0 } {
            Some(Self { id })
        } else {
            None
        }
    }

    /// Create an Lcore handle without checking if it's valid.
    ///
    /// # Safety
    ///
    /// The caller must ensure the lcore ID is valid and enabled.
    #[inline]
    pub unsafe fn from_id_unchecked(id: u32) -> Self {
        Self { id }
    }

    /// Get the current thread's lcore.
    ///
    /// Returns `None` if called from a non-EAL thread that hasn't been registered.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dpdk_net::api::rte::lcore::Lcore;
    ///
    /// if let Some(lcore) = Lcore::current() {
    ///     println!("Running on lcore {}", lcore.id());
    /// } else {
    ///     println!("Not running on an lcore");
    /// }
    /// ```
    pub fn current() -> Option<Self> {
        let id = unsafe { ffi::rust_rte_lcore_id() };
        if id == LCORE_ID_ANY {
            None
        } else {
            Some(Self { id })
        }
    }

    /// Get the main (initial) lcore.
    ///
    /// This is the lcore that called `rte_eal_init()`.
    pub fn main() -> Self {
        Self {
            id: unsafe { ffi::rust_rte_get_main_lcore() },
        }
    }

    // ==================== Iterators ====================

    /// Iterate over all enabled lcores (including main).
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dpdk_net::api::rte::lcore::Lcore;
    ///
    /// for lcore in Lcore::all() {
    ///     println!("Lcore {} on socket {}", lcore.id(), lcore.socket_id());
    /// }
    /// ```
    pub fn all() -> LcoreIter {
        LcoreIter {
            current: u32::MAX,
            skip_main: false,
        }
    }

    /// Iterate over worker lcores (excluding main).
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dpdk_net::api::rte::lcore::Lcore;
    ///
    /// for worker in Lcore::workers() {
    ///     println!("Worker lcore: {}", worker.id());
    /// }
    /// ```
    pub fn workers() -> LcoreIter {
        LcoreIter {
            current: u32::MAX,
            skip_main: true,
        }
    }

    /// Get the total number of enabled lcores.
    pub fn count() -> u32 {
        unsafe { ffi::rte_lcore_count() }
    }

    // ==================== Properties ====================

    /// Get this lcore's ID.
    #[inline]
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Check if this is the main lcore.
    #[inline]
    pub fn is_main(&self) -> bool {
        self.id == unsafe { ffi::rust_rte_get_main_lcore() }
    }

    /// Get the role of this lcore.
    #[inline]
    pub fn role(&self) -> Role {
        let role = unsafe { ffi::rte_eal_lcore_role(self.id) };
        Role::try_from(role as u32).unwrap_or(Role::Off)
    }

    /// Get the NUMA socket ID for this lcore.
    #[inline]
    pub fn socket_id(&self) -> u32 {
        unsafe { ffi::rte_lcore_to_socket_id(self.id) }
    }

    /// Get the physical CPU ID this lcore is pinned to.
    #[inline]
    pub fn cpu_id(&self) -> Option<i32> {
        let cpu = unsafe { ffi::rte_lcore_to_cpu_id(self.id as i32) };
        if cpu < 0 { None } else { Some(cpu) }
    }

    /// Get the current state of this lcore.
    #[inline]
    pub fn state(&self) -> State {
        let state = unsafe { ffi::rte_eal_get_lcore_state(self.id) };
        State::from(state as i32)
    }

    /// Check if this lcore is currently available (in Wait state).
    #[inline]
    pub fn is_available(&self) -> bool {
        self.state() == State::Wait
    }

    // ==================== Launch & Wait ====================

    /// Launch a closure on this lcore.
    ///
    /// The closure will be executed on this lcore's thread. The lcore must be
    /// in the `Wait` state (not currently running another task).
    ///
    /// # Arguments
    ///
    /// * `f` - The closure to execute (must be `Send` and return `i32`)
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the launch was successful
    /// * `Err` if the lcore is busy or invalid
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dpdk_net::api::rte::lcore::Lcore;
    ///
    /// for worker in Lcore::workers() {
    ///     let id = worker.id();
    ///     worker.launch(move || {
    ///         println!("Hello from lcore {}", id);
    ///         0
    ///     }).unwrap();
    /// }
    /// ```
    pub fn launch<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce() -> i32 + Send + 'static,
    {
        struct LaunchContext<F> {
            func: F,
        }

        unsafe extern "C" fn trampoline<F>(arg: *mut c_void) -> i32
        where
            F: FnOnce() -> i32 + Send,
        {
            let ctx = unsafe { Box::from_raw(arg as *mut LaunchContext<F>) };
            (ctx.func)()
        }

        let ctx = Box::new(LaunchContext { func: f });
        let arg = Box::into_raw(ctx) as *mut c_void;

        let ret = unsafe { ffi::rte_eal_remote_launch(Some(trampoline::<F>), arg, self.id) };

        if ret == 0 {
            Ok(())
        } else {
            // Clean up the leaked box on failure
            unsafe {
                drop(Box::from_raw(arg as *mut LaunchContext<F>));
            };
            Err(format!("Failed to launch on lcore {}: error {}", self.id, ret).into())
        }
    }

    /// Wait for this lcore to finish its current task.
    ///
    /// Blocks until the lcore enters the `Wait` state and returns the
    /// return value of the function that was launched on it.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dpdk_net::api::rte::lcore::Lcore;
    ///
    /// let worker = Lcore::workers().next().unwrap();
    /// worker.launch(|| 42).unwrap();
    ///
    /// let result = worker.wait();
    /// assert_eq!(result, 42);
    /// ```
    pub fn wait(&self) -> i32 {
        unsafe { ffi::rte_eal_wait_lcore(self.id) }
    }

    /// Launch a closure and wait for it to complete.
    ///
    /// This is a convenience method combining `launch()` and `wait()`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dpdk_net::api::rte::lcore::Lcore;
    ///
    /// let worker = Lcore::workers().next().unwrap();
    /// let result = worker.run(|| {
    ///     // Do work...
    ///     42
    /// }).unwrap();
    /// assert_eq!(result, 42);
    /// ```
    pub fn run<F>(&self, f: F) -> Result<i32>
    where
        F: FnOnce() -> i32 + Send + 'static,
    {
        self.launch(f)?;
        Ok(self.wait())
    }

    // ==================== Bulk Operations ====================

    /// Wait for all worker lcores to finish.
    ///
    /// This is equivalent to calling `wait()` on all worker lcores.
    /// Note: Does NOT return the individual return values.
    pub fn wait_all_workers() {
        unsafe { ffi::rte_eal_mp_wait_lcore() };
    }

    /// Launch a closure on all worker lcores.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dpdk_net::api::rte::lcore::Lcore;
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicU64, Ordering};
    ///
    /// let counter = Arc::new(AtomicU64::new(0));
    ///
    /// Lcore::launch_on_workers({
    ///     let counter = counter.clone();
    ///     move |lcore| {
    ///         counter.fetch_add(1, Ordering::Relaxed);
    ///         println!("Worker {} reporting", lcore.id());
    ///         0
    ///     }
    /// }).unwrap();
    ///
    /// Lcore::wait_all_workers();
    /// println!("Workers launched: {}", counter.load(Ordering::Relaxed));
    /// ```
    pub fn launch_on_workers<F>(f: F) -> Result<()>
    where
        F: Fn(Lcore) -> i32 + Send + Sync + 'static,
    {
        let f = Arc::new(f);

        for worker in Self::workers() {
            let f = f.clone();
            worker.launch(move || f(worker))?;
        }

        Ok(())
    }

    /// Execute a closure for each worker lcore.
    ///
    /// This is a convenience wrapper for configuration, not execution.
    pub fn foreach_worker<F: FnMut(Lcore)>(mut f: F) {
        for lcore in Self::workers() {
            f(lcore);
        }
    }
}

/// Iterator over lcores.
pub struct LcoreIter {
    current: u32,
    skip_main: bool,
}

impl Iterator for LcoreIter {
    type Item = Lcore;

    fn next(&mut self) -> Option<Self::Item> {
        let skip_main = if self.skip_main { 1 } else { 0 };

        let next = unsafe { ffi::rte_get_next_lcore(self.current, skip_main, 0) };

        if next >= ffi::RTE_MAX_LCORE {
            None
        } else {
            self.current = next;
            Some(Lcore { id: next })
        }
    }
}

impl ExactSizeIterator for LcoreIter {
    fn len(&self) -> usize {
        // Count remaining lcores
        let mut count = 0;
        let mut current = self.current;
        let skip_main = if self.skip_main { 1 } else { 0 };

        loop {
            let next = unsafe { ffi::rte_get_next_lcore(current, skip_main, 0) };
            if next >= ffi::RTE_MAX_LCORE {
                break;
            }
            count += 1;
            current = next;
        }
        count
    }
}

/// Builder for launching work on multiple lcores with filtering.
///
/// # Example
///
/// ```no_run
/// use dpdk_net::api::rte::lcore::{Lcore, LaunchBuilder};
///
/// // Launch only on NUMA socket 0
/// let results = LaunchBuilder::workers()
///     .on_socket(0)
///     .run(|lcore| {
///         println!("Processing on lcore {} (socket 0)", lcore.id());
///         lcore.id() as i32
///     }).unwrap();
///
/// for (lcore, result) in results {
///     println!("Lcore {} returned {}", lcore.id(), result);
/// }
/// ```
pub struct LaunchBuilder {
    lcores: Vec<Lcore>,
}

impl LaunchBuilder {
    /// Create a builder targeting all workers.
    pub fn workers() -> Self {
        Self {
            lcores: Lcore::workers().collect(),
        }
    }

    /// Create a builder targeting all lcores (including main).
    pub fn all() -> Self {
        Self {
            lcores: Lcore::all().collect(),
        }
    }

    /// Create a builder targeting specific lcores.
    pub fn with_lcores(lcores: impl IntoIterator<Item = Lcore>) -> Self {
        Self {
            lcores: lcores.into_iter().collect(),
        }
    }

    /// Filter lcores by NUMA socket.
    pub fn on_socket(mut self, socket_id: u32) -> Self {
        self.lcores.retain(|lcore| lcore.socket_id() == socket_id);
        self
    }

    /// Filter lcores by a custom predicate.
    pub fn filter<P: FnMut(&Lcore) -> bool>(mut self, mut predicate: P) -> Self {
        self.lcores.retain(|lcore| predicate(lcore));
        self
    }

    /// Keep only the first N lcores.
    pub fn take(mut self, n: usize) -> Self {
        self.lcores.truncate(n);
        self
    }

    /// Get the targeted lcores.
    pub fn lcores(&self) -> &[Lcore] {
        &self.lcores
    }

    /// Launch on all targeted lcores (non-blocking).
    pub fn launch<F>(self, f: F) -> Result<LaunchHandle>
    where
        F: Fn(Lcore) -> i32 + Send + Sync + 'static,
    {
        let f = Arc::new(f);

        for &lcore in &self.lcores {
            let f = f.clone();
            lcore.launch(move || f(lcore))?;
        }

        Ok(LaunchHandle {
            lcores: self.lcores,
        })
    }

    /// Launch and wait for all to complete.
    ///
    /// Returns (Lcore, return_value) pairs.
    pub fn run<F>(self, f: F) -> Result<Vec<(Lcore, i32)>>
    where
        F: Fn(Lcore) -> i32 + Send + Sync + 'static,
    {
        let handle = self.launch(f)?;
        Ok(handle.wait())
    }
}

/// Handle to a set of launched lcores.
///
/// Allows waiting for completion and collecting results.
pub struct LaunchHandle {
    lcores: Vec<Lcore>,
}

impl LaunchHandle {
    /// Wait for all launched lcores to complete.
    ///
    /// Returns (Lcore, return_value) pairs.
    pub fn wait(self) -> Vec<(Lcore, i32)> {
        self.lcores
            .into_iter()
            .map(|lcore| {
                let result = lcore.wait();
                (lcore, result)
            })
            .collect()
    }

    /// Check if all lcores have finished.
    pub fn is_done(&self) -> bool {
        self.lcores
            .iter()
            .all(|lcore| lcore.state() != State::Running)
    }

    /// Get the lcores being tracked.
    pub fn lcores(&self) -> &[Lcore] {
        &self.lcores
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_try_from() {
        assert_eq!(Role::try_from(0), Ok(Role::Rte));
        assert_eq!(Role::try_from(1), Ok(Role::Off));
        assert_eq!(Role::try_from(2), Ok(Role::Service));
        assert_eq!(Role::try_from(3), Ok(Role::NonEal));
        assert_eq!(Role::try_from(99), Err(()));
    }

    #[test]
    fn test_state_from() {
        assert_eq!(State::from(0), State::Wait);
        assert_eq!(State::from(1), State::Running);
        assert_eq!(State::from(2), State::Finished);
        assert_eq!(State::from(99), State::Wait); // default
    }
}
