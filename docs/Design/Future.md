# Future Improvements

This document outlines potential enhancements for `dpdk-net` based on analysis of other Rust DPDK projects and the current architecture's limitations.

---

## Priority 1: High-Impact Features

### 1.1 Hardware Offloads

**Current State:** All packet processing is software-only via smoltcp.

**Gap:** Modern NICs support offloading checksum, segmentation, and coalescing to hardware, which can significantly reduce CPU cycles per packet.

**Improvements:**

| Offload | Benefit | Implementation Effort |
|---------|---------|---------------------|
| **TX Checksum Offload** | Eliminate CPU checksum calculation on transmit | Medium |
| **RX Checksum Offload** | Validate checksums in hardware, skip in smoltcp | Medium |
| **TCP Segmentation Offload (TSO)** | Send large payloads; NIC segments | High |
| **Large Receive Offload (LRO)** | Coalesce packets in hardware | Medium |

**Implementation Notes:**
- The `EthConf` already has `offloads: u64` fields for `RxMode`/`TxMode`
- Need to set `RTE_ETH_TX_OFFLOAD_*` flags and configure mbuf offload fields
- smoltcp may need modifications to skip checksum computation when offload is enabled

**Reference:** lemonrock/dpdk attempted comprehensive offload support

---

### 1.2 Metrics & Observability

**Current State:** No built-in metrics; only ad-hoc `tracing` logs.

**Gap:** Production deployments need visibility into queue depths, packet rates, connection counts, and latency distributions.

**Improvements:**

```rust
pub struct ReactorMetrics {
    pub rx_packets: Counter,
    pub tx_packets: Counter,
    pub rx_bytes: Counter,
    pub tx_bytes: Counter,
    pub active_connections: Gauge,
    pub dropped_packets: Counter,
    pub poll_latency_us: Histogram,
}
```

