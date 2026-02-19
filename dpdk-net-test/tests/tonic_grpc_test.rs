//! Tonic gRPC Test over DPDK
//!
//! Validates the full gRPC stack: `dpdk_net_tonic::serve` runs a Greeter
//! gRPC server, and a `DpdkGrpcChannel`-backed client sends an RPC and
//! verifies the response. Both server and client run on the same lcore.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::TcpListener;
use dpdk_net_axum::{DpdkApp, WorkerContext};
use dpdk_net_tonic::{DpdkGrpcChannel, serve};

use smoltcp::wire::Ipv4Address;
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
const SERVER_PORT: u16 = 50051;
const CLIENT_PORT: u16 = 49152;

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

async fn worker_main(ctx: WorkerContext) {
    println!(
        "Worker starting on lcore {} (queue {})",
        ctx.lcore.id(),
        ctx.queue_id,
    );

    // Build gRPC service
    let greeter = GreeterServer::new(MyGreeter);
    let routes = tonic::service::Routes::new(greeter);

    // Bind listener
    let listener =
        TcpListener::bind(&ctx.reactor, SERVER_PORT, 4096, 4096).expect("Failed to bind listener");
    println!("Server: listening on port {}", SERVER_PORT);

    // Spawn the gRPC server
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::task::spawn_local(serve(listener, routes, async {
        let _ = shutdown_rx.await;
    }));

    // Give server a moment to start accepting
    tokio::task::yield_now().await;

    // --- gRPC client ---
    let uri: http::Uri = format!("http://{}:{}", SERVER_IP, SERVER_PORT)
        .parse()
        .unwrap();
    let channel = DpdkGrpcChannel::connect_with(&ctx.reactor, uri, CLIENT_PORT, 4096, 4096)
        .await
        .expect("Client: connect failed");

    println!("Client: connected via HTTP/2");

    let mut client = greeter::greeter_client::GreeterClient::new(channel);

    // Send SayHello RPC
    let request = Request::new(HelloRequest {
        name: "DPDK".into(),
    });
    let response = client.say_hello(request).await.expect("Client: RPC failed");

    let message = response.into_inner().message;
    println!("Client: response = '{}'", message);
    assert_eq!(message, "Hello, DPDK!");

    println!("\n✓ Tonic gRPC test PASSED!");

    // Shut down server
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

#[test]
#[serial]
fn test_tonic_grpc() {
    println!("\n=== Tonic gRPC Test ===\n");

    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    println!("EAL initialized");

    DpdkApp::new()
        .eth_dev(0)
        .ip(SERVER_IP)
        .gateway(GATEWAY_IP)
        .mbufs_per_queue(1024)
        .descriptors(128, 128)
        .run(worker_main);

    println!("\n=== Tonic gRPC Test Complete ===\n");
}
