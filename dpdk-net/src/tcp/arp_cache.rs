//! Shared ARP cache for multi-queue DPDK setups.
//!
//! When using multiple RX queues with RSS, ARP replies may arrive on a different
//! queue than the one needing the MAC address. This module provides a shared
//! ARP cache that all queues can read from.
//!
//! # Problem
//!
//! With TCP RSS (hash on 5-tuple), ARP packets (different ethertype) typically
//! go to queue 0. But a TCP connection might be handled by queue N, which needs
//! the peer's MAC to respond. Without shared ARP, queue N will timeout waiting
//! for an ARP reply that went to queue 0.
//!
//! # Solution
//!
//! 1. Queue 0 detects ARP replies and updates the shared cache
//! 2. All queues check the shared cache when smoltcp can't find a neighbor
//! 3. Queues can inject fake ARP replies into their local smoltcp interface
//!
//! # Performance
//!
//! Uses SPMC (Single Producer, Multi Consumer) pattern:
//! - ARP packets always go to queue 0 (not matched by TCP RSS)
//! - Queue 0 is the only writer (single producer)
//! - All queues read (multiple consumers)
//!
//! Implementation uses `arc-swap` for lock-free reads:
//! - Reads: Single atomic load, no contention
//! - Writes: Clone + atomic store (single producer, no concurrent writes)

use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A MAC address (6 bytes).
pub type MacAddress = [u8; 6];

/// Thread-safe shared ARP cache using lock-free SPMC pattern.
///
/// Optimized for single-producer (queue 0) multi-consumer (all queues):
/// - Reads: Lock-free atomic load
/// - Writes: Copy-on-write with atomic store (no concurrent writer synchronization)
/// - Length: Relaxed atomic for eventual consistency (avoids Arc load on hot path)
#[derive(Clone)]
pub struct SharedArpCache {
    inner: Arc<ArcSwap<HashMap<Ipv4Addr, MacAddress>>>,
    /// Version counter that increments on every insert (even updates).
    /// Used by consumers to detect any change, including MAC updates for existing IPs.
    version: Arc<AtomicUsize>,
}

impl Default for SharedArpCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedArpCache {
    /// Create a new empty shared ARP cache.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            version: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Look up a MAC address for an IP.
    ///
    /// Lock-free: single atomic load.
    #[inline]
    pub fn get(&self, ip: &Ipv4Addr) -> Option<MacAddress> {
        self.inner.load().get(ip).copied()
    }

    /// Insert or update a MAC address for an IP.
    ///
    /// SPMC optimization: Since only queue 0 writes, we use simple
    /// copy-on-write with atomic store (no rcu needed for concurrent writers).
    ///
    /// # Safety
    /// Only call this from the single producer (queue 0).
    pub fn insert(&self, ip: Ipv4Addr, mac: MacAddress) {
        // Load current map
        let current = self.inner.load();

        // Check if already present with same value
        // TODO: if number of queue is large, this is expensive.
        if current.get(&ip) == Some(&mac) {
            // MAC unchanged, but still bump version so consumers re-inject.
            // This is needed because smoltcp's internal neighbor cache expires
            // independently (60s) and needs periodic ARP refreshes.
            self.version.fetch_add(1, Ordering::Release);
            return;
        }

        // Copy-on-write: clone and update
        let mut new_map = (**current).clone();
        new_map.insert(ip, mac);

        // Atomic store - safe because we're the only writer (SPMC)
        self.inner.store(Arc::new(new_map));

        // Always bump version so consumers know to re-inject
        // (even if it was just an update of existing entry)
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Check if an IP is in the cache.
    ///
    /// Lock-free: single atomic load.
    #[inline]
    pub fn contains(&self, ip: &Ipv4Addr) -> bool {
        self.inner.load().contains_key(ip)
    }

    /// Get the version counter (increments on every insert/update).
    ///
    /// Use this to detect changes including MAC updates for existing IPs.
    #[inline(always)]
    pub fn version(&self) -> usize {
        self.version.load(Ordering::Relaxed)
    }

    /// Check if the cache is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.load().is_empty()
    }

    /// Get a snapshot of all entries for iteration.
    ///
    /// Lock-free: single atomic load, returns Arc to shared data.
    #[inline]
    pub fn snapshot(&self) -> arc_swap::Guard<Arc<HashMap<Ipv4Addr, MacAddress>>> {
        self.inner.load()
    }
}

