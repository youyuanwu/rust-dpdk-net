# DpdkApp Design

This document describes the design for `DpdkApp`, a replacement for `DpdkServerRunner` that uses DPDK's native lcore threading model.

## Implementation Status

✅ **Implemented** in `dpdk-net-axum` crate:
- `DpdkApp` builder with `eth_dev()`, `ip()`, `gateway()`, `mbufs_per_queue()`, `descriptors()`
- `WorkerContext` with `lcore`, `queue_id`, `socket_id`, `shutdown`, `reactor`
- `run()` method accepting generic shutdown future and server closure
- Uses `CancellationToken` from tokio-util for shutdown signaling
- Merged worker function with optional shutdown watcher
- Test: `dpdk-net-axum/tests/app_echo_test.rs`

---

## Motivation

### Current DpdkServerRunner Issues

1. **Uses `std::thread::spawn`** instead of DPDK-managed lcores
2. **Manual thread registration** via `ThreadRegistration::new()`
3. **Manual CPU affinity** via `set_cpu_affinity()`
4. **Mismatch with EAL cores** - EAL creates lcores from `-l` flag, but we spawn separate threads
5. **Complex Configuration** - Many builder methods, not idiomatic for DPDK users

### Benefits of Lcore-Based Design

| Aspect | DpdkServerRunner | DpdkApp (new) |
|--------|------------------|---------------|
| Thread creation | `std::thread::spawn` | EAL creates at init |
| CPU affinity | Manual `set_cpu_affinity()` | Automatic (EAL) |
| Thread registration | Manual `ThreadRegistration` | Automatic (EAL threads) |
| NUMA awareness | Limited | Via `lcore.socket_id()` |
| Lcore count | Based on hw_queues/ethtool | Based on `-l` flag |

---

## Design Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         DpdkApp                                 │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │  EthDev Setup │ smoltcp Interface │ Reactor per Lcore   │   │
│   └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                       Lcore Layer                               │
│   Main Lcore (0)  │  Worker 1  │  Worker 2  │  Worker N         │
│   [runs queue 0]  │ [queue 1]  │ [queue 2]  │  [queue N]        │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                         EAL + EthDev                            │
│   rte_eal_init("-l 0-3")  │  rte_eth_dev_configure(4 queues)    │
└─────────────────────────────────────────────────────────────────┘
```

---

## API Design

### DpdkApp Builder

```rust
use dpdk_net_axum::{DpdkApp, WorkerContext};
use smoltcp::wire::Ipv4Address;
use tokio_util::sync::CancellationToken;

// Create a shutdown signal using CancellationToken
let shutdown_token = CancellationToken::new();
let shutdown_clone = shutdown_token.clone();
ctrlc::set_handler(move || shutdown_clone.cancel()).unwrap();

// Simple usage - lcores determined by EAL init
DpdkApp::new()
    .eth_dev(0)                          // Use eth device 0
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(
        // Shutdown future - any Future<Output = ()> works
        shutdown_token.cancelled(),
        // Server closure - runs on each lcore
        |ctx: WorkerContext| async move {
            // ctx.lcore - the Lcore handle
            // ctx.queue_id - matches lcore index (0, 1, 2, ...)
            // ctx.reactor - create listeners or connections
            // ctx.shutdown - CancellationToken for graceful shutdown
            
            // Server: create a listener
            let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
            my_server(listener, ctx.shutdown.clone()).await;
            
            // Wait for shutdown
            ctx.shutdown.cancelled().await;
        },
    );
```

### WorkerContext

```rust
/// Context passed to each worker lcore.
pub struct WorkerContext {
    /// The lcore this worker is running on.
    pub lcore: Lcore,
    
    /// Queue ID (0 = main lcore, 1+ = workers).
    pub queue_id: u16,
    
    /// NUMA socket ID for this lcore.
    pub socket_id: u32,
    
    /// Cancellation token for graceful shutdown.
    /// Use `shutdown.cancelled().await` to wait for shutdown.
    /// Use `shutdown.is_cancelled()` for non-blocking check.
    pub shutdown: CancellationToken,
    
    /// Reactor handle for creating sockets.
    /// Use this to create TcpListener (server) or TcpStream (client).
    pub reactor: ReactorHandle,
}
```

### DpdkApp Struct

```rust
pub struct DpdkApp {
    port_id: u16,
    ip_addr: Option<Ipv4Address>,
    gateway: Option<Ipv4Address>,
    
    // Queue/buffer configuration
    mbufs_per_queue: u32,
    rx_desc: u16,
    tx_desc: u16,
}

impl DpdkApp {
    /// Create a new DpdkApp builder.
    pub fn new() -> Self;
    
    /// Set the DPDK port ID (default: 0).
    pub fn eth_dev(self, port_id: u16) -> Self;
    
