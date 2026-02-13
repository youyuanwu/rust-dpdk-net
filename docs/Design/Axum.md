# Axum Integration Design

This document describes the design for integrating `dpdk-net` with [axum](https://github.com/tokio-rs/axum), the popular async web framework built on top of hyper and tokio.

---

## Overview

Use axum's `Router` as a `tower::Service` and serve it with hyper directly, using a `LocalExecutor` to handle `!Send` futures.

**Key insight:** We bypass `axum::serve()` entirely and use hyper-util's `AutoBuilder` with our custom executor.

---

## Background: The `!Send` Constraint

dpdk-net's sockets use `Rc<RefCell<...>>` internally for zero-copy access to the reactor state, making them `!Send`.

| Constraint | dpdk-net | Standard axum |
|------------|----------|---------------|
| Thread model | `spawn_local` (single-threaded) | `spawn` (multi-threaded) |
| Socket type | `!Send` (uses `Rc<RefCell>`) | `Send` |
| Executor | `LocalExecutor` | `TokioExecutor` |
| Listener | Custom `TcpListener` | `TcpListener` from tokio |

Standard `axum::serve()` requires `Send` bounds that our types cannot satisfy. The solution is to serve axum's `Router` via hyper directly.

---

## Design

### LocalExecutor

```rust
/// Executor for !Send futures that uses spawn_local.
#[derive(Clone, Copy)]
pub struct LocalExecutor;

impl<F> hyper::rt::Executor<F> for LocalExecutor
where
    F: std::future::Future + 'static,  // Note: no Send bound
    F::Output: 'static,
{
    fn execute(&self, fut: F) {
        tokio::task::spawn_local(fut);
    }
}
```

### serve() Function

```rust
/// Serve an axum Router on a dpdk-net TcpListener.
/// 
/// The `shutdown` future completes when the server should stop accepting connections.
pub async fn serve<F>(
    mut listener: TcpListener,
    app: Router,
    shutdown: F,
)
where
    F: Future<Output = ()> + 'static,
{
    let make_service = app.into_make_service();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            result = listener.accept() => {
                match result {
                    Ok(stream) => {
                        let service = make_service.clone();
                        let io = TokioIo::new(TokioTcpStream::new(stream));
                        
                        tokio::task::spawn_local(async move {
                            let service = tower::MakeService::call(&mut service.clone(), ())
                                .await
                                .expect("infallible");
                            
                            let _ = AutoBuilder::new(LocalExecutor)
                                .serve_connection(io, service)
                                .await;
                        });
                    }
                    Err(e) => {
                        tracing::error!(error = ?e, "Accept failed");
                    }
                }
            }
        }
    }
}
```

---

## Limitations

### 1. Cannot Use `axum::serve()` Directly

```rust
// ❌ This won't work - requires Send bounds
axum::serve(listener, app).await;

// ✅ Use our serve() instead
dpdk_net_axum::serve(listener, app, shutdown_signal).await;
```

### 2. Single-Threaded Per Queue

Each queue runs on a dedicated thread with its own tokio `LocalSet`. Connections on a queue cannot migrate to other threads.

```
Queue 0 Thread: [Reactor] → [LocalSet] → [Connections A, B, C]
Queue 1 Thread: [Reactor] → [LocalSet] → [Connections D, E, F]
```

**Implication:** Connection handling is pinned to the queue that received the initial SYN packet (determined by RSS hash).

### 3. State Must Be `Send + Sync` for Multi-Queue

When using `DpdkServerRunner` with multiple queues, shared state must be thread-safe:

```rust
// ✅ Good - Arc<AtomicU64> is Send + Sync
let counter = Arc::new(AtomicU64::new(0));
let app = Router::new()
    .route("/", get(handler))
    .with_state(counter);

// ❌ Bad - Rc is !Send
let counter = Rc::new(Cell::new(0));
```

### 4. No WebSocket Upgrade with Task Migration

WebSocket connections stay on the same thread as the HTTP upgrade. They cannot be moved to a dedicated WebSocket thread pool.

### 5. Extractors That Require `Send`

Some axum extractors or middleware may require `Send` bounds on the response body. These won't work:

```rust
// May not compile if middleware requires Send bodies
app.layer(SomeMiddlewareThatRequiresSend)
```

Most standard extractors (`Json`, `Query`, `Path`, `State`) work fine.

---

## API Surface

```rust
/// Serve an axum Router on a dpdk-net TcpListener.
///
/// The `shutdown` future completes when the server should stop.
pub async fn serve<F>(
    listener: TcpListener,
    app: Router,
    shutdown: F,
)
where
    F: Future<Output = ()> + 'static;

/// Serve with additional configuration.
pub async fn serve_with_config<F>(
    listener: TcpListener,
    app: Router,
    shutdown: F,
    config: ServeConfig,
)
where
    F: Future<Output = ()> + 'static;

/// Configuration for the HTTP server.
pub struct ServeConfig {
    pub http2: Http2Config,
    pub max_connections: Option<usize>,
}

pub struct Http2Config {
    pub max_concurrent_streams: Option<u32>,
    pub initial_stream_window_size: Option<u32>,
    pub initial_connection_window_size: Option<u32>,
}
```

---

## Usage with DpdkServerRunner

```rust
use axum::{Router, routing::get};
use dpdk_net_axum::serve;
use dpdk_net_test::app::dpdk_server_runner::DpdkServerRunner;

async fn hello() -> &'static str {
    "Hello from DPDK + Axum!"
}

fn main() {
    // ... EAL init, hugepages setup ...
    
    let app = Router::new()
        .route("/", get(hello))
        .route("/health", get(|| async { "OK" }));
    
    DpdkServerRunner::new("eth1")
        .with_default_network_config()
        .with_default_hw_queues()
        .port(8080)
        .run(move |ctx| {
            let app = app.clone();
            async move {
                serve(ctx.listener, app, ctx.cancel.cancelled()).await
            }
        });
}
```

---

## Module Structure

```
dpdk-net-axum/
├── Cargo.toml
└── src/
    ├── lib.rs         # serve(), ServeConfig, re-exports
    ├── executor.rs    # LocalExecutor
    └── serve.rs       # Serve implementation
```

### Dependencies

```toml
[dependencies]
axum = { version = "0.8" }
dpdk-net = { path = "../dpdk-net" }
hyper = { version = "1.0" }
hyper-util = { version = "0.1", features = ["server-auto"] }
tokio = { version = "1", features = ["rt", "macros"] }
tower = { version = "0.5" }
tracing = "0.1"
```

---

## Feature Comparison

| Feature | `axum::serve` | `dpdk_net_axum::serve` |
|---------|---------------|------------------------|
| Router support | ✅ | ✅ |
| Middleware | ✅ | ✅ |
| State extraction | ✅ | ✅ |
| HTTP/1.1 | ✅ | ✅ |
| HTTP/2 (h2c) | ✅ | ✅ |
| Graceful shutdown | ✅ | ✅ (via shutdown future) |
| Multi-threaded | ✅ | ❌ (single-threaded per queue) |
| `Send` streams | Required | Not required |
| Cross-thread task spawn | ✅ | ❌ |

---

## References

- [axum source - serve.rs](https://github.com/tokio-rs/axum/blob/main/axum/src/serve.rs)
- [hyper-util AutoBuilder](https://docs.rs/hyper-util/latest/hyper_util/server/conn/auto/struct.Builder.html)
- [tower Service trait](https://docs.rs/tower/latest/tower/trait.Service.html)
- [Current hyper integration](../../dpdk-net-test/src/app/http_server.rs)
