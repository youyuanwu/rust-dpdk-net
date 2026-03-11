# OS Thread TCP Bridge

Allow non-DPDK OS threads to use TCP streams that are transparently proxied through DPDK lcore workers.

Crate: [`dpdk-net-util`](../../dpdk-net-util/src/bridge/)
Tests: [`bridge_stream_test.rs`](../../dpdk-net-test/tests/bridge_stream_test.rs), [`bridge_listener_test.rs`](../../dpdk-net-test/tests/bridge_listener_test.rs)

## Problem

The DPDK networking stack is `!Send` (`ReactorHandle`, `TcpStream`, `TcpListener` all hold `Rc<RefCell<ReactorInner>>`). OS threads cannot touch these types directly.

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│  OS Thread(s)                                                        │
│                                                                      │
│   BridgeTcpStream (Send)                                              │
│     ├── poll_write(buf) ─► data_tx: PollSender<Bytes>                │
│     ├── poll_read(buf)  ◄─ data_rx: mpsc::Receiver<Result<Bytes>>    │
│     └── poll_close()    ─► close_tx: oneshot::Sender<()>             │
│                        │                          ▲                   │
│                        │ mpsc (bounded)            │ mpsc (bounded)   │
│                        ▼                          │                   │
├──────────────────────────────────────────────────────────────────────┤
│  DPDK Lcore Thread (LocalSet + current_thread runtime)               │
│                                                                      │
│   bridge_worker (spawn_local via BridgeWorkers::spawn)                 │
│     ├── recv BridgeCommand::Connect { addr, port, reply_tx }         │
│     └── recv BridgeCommand::Listen { port, reply_tx }                 │
│                                                                      │
│   relay_task (per-connection, spawn_local)                            │
│     ├── rx_from_os.recv() ─► dpdk_stream.send()                      │
│     ├── dpdk_stream.recv() ─► tx_to_os.send()                        │
│     └── close_rx           ─► dpdk_stream.close()                    │
│                                                                      │
│   Reactor ◄──► DpdkDevice ◄──► NIC                                  │
└──────────────────────────────────────────────────────────────────────┘
```

All DPDK socket operations stay on the lcore. The bridge worker is a `spawn_local` task that owns the real `TcpStream` and relays data through `tokio::sync::mpsc` channels (`Send`).

## API

### `DpdkBridge` — `Send + Sync` handle factory

Created before `DpdkApp::run()` via `DpdkBridge::pair()`. The `DpdkBridge` half goes to OS threads; the `BridgeWorkers` half is captured in the `run()` closure.

```rust
impl DpdkBridge {
    pub fn pair() -> (DpdkBridge, BridgeWorkers);
    pub async fn connect(&self, remote_addr: IpAddress, remote_port: u16) -> Result<BridgeTcpStream, BridgeError>;
    pub async fn listen(&self, port: u16) -> Result<BridgeTcpListener, BridgeError>;
    pub async fn wait_ready(&self); // blocks until ≥1 lcore worker registered
}
```

Internally holds `Arc<WorkerRegistry>` — an `ArcSwap<Vec<mpsc::Sender<BridgeCommand>>>` with `AtomicUsize` round-robin counter and `Notify` for readiness signaling.

### `BridgeWorkers` — lcore-side factory

`Send + Sync`, captured in the `DpdkApp::run()` closure. Each lcore calls `spawn()` to register itself.

```rust
impl BridgeWorkers {
    /// Spawn a bridge worker on the current lcore's LocalSet.
    pub fn spawn(&self, reactor: &ReactorHandle);
}
```

### `BridgeTcpStream` — `Send`, `!Sync` async stream

Implements `futures_io::AsyncRead + AsyncWrite`, consistent with the lcore-side `TcpStream`. Bridge to tokio traits via `tokio_util::compat::FuturesAsyncReadCompatExt`:

```rust
use tokio_util::compat::FuturesAsyncReadCompatExt;
let tokio_stream = bridge_stream.compat(); // → tokio::io::AsyncRead + AsyncWrite
```

Uses `PollSender<Bytes>` for `poll_write`, `mpsc::Receiver` for `poll_read` (with a `read_buf: Bytes` for partial chunk buffering), and `oneshot::Sender` for `poll_close`.

### `BridgeTcpListener` — `Send`, `!Sync`

```rust
impl BridgeTcpListener {
    pub async fn accept(&mut self) -> Result<BridgeTcpStream, BridgeError>;
}
```

### `BridgeError`

```rust
pub enum BridgeError {
    Disconnected,                                // lcore shut down / channel closed
    ConnectionFailed,                            // TCP handshake failed
    Io(io::Error),                               // underlying stream error
    Connect(smoltcp::socket::tcp::ConnectError), // smoltcp connect error
    Listen(smoltcp::socket::tcp::ListenError),   // smoltcp listen error
}
```

Bidirectional `From` conversion with `io::Error`.

## Usage

```rust
// 1. Create the bridge pair before run() blocks
let (bridge, bridge_workers) = DpdkBridge::pair();

