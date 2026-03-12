# OS Thread Bridge

Allow non-DPDK OS threads to use TCP streams and UDP sockets transparently proxied through DPDK lcore workers.

Crate: [`dpdk-net-util`](../../dpdk-net-util/src/bridge/)
Tests: [`bridge_stream_test`](../../dpdk-net-test/tests/bridge_stream_test.rs), [`bridge_listener_test`](../../dpdk-net-test/tests/bridge_listener_test.rs), [`bridge_udp_test`](../../dpdk-net-test/tests/bridge_udp_test.rs)

## Problem

DPDK networking types are `!Send` (they hold `Rc<RefCell<ReactorInner>>`). OS threads cannot use them directly.

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│  OS Thread(s)                                                        │
│                                                                      │
│   BridgeTcpStream        BridgeUdpSocket (Send + Sync)               │
│     ├── poll_write ─►      ├── send_to ─►                            │
│     ├── poll_read  ◄─      ├── recv_from ◄─                          │
│     └── poll_close ─►      └── local_addr()                          │
│                    │                          ▲                       │
│                    │ mpsc (bounded)            │ mpsc (bounded)       │
│                    ▼                          │                       │
├──────────────────────────────────────────────────────────────────────┤
│  DPDK Lcore (LocalSet + current_thread runtime)                      │
│                                                                      │
│   bridge_worker ← BridgeCommand::{Connect, Listen, BindUdp}         │
│   tcp_relay_task (per-connection)                                     │
│   udp_relay_task (per-port)                                           │
│                                                                      │
│   Reactor ◄──► DpdkDevice ◄──► NIC                                  │
└──────────────────────────────────────────────────────────────────────┘
```

All DPDK socket operations stay on the lcore. Relay tasks own the real `!Send` sockets and shuttle data through `tokio::sync::mpsc` channels.

## API

### `DpdkBridge` — handle factory (`Send + Sync`)

```rust
let (bridge, bridge_workers) = DpdkBridge::pair();

// OS thread side:
bridge.wait_ready().await;
bridge.connect(addr, port).await      // → BridgeTcpStream
bridge.listen(port).await              // → BridgeTcpListener
bridge.bind_udp(port).await            // → BridgeUdpSocket
```

Holds `Arc<WorkerRegistry>` — `ArcSwap<Vec<Sender<BridgeCommand>>>` with round-robin selection.

### `BridgeWorkers` — lcore-side registration

```rust
// Inside DpdkApp::run() closure:
bridge_workers.spawn(&ctx.reactor);
```

### `BridgeTcpStream` — `Send`, `!Sync`

Implements `futures_io::AsyncRead + AsyncWrite`. Use `.compat()` for tokio traits.

### `BridgeTcpListener` — `Send`, `!Sync`

```rust
listener.accept().await  // → BridgeTcpStream
```

### `BridgeUdpSocket` — `Send + Sync`

Mirrors `tokio::net::UdpSocket`. All methods take `&self`.

```rust
// Connectionless
sock.send_to(buf, addr).await          // → io::Result<usize>
sock.recv_from(buf).await              // → io::Result<(usize, SocketAddr)>
sock.try_send_to(buf, addr)            // non-blocking
sock.try_recv_from(buf)                // non-blocking

// Poll variants (for quinn / manual Future impls)
sock.poll_send_to(cx, buf, addr)
sock.poll_recv_from(cx, buf)
sock.poll_send_ready(cx)
sock.poll_recv_ready(cx)

// Connected mode
sock.connect(addr).await
sock.send(buf).await
sock.recv(buf).await
sock.poll_send(cx, buf)
sock.poll_recv(cx, buf)

