# Limitations

This document describes the current limitations and constraints of the `dpdk-net` crate.

## smoltcp TCP Stack Limitations

### Basic Congestion Control

smoltcp implements simple congestion control, not advanced algorithms like CUBIC or BBR. This affects throughput on high-latency or lossy networks. For data center use cases with low-latency, reliable networks, this is less of an issue.

### Limited TCP Options

smoltcp does not support advanced TCP features:
- No TCP Fast Open (TFO)
- No Selective Acknowledgment (SACK)
- No Explicit Congestion Notification (ECN)
- No TCP window scaling beyond basic support

### Fixed Socket Buffers

smoltcp uses fixed-size socket buffers configured at socket creation time. This limits the number of concurrent connections that can be efficiently handled, as memory is pre-allocated rather than dynamically sized.

## Architecture Limitations

### Single-Threaded Per Queue

Each hardware queue is processed by a single thread. There is no work-stealing between queues, so uneven load distribution can occur if RSS hashing produces an imperfect distribution of connections.

### Queue 0 Dependency for ARP

Only queue 0 receives and processes ARP replies (since ARP is not matched by TCP RSS rules). Other queues depend on the `SharedArpCache` injection mechanism, which may have slight staleness before entries propagate.

### No Connection Migration

Packets that arrive on the wrong queue (due to RSS hash collisions or asymmetric routing) cannot be migrated to the correct queue. The packet is processed by the receiving queue's smoltcp instance, which may not have the connection state.

### Continuous Polling Overhead

DPDK is poll-based, not interrupt-driven. CPU cores running the reactor are always at 100% utilization even when idle. There is no power-saving mode or interrupt coalescing.

## Protocol Limitations

### TCP-Focused

The library is primarily designed for TCP workloads. UDP support is only partially implemented.

### IPv6 Untested

While smoltcp supports IPv6, the multi-queue RSS configuration has not been tested with IPv6 traffic.

### No TLS

The library provides raw TCP streams. TLS must be integrated separately using crates like `rustls` or `tokio-rustls`.

### Static IP Configuration

IP addresses must be configured statically. There is no DHCP client support.

## Deployment Limitations

### Requires Root and Hugepages

DPDK requires:
- Root privileges (or specific capabilities)
- Hugepage memory allocation
- Access to `/dev/hugepages` or equivalent

This prevents use in unprivileged containers without host configuration.

### Hardware NIC Required for Production

Production deployments require a DPDK-compatible NIC with a supported PMD (Poll Mode Driver). Virtual devices (`net_ring0`, `net_null0`) are only suitable for testing.

### No Hardware Offloads

The following hardware offloads are not implemented:
- TCP Segmentation Offload (TSO)
- Large Receive Offload (LRO)
- TCP/UDP checksum offload
- Receive Side Coalescing (RSC)

All packet processing is done in software by smoltcp.

## Known Issues

### Memory Usage

Each queue requires its own smoltcp interface with dedicated socket buffers. With many queues and connections, memory usage can grow significantly.

### Graceful Shutdown

Closing connections during shutdown may not complete TCP's FIN handshake if the reactor stops polling before the connection fully closes.

## OS Thread Bridge Limitations

### Extra Copy Overhead

The bridge relays data through `tokio::sync::mpsc` channels, adding one memcpy per direction. This adds ~1-5µs latency per hop. For maximum throughput, run code directly on lcores via `DpdkApp::run`.

### No Zero-Copy

Data is copied between `Bytes` buffers and DPDK mbufs. True zero-copy would require exposing mbuf lifetimes across threads, conflicting with DPDK's per-thread mempool caching.

### Sequential Ephemeral Ports

The bridge worker uses a simple sequential allocator (49152–65535) without reuse tracking. Under heavy churn, port exhaustion is possible.