    /// Set the IP address.
    pub fn ip(self, addr: Ipv4Address) -> Self;
    
    /// Set the gateway address.
    pub fn gateway(self, addr: Ipv4Address) -> Self;
    
    /// Set mbufs per queue (default: 8192).
    pub fn mbufs_per_queue(self, count: u32) -> Self;
    
    /// Set RX/TX descriptors (default: 1024).
    pub fn descriptors(self, rx: u16, tx: u16) -> Self;
    
    /// Run the application.
    ///
    /// Launches work on all worker lcores and runs queue 0 on the main lcore.
    /// Blocks until the shutdown future completes.
    pub fn run<S, F, Fut>(self, shutdown: S, server: F)
    where
        S: Future<Output = ()> + Send + 'static,
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + 'static;
}
```

---

## Lcore-to-Queue Mapping

The key insight: **lcore count determines queue count**.

```rust
// During DpdkApp::run():

// 1. Get available lcores from EAL
let lcores: Vec<Lcore> = Lcore::all().collect();
let num_queues = lcores.len();

// 2. Configure eth device with matching queue count
let eth_conf = EthConf::new()
    .rss_with_hash(rss_hf::NONFRAG_IPV4_TCP);
    
EthDev::new(port_id)
    .configure(num_queues as u16, num_queues as u16, &eth_conf)?;

// 3. Map: lcore index -> queue index
// lcores[0] (main) -> queue 0
// lcores[1] (worker) -> queue 1
// ...
```

---

## Execution Flow

### Startup

```
1. EAL already initialized (user responsibility)
   rte_eal_init("-l 0-3 -a <pci>")
   
2. DpdkApp::run() called
   │
   ├─ Query lcores: Lcore::all() → [0, 1, 2, 3]
   │
   ├─ Configure EthDev with 4 queues
   │
   ├─ Create SharedArpCache (if multi-queue)
   │
   ├─ Launch on workers (lcores 1, 2, 3):
   │   Lcore::launch_on_workers(|lcore| {
   │       run_worker(lcore, queue_id=lcore.id(), ...)
   │   })
   │
   └─ Run on main lcore (lcore 0):
       run_worker(main_lcore, queue_id=0, ...)
```

### Per-Worker Setup

```rust
fn run_worker(lcore: Lcore, queue_id: u16, shutdown_token: CancellationToken, ...) -> i32 {
    // 1. Create tokio runtime (current_thread)
    let rt = Builder::new_current_thread().build().unwrap();
    let local = tokio::task::LocalSet::new();
    
    local.block_on(&rt, async {
        // 2. Create DpdkDevice for this queue
        let device = eth_dev_config.create_device(mempool, queue_id);
        
        // 3. Create smoltcp Interface
        let iface = Interface::new(...);
        
        // 4. Create Reactor
        let reactor = Reactor::new(device, iface);
        let handle = reactor.handle();
        
        // 5. Spawn reactor task
        let reactor_cancel = Arc::new(AtomicBool::new(false));
        tokio::task::spawn_local(reactor.run(reactor_cancel.clone()));
        
        // 6. If main worker, spawn shutdown watcher
        if let Some(shutdown_future) = shutdown_watcher {
            tokio::task::spawn(async move {
                shutdown_future.await;
                shutdown_token.cancel();  // Broadcast to all workers
            });
        }
        
        // 7. Create context
        let ctx = WorkerContext {
            lcore,
            queue_id,
            socket_id: lcore.socket_id(),
            shutdown: shutdown_token,
            reactor: handle,
        };
        
        // 8. Run user's closure (server, client, or both)
        server(ctx).await;
        
        // 9. Cleanup
        reactor_cancel.store(true, Ordering::Relaxed);
    });
    
    0 // Return code for lcore
}
```

### Shutdown

```
1. User's shutdown future completes (e.g., Ctrl+C)
   │
   ├─ Main worker's shutdown watcher task triggers
   │
   ├─ CancellationToken.cancel() broadcasts to all workers
   │
   ├─ User closures see ctx.shutdown.is_cancelled() or .cancelled().await
   │
   ├─ User servers exit gracefully
   │
   └─ Reactors stop

2. Lcore::wait_all_workers() returns

3. Main lcore's run_worker returns

4. EthDev cleanup (stop + close)
```

---

## Differences from DpdkServerRunner

| Aspect | DpdkServerRunner | DpdkApp |
|--------|------------------|---------|
| Thread creation | `std::thread::spawn` | `Lcore::launch()` |
| Queue count source | `ethtool` / `hw_queues()` | Lcore count from EAL |
| CPU affinity | `set_cpu_affinity()` manual | Automatic (EAL lcores) |
| Thread naming | `"queue-N"` | N/A (EAL threads) |
| DPDK registration | `ThreadRegistration::new()` | Automatic |
| Main thread role | Queue 0 | Lcore::main() = queue 0 |
| Configuration | Many builder methods | Simpler, lcore-driven |

---

## Example Usage

### Basic Server

```rust
use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net_axum::DpdkApp;
use dpdk_net::socket::TcpListener;
use tokio_util::sync::CancellationToken;

