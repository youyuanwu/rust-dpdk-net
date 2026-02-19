# dpdk-net Architecture

This document describes the architecture of the `dpdk-net` crate, a Rust library that provides high-level async TCP/IP networking using [DPDK](https://www.dpdk.org/) for kernel-bypass packet I/O and [smoltcp](https://github.com/smoltcp-rs/smoltcp) for the TCP/IP stack.

## Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Application Layer                            │
│   (axum, tonic gRPC, hyper, TcpStream, TcpListener)                 │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    Framework Layer                                  │
│   dpdk-net-axum (serve) │ dpdk-net-tonic (serve, channel)           │
│   dpdk-net-util (DpdkApp, WorkerContext, HTTP client)               │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    Async Runtime Layer (tokio)                      │
│   ┌─────────────────────────────────────────────────────────────┐   │
│   │  Reactor  │  TokioTcpStream  │  TcpRecvFuture/TcpSendFuture │   │
│   └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      TCP/IP Stack (smoltcp)                         │
│       Interface │ SocketSet │ NeighborCache │ Routes                │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    DPDK Device Adapter Layer                        │
│         DpdkDevice (implements smoltcp::phy::Device)        │
│                   SharedArpCache (multi-queue)                      │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     DPDK RTE API Wrappers                           │
│   EAL │ EthDev │ MemPool │ Mbuf │ RxQueue/TxQueue │ RSS             │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│               dpdk-net-sys (FFI Bindings via bindgen)               │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                           DPDK C Library                            │
│                    (libdpdk, mlx5 PMD, etc.)                        │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         Hardware NIC                                │
│                (Azure accelerated networking, etc.)                 │
└─────────────────────────────────────────────────────────────────────┘
```

## Crate Structure

The project is organized into six crates:

### 1. `dpdk-net-sys` - FFI Bindings

Low-level Rust bindings to DPDK C library, generated via `bindgen`.

- **[build.rs](../dpdk-net-sys/build.rs)** - Generates bindings using `pkg-config` for DPDK discovery
- **[wrapper.h](../dpdk-net-sys/include/wrapper.h)** - C header with inline wrapper functions
- **[wrapper.c](../dpdk-net-sys/src/wrapper.c)** - C helper functions for complex macros

Key bindings include:
- `rte_eal_init` / `rte_eal_cleanup` - EAL lifecycle
- `rte_pktmbuf_*` - Packet buffer operations
- `rte_eth_*` - Ethernet device configuration
- RSS constants (`RUST_RTE_ETH_RSS_*`) for multi-queue distribution

### 2. `dpdk-net` - Core Library

The main library providing safe Rust abstractions over DPDK and smoltcp integration.

#### Module: `api::rte` - DPDK Runtime Environment Wrappers

| File | Purpose |
|------|---------|
| [eal.rs](../dpdk-net/src/api/rte/eal.rs) | EAL initialization builder (`EalBuilder`) with options like `--vdev`, `--no-huge`, `--allow` |
| [eth.rs](../dpdk-net/src/api/rte/eth.rs) | Ethernet device configuration (`EthDevBuilder`, `EthConf`), RSS setup, queue configuration |
| [pktmbuf.rs](../dpdk-net/src/api/rte/pktmbuf.rs) | Memory pool management (`MemPool`, `MemPoolConfig`) |
| [mbuf.rs](../dpdk-net/src/api/rte/mbuf.rs) | Packet buffer wrapper (`Mbuf`) with RAII and safe data access |
| [queue.rs](../dpdk-net/src/api/rte/queue.rs) | RX/TX queue handles (`RxQueue`, `TxQueue`) with burst operations |
| [thread.rs](../dpdk-net/src/api/rte/thread.rs) | Thread registration (`ThreadRegistration`) and CPU affinity (`set_cpu_affinity`) |

#### Module: `tcp` - TCP Stack Integration

| File | Purpose |
|------|---------|
| [dpdk_device.rs](../dpdk-net/src/tcp/dpdk_device.rs) | `DpdkDevice` - smoltcp `Device` trait implementation |
| [arp_cache.rs](../dpdk-net/src/tcp/arp_cache.rs) | `SharedArpCache` - Lock-free SPMC ARP cache for multi-queue |
| [async_net/mod.rs](../dpdk-net/src/tcp/async_net/mod.rs) | `Reactor` - Async polling loop driving smoltcp |
| [async_net/socket.rs](../dpdk-net/src/tcp/async_net/socket.rs) | `TcpStream`, `TcpListener` - Async TCP sockets |
| [async_net/tokio_compat.rs](../dpdk-net/src/tcp/async_net/tokio_compat.rs) | `TokioTcpStream` - Tokio `AsyncRead`/`AsyncWrite` adapter |

### 3. `dpdk-net-util` - Application Framework & HTTP Client

Shared utilities: `DpdkApp` (lcore-based application runner), `WorkerContext`, HTTP client, and `LocalExecutor`.

| File | Purpose |
|------|---------|---|
| [app.rs](../dpdk-net-util/src/app.rs) | `DpdkApp` - Builder and multi-lcore runner |
| [context.rs](../dpdk-net-util/src/context.rs) | `WorkerContext` - Per-lcore context (reactor, queue_id, etc.) |
| [client.rs](../dpdk-net-util/src/client.rs) | `DpdkHttpClient` - High-level HTTP client |
| [connection.rs](../dpdk-net-util/src/connection.rs) | `Connection` - Persistent HTTP/1.1 or HTTP/2 connection |
| [pool.rs](../dpdk-net-util/src/pool.rs) | `ConnectionPool` - Per-host connection reuse |
| [executor.rs](../dpdk-net-util/src/executor.rs) | `LocalExecutor` - `!Send` executor for hyper |

Design: [App.md](Design/App.md), [Client.md](Design/Client.md)

### 4. `dpdk-net-axum` - Axum Web Framework Integration

Serves axum `Router` on dpdk-net sockets, bypassing `axum::serve()` (which requires `Send`).

| File | Purpose |
|------|---------|---|
| [serve.rs](../dpdk-net-axum/src/serve.rs) | `serve()` - Accept loop with `AutoBuilder` + `LocalExecutor` |

Re-exports `DpdkApp`, `WorkerContext` from `dpdk-net-util`.  
Design: [Axum.md](Design/Axum.md)

### 5. `dpdk-net-tonic` - Tonic gRPC Integration

gRPC server and client for dpdk-net, built on top of `dpdk-net-axum`.

| File | Purpose |
|------|---------|---|
| [serve.rs](../dpdk-net-tonic/src/serve.rs) | `serve()` - Routes → axum Router → `dpdk_net_axum::serve()` |
| [channel.rs](../dpdk-net-tonic/src/channel.rs) | `DpdkGrpcChannel` - `!Send` gRPC client channel over HTTP/2 |

Design: [Tonic.md](Design/Tonic.md)

### 6. `dpdk-net-test` - Test Harness & Examples

Testing infrastructure and example servers.

| File | Purpose |
|------|---------|
| [dpdk_test.rs](../dpdk-net-test/src/dpdk_test.rs) | `DpdkTestContext` / `create_test_context()` - Test harness for virtual devices |
| [app/echo_server.rs](../dpdk-net-test/src/app/echo_server.rs) | TCP echo server implementation |
| [app/http_server.rs](../dpdk-net-test/src/app/http_server.rs) | HTTP/1.1 and HTTP/2 servers using hyper |
| [app/tokio_server.rs](../dpdk-net-test/src/app/tokio_server.rs) | Standard tokio HTTP servers for benchmarking comparison |

---

## Core Components

### DpdkDevice

The bridge between DPDK and smoltcp. Implements `smoltcp::phy::Device` to provide packet I/O.

```rust
pub struct DpdkDevice {
    rxq: RxQueue,              // DPDK receive queue
    txq: TxQueue,              // DPDK transmit queue
    mempool: Arc<MemPool>,     // Shared packet buffer pool
    rx_batch: ArrayVec<Mbuf, 64>,  // Buffered received packets
    tx_batch: ArrayVec<Mbuf, 64>,  // Buffered packets to transmit
    shared_arp_cache: Option<SharedArpCache>,  // Multi-queue ARP sharing
    // ...
}
```

**Key operations:**
- `receive()` - Returns `(RxToken, TxToken)` pair for smoltcp to consume
- `transmit()` - Allocates mbuf for smoltcp to fill with outgoing packet
- `inject_rx_packet()` - Injects fake packets (used for ARP pre-population)

**RX/TX Batching Strategy:**
- **RX**: Drain-then-refill pattern. Only polls hardware when `rx_batch` is empty.
  This minimizes DPDK API calls and improves cache locality.
- **TX**: Non-blocking flush. Attempts to send once per poll cycle without spinning.
  If the hardware TX ring is full, packets remain in `tx_batch` for the next cycle.
  This prevents TX backpressure from blocking RX (which would cause packet drops).

### Reactor

The async polling loop that drives network I/O. Uses cooperative scheduling with tokio.

```rust
impl Reactor<DpdkDevice> {
    pub async fn run_with<R: Runtime>(self, batch_size: usize) -> ! {
        loop {
            let timestamp = Instant::now();
            let mut packets_processed = 0;

            // Process ingress in batches
            loop {
                match inner.poll_ingress_single(timestamp) {
                    PollIngressSingleResult::None => break,
                    _ => {
                        packets_processed += 1;
                        if packets_processed >= batch_size {
                            // Hit batch limit - break to run egress
                            break;
                        }
                    }
                }
            }

            // Transmit queued packets (ACKs, responses)
            inner.poll_egress(timestamp);

            // Cleanup orphaned sockets
            inner.cleanup_orphaned();

            // Yield to let other async tasks run
            R::yield_now().await;
        }
    }
}
```

**Why continuous polling?** DPDK is poll-based, not interrupt-driven. Unlike kernel networking where `epoll` waits for interrupts, DPDK requires active polling to check for new packets.

### TcpStream / TcpListener

Async TCP sockets using smoltcp's TCP implementation.

```rust
// Connect to a remote host
let stream = TcpStream::connect(&handle, remote_addr, remote_port, local_port, 4096, 4096)?;
stream.wait_connected().await?;

// Send/receive data
stream.send(&data).await?;
let n = stream.recv(&mut buf).await?;
```

**Waker Integration:** When a socket operation would block, smoltcp registers the waker:
```rust
// In TcpRecvFuture::poll()
socket.register_recv_waker(cx.waker());  // smoltcp will wake us when data arrives
```

### TokioTcpStream

Adapter that implements `tokio::io::AsyncRead` and `tokio::io::AsyncWrite`, enabling use with:
- `hyper` for HTTP servers
- `tokio-util` codecs
- Any tokio async I/O utility

```rust
let stream = TokioTcpStream::new(tcp_stream);
let io = TokioIo::new(stream);  // For hyper
let (sender, conn) = http1::handshake(io).await?;
```

---

## Polling Strategy & DoS Avoidance

The reactor's polling loop is carefully designed to balance throughput, latency, and fairness.

### Batch Processing

Packets are processed in configurable batches (default: 32) before yielding:

```
┌─────────────────────────────────────────────────────────────────┐
│                      Reactor Loop Iteration                     │
├─────────────────────────────────────────────────────────────────┤
│  1. poll_ingress_single() × N    (up to batch_size packets)     │
│  2. poll_egress()                (send ACKs, responses)         │
│  3. cleanup_orphaned()           (remove closed sockets)        │
│  4. yield_now()                  (let application tasks run)    │
└─────────────────────────────────────────────────────────────────┘
```

### DoS Prevention: Ingress/Egress Balance

**Problem:** Under a packet flood, if we only process ingress, we would:
- Fill up socket receive buffers
- Never send ACKs (egress starved)
- Cause TCP connections to break

**Solution:** Break from ingress loop after `batch_size` packets to run egress:

```rust
if packets_processed >= batch_size {
    // Hit batch limit - break to run egress before yielding
    // This prevents DoS: we must send ACKs/responses, not just receive
    break;
}
```

This ensures that even during a flood:
1. We receive up to 32 packets
2. We send all queued ACKs/responses (egress)
3. We yield to application handlers
4. Repeat

### Yield Strategy

The reactor **always yields** after each loop iteration:

| Scenario | Behavior |
|----------|----------|
| No packets (idle) | ingress returns None → egress → yield |
| Light traffic (1-31) | ingress processes all → egress → yield |
| Heavy traffic (32+) | ingress hits batch limit → egress → yield |

**Why always yield?** The reactor runs in a tokio `LocalSet` alongside application tasks (accept handlers, recv/send futures). Without yielding, those tasks would never be polled:

```
┌─────────────────────────────────────────────────────────────────┐
│                    tokio LocalSet                               │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐           │
│  │   Reactor    │  │   Accept     │  │   HTTP       │           │
│  │  (polling)   │  │   Handler    │  │   Handler    │           │
│  └──────────────┘  └──────────────┘  └──────────────┘           │
│         │                 ▲                 ▲                   │
│         └─── yield_now() ─┴─────────────────┘                   │
│              (gives other tasks a chance to run)                │
└─────────────────────────────────────────────────────────────────┘
```

### Configurable Batch Size

The batch size controls the tradeoff between throughput and responsiveness:

| Batch Size | Throughput | Latency | Use Case |
|------------|------------|---------|----------|
| 1-8        | Lower      | Best    | Latency-critical applications |
| 16-32      | Balanced   | Good    | Mixed workloads (default) |
| 64-128     | Higher     | Worse   | High-throughput bulk transfers |

```rust
// Custom batch size
reactor.run_with_batch_size(64).await;

// Or with explicit runtime
reactor.run_with::<TokioRuntime>(128).await;
```

---

## Multi-Queue Architecture

For high-throughput scenarios, the library supports multiple hardware queues with RSS (Receive Side Scaling).

### RSS Distribution

```
                     ┌─────────────────┐
   Incoming Packets  │   Hardware NIC  │
   ─────────────────►│  RSS Hash Unit  │
                     └────────┬────────┘
                              │ Hash on 4-tuple
                              │ (src_ip, dst_ip, src_port, dst_port)
           ┌──────────────────┼──────────────────┐
           ▼                  ▼                  ▼
    ┌──────────────┐   ┌──────────────┐   ┌──────────────┐
    │   Queue 0    │   │   Queue 1    │   │   Queue N    │
    │  (Thread 0)  │   │  (Thread 1)  │   │  (Thread N)  │
    │              │   │              │   │              │
    │  smoltcp     │   │  smoltcp     │   │  smoltcp     │
    │  Interface   │   │  Interface   │   │  Interface   │
    └──────────────┘   └──────────────┘   └──────────────┘
```

**Configuration in code:**
```rust
use dpdk_net::api::rte::eth::rss_hf;

let eth_conf = EthConf::new()
    .rss_with_hash(rss_hf::NONFRAG_IPV4_TCP | rss_hf::NONFRAG_IPV6_TCP);
```

### CPU Affinity (Thread Pinning)

Each worker thread is pinned to a specific CPU core for optimal performance:

```rust
// DpdkApp uses EAL lcores which are automatically pinned to CPUs
// Each lcore thread has CPU affinity set by DPDK's EAL initialization
```

**Why CPU affinity matters:**

| Without Affinity | With Affinity |
|------------------|---------------|
| Thread migrates between CPUs | Thread stays on one CPU |
| L1/L2/L3 cache constantly invalidated | Caches stay warm |
| Cross-NUMA memory access possible | NUMA-local access |
| ~10-30% performance loss | Optimal performance |

`DpdkApp` uses native EAL lcores, where each lcore thread is pinned via `pthread_setaffinity_np()`.

```
┌─────────────────────────────────────────────────────────────────┐
│                         NUMA Node 0                             │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐             │
│  │  CPU 0  │  │  CPU 1  │  │  CPU 2  │  │  CPU 3  │             │
│  │ Queue 0 │  │ Queue 1 │  │ Queue 2 │  │ Queue 3 │             │
│  │ Thread  │  │ Thread  │  │ Thread  │  │ Thread  │             │
│  └─────────┘  └─────────┘  └─────────┘  └─────────┘             │
│                      │                                          │
│                      ▼                                          │
│              ┌──────────────┐                                   │
│              │   NIC + DMA  │                                   │
│              │   (Queue 0-3)│                                   │
│              └──────────────┘                                   │
└─────────────────────────────────────────────────────────────────┘
```

### Shared ARP Cache Problem

**Problem:** Each queue has an independent smoltcp interface with its own ARP cache. ARP replies (not matched by TCP RSS) typically all go to queue 0. Other queues never learn the gateway MAC.

**Solution:** `SharedArpCache` with SPMC (Single Producer, Multi Consumer) pattern:

```
┌─────────────┐          ┌─────────────────────────┐
│   Queue 0   │─────────►│    SharedArpCache       │
│  (Producer) │  insert  │  (lock-free arc-swap)   │
└─────────────┘          └───────────┬─────────────┘
                                     │ load
        ┌────────────────────────────┼────────────────────────────┐
        ▼                            ▼                            ▼
┌──────────────┐            ┌──────────────┐            ┌──────────────┐
│   Queue 1    │            │   Queue 2    │            │   Queue N    │
│  (Consumer)  │            │  (Consumer)  │            │  (Consumer)  │
│ inject ARP   │            │ inject ARP   │            │ inject ARP   │
└──────────────┘            └──────────────┘            └──────────────┘
```

**Version Counter:** The cache uses a version counter (not length) to detect changes.
This ensures consumers re-inject when a MAC is updated for an existing IP (e.g., when
smoltcp's neighbor cache expires and a fresh ARP reply arrives with the same gateway IP).

```rust
// Queue 0: Scans packets for ARP replies, updates cache
if self.queue_id == 0 {
    for mbuf in &self.rx_batch {
        if let Some((ip, mac)) = parse_arp_reply(mbuf.data()) {
            cache.insert(ip, mac);
        }
    }
}

// Other queues: Inject cached entries as fake ARP replies
for (&ip, &mac) in cache.snapshot().iter() {
    let arp_packet = build_arp_reply_for_injection(our_mac, our_ip, mac, ip);
    self.rx_batch.push(mbuf_with_arp_packet);
}
```

---

## Memory Management

### Mbuf Lifecycle

```
┌──────────────────┐
│     MemPool      │  Hugepage-backed pool of packet buffers
└────────┬─────────┘
         │ alloc
         ▼
┌──────────────────┐
│      Mbuf        │  Single packet buffer (2KB + headroom)
│  ┌────────────┐  │
│  │  headroom  │  │  128 bytes reserved at front
│  ├────────────┤  │
│  │   data     │  │  Packet data (up to MTU + headers)
│  ├────────────┤  │
│  │  tailroom  │  │  Unused space at end
│  └────────────┘  │
└────────┬─────────┘
         │
    ┌────┴────┐
    ▼         ▼
 transmit   drop
 (DPDK      (returned
  frees)     to pool)
```

**Key invariants:**
- `Mbuf` is `Send` but not `Sync` (single-threaded access)
- Transmitted mbufs are owned by DPDK (use `mem::forget`)
- Dropped mbufs are returned to the pool via DPDK free

### Zero-Copy Path

```rust
impl phy::RxToken for DpdkRxToken {
    fn consume<R, F>(self, f: F) -> R
    where F: FnOnce(&[u8]) -> R
    {
        f(self.mbuf.data())  // Direct pointer into DMA buffer
    }
}
```

Smoltcp reads directly from the DPDK mbuf - no copy needed for receive path.

---

## Test Infrastructure

### create_test_context()

For manual (non-async) tests using virtual devices (no hardware required):

```rust
let (ctx, device) = create_test_context()?;
```

Creates a `net_ring0` virtual loopback device with default settings.
For async tests, use `DpdkApp` from `dpdk-net-util` instead.

### Test Files

Async tests use `DpdkApp` + `WorkerContext`. Manual tests use `create_test_context()` with raw smoltcp.

| Test | Pattern | Description |
|------|---------|-------------|
| [app_echo_test.rs](../dpdk-net-axum/tests/app_echo_test.rs) | DpdkApp | Raw TCP echo via `DpdkApp` |
| [axum_client_test.rs](../dpdk-net-axum/tests/axum_client_test.rs) | DpdkApp | Axum server + `DpdkHttpClient` |
| [tonic_grpc_test.rs](../dpdk-net-test/tests/tonic_grpc_test.rs) | DpdkApp | gRPC server + `DpdkGrpcChannel` client |
| [tcp_echo_async_test.rs](../dpdk-net-test/tests/tcp_echo_async_test.rs) | DpdkApp | Async TCP echo with `EchoServer` |
| [http_echo_test.rs](../dpdk-net-test/tests/http_echo_test.rs) | DpdkApp | HTTP/1.1 with hyper |
| [http2_echo_test.rs](../dpdk-net-test/tests/http2_echo_test.rs) | DpdkApp | HTTP/2 (h2c) with hyper |
| [http_auto_echo_test.rs](../dpdk-net-test/tests/http_auto_echo_test.rs) | DpdkApp | Auto-detect HTTP version |
| [manual_tcp_echo_test.rs](../dpdk-net-test/tests/manual_tcp_echo_test.rs) | create_test_context | Manual smoltcp polling (loopback) |
| [manual_tcp_echo_stress_test.rs](../dpdk-net-test/tests/manual_tcp_echo_stress_test.rs) | create_test_context | Manual stress test (sequential clients) |
| [udp_echo_test.rs](../dpdk-net-test/tests/udp_echo_test.rs) | Raw EAL | UDP echo test |

---

## Typical Request Flow

```
1. Application calls stream.recv(&mut buf).await
         │
         ▼
2. TcpRecvFuture::poll() called by tokio
         │
         ▼
3. Check smoltcp socket buffer for data
         │
    ┌────┴────┐
    ▼         ▼
 Data?     No data
    │         │
    │         ▼
    │    Register waker with smoltcp
    │    Return Poll::Pending
    │
    ▼
4. Return data, Poll::Ready(Ok(n))

═══════════════════════════════════════════

Meanwhile, Reactor::run() loop:

1. DPDK RxQueue::rx() polls hardware
         │
         ▼
2. DpdkDevice buffers mbufs
         │
         ▼
3. smoltcp Interface::poll_ingress_single()
   - Parses Ethernet/IP/TCP headers
   - Updates socket state
   - Copies payload to socket buffer
   - Wakes registered wakers ◄─────── This wakes our recv future!
         │
         ▼
4. smoltcp Interface::poll_egress()
   - Generates outgoing packets (ACKs, data)
         │
         ▼
5. TxQueue::tx() sends to hardware
```

---

## Configuration Constants

| Constant | Default | Purpose |
|----------|---------|---------|
| `DEFAULT_MTU` | 1500 | Maximum payload size |
| `DEFAULT_MBUF_HEADROOM` | 128 | Reserved space at front of mbuf |
| `DEFAULT_MBUF_DATA_ROOM_SIZE` | 2176 | Total mbuf data capacity |
| `DEFAULT_NUM_MBUFS` | 8191 | Mempool size (2^n - 1 optimal) |
| `DEFAULT_NB_DESC` | 1024 | Ring buffer descriptors per queue |
| `DEFAULT_INGRESS_BATCH_SIZE` | 32 | Packets processed before yield |

---

## Dependencies

| Crate | Purpose |
|-------|---------|
| `smoltcp` | User-space TCP/IP stack |
| `tokio` | Async runtime |
| `hyper` / `hyper-util` | HTTP client/server |
| `axum` | Web framework (dpdk-net-axum) |
| `tonic` / `prost` | gRPC framework + protobuf (dpdk-net-tonic) |
| `arrayvec` | Fixed-size stack-allocated vectors |
| `arc-swap` | Lock-free atomic Arc operations |
| `nix` | Unix system calls |
| `tracing` | Structured logging |

---

## Future Improvements

1. **UDP support** - Currently focused on TCP; UDP sockets partially implemented
2. **IPv6** - smoltcp supports it; needs testing with RSS
3. **Connection migration** - Handle packets arriving on wrong queue
4. **Hardware offloads** - TCP checksum offload, TSO/LRO
5. **Metrics** - Prometheus integration for queue stats
