# DpdkApp Design

This document describes the design for `DpdkApp`, a replacement for `DpdkServerRunner` that uses DPDK's native lcore threading model.

## Implementation Status

✅ **Implemented** in `dpdk-net-axum` crate:
- `DpdkApp` builder with `eth_dev()`, `ip()`, `gateway()`, `mbufs_per_queue()`, `descriptors()`
- `WorkerContext` with `lcore`, `queue_id`, `socket_id`, `shutdown`, `reactor`
- `run()` method accepting generic shutdown future and server closure
- Uses `CancellationToken` from tokio-util for shutdown signaling
- Merged worker function with optional shutdown watcher
- `serve()` for axum `Router` integration (see [Axum.md](Axum.md))

✅ **Tests:**
- `dpdk-net-axum/tests/app_echo_test.rs` — raw TCP echo test
- `dpdk-net-axum/tests/axum_client_test.rs` — axum server + HTTP client integration test

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

## API

The API is defined in [app.rs](../../dpdk-net-axum/src/app.rs) and [context.rs](../../dpdk-net-axum/src/context.rs).

### DpdkApp Builder

```rust
DpdkApp::new()
    .eth_dev(0)                          // DPDK port ID (default: 0)
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .mbufs_per_queue(8192)               // Optional (default: 8192)
    .descriptors(1024, 1024)             // Optional RX/TX descriptors
    .run(
        shutdown_token.cancelled(),       // Any Future<Output = ()>
        |ctx: WorkerContext| async move {
            // Server/client logic using ctx.reactor, ctx.shutdown
        },
    );
```

### WorkerContext Fields

| Field | Type | Description |
|-------|------|-------------|
| `lcore` | `Lcore` | The lcore this worker runs on |
| `queue_id` | `u16` | Queue ID (0 = main lcore, 1+ = workers) |
| `socket_id` | `u32` | NUMA socket ID |
| `shutdown` | `CancellationToken` | Graceful shutdown signal |
| `reactor` | `ReactorHandle` | Create `TcpListener` or `TcpStream` |

---

## Lcore-to-Queue Mapping

**Key insight:** lcore count determines queue count. Each lcore gets one RX/TX queue pair.

```
lcores[0] (main)   → queue 0
lcores[1] (worker)  → queue 1
lcores[2] (worker)  → queue 2
...
```

Queue count is set during `EthDev::configure()` based on `Lcore::all().count()`.

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

### Per-Worker Flow

Each worker (including main) performs:
1. Create `tokio` current_thread runtime + `LocalSet`
2. Create `DpdkDevice` for its queue
3. Create `smoltcp::Interface` and `Reactor`
4. Spawn reactor task via `spawn_local`
5. On main worker only: spawn shutdown watcher task
6. Build `WorkerContext` and run user closure
7. Cleanup on return

See: [app.rs](../../dpdk-net-axum/src/app.rs)

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

```rust
use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::TcpListener;
use dpdk_net_axum::{DpdkApp, WorkerContext, serve};
use axum::{Router, routing::get};
use smoltcp::wire::Ipv4Address;
use tokio_util::sync::CancellationToken;

fn main() {
    let _eal = EalBuilder::new()
        .core_list("0-3")
        .allow("0000:00:04.0")
        .init()
        .expect("EAL init failed");

    let shutdown_token = CancellationToken::new();
    let shutdown_clone = shutdown_token.clone();
    ctrlc::set_handler(move || shutdown_clone.cancel()).unwrap();

    let app = Router::new()
        .route("/", get(|| async { "Hello from DPDK + Axum!" }));

    DpdkApp::new()
        .eth_dev(0)
        .ip(Ipv4Address::new(10, 0, 0, 10))
        .gateway(Ipv4Address::new(10, 0, 0, 1))
        .run(
            shutdown_token.cancelled(),
            move |ctx: WorkerContext| {
                let app = app.clone();
                async move {
                    let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
                    serve(listener, app, ctx.shutdown).await;
                }
            },
        );
}
```

See also: [axum_client_test.rs](../../dpdk-net-axum/tests/axum_client_test.rs), [app_echo_test.rs](../../dpdk-net-axum/tests/app_echo_test.rs)

---

## Testing with Virtual Devices

### net_ring (Loopback Testing)

Two `net_ring` vdevs connected via `nodeaction` share an underlying ring:

```bash
--vdev=net_ring0,nodeaction=CREATE    # Port 0: creates the shared ring
--vdev=net_ring1,nodeaction=ATTACH    # Port 1: attaches (TX/RX swapped)
```

Packets sent from port 0 arrive at port 1, and vice versa. See [app_echo_test.rs](../../dpdk-net-axum/tests/app_echo_test.rs) for a working example.

### net_tap (External Testing)

For testing with `curl`, `wrk`, etc.:

```bash
# EAL args
-l 0-3 --no-pci --vdev=net_tap0,iface=dtap0

# Host setup
sudo ip addr add 10.0.0.1/24 dev dtap0
sudo ip link set dtap0 up
curl http://10.0.0.10:8080/
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

## Module Structure

```
dpdk-net-axum/
├── Cargo.toml
├── src/
│   ├── lib.rs         # Re-exports: DpdkApp, WorkerContext, serve
│   ├── app.rs         # DpdkApp builder and run logic
│   ├── context.rs     # WorkerContext definition
│   └── serve.rs       # serve() for axum Router
└── tests/
    ├── app_echo_test.rs       # Raw TCP echo test
    └── axum_client_test.rs    # Axum server + HTTP client test
```

---

## Migration Guide

### Before (DpdkServerRunner)

```rust
ensure_hugepages()?;
let pci_addr = get_pci_addr("eth1")?;
let _eal = EalBuilder::new().allow(&pci_addr).init()?;
DpdkServerRunner::new("eth1")
    .with_default_network_config()
    .with_default_hw_queues()
    .port(8080)
    .run(|ctx| async move { ... });
```

### After (DpdkApp)

```rust
let _eal = EalBuilder::new()
    .args(["-l", "0-3", "-a", "0000:00:04.0"])
    .init()?;

DpdkApp::new()
    .eth_dev(0)
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(shutdown, |ctx| async move {
        let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
        // ...
    });
```

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
5. ~~**Axum integration**~~ - ✅ Implemented (see [Axum.md](Axum.md))

---

## References

- [Lcore API Design](LcoreAPI.md)
- [Axum Integration Design](Axum.md)
- [HTTP Client Design](Client.md)
- [Current DpdkServerRunner](../../dpdk-net-test/src/app/dpdk_server_runner.rs)
- [DPDK EAL Threading](https://doc.dpdk.org/guides/prog_guide/env_abstraction_layer.html#threading)