fn main() {
    // 1. Initialize EAL with desired lcores
    let _eal = EalBuilder::new()
        .core_list("0-3")
        .allow("0000:00:04.0")
        .init()
        .expect("EAL init failed");
    
    // 2. Setup shutdown signal with CancellationToken
    let shutdown_token = CancellationToken::new();
    let shutdown_clone = shutdown_token.clone();
    ctrlc::set_handler(move || shutdown_clone.cancel()).unwrap();
    
    // 3. Run app - uses all 4 lcores, 4 queues
    DpdkApp::new()
        .eth_dev(0)
        .ip(Ipv4Address::new(10, 0, 0, 10))
        .gateway(Ipv4Address::new(10, 0, 0, 1))
        .run(
            shutdown_token.cancelled(),
            |ctx| async move {
                let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
                echo_server(listener, ctx.shutdown.clone()).await;
                ctx.shutdown.cancelled().await;
            },
        );
}
```

### Basic Client

```rust
use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net_axum::DpdkApp;
use dpdk_net::socket::TcpStream;
use tokio_util::sync::CancellationToken;

fn main() {
    let _eal = EalBuilder::new()
        .core_list("0")
        .allow("0000:00:04.0")
        .init()
        .expect("EAL init failed");
    
    let shutdown_token = CancellationToken::new();
    let shutdown_clone = shutdown_token.clone();
    ctrlc::set_handler(move || shutdown_clone.cancel()).unwrap();
    
    DpdkApp::new()
        .eth_dev(0)
        .ip(Ipv4Address::new(10, 0, 0, 20))
        .gateway(Ipv4Address::new(10, 0, 0, 1))
        .run(
            shutdown_token.cancelled(),
            |ctx| async move {
                let server_addr = "10.0.0.10:8080".parse().unwrap();
                let stream = TcpStream::connect(&ctx.reactor, server_addr).await.unwrap();
                send_requests(stream, ctx.shutdown.clone()).await;
                ctx.shutdown.cancelled().await;
            },
        );
}
```

### 
            async move { while !shutdown.load(Ordering::Relaxed) { tokio::task::yield_now().await; } },
            move |ctx| {
                let app = app.clone();
                async move {
                    let listener = TcpListener::bind(&ctx.reactor, 8080, 16).unwrap();
                    serve(listener, app, ctx.shutdown).await
                }
            },
        );
}
```

### NUMA-Aware Socket Selection (Future)

> **Note**: `lcores()` filter is not yet implemented. For now, use EAL's `-l` flag to select specific lcores.

```rust
// Future API - only use lcores on NUMA socket 0
DpdkApp::new()
    .eth_dev(0)
    .lcores(|lcore| lcore.socket_id() == 0)  // Not yet implemented
    .run(
        shutdown,
        |ctx| async move {
            // Only lcores from socket 0 will run workers
        },
    );

// Current workaround: specify lcores via EAL init
let _eal = EalBuilder::new()
    .core_list("0-3")  // Only lcores on socket 0
    .allow("0000:00:04.0")
    .init()?;
```

---

## Module Structure

```
dpdk-net-axum/
├── Cargo.toml       # Dependencies: dpdk-net, tokio, tokio-util, smoltcp, tracing, axum
├── src/
│   ├── lib.rs       # Re-exports DpdkApp, WorkerContext
│   ├── app.rs       # DpdkApp builder and run logic
│   └── context.rs   # WorkerContext definition
└── tests/
    └── app_echo_test.rs  # Echo test demonstrating API usage
```

---

## Migration Guide

### Before (DpdkServerRunner)

```rust
// Old: Configure everything manually
ensure_hugepages()?;
let pci_addr = get_pci_addr("eth1")?;
let _eal = EalBuilder::new().allow(&pci_addr).init()?;

DpdkServerRunner::new("eth1")
    .with_default_network_config()  // Reads from interface
    .with_default_hw_queues()       // Reads from ethtool
    .port(8080)
    .run(|ctx| async move { ... });
```

### After (DpdkApp)

```rust
// New: EAL config determines everything
let _eal = EalBuilder::new()
    .args(["-l", "0-3", "-a", "0000:00:04.0"])
    .init()?;

let shutdown = /* user provides shutdown future */;

DpdkApp::new()
    .eth_dev(0)
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(
        shutdown,
        |ctx| async move {
            let listener = TcpListener::bind(&ctx.reactor, 8080, 16).unwrap();
            // ...
        },
    );
```

---