/// Check if a packet is an ARP reply and extract the sender's IP and MAC.
///
/// # Arguments
/// * `packet` - Raw Ethernet frame
///
/// # Returns
/// `Some((sender_ip, sender_mac))` if this is an ARP reply, `None` otherwise.
#[inline(always)]
pub fn parse_arp_reply(packet: &[u8]) -> Option<(Ipv4Addr, MacAddress)> {
    // Minimum ARP packet: Ethernet header (14) + ARP (28) = 42 bytes
    if packet.len() < 42 {
        return None;
    }

    // Check ethertype is ARP (0x0806)
    if packet[12] != 0x08 || packet[13] != 0x06 {
        return None;
    }

    // Check it's an ARP reply (operation = 2)
    // ARP header starts at offset 14
    // Operation is at offset 20-21 (6-7 within ARP header)
    if packet[20] != 0x00 || packet[21] != 0x02 {
        return None;
    }

    // Sender MAC is at offset 22-27 (8-13 within ARP header)
    let mut sender_mac = [0u8; 6];
    sender_mac.copy_from_slice(&packet[22..28]);

    // Sender IP is at offset 28-31 (14-17 within ARP header)
    let sender_ip = Ipv4Addr::new(packet[28], packet[29], packet[30], packet[31]);

    Some((sender_ip, sender_mac))
}

/// Build an ARP reply packet for injection into smoltcp.
///
/// This creates a fake ARP reply that looks like it came from the specified
/// IP/MAC, targeted at our interface. When injected and processed by smoltcp,
/// it will populate the neighbor cache.
///
/// # Arguments
/// * `our_mac` - Our interface's MAC address
/// * `our_ip` - Our interface's IP address
/// * `peer_mac` - The peer's MAC address (to be cached)
/// * `peer_ip` - The peer's IP address (to be cached)
///
/// # Returns
/// A complete Ethernet frame containing the ARP reply.
pub fn build_arp_reply_for_injection(
    our_mac: MacAddress,
    our_ip: Ipv4Addr,
    peer_mac: MacAddress,
    peer_ip: Ipv4Addr,
) -> Vec<u8> {
    let mut packet = vec![0u8; 42]; // Ethernet (14) + ARP (28)

    // Ethernet header
    packet[0..6].copy_from_slice(&our_mac); // Destination MAC (us)
    packet[6..12].copy_from_slice(&peer_mac); // Source MAC (peer)
    packet[12..14].copy_from_slice(&[0x08, 0x06]); // EtherType: ARP

    // ARP header
    packet[14..16].copy_from_slice(&[0x00, 0x01]); // Hardware type: Ethernet
    packet[16..18].copy_from_slice(&[0x08, 0x00]); // Protocol type: IPv4
    packet[18] = 6; // Hardware address length
    packet[19] = 4; // Protocol address length
    packet[20..22].copy_from_slice(&[0x00, 0x02]); // Operation: ARP Reply

    // Sender (peer) hardware and protocol address
    packet[22..28].copy_from_slice(&peer_mac);
    packet[28..32].copy_from_slice(&peer_ip.octets());

    // Target (us) hardware and protocol address
    packet[32..38].copy_from_slice(&our_mac);
    packet[38..42].copy_from_slice(&our_ip.octets());

    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_arp_cache() {
        let cache = SharedArpCache::new();
        let ip = Ipv4Addr::new(10, 0, 0, 1);
        let mac = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];

        assert!(cache.is_empty());
        assert!(cache.get(&ip).is_none());

        cache.insert(ip, mac);

        assert!(!cache.is_empty());
        assert_eq!(cache.get(&ip), Some(mac));
        assert!(cache.contains(&ip));
    }

    #[test]
    fn test_parse_arp_reply() {
        // Build a test ARP reply
        let our_mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let our_ip = Ipv4Addr::new(10, 0, 0, 5);
        let peer_mac = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];
        let peer_ip = Ipv4Addr::new(10, 0, 0, 1);

        let packet = build_arp_reply_for_injection(our_mac, our_ip, peer_mac, peer_ip);

        let result = parse_arp_reply(&packet);
        assert_eq!(result, Some((peer_ip, peer_mac)));
    }

    #[test]
    fn test_parse_non_arp_packet() {
        // IPv4 packet (not ARP)
        let mut packet = vec![0u8; 60];
        packet[12] = 0x08;
        packet[13] = 0x00; // IPv4 ethertype

        assert!(parse_arp_reply(&packet).is_none());
    }

    #[test]
    fn test_parse_arp_request() {
        // ARP request (operation = 1, not reply)
        let mut packet = vec![0u8; 42];
        packet[12] = 0x08;
        packet[13] = 0x06; // ARP ethertype
        packet[20] = 0x00;
        packet[21] = 0x01; // ARP request

        assert!(parse_arp_reply(&packet).is_none());
    }
}
