# dpdk-net

[![CI](https://github.com/youyuanwu/rust-dpdk-net/actions/workflows/CI.yml/badge.svg)](https://github.com/youyuanwu/rust-dpdk-net/actions/workflows/CI.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

High-level async TCP/IP networking for Rust using [DPDK](https://github.com/DPDK/dpdk) for kernel-bypass packet I/O.

## What is this?

`dpdk-net` combines three technologies to provide high-performance networking:

- **[DPDK](https://github.com/DPDK/dpdk)** - Kernel-bypass packet I/O directly to/from the NIC
- **[smoltcp](https://github.com/smoltcp-rs/smoltcp)** - User-space TCP/IP stack
- **Async runtime** - Uses rust async runtime for task scheduling

This enables building network applications (HTTP servers, proxies, etc.) that bypass the kernel network stack entirely, achieving lower latency and higher throughput.

[Benchmarks](docs/Bench/Benchmark.md) shows 2X throughput and half latency than tokio server. 

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Application Layer                            │
│   (hyper HTTP servers, TcpStream, TcpListener, custom protocols)    │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    Async Runtime Layer (tokio)                      │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      TCP/IP Stack (smoltcp)                         │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     DPDK (kernel-bypass I/O)                        │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         Hardware NIC                                │
└─────────────────────────────────────────────────────────────────────┘
```

## Features

- **Async/await support** - Custom `TcpListener`, `TcpStream` APIs (not tokio's) compatible with any async runtime
- **Multi-queue scaling** - RSS (Receive Side Scaling) distributes connections across CPU cores
- **CPU affinity** - Worker threads pinned to cores for optimal cache locality
- **hyper compatible** - Use with hyper for HTTP/1.1 and HTTP/2 servers

## Documentation
- [Architecture](docs/Architecture.md) - Implementation details.
- [Benchmarks](docs/Bench/Benchmark.md) - Performance comparison with tokio on Azure.
- [Limitations](docs/Limitations.md) - Known limitations and constraints

## Requirements

- Linux with hugepages configured
- DPDK-compatible NIC (Intel, Mellanox, etc.) or virtual device for testing
- Root privileges (for DPDK memory and device access)

## Getting Started

### 1. Install DPDK

From package manager or build from source:

```sh
# Clone this repo
cmake -S . -B build
cmake --build build --target dpdk_configure
cmake --build build --target dpdk_build --parallel
sudo cmake --build build --target dpdk_install
```

### 2. Add dependency

```toml
[dependencies]
dpdk-net = "0.1"
```

### 3. Run examples

```sh
# Build examples
cargo build --release --examples

# Run HTTP server (requires sudo and DPDK-compatible NIC)
sudo ./target/release/examples/dpdk_http_server --interface eth1
```

## Examples

- [dpdk_http_server](dpdk-net-test/examples/dpdk_http_server.rs) - HTTP server with DPDK or tokio backend
- [dpdk_tcp_server](dpdk-net-test/examples/dpdk_tcp_server.rs) - Simple TCP echo server

## Project Status

⚠️ **APIs are unstable and subject to change.**

This project is under active development. The core functionality works, but the API surface is evolving.

## References
[References](./docs/References/OtherProjects.md)

## License

MIT