## Testing with Virtual Devices

> **Note**: See `dpdk-net-axum/tests/app_echo_test.rs` for a complete working example using `net_ring0` with single-lcore echo test.

### net_ring (Loopback Testing)

The simplest approach for loopback testing:

Two `net_ring` vdevs can be connected using `nodeaction` to share the underlying ring:

```bash
# Port 0: Creates the shared ring
--vdev=net_ring0,nodeaction=CREATE

# Port 1: Attaches to the same ring (TX/RX swapped)
--vdev=net_ring1,nodeaction=ATTACH
```

Packets sent from port 0 arrive at port 1, and vice versa.

### Integration Test Example

```rust
#[test]
fn tecore_list("0-3")
    .allow("0000:00:04.0")
    .init()?;

let shutdown_token = CancellationToken::new();
let shutdown_clone = shutdown_token.clone();
ctrlc::set_handler(move || shutdown_clone.cancel()).unwrap();

DpdkApp::new()
    .eth_dev(0)
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(
        shutdown_token.cancelled(),
        |ctx| async move {
            let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
            // ctx.shutdown is a CancellationToken
            ctx.shutdown.cancelled().await;w: port0 TX → ring → port1 RX
    //               port1 TX → ring → port0 RX
    
    let server_handle = std::thread::spawn(|| {
        let shutdown = /* create shutdown signal */;
        DpdkApp::new()
            .eth_dev(0)  // Server on port 0
            .ip(Ipv4Address::new(10, 0, 0, 10))
            .gateway(Ipv4Address::new(10, 0, 0, 20))
            .run(
                shutdown,
                |ctx| async move {
                    let listener = TcpListener::bind(&ctx.reactor, 8080, 16).unwrap();
                    echo_server(listener, ctx.shutdown).await
                },
            );
    });
    
    // Client on port 1
    let client_handle = std::thread::spawn(|| {
        let shutdown = /* create shutdown signal */;
        DpdkApp::new()
            .eth_dev(1)  // Client on port 1
            .ip(Ipv4Address::new(10, 0, 0, 20))
            .gateway(Ipv4Address::new(10, 0, 0, 10))
            .run(
                shutdown,
                |ctx| async move {
                    // Connect to server and send requests
                    let stream = TcpStream::connect(
                        &ctx.reactor,
                        "10.0.0.10:8080".parse().unwrap()
                    ).await.unwrap();
                    // ... test logic
                },
            );
    });
    
    server_handle.join().unwrap();
    client_handle.join().unwrap();
}
```

### net_tap for External Testing

For testing with external tools (curl, wrk, etc.):

```rust
let _eal = EalBuilder::new()
    .args([
        "-l", "0-3",
        "--no-pci",
        "--vdev=net_tap0,iface=dtap0",
    ])
    .init()?;

let shutdown = /* user provides shutdown future */;

DpdkApp::new()
    .eth_dev(0)
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(
        shutdown,
        |ctx| async move {
            let listener = TcpListener::bind(&ctx.reactor, 8080, 16).unwrap();
            // ...
        },
    );
```

Then configure the TAP interface:

```bash
sudo ip addr add 10.0.0.1/24 dev dtap0
sudo ip link set dtap0 up

# Test with standard tools
curl http://10.0.0.10:8080/
wrk -t4 -c100 http://10.0.0.10:8080/
```

### vdev Comparison

| vdev | Use Case | Same Process? | External Tools? |
|------|----------|---------------|-----------------|
| `net_null` | Smoke tests, API testing | N/A | No |
| `net_ring` (single) | Loopback testing | Self only | No |
| `net_ring` + nodeaction | Integration tests | Yes | No |
| `net_tap` | External/benchmark testing | Via kernel | Yes |
| `net_tap` x2 + bridge | Integration tests | Via kernel | Yes |

---

## Limitations

1. **EAL must be initialized first** - Cannot auto-init (user controls `-l` flag)
2. **Queue count == Lcore count** - No independent configuration
3. **IP/Gateway not auto-detected** - Must be specified explicitly
4. **Main lcore blocked** - Runs queue 0, cannot be used for other work

---

## Future Enhancements

1. **Lcore filter** - `lcores(|lcore| lcore.socket_id() == 0)` for NUMA-aware selection
2. **Config file support** - Load IP/gateway from TOML
3. **Auto IP detection** - Query interface like DpdkServerRunner did
4. **Metrics integration** - Built-in prometheus metrics per queue
5. **Axum integration** - `serve()` function for axum routers

---

## References

- [Lcore API Design](LcoreAPI.md)
- [Current DpdkServerRunner](../../dpdk-net-test/src/app/dpdk_server_runner.rs)
- [DPDK EAL Threading](https://doc.dpdk.org/guides/prog_guide/env_abstraction_layer.html#threading)