// Metadata
sock.local_addr()
sock.peer_addr()
```

Internal structure:

```rust
pub struct BridgeUdpSocket {
    tx: Mutex<PollSender<UdpDatagram>>,  // OS → lcore
    rx: Mutex<RxState>,                   // lcore → OS (with peek buffer)
    local_addr: SocketAddr,
    peer_addr: Mutex<Option<SocketAddr>>,
}
```

- **RxState** has a `peeked: Option<UdpDatagram>` — `poll_recv_ready` stashes a consumed datagram that the next `poll_recv_from` / `recv_from` drains first.
- **Mutex** gives `Sync`. Uncontended in common single-task usage.
- **Connected mode** stores `peer_addr`. `recv` delivers datagrams from any source (matches tokio behavior).

### `udp_relay_task`

One relay per bound port. Owns the `!Send` `dpdk_net::UdpSocket`.

```rust
loop {
    tokio::select! {
        // Egress: OS → NIC (send errors silently dropped)
        dg = rx_from_os.recv() => { socket.send_to(&dg.payload, endpoint).await; }
        // Ingress: NIC → OS (drop on full channel via try_send)
        (len, meta) = socket.recv_from(&mut buf) => { tx_to_os.try_send(dg); }
    }
}
```

RX uses `try_send` (not `.send().await`) — datagrams drop when the OS thread falls behind, preventing one slow consumer from stalling the lcore.

### `BridgeCommand`

```rust
pub enum BridgeCommand {
    Connect { addr, port, reply_tx },
    Listen  { port, reply_tx },
    BindUdp { port, reply_tx },
}
```

### `BridgeError`

```rust
pub enum BridgeError {
    Disconnected,
    ConnectionFailed,
    Io(io::Error),
    Connect(smoltcp::socket::tcp::ConnectError),
    Listen(smoltcp::socket::tcp::ListenError),
    UdpBind(smoltcp::socket::udp::BindError),
}
```

## Usage

```rust
let (bridge, bridge_workers) = DpdkBridge::pair();

let handle = bridge.clone();
std::thread::spawn(move || {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        handle.wait_ready().await;

        // TCP
        let stream = handle.connect(IpAddress::v4(10,0,0,2), 8080).await.unwrap();

        // UDP
        let sock = handle.bind_udp(4433).await.unwrap();
        sock.send_to(b"hello", "10.0.0.2:4433".parse().unwrap()).await.unwrap();
        let mut buf = [0u8; 1500];
        let (len, src) = sock.recv_from(&mut buf).await.unwrap();
    });
});

DpdkApp::new()
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(move |ctx| {
        let workers = bridge_workers.clone();
        async move { workers.spawn(&ctx.reactor); }
    });
```

## Backpressure

| Channel | Capacity | Behavior when full |
|---------|----------|-------------------|
| Command | 1024 | Sender awaits |
| TCP data (per direction) | 256 | `poll_write`/relay blocks |
| TCP accept | 64 | Accept loop blocks |
| UDP TX (per socket) | 1024 | `poll_send_to` returns `Pending` |
| UDP RX (per socket) | 1024 | Datagrams dropped (best-effort) |

## Drop Behavior

**TCP:** Dropping `BridgeTcpStream` without `poll_close` → sender channel closes → relay exits → `TcpStream` drops → RST. Call `poll_close` for graceful FIN.

**UDP:** Dropping `BridgeUdpSocket` → sender channel closes → relay exits → `UdpSocket` drops → port freed. No close protocol needed.

## Limitations

1. One memcpy per direction across the channel boundary
2. No zero-copy (relay copies between `Bytes` and DPDK mbufs)
3. Lcore-direct code via `DpdkApp::run` is faster — use bridge only when needed
4. Simple sequential ephemeral port allocator (49152–65535), no reuse tracking
5. UDP RX drops on full channel (consistent with UDP semantics)
6. UDP TX is fire-and-forget — smoltcp send errors on the lcore are silently dropped
7. UDP datagrams exceeding MTU (~1472 bytes) are silently failed by smoltcp

## Design Decisions

| Concern | Decision |
|---------|----------|
| No new `DpdkApp` methods | Bridge is opt-in composition |
| Dynamic registration | Lcores register via `spawn()` at runtime |
| `ArcSwap` for registry | Lock-free reads; writes only at startup |
| `futures_io` traits (TCP) | Consistent with lcore-side `TcpStream` |
| tokio-like API (UDP) | Familiar API, easy quinn integration |
| `Send + Sync` UDP socket | Enables `Arc` sharing + quinn's `AsyncUdpSocket` bound |
| `&self` methods (UDP) | Matches tokio; allows multi-task sharing |
| Round-robin queue selection | Simple, even distribution |

## File Layout

```
dpdk-net-util/src/bridge/
├── mod.rs        re-exports
├── handle.rs     DpdkBridge, WorkerRegistry
├── stream.rs     BridgeTcpStream
├── listener.rs   BridgeTcpListener
├── udp.rs        BridgeUdpSocket, RxState, UdpDatagram, address helpers
├── worker.rs     BridgeWorkers, bridge_worker, relay tasks, EphemeralPorts
├── command.rs    BridgeCommand, BridgeStreamChannels
└── error.rs      BridgeError
```
