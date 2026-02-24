//! UDP echo benchmark client
//!
//! Sends datagrams to a UDP echo server and measures PPS, RTT, and loss.
//! Each datagram contains a 16-byte header (8-byte sequence + 8-byte timestamp)
//! followed by padding to reach the configured packet size.

use crate::stats::{BenchStats, LocalStats};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

/// Run the UDP echo benchmark
pub async fn run_benchmark(
    target: &str,
    sockets: usize,
    duration: Duration,
    timeout: Duration,
    packet_size: usize,
    stats: Arc<BenchStats>,
) {
    let end_time = Instant::now() + duration;

    let mut handles = Vec::with_capacity(sockets);
    for _ in 0..sockets {
        let target = target.to_string();
        let stats = stats.clone();

        let handle = tokio::spawn(async move {
            run_socket(target, stats, end_time, timeout, packet_size).await;
        });
        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }
}

async fn run_socket(
    target: String,
    stats: Arc<BenchStats>,
    end_time: Instant,
    timeout: Duration,
    packet_size: usize,
) {
    stats.active_connections.fetch_add(1, Ordering::Relaxed);

    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            stats.record_error_sample(format!("bind: {}", e));
            stats.active_connections.fetch_sub(1, Ordering::Relaxed);
            return;
        }
    };

    if let Err(e) = socket.connect(&target).await {
        stats.record_error_sample(format!("connect: {}", e));
        stats.active_connections.fetch_sub(1, Ordering::Relaxed);
        return;
    }

    let mut local = LocalStats::new();
    let pkt_size = packet_size.max(16); // minimum 16 bytes for header
    let mut send_buf = vec![0u8; pkt_size];
    let mut recv_buf = vec![0u8; pkt_size + 64]; // extra room for safety
    let mut seq: u64 = 0;

    while Instant::now() < end_time {
        // Encode header: [seq:8][timestamp_nanos:8]
        let send_time = Instant::now();
        send_buf[..8].copy_from_slice(&seq.to_le_bytes());

        // Use elapsed since epoch-like anchor for the timestamp
        let ts_nanos = send_time.elapsed().as_nanos() as u64;
        send_buf[8..16].copy_from_slice(&ts_nanos.to_le_bytes());

        // Send
        match socket.send(&send_buf).await {
            Ok(_) => {}
            Err(e) => {
                local.record_error();
                stats.record_error_sample(format!("send: {}", e));
                seq += 1;
                continue;
            }
        }

        // Receive with timeout
        match tokio::time::timeout(timeout, socket.recv(&mut recv_buf)).await {
            Ok(Ok(n)) => {
                let latency_us = send_time.elapsed().as_micros() as u64;
                local.record_success(latency_us, n);
            }
            Ok(Err(e)) => {
                local.record_error();
                stats.record_error_sample(format!("recv: {}", e));
            }
            Err(_) => {
                local.record_error();
                // Don't log every timeout - too noisy
            }
        }

        seq += 1;
    }

    local.merge_into(&stats);
    stats.active_connections.fetch_sub(1, Ordering::Relaxed);
}
