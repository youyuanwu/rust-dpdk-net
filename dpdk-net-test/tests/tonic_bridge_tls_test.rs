//! Tonic gRPC over OS Thread Bridge with TLS
//!
//! Same as `tonic_bridge_test` but wraps the connection with TLS using
//! `tonic_tls::openssl`. Certificates are self-signed via `rcgen`.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net_util::tonic::bridge::BridgeIncoming;
use dpdk_net_util::tonic::bridge::tls::BridgeTransport;
use dpdk_net_util::{DpdkApp, DpdkBridge, WorkerContext};

use openssl::pkey::PKey;
use openssl::ssl::{SslAcceptor, SslConnector, SslMethod, SslVerifyMode};
use openssl::x509::X509;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use smoltcp::wire::Ipv4Address;
use std::sync::Arc;
use tokio::sync::Notify;
use tonic::transport::{Endpoint, Server};
use tonic::{Request, Response, Status};

use serial_test::serial;

mod greeter {
    tonic::include_proto!("greeter");
}

use greeter::greeter_client::GreeterClient;
use greeter::greeter_server::{Greeter, GreeterServer};
use greeter::{HelloReply, HelloRequest};

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const GATEWAY_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 254);
const SERVER_PORT: u16 = 50053;

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
            message: format!("Hello TLS, {}!", name),
        }))
    }
}

/// Build an `SslAcceptor` (server) from rcgen cert + key PEM.
fn build_acceptor(cert_pem: &[u8], key_pem: &[u8]) -> SslAcceptor {
    let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls_server()).unwrap();
    let cert = X509::from_pem(cert_pem).unwrap();
    let pkey = PKey::private_key_from_pem(key_pem).unwrap();
    builder.set_certificate(&cert).unwrap();
    builder.set_private_key(&pkey).unwrap();
    builder.build()
}

/// Build an `SslConnector` (client) that trusts the given CA cert PEM.
fn build_connector(ca_pem: &[u8]) -> SslConnector {
    let mut builder = SslConnector::builder(SslMethod::tls_client()).unwrap();
    let ca = X509::from_pem(ca_pem).unwrap();
    builder.cert_store_mut().add_cert(ca).unwrap();
    builder.set_verify(SslVerifyMode::PEER);
    builder.build()
}

#[test]
#[serial]
fn test_tonic_bridge_tls_grpc() {
    println!("\n=== Tonic Bridge TLS gRPC Test ===\n");

    // Generate self-signed cert with SAN matching "localhost" (used as TLS domain).
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_pem = cert.pem().into_bytes();
    let key_pem = signing_key.serialize_pem().into_bytes();

    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    println!("EAL initialized");

    let (bridge, bridge_workers) = DpdkBridge::pair();
    let done = Arc::new(Notify::new());

    let bridge_handle = bridge.clone();
    let done_clone = done.clone();
    let cert_pem_clone = cert_pem.clone();
    let key_pem_clone = key_pem.clone();

    let bg_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            bridge_handle.wait_ready().await;
            println!("OS thread: bridge ready");

            let listener = bridge_handle
                .listen(SERVER_PORT)
                .await
                .expect("bridge listen failed");
            println!("OS thread: listening on port {SERVER_PORT}");

            // --- TLS server ---
            let acceptor = build_acceptor(&cert_pem_clone, &key_pem_clone);
            let tls_incoming =
                tonic_tls::openssl::TlsIncoming::new(BridgeIncoming::new(listener), acceptor);

            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
            let server_task = tokio::spawn(async move {
                Server::builder()
                    .add_service(GreeterServer::new(MyGreeter))
                    .serve_with_incoming_shutdown(tls_incoming, async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .unwrap();
            });

            tokio::task::yield_now().await;

            // --- TLS client ---
            let transport = BridgeTransport::new(bridge_handle);
            let ssl_connector = build_connector(&cert_pem);
            let tls_connector = tonic_tls::openssl::TlsConnector::new(
                transport,
                ssl_connector,
                "localhost".to_string(),
            );

            let uri = format!("https://{}:{}", SERVER_IP, SERVER_PORT);
            let channel = Endpoint::from_shared(uri)
                .expect("valid endpoint")
                .connect_with_connector(tls_connector)
                .await
                .expect("Client: TLS connect failed");

            println!("OS thread: TLS client connected via bridge");

            let mut client = GreeterClient::new(channel);
            let response = client
                .say_hello(Request::new(HelloRequest {
                    name: "Bridge".into(),
                }))
                .await
                .expect("Client: RPC failed");

            let message = response.into_inner().message;
            println!("OS thread: response = '{message}'");
            assert_eq!(message, "Hello TLS, Bridge!");

            println!("\n✓ Tonic Bridge TLS gRPC test PASSED!");

            let _ = shutdown_tx.send(());
            let _ = server_task.await;
        });
        done_clone.notify_one();
    });

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

    println!("\n=== Tonic Bridge TLS gRPC Test Complete ===\n");

    bg_thread.join().expect("OS thread panicked");
}
