//! Tonic gRPC over OS Thread Bridge Test
//!
//! Validates the full bridge gRPC stack: a tonic server runs on an OS thread
//! using `serve_with_incoming_shutdown` with `BridgeIncoming`, and a client
//! connects via `Endpoint::connect_with_connector` with `BridgeConnector`.
//! The DPDK lcore only provides bridge workers.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net_tonic::tonic::bridge::{BridgeConnector, BridgeIncoming};
use dpdk_net_util::{DpdkApp, DpdkBridge, WorkerContext};

use smoltcp::wire::Ipv4Address;
use std::sync::Arc;
use tokio::sync::Notify;
use tonic::transport::{Endpoint, Server};
use tonic::{Request, Response, Status};

use serial_test::serial;

/// Generated protobuf/gRPC code from `proto/greeter.proto`.
mod greeter {
    tonic::include_proto!("greeter");
}

use greeter::greeter_server::{Greeter, GreeterServer};
use greeter::{HelloReply, HelloRequest};

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const GATEWAY_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 254);
const SERVER_PORT: u16 = 50052;

/// Greeter service implementation.
#[derive(Debug, Default)]
struct MyGreeter;

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let name = request.into_inner().name;
        let reply = HelloReply {
            message: format!("Hello, {}!", name),
        };
        Ok(Response::new(reply))
    }
}

#[test]
#[serial]
fn test_tonic_bridge_grpc() {
    println!("\n=== Tonic Bridge gRPC Test ===\n");

    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    println!("EAL initialized");

    // Create bridge pair
    let (bridge, bridge_workers) = DpdkBridge::pair();
    let done = Arc::new(Notify::new());

    // OS thread: run both gRPC server and client via bridge
    let bridge_handle = bridge.clone();
    let done_clone = done.clone();
    let bg_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            bridge_handle.wait_ready().await;
            println!("OS thread: bridge ready");

            // Bind bridge listener
            let listener = bridge_handle
                .listen(SERVER_PORT)
                .await
                .expect("bridge listen failed");
            println!("OS thread: listening on port {SERVER_PORT}");

            // Start gRPC server in background
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
            let server_task = tokio::spawn(async move {
                Server::builder()
                    .add_service(GreeterServer::new(MyGreeter))
                    .serve_with_incoming_shutdown(BridgeIncoming::new(listener), async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .unwrap();
            });

            // Give server a moment to start accepting
            tokio::task::yield_now().await;

            // --- gRPC client via bridge connector ---
            let connector = BridgeConnector::new(bridge_handle);
            let uri = format!("http://{}:{}", SERVER_IP, SERVER_PORT);
            let channel = Endpoint::from_shared(uri)
                .expect("valid endpoint")
                .connect_with_connector(connector)
                .await
                .expect("Client: connect failed");

            println!("OS thread: client connected via bridge");

            let mut client = greeter::greeter_client::GreeterClient::new(channel);

            // Send SayHello RPC
            let request = Request::new(HelloRequest {
                name: "Bridge".into(),
            });
            let response = client.say_hello(request).await.expect("Client: RPC failed");

            let message = response.into_inner().message;
            println!("OS thread: response = '{message}'");
            assert_eq!(message, "Hello, Bridge!");

            println!("\n✓ Tonic Bridge gRPC test PASSED!");

            // Shut down server
            let _ = shutdown_tx.send(());
            let _ = server_task.await;
        });
        done_clone.notify_one();
    });

    // Lcore: register bridge worker and wait for OS thread to finish
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

    println!("\n=== Tonic Bridge gRPC Test Complete ===\n");

    bg_thread.join().expect("OS thread panicked");
}
