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
│   (axum, tonic gRPC, hyper, TcpStream, TcpListener)                 │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                       Framework Layer                               │
│   dpdk-net-axum (serve) │ dpdk-net-tonic (serve, channel)           │
│   dpdk-net-util (DpdkApp, WorkerContext, HTTP client)               │
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

- **Async/await support** - Custom `TcpListener`, `TcpStream` APIs compatible with tokio
- **axum integration** - Serve axum `Router` directly on DPDK sockets (`dpdk-net-axum`)
- **tonic gRPC** - gRPC server and client over DPDK (`dpdk-net-tonic`)
- **HTTP client** - `DpdkHttpClient` for HTTP/1.1 and HTTP/2 requests (`dpdk-net-util`)
- **Multi-queue scaling** - RSS (Receive Side Scaling) distributes connections across CPU cores
- **DpdkApp framework** - Lcore-based application runner with per-queue smoltcp stacks
- **CPU affinity** - Worker threads pinned to cores for optimal cache locality

## Crates

| Crate | Description |
|-------|-------------|
| `dpdk-net` | Core library: DPDK wrappers, smoltcp integration, async TCP sockets |
| `dpdk-net-sys` | FFI bindings to DPDK C library (generated via bindgen) |
| `dpdk-net-util` | `DpdkApp`, `WorkerContext`, HTTP client, `LocalExecutor` |
| `dpdk-net-axum` | Axum web framework integration (`serve()`) |
| `dpdk-net-tonic` | Tonic gRPC integration (server `serve()` + `DpdkGrpcChannel` client) |
| `dpdk-net-test` | Test harness, example servers, integration tests |

## Documentation
- [Architecture](docs/Architecture.md) - Crate structure and implementation details
- [Design](docs/Design/) - Design docs for DpdkApp, Axum, Tonic, HTTP Client
- [Benchmarks](docs/Bench/Benchmark.md) - Performance comparison with tokio on Azure
- [Limitations](docs/Limitations.md) - Known limitations and constraints

## Requirements

- Linux with hugepages configured
- DPDK-compatible NIC (Intel, Mellanox, etc.) or virtual device for testing
- Root privileges (for DPDK memory and device access)

## Getting Started

### Install DPDK

From package manager or build from source:

```sh
cmake -S . -B build
cmake --build build --target dpdk_configure
cmake --build build --target dpdk_build --parallel
sudo cmake --build build --target dpdk_install
```

### Axum HTTP Server

```rust
use dpdk_net_axum::{DpdkApp, WorkerContext, serve};
use dpdk_net::socket::TcpListener;
use axum::{Router, routing::get};
use smoltcp::wire::Ipv4Address;

fn main() {
    // Initialize EAL (e.g., via EalBuilder)
    // ...

    let app = Router::new().route("/", get(|| async { "Hello from DPDK!" }));

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
}
```

### Tonic gRPC Server

```rust
use dpdk_net_tonic::serve;
use dpdk_net::socket::TcpListener;

// Inside DpdkApp::run() closure:
let greeter = GreeterServer::new(MyGreeter::default());
let routes = tonic::service::Routes::new(greeter);
let listener = TcpListener::bind(&ctx.reactor, 50051, 4096, 4096).unwrap();
serve(listener, routes, std::future::pending::<()>()).await;
```

## Project Status

⚠️ **APIs are unstable and subject to change.**

This project is under active development. The core functionality works, but the API surface is evolving.

## References
[References](./docs/References/OtherProjects.md)

## License

MIT
