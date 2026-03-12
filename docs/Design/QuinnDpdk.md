# Quinn (QUIC) on DPDK-Net UDP

Run [Quinn](https://github.com/quinn-rs/quinn) QUIC endpoints on the DPDK userspace network stack, bypassing the kernel UDP path.

Crate: `dpdk-net-util` (proposed module: `quinn/`)
Depends on: [`OsThreadBridge`](OsThreadBridge.md) — `BridgeUdpSocket`, `DpdkBridge`

## Problem

Quinn requires `AsyncUdpSocket: Send + Sync + 'static`. DPDK-net's `UdpSocket` is `!Send` (holds `Rc<RefCell<ReactorInner>>`). The [OsThreadBridge](OsThreadBridge.md) `BridgeUdpSocket` already provides a `Send + Sync` tokio-compatible UDP socket over DPDK. This module is a thin adapter translating between Quinn's `Transmit`/`RecvMeta` types and the bridge's `send_to`/`recv_from` API.

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│  OS Thread (Quinn)                                                    │
│                                                                      │
│   quinn::Endpoint                                                    │
│     └── DpdkQuinnSocket (impl AsyncUdpSocket)                        │
│           ├── create_sender() → DpdkUdpSender                        │
│           │     └── poll_send(Transmit) ─► bridge.poll_send_to()     │
│           ├── poll_recv(bufs, meta)      ◄─ bridge.poll_recv_from()  │
│           └── local_addr()               → bridge.local_addr()       │
│                          │                                           │
│                          │  wraps Arc<BridgeUdpSocket>               │
│                          ▼                                           │
│   BridgeUdpSocket ── mpsc channels ──► lcore udp_relay_task          │
├──────────────────────────────────────────────────────────────────────┤
│  DPDK Lcore                                                          │
│   udp_relay_task ◄──► dpdk_net::UdpSocket ◄──► Reactor ◄──► NIC     │
└──────────────────────────────────────────────────────────────────────┘
```

## API

### `DpdkQuinnRuntime` — `impl quinn::Runtime`

```rust
#[derive(Debug, Clone)]
pub struct DpdkQuinnRuntime {
    bridge: DpdkBridge,
}

impl quinn::Runtime for DpdkQuinnRuntime {
    fn new_timer(&self, i: Instant) -> Pin<Box<dyn AsyncTimer>> {
        Box::pin(tokio::time::sleep_until(i.into()))
    }

    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        tokio::spawn(future);
    }

    fn wrap_udp_socket(&self, _sock: std::net::UdpSocket) -> io::Result<Box<dyn AsyncUdpSocket>> {
        Err(io::Error::new(io::ErrorKind::Unsupported,
            "use DpdkQuinnRuntime::endpoint() instead"))
    }

    fn now(&self) -> Instant { Instant::now() }
}
```

`wrap_udp_socket` is unsupported — Quinn hands us an OS socket we don't want. Instead, use the `endpoint()` constructor which bypasses it via `Endpoint::new_with_abstract_socket()`:

```rust
impl DpdkQuinnRuntime {
    pub async fn endpoint(
        &self,
        config: EndpointConfig,
        server_config: Option<ServerConfig>,
        port: u16,
    ) -> Result<quinn::Endpoint, BridgeError> {
        let bridge_socket = self.bridge.bind_udp(port).await?;
        let quinn_socket = Box::new(DpdkQuinnSocket::new(bridge_socket));
        let runtime = Arc::new(self.clone());
        Ok(quinn::Endpoint::new_with_abstract_socket(
            config, server_config, quinn_socket, runtime,
        )?)
    }
}
```

### `DpdkQuinnSocket` — `impl AsyncUdpSocket`

```rust
#[derive(Debug)]
pub struct DpdkQuinnSocket {
    inner: Arc<BridgeUdpSocket>,
}

impl AsyncUdpSocket for DpdkQuinnSocket {
    fn create_sender(&self) -> Pin<Box<dyn UdpSender>> {
        Box::pin(DpdkUdpSender { socket: self.inner.clone() })
    }

