# Other DPDK Rust Projects

- [rust-dpdk](https://github.com/ANLAB-KAIST/rust-dpdk)
- [rust-dpdk](https://github.com/codilime/rust-dpdk)
- [rust-dpdk](https://github.com/flier/rust-dpdk)
- [dpdk](https://github.com/lemonrock/dpdk)
- [rust-dpdk](https://github.com/libpnet/rust-dpdk)
- [capsule](https://github.com/capsule-rs/capsule)
- [rpkt](https://github.com/duanjp8617/rpkt)
- [async-dpdk](https://github.com/datenlord/async-dpdk)

---

## Project Comparison Summary

| Project | Stars | Approach | Threading | Async | Last Active |
|---------|-------|----------|-----------|-------|-------------|
| **capsule** | 435 | Framework | Run-to-completion | No | 2021 |
| **ANLAB-KAIST/rust-dpdk** | 134 | Bindgen | DPDK lcores | No | 2024 |
| **codilime/rust-dpdk** | 101 | Bindgen (fork) | DPDK lcores | No | 2021 |
| **flier/rust-dpdk** | 80 | Bindgen snapshot | DPDK lcores | No | 2018 |
| **libpnet/rust-dpdk** | 50 | Basic bindings | N/A | No | 2013 |
| **lemonrock/dpdk** | 47 | Comprehensive | Multi-core (256) | No | 2017 |
| **rpkt** | 45 | Packet parsing lib | Agnostic | No | Active |
| **async-dpdk** | 11 | Async wrapper | Tokio runtime | Yes | 2022 |

---

## Detailed Analysis

### 1. capsule-rs/capsule (Most Featured)

**Architecture:**
- Framework-level abstraction inspired by [NetBricks](https://www.usenix.org/system/files/conference/osdi16/osdi16-panda.pdf)
- Provides a declarative programming model with batch combinators
- Modular design: `batch`, `config`, `metrics`, `net`, `packets` modules

**Threading Model:**
- **Run-to-completion model**: Each core processes packets from receive to transmit
- Uses `PortQueue` to pair RX/TX queues with specific cores
- Pipeline execution is core-affine (no cross-core packet sharing)

**API Design:**
- Rich type system for packets with compile-time safety guarantees
- Combinator-based batch processing: `filter`, `map`, `group_by`, `emit`
- TOML-based configuration for runtime setup
- `Mbuf` abstraction wraps DPDK message buffers
- Procedural macros for testing (`#[capsule::test]`) and benchmarking (`#[capsule::bench]`)

**Key Features:**
- Memory-safe and thread-safe packet manipulation
- KNI (Kernel NIC Interface) support
- PCAP dump capability for debugging
- Metrics collection built-in
- Examples: NAT64, ping daemon, SYN flood

**Limitations:**
- Built on DPDK 19.11 (dated)
- Not actively maintained (last update 2021)

---

### 2. ANLAB-KAIST/rust-dpdk

**Architecture:**
- Low-level bindings using `bindgen` for FFI generation
- On-demand bindings creation (not snapshot-based)
- Statically links DPDK libraries

**Threading Model:**
- Standard DPDK lcore model
- Uses `lcore::foreach_slave()` for distributing work
- `launch::remote_launch()` for spawning tasks on lcores

**API Design:**
- Minimal wrapper approach - stays close to DPDK C API
- Uses `RTE_SDK` environment variable for DPDK path discovery
- `dpdk-sys` crate for raw bindings

**Key Features:**
- Automatically regenerates bindings for different DPDK versions
- Clean separation between sys bindings and higher-level abstractions
- Actively maintained (tested with DPDK v22.11)

---

### 3. codilime/rust-dpdk

**Architecture:**
- Fork of ANLAB-KAIST with additional features
- Focus on performance validation (includes C vs Rust benchmarks)

**Threading Model:**
- Inherits DPDK lcore model from ANLAB-KAIST
- Run-to-completion processing

**API Design:**
- Higher-level API additions to ANLAB-KAIST base
- Designed to hide non-obvious DPDK dependencies
- Includes packet library comparisons (`pkt_perf`)

**Key Features:**
- L2 forwarding example with performance fixes
- Performance benchmarks against C implementation
- BSD-3-Clause license

---

### 4. flier/rust-dpdk

**Architecture:**
- Bindgen snapshot approach (pre-generated bindings committed)
- Split into `rte`, `rte-sys`, and `rte-build` crates

**Threading Model:**
- Standard DPDK lcore iteration
- `lcore::foreach_slave()` pattern for task distribution
- `launch::mp_wait_lcore()` for synchronization

**API Design:**
- Idiomatic Rust wrappers over raw FFI
- `AsResult` trait for error handling
- `extern "C" fn` callbacks for lcore functions

**Key Features:**
- Older but simpler approach
- Good as a reference implementation
- Not actively maintained (last update 2018)

---

### 5. lemonrock/dpdk

**Architecture:**
- Comprehensive binding targeting high scalability
- Claims support for 256 cores and 32 NICs
- Workspace-based multi-crate structure

**Threading Model:**
- Designed for massively parallel deployments
- High-performance userspace networking focus

**API Design:**
- Includes TCP/UDP/VLAN/ICMPv4/ICMPv6 packet types
- Memory pool (`mempool`) abstractions
- Ethernet port management

**Key Features:**
- AGPL-3.0 license (restrictive)
- Most ambitious scope but abandoned (2017)

---

### 6. libpnet/rust-dpdk

**Architecture:**
- Earliest Rust DPDK binding (12 years old)
- Basic bindgen-generated FFI bindings

**API Design:**
- Minimal wrapper, mostly raw bindings
- Makefile-based build system

**Status:**
- Historical interest only, not usable with modern DPDK

---

### 7. rpkt (Active Development)

**Architecture:**
- **Packet parsing library** (not full DPDK wrapper)
- `no_std` compatible for bare-metal/embedded use
- Separate `rpkt` (parsing) and `rpkt-dpdk` (DPDK integration) crates

**Threading Model:**
- Agnostic - library provides packet types, not threading

**API Design:**
- `Buf`, `PktBuf`, `PktBufMut` traits for buffer abstraction
- `Cursor`/`CursorMut` for zero-copy packet traversal
- DSL-based code generation for protocol definitions (`pktfmt`)
- Comprehensive protocol support: Ethernet, IPv4/IPv6, TCP, UDP, GRE, GTPv1/v2, MPLS, VXLAN, PPPoE, etc.

**Key Features:**
- Modern, actively maintained
- Protocol-focused without runtime coupling
- Apache-2.0 license

---

### 8. async-dpdk (Datenlord)

**Architecture:**
- **Async Rust wrapper** for DPDK
- Integrates with Tokio runtime (`current_thread`)
- Agent-based design for packet handling

**Threading Model:**
- Single-threaded Tokio runtime per core
- Async/await semantics for packet processing
- Non-blocking I/O model

**API Design:**
- Modules: `eal`, `lcore`, `mbuf`, `mempool`, `net_dev`, `packet`
- Protocol parsing: `proto` module for network protocols
- Strict lint configuration for safety

**Key Features:**
- Only async DPDK implementation
- GPL-3.0 license
- IP fragmentation/reassembly support
- Suitable for async network applications

---

## Design Pattern Comparison

### Binding Generation Approaches

| Approach | Projects | Pros | Cons |
|----------|----------|------|------|
| **On-demand bindgen** | ANLAB-KAIST, codilime | Always current, version flexible | Build time, requires DPDK installed |
| **Snapshot bindings** | flier, libpnet | Fast builds | Stale, manual updates |
| **Framework abstraction** | capsule | High-level, safe | Less flexible, DPDK version locked |

### Memory Management

| Project | Approach |
|---------|----------|
| **capsule** | `Mbuf` wrapper with ownership semantics |
| **rpkt** | `PktBuf`/`PktBufMut` traits, cursor-based access |
| **async-dpdk** | `mbuf` module with async-aware allocation |
| **others** | Thin wrappers over `rte_mbuf` |

### Threading Models

1. **Run-to-completion (capsule, codilime)**: Each core handles packets from RX to TX independently
2. **DPDK lcore model (ANLAB-KAIST, flier)**: Traditional `rte_eal_remote_launch` pattern
3. **Async runtime (async-dpdk)**: Tokio integration with cooperative scheduling
4. **Library-agnostic (rpkt)**: No threading model, pure packet parsing

---

## Recommendations

| Use Case | Recommended Project |
|----------|-------------------|
| **High-level NF development** | capsule (if DPDK 19.11 acceptable) |
| **Modern DPDK versions** | ANLAB-KAIST/rust-dpdk |
| **Async network apps** | async-dpdk |
| **Packet parsing only** | rpkt |
| **Learning/reference** | flier/rust-dpdk |

