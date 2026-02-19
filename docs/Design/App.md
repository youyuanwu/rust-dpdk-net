# DpdkApp Design

High-level application framework using DPDK's native lcore threading model. Each lcore gets its own RX/TX queue, smoltcp network stack, and tokio runtime.

Crate: [`dpdk-net-util`](../../dpdk-net-util/src/app.rs) (`DpdkApp`, `WorkerContext`)  
Tests: [`app_echo_test.rs`](../../dpdk-net-axum/tests/app_echo_test.rs), [`axum_client_test.rs`](../../dpdk-net-axum/tests/axum_client_test.rs), [`tonic_grpc_test.rs`](../../dpdk-net-test/tests/tonic_grpc_test.rs), and all async tests in `dpdk-net-test/tests/`

## Architecture

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
```

Lcore count determines queue count (1:1 mapping). EAL must be initialized before calling `run()`.

## Usage

```rust
DpdkApp::new()
    .eth_dev(0)
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(|ctx: WorkerContext| async move {
        let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
        serve(listener, app, std::future::pending::<()>()).await;
    });
```

### WorkerContext

| Field | Type | Description |
|-------|------|-------------|
| `lcore` | `Lcore` | The lcore this worker runs on |
| `queue_id` | `u16` | Queue ID (0 = main lcore, 1+ = workers) |
| `socket_id` | `u32` | NUMA socket ID |
| `reactor` | `ReactorHandle` | Create `TcpListener` or `TcpStream` |

## Shutdown

`run()` blocks until all worker closures return. After all workers exit, the EthDev is stopped and closed.

## Testing with Virtual Devices

| vdev | Use Case | External Tools? |
|------|----------|-----------------|
| `net_ring` + nodeaction | Integration tests (loopback) | No |
| `net_tap` | External/benchmark testing | Yes (`curl`, `wrk`) |

```bash
# net_ring loopback (two ports sharing a ring)
--vdev=net_ring0,nodeaction=CREATE --vdev=net_ring1,nodeaction=ATTACH

# net_tap (accessible from host)
--no-pci --vdev=net_tap0,iface=dtap0
# then: sudo ip addr add 10.0.0.1/24 dev dtap0 && sudo ip link set dtap0 up
```

## Limitations

1. EAL must be initialized first — user controls `-l` flag
2. Queue count == lcore count — no independent configuration
3. IP/gateway must be specified explicitly
4. Main lcore runs queue 0 and blocks until shutdown

## References

- [Axum Integration](Axum.md)
- [HTTP Client](Client.md)
- [Tonic gRPC](Tonic.md)
- [DPDK EAL Threading](https://doc.dpdk.org/guides/prog_guide/env_abstraction_layer.html#threading)