    fn poll_recv(
        &self, cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>], meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let mut read_buf = ReadBuf::new(&mut bufs[0]);
        match self.inner.poll_recv_from(cx, &mut read_buf) {
            Poll::Ready(Ok(addr)) => {
                let len = read_buf.filled().len();
                meta[0] = RecvMeta {
                    addr, len, stride: len, ecn: None,
                    dst_ip: self.inner.local_addr().ok().map(|a| a.ip()),
                    interface_index: None,
                };
                Poll::Ready(Ok(1))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> { self.inner.local_addr() }
    fn max_receive_segments(&self) -> usize { 1 }  // no GRO
    fn may_fragment(&self) -> bool { false }        // no kernel fragmentation
}
```

### `DpdkUdpSender` — `impl UdpSender`

```rust
#[derive(Debug)]
pub struct DpdkUdpSender {
    socket: Arc<BridgeUdpSocket>,
}

impl UdpSender for DpdkUdpSender {
    fn poll_send(
        self: Pin<&mut Self>, transmit: &Transmit<'_>, cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        self.socket.poll_send_to(cx, &transmit.contents, transmit.destination)
            .map_ok(|_| ())
    }

    fn max_transmit_segments(&self) -> usize { 1 }  // no GSO
}
```

`create_sender()` clones the `Arc<BridgeUdpSocket>` — multiple senders coexist because `BridgeUdpSocket` is `Send + Sync` with `&self` methods.

## Usage

```rust
let (bridge, bridge_workers) = DpdkBridge::pair();
let quinn_runtime = DpdkQuinnRuntime::new(bridge.clone());

// OS thread — Quinn server
let rt_handle = quinn_runtime.clone();
std::thread::spawn(move || {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        rt_handle.bridge.wait_ready().await;
        let endpoint = rt_handle
            .endpoint(EndpointConfig::default(), Some(server_config()), 4433)
            .await.unwrap();

        while let Some(incoming) = endpoint.accept().await {
            let conn = incoming.await.unwrap();
            tokio::spawn(handle_connection(conn));
        }
    });
});

// OS thread — Quinn client
let endpoint = quinn_runtime
    .endpoint(EndpointConfig::default(), None, 0).await?;
let conn = endpoint.connect(server_addr, "localhost")?.await?;
let (mut send, mut recv) = conn.open_bi().await?;
send.write_all(b"hello QUIC over DPDK").await?;
send.finish()?;
let response = recv.read_to_end(1024).await?;

// Lcore
DpdkApp::new()
    .ip(Ipv4Address::new(10, 0, 0, 10))
    .gateway(Ipv4Address::new(10, 0, 0, 1))
    .run(move |ctx| {
        let workers = bridge_workers.clone();
        async move { workers.spawn(&ctx.reactor); }
    });
```

## Design Decisions

| Concern | Decision |
|---------|----------|
| Bypass `wrap_udp_socket` | `Endpoint::new_with_abstract_socket()` — designed for custom sockets |
| Reuse OsThreadBridge | No Quinn-specific bridge/relay/channel code |
| `Arc<BridgeUdpSocket>` sharing | Supports Quinn's multi-sender pattern |
| No GSO/GRO | `max_transmit_segments() = 1`, `max_receive_segments() = 1` |
| No ECN | `ecn: None` — Quinn degrades gracefully to loss-based congestion detection |
| `may_fragment() = false` | No kernel IP stack; QUIC does its own path MTU discovery |
| Timers + spawn via tokio | CPU-only operations, no NIC involvement |

## Backpressure

Handled by [OsThreadBridge](OsThreadBridge.md#backpressure) channels:

| Channel | Capacity | Effect on Quinn |
|---------|----------|----------------|
| UDP TX | 1024 | `poll_send` returns `Pending` → Quinn congestion control throttles |
| UDP RX | 1024 | Datagrams dropped when full → Quinn detects loss via QUIC |

## Limitations

1. No ECN — congestion signaling degrades to loss-based detection
2. No GSO/GRO — each datagram is a separate channel message
3. Single lcore per socket — no multi-queue RSS
4. IPv4 only (smoltcp configuration)
5. One memcpy per datagram per direction across channel boundary
6. Bridge overhead negligible vs. TLS crypto (~1-5µs per packet)

## Future Work

1. ECN passthrough from IP header
2. GSO/GRO at DPDK mbuf level
3. Multi-queue RSS across lcores
4. Zero-copy TX via mbuf pool integration

## File Layout

```
dpdk-net-util/src/quinn/
├── mod.rs       re-exports
├── runtime.rs   DpdkQuinnRuntime
├── socket.rs    DpdkQuinnSocket, DpdkUdpSender
```

Relay, channels, errors — all in `dpdk-net-util/src/bridge/` (see [OsThreadBridge](OsThreadBridge.md)).
