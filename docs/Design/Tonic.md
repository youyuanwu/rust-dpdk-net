# Tonic gRPC Integration Design

gRPC support for dpdk-net via [tonic](https://github.com/hyperium/tonic). Built on top of `DpdkApp` and reuses the axum module for the server side (see [Axum.md](Axum.md)).

Module: [`dpdk-net-util/src/tonic/`](../../dpdk-net-util/src/tonic/)  
Tests: [`tonic_grpc_test.rs`](../../dpdk-net-test/tests/tonic_grpc_test.rs) (on-lcore), [`tonic_bridge_test.rs`](../../dpdk-net-test/tests/tonic_bridge_test.rs) (OS thread bridge)

## Server (on-lcore)

`dpdk_net_util::tonic::serve()` accepts `tonic::service::Routes`, converts via `.into_axum_router()`, and delegates to `dpdk_net_util::axum::serve()`. Bypasses `tonic::transport::Server::serve()` which requires `Send` streams.

Mixed REST + gRPC: call `.into_axum_router()` yourself, merge with axum routes, and use `dpdk_net_util::axum::serve()` directly.

## Client (on-lcore)

`DpdkGrpcChannel` wraps a persistent HTTP/2 `Connection` and implements `tower::Service<http::Request<tonic::body::Body>>`, satisfying `GrpcService` via blanket impl. Replaces `tonic::transport::Channel` which requires `Send`.

The channel injects scheme and authority from the connect URI into outgoing requests (tonic generates path-only URIs, hyper requires full URIs). This mirrors tonic's internal `AddOrigin` middleware.

---

## OS Thread Bridge Integration

For OS threads, `BridgeTcpStream` is `Send`, unlocking tonic's native `tonic::transport` APIs — `serve_with_incoming_shutdown` for the server and `Endpoint::connect_with_connector` for the client.

Module: [`dpdk-net-util/src/tonic/bridge/`](../../dpdk-net-util/src/tonic/bridge/)

### Architecture

```
OS Thread (tokio runtime)                      DPDK Lcore
─────────────────────────                      ──────────
                                               bridge_worker (spawn_local)
                                                 │
Endpoint::connect_with_connector(BridgeConnector)│
  ├── BridgeConnector::call(uri)                 │
  │     bridge.connect(addr, port) ───cmd──►     TcpStream::connect()
  │     → TokioIo<BridgeIo> (Send) ◄──chan──     relay_task (owns !Send TcpStream)
  ├── tonic builds HTTP/2 conn internally        │
  └── Channel (Clone + Send)                     │
       client.say_hello(req)                     │
       → HTTP/2 frames            ───data──►     relay → dpdk_stream.send()
       ← response                 ◄──data──      relay ← dpdk_stream.recv()
                                                 │
Server::builder()                                │
  .add_service(greeter)                          │
  .serve_with_incoming_shutdown(                 │
      BridgeIncoming::new(listener), ◄──chan──   accept_loop → relay_task per conn
      signal)                                    │
  ├── tonic manages per-conn tasks               │
  └── tokio::spawn (not spawn_local)             │
```

### Bridge Types

**`BridgeIo`** — Newtype wrapping `Compat<BridgeTcpStream>`. Converts `futures_io` traits to `tokio::io` via `.compat()` and implements tonic's `Connected` trait (with `ConnectInfo = ()`).

**`BridgeIncoming`** — Concrete `Stream<Item = Result<BridgeIo, BridgeError>>` wrapping `BridgeTcpListener`. Implements `Stream` by delegating to the listener's internal `mpsc::Receiver::poll_recv`. No `async_stream` dependency, no boxing.

**`BridgeConnector`** — `tower::Service<Uri>` that parses host/port from the URI, calls `bridge.connect()`, and returns `TokioIo<BridgeIo>`. The `TokioIo` wrapper is needed because tonic's connector path requires `hyper::rt::io::Read + Write` (hyper 1.x IO traits), while the server path (`serve_with_incoming_shutdown`) uses `tokio::io::AsyncRead + AsyncWrite` directly.

### What tonic handles natively

By using tonic's transport APIs instead of manual hyper plumbing:
- **Server**: connection management, HTTP/2 tuning, graceful shutdown/drain, interceptors/layers, health checking, reflection
- **Client**: `Channel` is `Clone + Send`, automatic reconnect, scheme/authority injection (`AddOrigin`), configurable timeouts/keepalive, interceptors, load balancing

Mixed REST + gRPC on OS threads: convert tonic routes to an axum `Router` and use a manual bridge accept loop instead of `serve_with_incoming_shutdown`.

### Module Layout

```
dpdk-net-util/src/tonic/
├── mod.rs               # re-exports serve, DpdkGrpcChannel, bridge::*
├── serve.rs             # on-lcore serve() — delegates to axum::serve
├── channel.rs           # DpdkGrpcChannel — on-lcore !Send client
└── bridge/
    ├── mod.rs           # re-exports BridgeIo, BridgeConnector, BridgeIncoming
    ├── io.rs            # BridgeIo newtype + Connected impl
    ├── connector.rs     # BridgeConnector — tower::Service<Uri> → TokioIo<BridgeIo>
    └── incoming.rs      # BridgeIncoming — concrete Stream adapter
```

### Comparison

| Property | On-lcore | OS Thread Bridge |
|----------|----------|------------------|
| `Send` | ❌ | ✅ |
| `Clone` (channel) | ❌ | ✅ |
| TCP transport | `dpdk_net::TcpStream` (!Send) | `BridgeTcpStream` → `BridgeIo` (Send) |
| HTTP/2 | manual `Connection` wrapper | tonic-managed |
| Accept loop | manual in `axum::serve` | tonic-managed (`serve_with_incoming_shutdown`) |
| Per-conn spawn | `spawn_local` | `tokio::spawn` |
| HTTP/2 tuning | ❌ | ✅ (`Server::builder()` / `Endpoint` knobs) |
| Interceptors | manual | ✅ native |
| Multi-threaded | ❌ (single lcore) | ✅ (standard tokio runtime) |
| Latency | direct NIC (~μs) | +channel relay (~10-50μs) |

### When to Use Which

| Scenario | Use |
|----------|-----|
| Max throughput, direct NIC access | On-lcore `serve()` + `DpdkGrpcChannel` |
| Standard gRPC server over DPDK | `serve_with_incoming_shutdown` + `BridgeIncoming` |
| gRPC client from test / CLI / microservice | `Endpoint` + `BridgeConnector` |
| Share one channel across many tasks | `BridgeConnector` → `Channel` (Clone) |
| Mixed: fast-path on lcore, slow RPCs on OS threads | Both — coexist on same `DpdkApp` |

---

## Tonic 0.14 Notes

- Prost split: `tonic-prost-build` (codegen) + `tonic-prost` (runtime codec)
- `tonic::body::Body` replaces private `BoxBody`
- `Routes::into_axum_router()` replaces deprecated `into_router()`
- `serve_with_incoming_shutdown` is on the router returned by `Server::builder().add_service()`
- `Endpoint::connect_with_connector` accepts any `tower::Service<Uri>` — no need for `tonic::transport::Channel` directly
- `build_transport(false)` omits the generated `connect()` convenience method on clients. Recommended for both on-lcore and bridge — bridge clients construct channels via `Endpoint::connect_with_connector()` + `Client::new(channel)`, not the generated `connect()`

## Limitations

### On-lcore (existing)

1. Cannot use `tonic::transport::Server` or `tonic::transport::Channel` — both require `Send`
2. No TLS — cleartext HTTP/2 (h2c) only
3. Single-threaded per lcore — one slow RPC blocks others on the same lcore
4. `DpdkGrpcChannel` is not `Clone` — create one per tonic client instance
5. Generated clients are `!Send` — cannot be moved between lcores
6. Must use `build_transport(false)` in codegen

### Bridge (new)

7. Channel relay adds latency (~10-50μs per hop) compared to on-lcore direct path
8. Throughput limited by mpsc channel capacity (256 per direction) and copy overhead (`Bytes::copy_from_slice`)
9. Bridge worker must be spawned on an lcore before OS thread can connect — requires `wait_ready()` coordination
10. No streaming backpressure signal from gRPC layer to bridge — relies on mpsc bounded channels and TCP window
11. Enabling `transport` feature pulls in additional dependencies (hyper server stack, tower layers)

## References

- [Axum Integration Design](Axum.md)
- [HTTP Client Design](Client.md)
- [OS Thread Bridge Design](OsThreadBridge.md)
- [tonic `GrpcService` trait](https://docs.rs/tonic/latest/tonic/client/trait.GrpcService.html)
- [tonic `serve_with_incoming_shutdown`](https://docs.rs/tonic/latest/tonic/transport/server/struct.Router.html#method.serve_with_incoming_shutdown)
- [tonic `Endpoint::connect_with_connector`](https://docs.rs/tonic/latest/tonic/transport/struct.Endpoint.html#method.connect_with_connector)
- [gRPC over HTTP/2 spec](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md)
