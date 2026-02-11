# Rust Idiomatic Lcore API Design

This document proposes a safe, ergonomic Rust API for DPDK's lcore (logical core) functionality.

---

## Implementation Status

**Status: ✅ Implemented**

The lcore API has been implemented as designed. Key files:

| File | Description |
|------|-------------|
| `dpdk-net/src/api/rte/lcore.rs` | Main implementation (~635 lines) |
| `dpdk-net-sys/build.rs` | FFI bindings for lcore functions |
| `dpdk-net-sys/include/wrapper.h` | Wrapper for inline functions |
| `dpdk-net-sys/src/wrapper.c` | `rust_rte_lcore_id()`, `rust_rte_get_main_lcore()` |
| `dpdk-net-test/tests/lcore_test.rs` | Integration tests (9 tests) |

### Implemented Features

- ✅ `Lcore` struct with `id()`, `socket_id()`, `role()`, `state()`, `cpu_id()`
- ✅ Constructors: `from_id()`, `current()`, `main()`
- ✅ Iterators: `all()`, `workers()`
- ✅ Launch: `launch()`, `wait()`, `run()`
- ✅ Bulk ops: `wait_all_workers()`, `launch_on_workers()`
- ✅ `LaunchBuilder` with `on_socket()`, `filter()`, `take()`
- ✅ `LaunchHandle` for non-blocking wait
- ✅ `Role` and `State` enums

### Not Yet Implemented

- ⏳ Service cores (marked as future extension in appendix)
- ⏳ Lcore-local storage
- ⏳ Lcore groups

---

## Overview

DPDK lcores are EAL-managed threads pinned to specific CPU cores. The traditional C API uses:
- `rte_eal_remote_launch()` to spawn work on lcores
- `rte_lcore_id()` to get current lcore ID
- `rte_eal_mp_wait_lcore()` to join all lcores

This design provides Rust-idiomatic wrappers that are:
- **Safe**: No raw pointers in public API
- **Ergonomic**: Closures instead of C function pointers
- **Composable**: Works with Rust's ownership model

---

## Module Structure

```
dpdk-net/src/api/rte/
├── lcore.rs      # Lcore struct, iterators, LaunchBuilder, LaunchHandle
└── mod.rs        # Re-exports
```

All lcore functionality is unified in a single module with the `Lcore` struct as the central type.

---

## API Design

**Implementation:** See [dpdk-net/src/api/rte/lcore.rs](../../dpdk-net/src/api/rte/lcore.rs)

### Summary

The module provides:

| Type | Description |
|------|-------------|
| `Lcore` | Handle to a DPDK logical core |
| `Role` | Enum: `Rte`, `Off`, `Service`, `NonEal` |
| `State` | Enum: `Wait`, `Running`, `Finished` |
| `LcoreIter` | Iterator over lcores |
| `LaunchBuilder` | Builder for filtered multi-lcore launches |
| `LaunchHandle` | Handle for non-blocking wait on launched lcores |

### Key Methods on `Lcore`

```rust
// Constructors
Lcore::from_id(id: u32) -> Option<Lcore>
Lcore::current() -> Option<Lcore>  
Lcore::main() -> Lcore

// Iterators  
Lcore::all() -> LcoreIter
Lcore::workers() -> LcoreIter
Lcore::count() -> u32

// Properties
lcore.id() -> u32
lcore.socket_id() -> u32
lcore.role() -> Role
lcore.state() -> State
lcore.is_main() -> bool
lcore.is_available() -> bool

// Launch & Wait
lcore.launch(f: FnOnce() -> i32) -> Result<()>
lcore.wait() -> i32
lcore.run(f: FnOnce() -> i32) -> Result<i32>

// Bulk Operations
Lcore::wait_all_workers()
Lcore::launch_on_workers(f: Fn(Lcore) -> i32) -> Result<()>
```

### LaunchBuilder Pattern

```rust
LaunchBuilder::workers()
    .on_socket(0)        // Filter by NUMA socket
    .filter(|l| ...)     // Custom filter
    .take(n)             // Limit count
    .launch(|lcore| ...) // Returns LaunchHandle
    .run(|lcore| ...)    // Launch and wait, returns Vec<(Lcore, i32)>
```


---

## Appendix: Service Cores (Future Extension)

Service cores are a separate DPDK subsystem for running background tasks. They could
integrate with the `Lcore` struct in the future.

