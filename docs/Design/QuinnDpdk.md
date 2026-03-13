# Quinn (QUIC) on DPDK-Net UDP

Thin adapter running [Quinn](https://github.com/quinn-rs/quinn) QUIC over DPDK, bypassing the kernel UDP path.

Module: `dpdk-net-util::quinn` — depends on [`OsThreadBridge`](OsThreadBridge.md) (`BridgeUdpSocket`, `DpdkBridge`).

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│  OS Thread (Quinn)                                                    │
│                                                                      │
│   quinn::Endpoint                                                    │
│     └── DpdkQuinnSocket (impl AsyncUdpSocket)                        │
│           ├── try_send(Transmit)          ─► bridge.try_send_to()    │
│           ├── create_io_poller() → DpdkUdpPoller                     │
│           │     └── poll_writable()       ─► bridge.poll_send_ready()│
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

Quinn needs `AsyncUdpSocket: Send + Sync + 'static`. DPDK-net's `UdpSocket` is `!Send`. The bridge already solves this — this module just translates between Quinn's `Transmit`/`RecvMeta` types and the bridge's `send_to`/`recv_from` API.

## Key Types

| Type | Role |
|------|------|
| `DpdkQuinnRuntime` | `impl quinn::Runtime` — timers/spawn via tokio, `endpoint()` constructor |
| `DpdkQuinnSocket` | `impl AsyncUdpSocket` — wraps `Arc<BridgeUdpSocket>` |
| `DpdkUdpPoller` | `impl UdpPoller` — delegates to `bridge.poll_send_ready()` |

`wrap_udp_socket()` returns `Unsupported` — use `DpdkQuinnRuntime::endpoint()` instead, which calls `Endpoint::new_with_abstract_socket()` to bypass it.

Source: `dpdk-net-util/src/quinn/{mod.rs, runtime.rs, socket.rs}`

## Usage

```rust
let (bridge, bridge_workers) = DpdkBridge::pair();
let quinn_rt = DpdkQuinnRuntime::new(bridge);

// OS thread
std::thread::spawn(move || {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        quinn_rt.bridge.wait_ready().await;

        // Server
        let server = quinn_rt
            .endpoint(EndpointConfig::default(), Some(server_config), 4433)
            .await.unwrap();

        // Client
        let mut client = quinn_rt
            .endpoint(EndpointConfig::default(), None, 4434)
            .await.unwrap();
        client.set_default_client_config(client_config);

        let conn = client.connect(server_addr, "localhost")?.await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(b"hello QUIC over DPDK").await?;
        send.finish()?;
        let response = recv.read_to_end(1024).await?;
    });
});

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
| Custom socket | `Endpoint::new_with_abstract_socket()` bypasses `wrap_udp_socket` |
| No new bridge code | Reuses OsThreadBridge entirely |
| No GSO/GRO | `max_transmit_segments() = 1`, `max_receive_segments() = 1` |
| No ECN | Quinn degrades to loss-based congestion detection |
| `may_fragment() = false` | No kernel IP stack; QUIC handles path MTU discovery |
| Backpressure | TX channel full → `WouldBlock` → Quinn congestion control throttles; RX full → dropped → QUIC loss detection |

## Limitations

1. No ECN, GSO/GRO — single datagram per channel message
2. Single lcore per socket (no multi-queue RSS)
3. IPv4 only (smoltcp)
4. One memcpy per datagram per direction (negligible vs TLS crypto)

## Future Work

- ECN passthrough from IP header
- GSO/GRO at DPDK mbuf level
- Multi-queue RSS across lcores
- Zero-copy TX via mbuf pool integration
