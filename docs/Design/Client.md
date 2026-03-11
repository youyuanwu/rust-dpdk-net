# DPDK-net HTTP Client Design

This document describes the HTTP client design for `dpdk-net`, addressing the `!Send` constraint.

## Implementation Status

✅ **Implemented** in `dpdk-net-util` crate:
- `DpdkHttpClient` — high-level client with `connect()` and `request()` methods
- `Connection` — persistent HTTP connection with `send_request()`
- `ConnectionPool` — simple per-host connection pool (`!Send`, one per lcore)
- `http1_connect()` / `http2_connect()` — low-level helper functions
- `LocalExecutor` — `hyper::rt::Executor` for `!Send` futures (shared with axum server)
- `ClientConfig` — configurable buffer sizes and HTTP version
- `Error` type — connection, handshake, and request errors

✅ **Tests:**
- `dpdk-net-test/tests/axum_client_test.rs` — `DpdkHttpClient` + axum server integration test
- Existing lower-level tests in `dpdk-net-test/tests/http_echo_test.rs`, `http2_echo_test.rs`, `http_auto_echo_test.rs`

### Module Structure

```
dpdk-net-util/
├── Cargo.toml
└── src/
    ├── lib.rs          # Re-exports
    ├── client.rs       # DpdkHttpClient, ClientConfig
    ├── connect.rs      # http1_connect(), http2_connect()
    ├── connection.rs   # Connection, ConnectionSender, HttpVersion
    ├── error.rs        # Error types
    ├── executor.rs     # LocalExecutor
    └── pool.rs         # ConnectionPool
```

---

## The `!Send` Constraint

dpdk-net's `TcpStream` uses `Rc<RefCell<...>>` internally, making it `!Send`. This rules out `hyper-util::Client` and `reqwest` (both require `Send + Sync`). Only hyper's low-level `client::conn` API works — it does not require `Send` on the IO type.

---

## API Layers

### Low-Level: `http1_connect` / `http2_connect`

Direct hyper handshake wrappers. Returns a `(SendRequest, connection_future)` pair. The connection future must be spawned via `spawn_local`.

See: [connect.rs](../../dpdk-net-util/src/connect.rs)

### High-Level: `DpdkHttpClient`

Wraps the low-level API with connection management:

```rust
let client = DpdkHttpClient::new(ctx.reactor.clone());

// One-shot request (creates + tears down connection)
let response = client.request(Request::get("http://10.0.0.10:8080/").body(Empty::new())?).await?;

// Persistent connection for multiple requests
let mut conn = client.connect("10.0.0.10", 8080).await?;
let response = conn.send_request(Request::get("/health").body(Empty::new())?).await?;
```

See: [client.rs](../../dpdk-net-util/src/client.rs), [connection.rs](../../dpdk-net-util/src/connection.rs)

### Connection Pool: `ConnectionPool`

Simple per-host pool for workloads needing connection reuse. `!Send` — one pool per lcore.

See: [pool.rs](../../dpdk-net-util/src/pool.rs)

---

## Rejected Alternatives

- **hyper-util `Client`**: Requires `C: Connect + Clone + Send + Sync + 'static`. dpdk-net's `TcpStream` is `!Send`.
- **reqwest**: Hardcodes `tokio::net::TcpStream`, requires `Send + Sync` on `Client`, no pluggable connector.

---

## Comparison with Server Side

| Aspect | Server (Axum) | Client |
|--------|---------------|--------|
| Primary crate | hyper-util server | hyper client |
| Connection handling | Accept loop | Connect per request |
| Multiplexing | HTTP/2 streams | HTTP/2 streams |
| Pool needed | No (listener) | Yes (for efficiency) |
| Executor | `LocalExecutor` | `LocalExecutor` (HTTP/2) |

---

## References

- [Implementation: dpdk-net-util](../../dpdk-net-util/src/lib.rs)
- [Test: axum_client_test.rs](../../dpdk-net-test/tests/axum_client_test.rs)
- [hyper client module](https://docs.rs/hyper/latest/hyper/client/index.html)
- [Lower-level HTTP tests](../../dpdk-net-test/tests/http_auto_echo_test.rs)
- [Axum Integration Design](Axum.md)
- [DpdkApp Design](App.md)
