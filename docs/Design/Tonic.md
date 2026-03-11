# Tonic gRPC Integration Design

gRPC support for dpdk-net via [tonic](https://github.com/hyperium/tonic). Built on top of `DpdkApp` and reuses the axum module for the server side (see [Axum.md](Axum.md)).

Module: [`dpdk-net-util/src/tonic/`](../../dpdk-net-util/src/tonic/)  
Tests: [`tonic_grpc_test.rs`](../../dpdk-net-test/tests/tonic_grpc_test.rs) (on-lcore), [`tonic_bridge_test.rs`](../../dpdk-net-test/tests/tonic_bridge_test.rs) (bridge), [`tonic_bridge_tls_test.rs`](../../dpdk-net-test/tests/tonic_bridge_tls_test.rs) (bridge + TLS)

## Server (on-lcore)

`dpdk_net_util::tonic::serve()` accepts `tonic::service::Routes`, converts via `.into_axum_router()`, and delegates to `dpdk_net_util::axum::serve()`. Bypasses `tonic::transport::Server::serve()` which requires `Send` streams.

Mixed REST + gRPC: call `.into_axum_router()` yourself, merge with axum routes, and use `dpdk_net_util::axum::serve()` directly.

## Client (on-lcore)

`DpdkGrpcChannel` wraps a persistent HTTP/2 `Connection` and implements `tower::Service<http::Request<tonic::body::Body>>`, satisfying `GrpcService` via blanket impl. Replaces `tonic::transport::Channel` which requires `Send`.

The channel injects scheme and authority from the connect URI into outgoing requests (tonic generates path-only URIs, hyper requires full URIs). This mirrors tonic's internal `AddOrigin` middleware.

---

## OS Thread Bridge

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
                                                 │
Server::builder()                                │
  .serve_with_incoming_shutdown(                 │
      BridgeIncoming, ...) ◄──────────chan──     accept_loop → relay_task per conn
```

### Bridge Types

| Type | Role |
|------|------|
| `BridgeIo` | `Compat<BridgeTcpStream>` + `Connected`. Adapts futures-io to tokio-io. |
| `BridgeIncoming` | `Stream<Item = Result<BridgeIo, BridgeError>>` wrapping `BridgeTcpListener`. |
| `BridgeConnector` | `tower::Service<Uri>` → `TokioIo<BridgeIo>`. For `Endpoint::connect_with_connector()`. |

`BridgeConnector` returns `TokioIo<BridgeIo>` because tonic's connector path requires hyper 1.x IO traits, while the server path uses tokio IO traits directly.

### TLS (feature-gated: `tonic-tls`)

TLS for the bridge path via [`tonic-tls`](https://github.com/youyuanwu/tonic-tls) with OpenSSL. TLS termination runs on the OS thread — the DPDK lcore relays raw TCP bytes.

| Type | Role |
|------|------|
| `BridgeTransport` | `tonic_tls::Transport<Io = BridgeIo>`. Like `BridgeConnector` but returns `BridgeIo` directly — tonic-tls adds its own `TokioIo` + TLS wrapping. |
| `BridgeIncoming` | Also implements `tonic_tls::Incoming<Io = BridgeIo>` (blanket — already a compatible `Stream`). |

**Client:**
```rust
let transport = BridgeTransport::new(bridge);
let tls_conn = tonic_tls::openssl::TlsConnector::new(
    transport, ssl_connector, "server.example.com".into(),
);
let channel = Endpoint::from_static("https://10.0.0.1:8443")
    .connect_with_connector(tls_conn).await?;
```

**Server:**
```rust
let tls_incoming = tonic_tls::openssl::TlsIncoming::new(
    BridgeIncoming::new(listener), ssl_acceptor,
);
Server::builder()
    .add_service(GreeterServer::new(greeter))
    .serve_with_incoming_shutdown(tls_incoming, signal).await?;
```

### Module Layout

```
dpdk-net-util/src/tonic/
├── mod.rs               # re-exports serve, DpdkGrpcChannel, bridge::*
├── serve.rs             # on-lcore serve() — delegates to axum::serve
├── channel.rs           # DpdkGrpcChannel — on-lcore !Send client
└── bridge/
    ├── mod.rs           # re-exports + cfg(feature = "tonic-tls") pub mod tls
    ├── io.rs            # BridgeIo newtype + Connected impl
    ├── connector.rs     # BridgeConnector — tower::Service<Uri>
    ├── incoming.rs      # BridgeIncoming — Stream adapter
    └── tls/             # feature-gated: tonic-tls + openssl
        ├── mod.rs       # re-exports BridgeTransport
        ├── transport.rs # BridgeTransport — tonic_tls::Transport
        └── incoming.rs  # tonic_tls::Incoming impl for BridgeIncoming
```

### Comparison

| Property | On-lcore | OS Thread Bridge |
|----------|----------|------------------|
| `Send` | ❌ | ✅ |
| `Clone` (channel) | ❌ | ✅ |
| TCP transport | `dpdk_net::TcpStream` (!Send) | `BridgeTcpStream` → `BridgeIo` (Send) |
| HTTP/2 | manual `Connection` wrapper | tonic-managed |
| TLS | ❌ (cleartext h2c only) | ✅ via tonic-tls |
| HTTP/2 tuning | ❌ | ✅ (`Server::builder()` / `Endpoint` knobs) |
| Interceptors | manual | ✅ native |
| Multi-threaded | ❌ (single lcore) | ✅ (standard tokio runtime) |
| Latency | direct NIC (~μs) | +channel relay (~10-50μs) |

| Scenario | Use |
|----------|-----|
| Max throughput, direct NIC access | On-lcore `serve()` + `DpdkGrpcChannel` |
| Standard gRPC server over DPDK | `serve_with_incoming_shutdown` + `BridgeIncoming` |
| gRPC client from test / CLI / microservice | `Endpoint` + `BridgeConnector` (or `TlsConnector`) |
| Share one channel across many tasks | Bridge → `Channel` (Clone) |

---

## Tonic 0.14 Notes

- Prost split: `tonic-prost-build` (codegen) + `tonic-prost` (runtime codec)
- `tonic::body::Body` replaces private `BoxBody`
- `Routes::into_axum_router()` replaces deprecated `into_router()`
- `build_transport(false)` recommended — bridge clients construct channels via `Endpoint::connect_with_connector()`

## Limitations

### On-lcore

1. Cannot use `tonic::transport::Server` or `tonic::transport::Channel` — both require `Send`
2. No TLS — cleartext HTTP/2 (h2c) only
3. Single-threaded per lcore — one slow RPC blocks others
4. `DpdkGrpcChannel` is not `Clone` — one per client instance
5. Generated clients are `!Send` — cannot move between lcores

### Bridge

6. Channel relay adds ~10-50μs latency per hop vs on-lcore
7. Bridge worker must be spawned on lcore before OS thread can connect (`wait_ready()`)
8. No streaming backpressure from gRPC layer to bridge — relies on mpsc bounded channels

### TLS

9. ~1-2ms handshake latency per new connection (amortized over connection lifetime)
10. OpenSSL required at build time (`pkg-config openssl`)
11. On-lcore path remains cleartext — TLS requires `Send` (bridge only)
12. Certificate management is user responsibility via `SslAcceptor` / `SslConnector`

## References

- [Axum Integration Design](Axum.md)
- [HTTP Client Design](Client.md)
- [OS Thread Bridge Design](OsThreadBridge.md)
- [tonic `serve_with_incoming_shutdown`](https://docs.rs/tonic/latest/tonic/transport/server/struct.Router.html#method.serve_with_incoming_shutdown)
- [tonic `Endpoint::connect_with_connector`](https://docs.rs/tonic/latest/tonic/transport/struct.Endpoint.html#method.connect_with_connector)
- [`tonic-tls`](https://github.com/youyuanwu/tonic-tls)
- [`openssl`](https://docs.rs/openssl/latest/openssl/)
