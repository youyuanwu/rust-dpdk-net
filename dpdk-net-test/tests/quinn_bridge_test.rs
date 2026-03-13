//! Quinn QUIC over DPDK bridge test.
//!
//! Tests that two Quinn endpoints (client and server) can communicate
//! over DPDK bridge UDP sockets. Both endpoints run on an OS thread
//! using `DpdkQuinnRuntime`; the DPDK lcore provides the underlying
//! UDP transport via bridge workers.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net_util::quinn::DpdkQuinnRuntime;
use dpdk_net_util::{DpdkApp, DpdkBridge, WorkerContext};
use quinn::EndpointConfig;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls_pki_types::PrivateKeyDer;
use serial_test::serial;
use smoltcp::wire::Ipv4Address;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const GATEWAY_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 254);
const QUIC_PORT: u16 = 4433;
const CLIENT_PORT: u16 = 4434;

#[test]
#[serial]
fn test_quinn_bridge_echo() {
    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    // Generate self-signed certificate for the QUIC server.
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_der = cert.der().clone();
    let key_der = PrivateKeyDer::Pkcs8(signing_key.serialize_der().into());

    let server_config = quinn::ServerConfig::with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");

    // Client trusts our self-signed CA.
    let mut roots = quinn::rustls::RootCertStore::empty();
    roots.add(cert_der).unwrap();
    let client_config =
        quinn::ClientConfig::with_root_certificates(Arc::new(roots)).expect("client config");

    let (bridge, bridge_workers) = DpdkBridge::pair();
    let done = Arc::new(Notify::new());

    let bridge_handle = bridge.clone();
    let done_clone = done.clone();
    let os_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rt.block_on(async {
                bridge_handle.wait_ready().await;

                let quinn_rt = DpdkQuinnRuntime::new(bridge_handle);

                // --- Server endpoint ---
                let server_endpoint = quinn_rt
                    .endpoint(EndpointConfig::default(), Some(server_config), QUIC_PORT)
                    .await
                    .expect("server endpoint");

                // --- Client endpoint ---
                let mut client_endpoint = quinn_rt
                    .endpoint(EndpointConfig::default(), None, CLIENT_PORT)
                    .await
                    .expect("client endpoint");
                client_endpoint.set_default_client_config(client_config);

                let server_addr: SocketAddr =
                    format!("{}:{}", SERVER_IP, QUIC_PORT).parse().unwrap();

                // Spawn server accept loop.
                let server_task = tokio::spawn(async move {
                    let incoming = server_endpoint.accept().await.unwrap();
                    let conn = incoming.await.unwrap();

                    let (mut send, mut recv) = conn.accept_bi().await.unwrap();
                    let data = recv.read_to_end(64 * 1024).await.unwrap();
                    send.write_all(&data).await.unwrap();
                    send.finish().unwrap();

                    // Wait for the peer to consume the data before closing.
                    conn.closed().await;
                });

                // Client: connect, send, receive echo.
                let conn = tokio::time::timeout(
                    Duration::from_secs(10),
                    client_endpoint.connect(server_addr, "localhost").unwrap(),
                )
                .await
                .expect("client connect timeout")
                .expect("client connect");

                let (mut send, mut recv) = conn.open_bi().await.unwrap();
                let message = b"hello QUIC over DPDK";
                send.write_all(message).await.unwrap();
                send.finish().unwrap();

                let response = recv.read_to_end(64 * 1024).await.unwrap();
                assert_eq!(response, message, "echo mismatch");

                // Graceful shutdown.
                conn.close(0u8.into(), b"done");
                tokio::time::timeout(Duration::from_secs(5), server_task)
                    .await
                    .expect("server task timeout")
                    .expect("server task");
            });
        }));

        done_clone.notify_one();

        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    });

    // Lcore: register bridge worker and wait for the OS thread.
    DpdkApp::new()
        .eth_dev(0)
        .ip(SERVER_IP)
        .gateway(GATEWAY_IP)
        .mbufs_per_queue(1024)
        .descriptors(128, 128)
        .run(move |ctx: WorkerContext| {
            let bridge_workers = bridge_workers.clone();
            let done = done.clone();
            async move {
                bridge_workers.spawn(&ctx.reactor);
                done.notified().await;
            }
        });

    os_thread.join().expect("OS thread panicked");
}