```rust
impl Lcore {
    /// Check if this lcore is a service core.
    pub fn is_service_core(&self) -> bool {
        self.role() == Role::Service
    }
    
    /// Convert this lcore to a service core.
    pub fn as_service_core(&self) -> Result<ServiceCore> {
        // ...
    }
}

/// A service core handle for running background services.
pub struct ServiceCore {
    lcore: Lcore,
}

impl ServiceCore {
    /// Register a service to run on this core.
    pub fn register_service(&self, service: Service) -> Result<()>;
    
    /// Start running registered services.
    pub fn start(&self) -> Result<()>;
    
    /// Stop the service core.
    pub fn stop(&self) -> Result<()>;
}
```

This is marked as future work and not part of the initial implementation.

---

## Usage Examples

### Example 1: Simple Worker Launch

```rust
use dpdk_net::api::rte::{eal, lcore::Lcore};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize EAL with 4 lcores
    eal::Builder::new()
        .args(["-l", "0-3"])
        .init()?;

    let main = Lcore::main();
    println!("Main lcore: {} on socket {}", main.id(), main.socket_id());
    println!("Total lcores: {}", Lcore::count());

    // Launch work on all workers
    for worker in Lcore::workers() {
        let socket = worker.socket_id();
        worker.launch(move || {
            println!("Worker {} on socket {}", 
                Lcore::current().unwrap().id(), 
                socket
            );
            
            // Do some work...
            std::thread::sleep(std::time::Duration::from_millis(100));
            
            0 // Success
        })?;
    }

    // Wait for all workers
    Lcore::wait_all_workers();
    println!("All workers finished");

    Ok(())
}
```

### Example 2: NUMA-Aware Launch

```rust
use dpdk_net::api::rte::lcore::{Lcore, LaunchBuilder};

// Launch work only on NUMA socket 0
let results = LaunchBuilder::workers()
    .on_socket(0)
    .run(|lcore| {
        println!("Processing on lcore {} (socket 0)", lcore.id());
        lcore.id() as i32
    })?;

for (lcore, result) in results {
    println!("Lcore {} returned {}", lcore.id(), result);
}
```

### Example 3: Shared State Across Workers

```rust
use dpdk_net::api::rte::lcore::Lcore;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

let packets_processed = Arc::new(AtomicU64::new(0));

Lcore::launch_on_workers({
    let counter = packets_processed.clone();
    move |lcore| {
        // Each worker processes packets
        let processed = do_packet_processing(lcore.id());
        counter.fetch_add(processed, Ordering::Relaxed);
        0
    }
})?;

Lcore::wait_all_workers();
println!("Total packets: {}", packets_processed.load(Ordering::Relaxed));
```

### Example 4: Launch and Wait with Result

```rust
use dpdk_net::api::rte::lcore::Lcore;

// Run synchronously on a specific worker
let worker = Lcore::workers().next().unwrap();
let result = worker.run(|| {
    // Heavy computation
    compute_something()
})?;

println!("Worker returned: {}", result);
```

### Example 5: Non-blocking Launch with Handle

```rust
use dpdk_net::api::rte::lcore::LaunchBuilder;

// Launch on all workers
let handle = LaunchBuilder::workers()
    .on_socket(0)
    .take(4)  // Only first 4 workers on socket 0
    .launch(|lcore| {
        do_work(lcore.id());
        0
    })?;

// Do other work while lcores are running...
do_main_work();

// Now wait and collect results
if !handle.is_done() {
    println!("Still waiting...");
}

let results = handle.wait();
for (lcore, result) in results {
    println!("Lcore {} finished with {}", lcore.id(), result);
}
```

---

## Comparison with C API

| C API | Rust API | Notes |
|-------|----------|-------|
| `rte_lcore_id()` | `Lcore::current() -> Option<Lcore>` | Returns typed handle |
| `rte_get_main_lcore()` | `Lcore::main() -> Lcore` | Returns typed handle |
| `rte_lcore_count()` | `Lcore::count() -> u32` | Static method |
| `rte_get_next_lcore()` | `Lcore::all()` / `Lcore::workers()` | Iterator of `Lcore` |
| `RTE_LCORE_FOREACH_WORKER` | `Lcore::foreach_worker(f)` | Closure receives `Lcore` |
| `rte_lcore_to_socket_id(id)` | `lcore.socket_id()` | Method on `Lcore` |
| `rte_lcore_to_cpu_id(id)` | `lcore.cpu_id()` | Method on `Lcore` |
| `rte_eal_lcore_role(id)` | `lcore.role()` | Method on `Lcore` |
| `rte_eal_get_lcore_state(id)` | `lcore.state()` | Method on `Lcore` |
| `rte_eal_remote_launch(fn, arg, id)` | `lcore.launch(closure)` | Method, no raw pointers |
| `rte_eal_wait_lcore(id) -> int` | `lcore.wait() -> i32` | Method, returns result |
| N/A | `lcore.run(closure) -> i32` | Combined launch+wait |
| `rte_eal_mp_wait_lcore()` | `Lcore::wait_all_workers()` | Static method |
| N/A | `LaunchBuilder` | Fluent API for bulk ops |
| N/A | `LaunchHandle` | Non-blocking wait pattern |

