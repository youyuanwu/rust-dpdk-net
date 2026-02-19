# Axum Integration Design

This document describes the design for integrating `dpdk-net` with [axum](https://github.com/tokio-rs/axum), the popular async web framework built on top of hyper and tokio.

The axum integration is built **on top of `DpdkApp`** (see [App.md](App.md)). `DpdkApp` handles EAL lcores, queues, reactors, and shutdown; the axum layer adds only HTTP serving.

## Implementation Status

✅ **Implemented** in `dpdk-net-axum` crate:
- `serve()` function — accepts `TcpListener`, `Router`
- Uses `AutoBuilder` from hyper-util with `LocalExecutor` from `dpdk-net-util`
- Bridges axum's tower `Service` to hyper via `TowerToHyperService`
- Auto-detects HTTP/1.1 and HTTP/2 (cleartext h2c)
- Re-exported from `dpdk_net_axum::serve`

🔲 **Not yet implemented:**
- `serve_with_config()` / `ServeConfig` (HTTP/2 tuning, max connections)

✅ **Tests:**
- `dpdk-net-axum/tests/axum_client_test.rs` — axum server + `DpdkHttpClient` GET requests on same lcore
- `dpdk-net-axum/tests/app_echo_test.rs` — raw TCP echo test for `DpdkApp`

---

## Design

**Key insight:** We bypass `axum::serve()` entirely because it requires `Send` streams. Instead we use hyper-util's `AutoBuilder` with a `LocalExecutor` (from `dpdk-net-util`) that spawns tasks via `tokio::task::spawn_local`.

| Constraint | dpdk-net | Standard axum |
|------------|----------|---------------|
| Thread model | `spawn_local` (single-threaded) | `spawn` (multi-threaded) |
| Socket type | `!Send` (uses `Rc<RefCell>`) | `Send` |
| Executor | `LocalExecutor` | `TokioExecutor` |
| Listener | Custom `TcpListener` | `TcpListener` from tokio |

### How It Works

1. `serve()` runs an accept loop on `listener.accept()`
2. Each accepted stream is wrapped: `TokioIo::new(TokioTcpStream::new(stream))`
3. `Router` is cloned per connection and wrapped with `TowerToHyperService` to bridge tower's `Service` to hyper's `Service`
4. `AutoBuilder::new(LocalExecutor).serve_connection(io, service)` handles HTTP/1.1 or HTTP/2

**Key detail:** `serve_connection()` requires `I: Read + Write + Unpin + 'static` but does **not** require `Send` on `I`. Only `serve_connection_with_upgrades()` requires `Send`. This is why our `!Send` streams work.

See: [serve.rs](../../dpdk-net-axum/src/serve.rs)

---

## API

```rust
/// Serve an axum Router on a dpdk-net TcpListener.
/// Runs until the `shutdown` future completes.
pub async fn serve(listener: TcpListener, app: Router, shutdown: impl Future<Output = ()>);
```

### Usage

```rust
DpdkApp::new()
    .eth_dev(0)
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(move |ctx: WorkerContext| {
        let app = app.clone();
        async move {
            let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
            serve(listener, app, std::future::pending::<()>()).await;
        }
    });
```

---

## Limitations

1. **Cannot use `axum::serve()` directly** — requires `Send` bounds. Use `dpdk_net_axum::serve()` instead.
2. **Single-threaded per lcore** — connections are pinned to the lcore that received the SYN (via RSS hash).
3. **Shared state must be `Send + Sync`** — each lcore is a separate OS thread. Use `Arc<AtomicU64>`, not `Rc<Cell<_>>`.
4. **No WebSocket upgrade with task migration** — WebSocket connections stay on the same lcore.
5. **Some middleware may not compile** — if it requires `Send` on response bodies. Standard extractors (`Json`, `Query`, `Path`, `State`) work fine.

---

## Module Structure

```
dpdk-net-util/
├── src/
│   ├── app.rs         # DpdkApp builder and run logic
│   └── context.rs     # WorkerContext definition

dpdk-net-axum/
├── src/
│   ├── lib.rs         # Re-exports DpdkApp, WorkerContext from dpdk-net-util; exports serve
│   └── serve.rs       # serve()
└── tests/
    ├── app_echo_test.rs       # Raw TCP echo test
    └── axum_client_test.rs    # Axum server + HTTP client test
```

No `tower` dependency needed — `hyper_util::service::TowerToHyperService` handles the bridging.

---

## Feature Comparison

| Feature | `axum::serve` | `dpdk_net_axum::serve` |
|---------|---------------|------------------------|
| Router support | ✅ | ✅ |
| Middleware | ✅ | ✅ |
| State extraction | ✅ | ✅ |
| HTTP/1.1 | ✅ | ✅ |
| HTTP/2 (h2c) | ✅ | ✅ |
| Graceful shutdown | ✅ | ✅ (worker closure returns) |
| Multi-threaded | ✅ | ❌ (single-threaded per lcore) |
| `Send` streams | Required | Not required |
| Cross-thread task spawn | ✅ | ❌ |

---

## References

- [DpdkApp Design](App.md)
- [HTTP Client Design](Client.md)
- [Implementation: serve.rs](../../dpdk-net-axum/src/serve.rs)
- [Test: axum_client_test.rs](../../dpdk-net-axum/tests/axum_client_test.rs)
- [axum source - serve.rs](https://github.com/tokio-rs/axum/blob/main/axum/src/serve.rs)
- [hyper-util AutoBuilder](https://docs.rs/hyper-util/latest/hyper_util/server/conn/auto/struct.Builder.html)
- [hyper-util TowerToHyperService](https://docs.rs/hyper-util/latest/hyper_util/service/struct.TowerToHyperService.html)
- [Lower-level hyper integration](../../dpdk-net-test/src/app/http_server.rs)
