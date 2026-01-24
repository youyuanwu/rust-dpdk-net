# Internals

## Multi-Queue ARP Problem

With TCP RSS (Receive Side Scaling), packets are distributed across queues based on 5-tuple hash:

```
TCP packets → hashed by (src_ip, dst_ip, src_port, dst_port) → distributed to queues 0..N
ARP packets → NOT matched by TCP RSS → always go to queue 0
```

### The Problem

Each queue has its own smoltcp interface with its own ARP cache. When a queue needs to send a packet:

1. Queue N receives TCP SYN from client
2. Queue N needs client's MAC to send SYN-ACK
3. Queue N sends ARP request
4. Client's ARP reply arrives at **queue 0** (not queue N!)
5. Queue N never gets the MAC → timeout

This causes:
- ARP storm: All queues independently sending ARP requests for the same IP
- Connection failures: SYN-ACK can't be sent without MAC address

### The Solution: Shared ARP Cache (SPMC Pattern)

We use a **Single Producer, Multi Consumer** lock-free cache:

```
┌─────────────────────────────────────────────────────────────────┐
│                    SharedArpCache                               │
│              (lock-free via arc-swap)                           │
│                                                                 │
│  Reads:  Single atomic load (no contention)                     │
│  Writes: Copy-on-write + atomic store (queue 0 only)            │
└─────────────────────────────────────────────────────────────────┘
                    ▲                          │
                    │ insert()                 │ snapshot()
                    │                          ▼
┌───────────────────┴───────────┐   ┌─────────────────────────────┐
│           Queue 0             │   │      Queues 1..N            │
│   (SPMC Producer)             │   │   (SPMC Consumers)          │
│                               │   │                             │
│ 1. poll_rx() gets ARP reply   │   │ 1. Check cache for new IPs  │
│ 2. parse_arp_reply() → IP/MAC │   │ 2. Build ARP reply packet   │
│ 3. cache.insert(ip, mac)      │   │ 3. Inject into smoltcp      │
└───────────────────────────────┘   └─────────────────────────────┘
```

### How It Works

1. **Queue 0 receives all ARP replies** (not matched by TCP RSS)
2. **Queue 0 parses ARP and updates shared cache** (lock-free write)
3. **All queues poll the cache** on each `poll_rx()` (lock-free read)
4. **New entries are injected** as fake ARP replies into each queue's smoltcp

### Implementation

```rust
// In DpdkDevice::poll_rx()

// Queue 0: Update shared cache from received ARP replies
if self.queue_id == 0 {
    for mbuf in &self.rx_batch {
        if let Some((ip, mac)) = parse_arp_reply(mbuf.data()) {
            self.shared_arp_cache.insert(ip, mac);  // Lock-free
        }
    }
}

// All queues: Inject new cache entries into smoltcp
let snapshot = self.shared_arp_cache.snapshot();  // Lock-free
for (ip, mac) in snapshot.iter() {
    if !self.injected_ips.contains(ip) {
        let arp_packet = build_arp_reply_for_injection(...);
        self.rx_batch.push(arp_packet);
        self.injected_ips.push(ip);
    }
}
```

### Performance

- **No startup injection needed** - ARP is learned dynamically
- **Lock-free reads** - single atomic load per poll cycle
- **Minimal writes** - only on new ARP replies (rare)
- **No contention** - SPMC pattern, single writer

### Files

- `dpdk-net/src/tcp/arp_cache.rs` - SharedArpCache implementation
- `dpdk-net/src/tcp/dpdk_device.rs` - Integration with DpdkDevice