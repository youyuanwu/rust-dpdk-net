//! Tonic gRPC over HTTP/3 (QUIC) via DPDK bridge test.
//!
//! Tests the full gRPC-over-H3 stack: a tonic-h3 server using H3QuinnAcceptor
//! and a client using H3QuinnConnector + H3Channel, both running over DPDK
//! bridge UDP sockets via DpdkQuinnRuntime.

use std::sync::Arc;

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net_quinn::DpdkQuinnRuntime;
use dpdk_net_util::{DpdkApp, DpdkBridge, WorkerContext};
use quinn::EndpointConfig;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls_pki_types::PrivateKeyDer;
use serial_test::serial;
use smoltcp::wire::Ipv4Address;
use tokio::sync::Notify;
use tonic::{Request, Response, Status};

mod greeter {
    tonic::include_proto!("greeter");
}

use greeter::greeter_server::{Greeter, GreeterServer};
use greeter::{HelloReply, HelloRequest};

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const GATEWAY_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 254);
const QUIC_PORT: u16 = 4443;
const CLIENT_PORT: u16 = 4444;

#[derive(Debug, Default)]
struct MyGreeter;

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let name = request.into_inner().name;
        Ok(Response::new(HelloReply {
            message: format!("Hello H3, {}!", name),
        }))
    }
}

#[test]
#[serial]
fn test_tonic_h3_grpc() {
    println!("\n=== Tonic H3 gRPC Test ===\n");

    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    // Generate self-signed certificate.
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_der = cert.der().clone();
    let key_der = PrivateKeyDer::Pkcs8(signing_key.serialize_der().into());

    // Server TLS config with h3 ALPN.
    let provider = quinn::rustls::crypto::ring::default_provider();
    let mut server_tls = quinn::rustls::ServerConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der.clone_key())
        .unwrap();
    server_tls.alpn_protocols = vec![b"h3".to_vec()];
    server_tls.max_early_data_size = u32::MAX;

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_tls).unwrap(),
    ));

    // Client TLS config with h3 ALPN, trusting our self-signed cert.
    let mut roots = quinn::rustls::RootCertStore::empty();
    roots.add(cert_der).unwrap();
    let provider = quinn::rustls::crypto::ring::default_provider();
    let mut client_tls = quinn::rustls::ClientConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_tls.alpn_protocols = vec![b"h3".to_vec()];

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_tls).unwrap(),
    ));

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

                // --- Start tonic-h3 server ---
                let greeter = GreeterServer::new(MyGreeter);
                let routes = tonic::service::Routes::new(greeter);

                let acceptor = tonic_h3::quinn::H3QuinnAcceptor::new(server_endpoint.clone());
                let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
                let server_task = tokio::spawn(async move {
                    tonic_h3::server::H3Router::new(routes)
                        .serve_with_shutdown(acceptor, async {
                            let _ = shutdown_rx.await;
                        })
                        .await
                        .expect("tonic-h3 server failed");
                });

                // Give server time to start accepting.
                tokio::task::yield_now().await;

                // --- gRPC client via tonic-h3 ---
                let uri: http::Uri = format!("https://{}:{}", SERVER_IP, QUIC_PORT)
                    .parse()
                    .unwrap();

                let connector = tonic_h3::quinn::H3QuinnConnector::new(
                    uri.clone(),
                    "localhost".to_string(),
                    client_endpoint.clone(),
                );
                let channel = tonic_h3::H3Channel::new(connector, uri);
                let mut client = greeter::greeter_client::GreeterClient::new(channel);

                let response = client
                    .say_hello(Request::new(HelloRequest {
                        name: "DPDK-H3".into(),
                    }))
                    .await
                    .expect("RPC failed");

                let message = response.into_inner().message;
                println!("Response: '{message}'");
                assert_eq!(message, "Hello H3, DPDK-H3!");

                println!("\n✓ Tonic H3 gRPC test PASSED!");

                // Cleanup
                let _ = shutdown_tx.send(());
                let _ = server_task.await;
                client_endpoint.close(0u8.into(), b"done");
                server_endpoint.close(0u8.into(), b"done");
            });
        }));

        done_clone.notify_one();

        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    });

    // Lcore: register bridge workers and wait for the OS thread.
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

    println!("\n=== Tonic H3 gRPC Test Complete ===\n");

    os_thread.join().expect("OS thread panicked");
}