---

## Thread Safety Considerations

1. **`Lcore` is `Copy + Send + Sync`**
   - It's just a wrapper around `u32` (the lcore ID)
   - Safe to pass between threads and clone freely

2. **`lcore.launch()` closures must be `Send + 'static`**
   - The closure is transferred to the lcore's thread
   - Cannot capture references to local variables

3. **`LaunchHandle` is `Send` but not `Sync`**
   - Can be moved to another thread
   - Should only be used by one thread at a time

4. **Shared state requires synchronization**
   - Use `Arc<AtomicT>` for counters
   - Use `Arc<Mutex<T>>` or lock-free structures for complex state

5. **Lcore queries are safe from any thread**
   - `lcore.socket_id()`, `Lcore::count()`, etc. read immutable EAL state

---

## Integration with Async

The lcore API is designed for synchronous run-to-completion workloads. For async:

```rust
use dpdk_net::api::rte::lcore::Lcore;

// Option 1: Use lcore for setup, then spawn tokio runtime
for worker in Lcore::workers() {
    worker.launch(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        
        rt.block_on(async {
            // Async work per lcore
            my_async_worker().await
        });
        
        0
    })?;
}

// Option 2: Use dpdk-net's Rust-native threading (recommended for async)
// See ThreadRegistration and set_cpu_affinity in thread.rs
```

---

## Implementation Notes

### FFI Bindings (Implemented)

The following were added to `dpdk-net-sys/build.rs`:

```rust
// Lcore management functions
.allowlist_function("rte_eal_remote_launch")
.allowlist_function("rte_eal_mp_wait_lcore")
.allowlist_function("rte_eal_wait_lcore")
.allowlist_function("rte_eal_get_lcore_state")
.allowlist_function("rte_eal_lcore_role")
.allowlist_function("rte_lcore_count")
.allowlist_function("rte_lcore_is_enabled")
.allowlist_function("rte_lcore_to_socket_id")
.allowlist_function("rte_lcore_to_cpu_id")
.allowlist_function("rte_get_next_lcore")
// Wrapper functions for inlines
.allowlist_function("rust_rte_lcore_id")
.allowlist_function("rust_rte_get_main_lcore")
// Types
.allowlist_type("rte_lcore_state_t")
.allowlist_type("rte_lcore_role_t")
.allowlist_type("lcore_function_t")
```

### Wrapper Functions (Implemented)

Added to `wrapper.h` / `wrapper.c`:

```c
// wrapper.h
unsigned rust_rte_lcore_id(void);
unsigned rust_rte_get_main_lcore(void);

// wrapper.c
unsigned rust_rte_lcore_id(void) {
    return rte_lcore_id();
}

unsigned rust_rte_get_main_lcore(void) {
    return rte_get_main_lcore();
}
```

These wrap inline functions that bindgen cannot handle directly.

---

## Future Extensions

1. **Service cores** - See Appendix for initial design sketch
2. **Lcore-local storage** - Like thread-local but for lcores
3. **Lcore scheduling hints** - Suggest load balancing
4. **Integration with DPDK eventdev** - For event-driven processing
5. **Lcore groups** - Named sets of lcores for logical partitioning

---

## Test Coverage

Tests are in `dpdk-net-test/tests/lcore_test.rs`:

| Test | Description |
|------|-------------|
| `test_main_lcore` | Verify main lcore properties |
| `test_lcore_iteration` | Test `all()` and `workers()` iterators |
| `test_lcore_launch` | Launch closure on worker, verify execution |
| `test_lcore_run` | Test `run()` convenience method |
| `test_launch_on_workers` | Launch on all workers simultaneously |
| `test_launch_builder` | Test `LaunchBuilder` API |
| `test_launch_handle` | Test non-blocking `LaunchHandle` |
| `test_lcore_from_id` | Validate `from_id()` with valid/invalid IDs |
| `test_sequential_launches` | Multiple sequential launches on same lcore |

**Note:** Tests use a static `OnceLock<Eal>` since EAL can only be initialized once per process. Rust's test harness runs tests on multiple threads, so `Lcore::current()` may return `None` on test threads that aren't the EAL-initializing thread.
