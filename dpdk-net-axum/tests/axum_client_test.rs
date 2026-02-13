//! Axum + HTTP Client Integration Test
//!
//! Validates the full stack: DpdkApp starts an axum server with `serve()`,
//! then an HTTP client (dpdk-net-hyper) sends a GET request and verifies
//! the response. Both server and client run on the same lcore.

use axum::Router;
use axum::routing::get;

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::TcpListener;
use dpdk_net_axum::{DpdkApp, WorkerContext, serve};
use dpdk_net_hyper::DpdkHttpClient;

use http_body_util::BodyExt;
use hyper::Request;
use hyper::body::Bytes;
use smoltcp::wire::{IpAddress, Ipv4Address};
use tokio_util::sync::CancellationToken;

use serial_test::serial;

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const GATEWAY_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 254);
const SERVER_PORT: u16 = 8080;
const CLIENT_PORT: u16 = 49152;

async fn hello() -> &'static str {
    "Hello from DPDK + Axum!"
}

async fn worker_main(ctx: WorkerContext) {
    println!(
        "Worker starting on lcore {} (queue {})",
        ctx.lcore.id(),
        ctx.queue_id,
    );

    // Build axum router
    let app = Router::new()
        .route("/", get(hello))
        .route("/health", get(|| async { "OK" }));

    // Bind listener
    let listener =
        TcpListener::bind(&ctx.reactor, SERVER_PORT, 4096, 4096).expect("Failed to bind listener");
    println!("Server: listening on port {}", SERVER_PORT);

    // Create a child token so we can stop serve() from inside the test
    let server_shutdown = ctx.shutdown.child_token();

    // Spawn the axum server
    let server_task = tokio::task::spawn_local({
        let shutdown = server_shutdown.clone();
        async move {
            serve(listener, app, shutdown).await;
        }
    });

    // Give server a moment to start accepting
    tokio::task::yield_now().await;

    // --- HTTP client ---
    let client = DpdkHttpClient::new(ctx.reactor.clone());
    let mut conn = client
        .connect(IpAddress::Ipv4(SERVER_IP), SERVER_PORT, CLIENT_PORT)
        .await
        .expect("Client: connect failed");

    println!("Client: connected");

    // Send GET /
    let req = Request::get("/")
        .header("Host", "192.168.1.1:8080")
        .body(http_body_util::Empty::<Bytes>::new())
        .unwrap();
    let resp = conn
        .send_request(req)
        .await
        .expect("Client: request failed");

    println!("Client: response status = {}", resp.status());
    assert_eq!(resp.status(), 200);

    let body = resp
        .into_body()
        .collect()
        .await
        .expect("Client: body collect failed")
        .to_bytes();
    let body_str = std::str::from_utf8(&body).expect("Client: invalid utf8");
    println!("Client: body = '{}'", body_str);
    assert_eq!(body_str, "Hello from DPDK + Axum!");

    // Send GET /health
    let req = Request::get("/health")
        .header("Host", "192.168.1.1:8080")
        .body(http_body_util::Empty::<Bytes>::new())
        .unwrap();
    let resp = conn
        .send_request(req)
        .await
        .expect("Client: /health failed");

    assert_eq!(resp.status(), 200);
    let body = resp
        .into_body()
        .collect()
        .await
        .expect("Client: body collect failed")
        .to_bytes();
    assert_eq!(&body[..], b"OK");
    println!("Client: /health = OK");

    println!("\n✓ Axum + Client test PASSED!");

    // Shut down server and app
    server_shutdown.cancel();
    let _ = server_task.await;
    ctx.shutdown.cancel();
}

#[test]
#[serial]
fn test_axum_serve_with_client() {
    println!("\n=== Axum + HTTP Client Test ===\n");

    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    println!("EAL initialized");

    let shutdown_token = CancellationToken::new();
    let shutdown_clone = shutdown_token.clone();

    DpdkApp::new()
        .eth_dev(0)
        .ip(SERVER_IP)
        .gateway(GATEWAY_IP)
        .mbufs_per_queue(1024)
        .descriptors(128, 128)
        .run(shutdown_clone.cancelled_owned(), worker_main);

    println!("\n=== Axum + HTTP Client Test Complete ===\n");
}