// 2. Hand DpdkBridge to OS threads
let handle = bridge.clone();
std::thread::spawn(move || {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        handle.wait_ready().await;
        let stream = handle.connect(IpAddress::v4(10, 0, 0, 2), 8080).await.unwrap();
        // futures_io::AsyncRead + AsyncWrite, or .compat() for tokio traits
    });
});

// 3. Pass BridgeWorkers into the run() closure
DpdkApp::new()
    .eth_dev(0)
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(move |ctx| {
        let bridge_workers = bridge_workers.clone();
        async move {
            bridge_workers.spawn(&ctx.reactor);
            // ... normal server code runs alongside bridge
        }
    });
```

## Design Decisions

| Concern | Decision |
|---------|----------|
| **No new `DpdkApp` methods** | `run()` stays unchanged; bridge is opt-in composition |
| **Dynamic registration** | Lcores register at runtime via `spawn()` — no upfront lcore count needed |
| **`ArcSwap` for registry** | Lock-free reads on the hot path; writes only at startup (one per lcore) |
| **`futures_io` traits** | Consistent with lcore-side `TcpStream`; bridges to tokio via `.compat()` |
| **Round-robin queue selection** | Simple, even distribution. Hash-based selection deferred to [Future.md](Future.md) §4.1 |

## Backpressure

Three bounded queues provide natural backpressure:

1. **Write channel** (OS → lcore, 256): `poll_write` returns `Pending` via `PollSender::poll_reserve`
2. **Read channel** (lcore → OS, 256): relay task blocks on `tx_to_os.send()`, applying TCP window pressure
3. **DPDK TX ring**: NIC saturation → TCP retransmit handles recovery

| Channel | Capacity | Purpose |
|---------|----------|---------|
| Command | 1024 | Pending connect/listen requests |
| Data (per direction) | 256 | Per-connection read/write |
| Accept | 64 | Pending accepted connections |

TCP socket buffer sizes: 64 KB RX, 64 KB TX.

## Lifecycle

```
Main Thread              OS Thread                Lcore Thread
───────────              ─────────                ────────────
pair() → (bridge, workers)
  │
clone bridge ───────► held by OS thread
  │
app.run(workers)─────────────────────► workers.spawn(&reactor)
  │ (blocks)                           │ registers channel
  │                                    │ spawns bridge_worker
  │                │                   │
  │                wait_ready()
  │                  │
  │                bridge.connect(addr, port)
  │                  │
  │                  ├─► BridgeCommand::Connect ─► bridge_worker recv
  │                  │                          TcpStream::connect()
  │                  │                          wait_connected()
  │                  │◄─ BridgeStreamChannels ◄─ reply_tx.send()
  │                  │                         spawn_local(relay_task)
  │                  │
  │                stream.write / read / close
  │                  │
  │                  ├─► data_tx ──────────────► relay_task → dpdk_stream
  │                  │◄─ data_rx ◄────────────── relay_task ← dpdk_stream
  │                  │
  │                  ├─► close_tx ─────────────► relay_task → stream.close()
  │
run() returns
```

## Drop Behavior

Dropping `BridgeTcpStream` without calling `poll_close`:

1. `data_tx` drops → `rx_from_os.recv()` returns `None` → relay task exits
2. `TcpStream` dropped → RST sent (abrupt close)

This matches standard TCP semantics. For graceful shutdown (FIN), call `poll_close` before dropping.

## Limitations

1. **Extra copy**: one memcpy per direction across the channel boundary (~1-5µs per hop)
2. **No zero-copy**: relay copies between `Bytes` and DPDK mbuf pool
3. **Lcore-direct is faster**: if code can run on the lcore via `DpdkApp::run`, skip the bridge
4. **Ephemeral port allocation**: simple sequential allocator (49152–65535), no reuse tracking

## Alternatives Considered

| Alternative | Why not |
|-------------|---------|
| **`Arc<Mutex<ReactorInner>>`** | Reactor polls at ~1M iter/sec; mutex contention destroys throughput |
| **`io_uring`-style submission queue** | More complex, marginal benefit over `tokio::sync::mpsc` (lock-free internally) |
| **OS threads writing directly to DPDK mbufs** | Only eliminates TX copy; requires unsafe cross-thread mbuf management |

## File Layout

```
dpdk-net-util/src/bridge/
├── mod.rs          // re-exports
├── handle.rs       // DpdkBridge, WorkerRegistry
├── stream.rs       // BridgeTcpStream (AsyncRead + AsyncWrite)
├── listener.rs     // BridgeTcpListener
├── worker.rs       // BridgeWorkers, bridge_worker, relay_task, accept_loop, EphemeralPorts
├── command.rs      // BridgeCommand, BridgeStreamChannels
└── error.rs        // BridgeError
```

## Dependencies

| Crate | Use |
|-------|-----|
| `arc-swap` | Lock-free `WorkerRegistry` |
| `bytes` | Zero-copy slicing for channel data |
| `futures-io` | `AsyncRead + AsyncWrite` traits |
| `tokio` (`sync`, `macros`) | `mpsc`, `oneshot`, `Notify`, `select!` |
| `tokio-util` | `PollSender` for poll-based writes |