**Options:**
1. **Feature-gated metrics** (like capsule's `metrics` feature)
2. **Prometheus exposition** via `prometheus` or `metrics` crates
3. **Per-queue stats** accessible via `rte_eth_stats_get()`

**Reference:** Capsule has a complete `metrics` module with port, mempool, and pipeline metrics.

---

### 1.3 Connection Migration / Cross-Queue Handling

**Current State:** Packets arriving on the wrong queue (due to RSS imperfections or asymmetric routing) are dropped or mishandled.

**Gap:** No mechanism to forward packets between queues.

**Improvements:**

1. **Lock-free SPMC queues** for cross-queue packet forwarding
2. **Connection affinity lookup** table (connection → queue mapping)
3. **Flow Director rules** to pin specific connections to specific queues

```rust
// Cross-queue forwarding channel
pub struct CrossQueueForwarder {
    // One channel per destination queue
    channels: Vec<crossbeam::channel::Sender<Mbuf>>,
}
```

**Trade-off:** Adds latency for misrouted packets but prevents connection breakage.

---

## Priority 2: Protocol Extensions

### 2.1 IPv6 Support

**Current State:** RSS constants for IPv6 exist; smoltcp supports IPv6; untested in multi-queue.

**Improvements:**
- Test IPv6 with RSS distribution
- Verify NDP (Neighbor Discovery Protocol) works with `SharedArpCache` equivalent
- Add IPv6 address configuration to `DpdkApp`

---

### 2.2 TLS Integration

**Current State:** Raw TCP only; no encryption.

**Gap:** Most production applications require TLS.

**Improvements:**

```rust
// Option 1: rustls integration
pub struct TlsTcpStream {
    inner: TokioTcpStream,
    tls: rustls::StreamOwned<...>,
}

// Option 2: Provide TokioTcpStream for external integration
let stream = TokioTcpStream::new(tcp_stream);
let tls_stream = tokio_rustls::TlsAcceptor::accept(stream).await?;
```

Since `TokioTcpStream` already implements `AsyncRead`/`AsyncWrite`, external TLS is already possible. Document the pattern and add examples.

---

### 2.3 UDP Improvements

**Current State:** Basic `UdpSocket` with `send_to`/`recv_from`.

**Gap:** Missing features for high-performance UDP services:
- **Multicast** support
- **GSO (Generic Segmentation Offload)** for large UDP payloads
- **recvmmsg/sendmmsg** batch semantics
- **Connected UDP** for stateless load balancing

---

## Priority 3: API & Developer Experience

### 3.1 Batch Processing Combinators (Capsule-style)

**Current State:** Manual packet loop in reactor.

**Gap:** Capsule provides declarative combinators for pipeline construction.

**Potential API:**

```rust
// Capsule-inspired batch combinators
pipeline
    .receive()
    .filter(|pkt| pkt.is_tcp())
    .map(|pkt| process(pkt))
    .group_by(|pkt| pkt.flow_hash(), compose! {
        0 => handle_queue_0,
        _ => handle_other,
    })
    .emit();
```

**Decision:** Evaluate if this abstraction is worth the complexity. The current model (smoltcp handles parsing) may be simpler for TCP-focused use cases.

---

### 3.2 Configuration File Support

**Current State:** Programmatic configuration only.

**Gap:** Capsule supports TOML-based configuration for runtime settings. SPDK has a comprehensive JSON-based subsystem configuration that auto-allocates devices and threads. DPDK has neither.

**SPDK-style vs DPDK:**

| Feature | SPDK | DPDK | dpdk-net |
|---------|------|------|----------|
| Config format | JSON | CLI args | Programmatic |
| Auto-create threads | Yes | Partial (`-l`) | Yes (DpdkApp) |
| Auto-probe devices | Yes | No | Partial |
| Device-to-queue mapping | Config-driven | Manual | Automatic |

**Improvements:**

```toml
# dpdk-net.toml
[dpdk]
eal_args = ["-l", "0-3", "--no-huge"]
port = "eth0"

[network]
ip = "10.0.0.10/24"
gateway = "10.0.0.1"
mtu = 1500

[queues]
count = 4
rx_desc = 1024
tx_desc = 1024

[tcp]
rx_buffer_size = 65536
tx_buffer_size = 65536
```

**Alternative:** JSON config for SPDK compatibility:
```json
{
  "subsystems": [
    {
      "subsystem": "dpdk_net",
      "config": {
        "ports": [{"name": "eth0", "queues": 4}]
      }
    }
  ]
}
```

---

### 3.3 Procedural Macros for Testing

**Current State:** Manual test harness setup.

**Reference:** Capsule provides `#[capsule::test]` and `#[capsule::bench]` macros.

**Improvements:**

```rust
#[dpdk_net::test]
async fn test_tcp_echo() {
    let (ctx, handle) = test_context().await;
    // ... test code
}
```

---

## Priority 4: Advanced DPDK Features

### 4.0 Lcore API Wrappers ✅ DONE

**Status:** Implemented in [dpdk-net/src/api/rte/lcore.rs](../../dpdk-net/src/api/rte/lcore.rs)

**Design:** See [LcoreAPI.md](LcoreAPI.md)

**Implemented Features:**
- `Lcore` struct with `id()`, `socket_id()`, `role()`, `state()`, `cpu_id()`
- Constructors: `from_id()`, `current()`, `main()`
- Iterators: `all()`, `workers()`
- Launch: `launch()`, `wait()`, `run()`
- Bulk ops: `wait_all_workers()`, `launch_on_workers()`
- `LaunchBuilder` with NUMA filtering
- `LaunchHandle` for non-blocking wait

**Tests:** 9 integration tests in [dpdk-net-test/tests/lcore_test.rs](../../dpdk-net-test/tests/lcore_test.rs)

---

### 4.0.1 SPDK-style Application Framework

**Current State:** `DpdkApp` provides automatic multi-queue setup, but requires programmatic configuration.

**Gap:** SPDK provides a declarative application framework where:
- Threads are auto-created from config
- Devices are auto-probed and assigned
- Subsystems register themselves
- `spdk_app_start()` does everything

**DPDK has no equivalent.** You must manually:
1. Call `rte_eal_init()` with CLI args
2. Probe devices via `rte_eth_dev_*` APIs
3. Create lcores via `rte_eal_remote_launch()` or spawn threads
4. Map queues to threads yourself

**Potential High-Level App Framework:**

```rust
// Config-driven initialization (SPDK-like)
let app = DpdkApp::from_config("dpdk-net.toml")?;

// Auto-creates threads, probes devices, sets up queues
app.run(|ctx| async {
    // ctx.port - configured port
    // ctx.queue_id - assigned queue
    // ctx.listener - TCP listener (if configured)
    my_server(ctx).await
})?;
```

**Implementation would wrap:**
- EAL init with parsed config
- Device enumeration and configuration
- Queue-to-thread assignment based on core count
- RSS/RETA automatic configuration

---

### 4.1 Flow Director / rte_flow API

**Current State:** Only RSS for packet distribution.

**Gap:** Flow Director allows hardware-based steering of specific flows to specific queues.

**Use Cases:**
- Pin high-priority connections to dedicated queues
- Steer control plane traffic to queue 0
- Implement per-connection QoS

**Implementation:**
```rust
pub struct FlowRule {
    pub pattern: FlowPattern,  // Match on 5-tuple, VLAN, etc.
    pub action: FlowAction,    // Queue, drop, mark, etc.
}

impl EthDev {
    pub fn flow_create(&self, rule: &FlowRule) -> Result<FlowHandle>;
    pub fn flow_destroy(&self, handle: FlowHandle) -> Result<()>;
}
```

---

### 4.2 DPDK Event Device (Interrupts)

**Current State:** Pure poll mode; 100% CPU even when idle.

**Gap:** Event devices allow hybrid interrupt/poll modes for power efficiency.

**Trade-off:** Adds latency when transitioning from interrupt to poll mode.

**Use Case:** Development/testing where power consumption matters.

---

### 4.3 Mempool Per-Queue Isolation

**Current State:** Single shared mempool across all queues.

**Gap:** Shared mempool requires atomic operations; can become a contention point.

**Improvement:** Per-queue mempool with local cache optimization.

```rust
pub struct PerQueueMempool {
    local: MemPool,           // Queue-local pool
    shared: Arc<MemPool>,     // Fallback when local exhausted
}
```

---

## Priority 5: Ecosystem Integration

### 5.1 Async Runtime Abstraction

**Current State:** Tokio-specific with `TokioRuntime` trait.

**Gap:** Some users prefer `async-std`, `smol`, or `glommio`.

**Improvements:**
- Complete the `Runtime` trait abstraction
- Add `async-std` and `smol` implementations
- Consider `glommio` for io_uring integration

---

### 5.2 no_std Core Library

**Current State:** Requires std.

**Reference:** rpkt is `no_std` compatible for embedded use.

**Use Case:** Bare-metal or embedded DPDK deployments (e.g., custom NICs).

---

### 5.3 Packet Parsing Library Integration

**Current State:** smoltcp handles parsing internally.

**Gap:** For custom protocols or deep packet inspection, a dedicated parsing library may be useful.

**Options:**
- Integrate [rpkt](https://github.com/duanjp8617/rpkt) for zero-copy protocol parsing
- Use `etherparse` for simpler cases
- Expose raw mbuf access for custom parsing

---

## Comparison Summary: What Others Have

| Feature | capsule | ANLAB-KAIST | async-dpdk | rpkt | **dpdk-net** |
|---------|---------|-------------|------------|------|--------------|
| Async Runtime | No | No | Tokio | No | **Tokio** |
| TCP Stack | No (raw) | No | No | No | **smoltcp** |
| Multi-queue RSS | Manual | Manual | Manual | N/A | **Automatic** |
| Lcore APIs | No | **Yes** | Partial | N/A | **Yes** ✅ |
| Batch Combinators | **Yes** | No | No | No | No |
| Metrics | **Yes** | No | No | No | No |
| HW Offloads | Partial | No | No | N/A | No |
| Flow Director | No | No | No | N/A | No |
| TOML Config | **Yes** | No | No | N/A | No |
| Test Macros | **Yes** | No | No | No | No |
| IPv6 Tested | Unknown | Unknown | Unknown | **Yes** | No |
| Protocol Parsing | Internal | N/A | Internal | **Comprehensive** | smoltcp |
| License | Apache-2.0 | BSD-3 | GPL-3.0 | Apache-2.0 | TBD |

---

## Recommended Roadmap

### Phase 1: Production Hardening (Near-term)
1. ✅ TCP checksum offload (TX/RX)
2. ✅ Basic metrics (rx/tx counters, connection count)
3. ✅ IPv6 testing and documentation

### Phase 2: Performance Optimization (Medium-term)
4. TSO/LRO for high-throughput workloads
5. Per-queue mempool isolation
6. Flow Director for connection pinning

### Phase 3: Developer Experience (Longer-term)
7. TOML configuration support
8. Test/bench procedural macros
9. TLS integration examples

### Phase 4: Advanced Features (Future)
10. Batch combinators (if demand exists)
11. Event device for hybrid interrupt/poll
12. Cross-queue connection migration
