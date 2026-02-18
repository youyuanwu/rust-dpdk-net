# Tonic gRPC Integration Design

gRPC support for dpdk-net via [tonic](https://github.com/hyperium/tonic). Built on top of `DpdkApp` and reuses `dpdk-net-axum` for the server side (see [Axum.md](Axum.md)).

Crate: [`dpdk-net-tonic`](../../dpdk-net-tonic/src/)  
Test: [`tonic_grpc_test.rs`](../../dpdk-net-test/tests/tonic_grpc_test.rs)

## Server

`dpdk_net_tonic::serve()` accepts `tonic::service::Routes`, converts via `.into_axum_router()`, and delegates to `dpdk_net_axum::serve()`. No separate server implementation — tonic services become an axum `Router`.

We bypass `tonic::transport::Server::serve()` which requires `Send` streams.

```rust
let greeter = GreeterServer::new(MyGreeter::default());
let routes = tonic::service::Routes::new(greeter);

let listener = TcpListener::bind(&ctx.reactor, 50051, 4096, 4096).unwrap();
serve(listener, routes, ctx.shutdown).await;
```

Mixed REST + gRPC: call `.into_axum_router()` yourself, merge with axum routes, and use `dpdk_net_axum::serve()` directly.

## Client

`DpdkGrpcChannel` wraps a persistent HTTP/2 `Connection` and implements `tower::Service<http::Request<tonic::body::Body>>`, satisfying `GrpcService` via blanket impl. This replaces `tonic::transport::Channel` which requires `Send`.

The channel stores the scheme and authority from the connect URI and injects them into outgoing requests (tonic generates path-only URIs, hyper requires full URIs). This mirrors tonic's internal `AddOrigin` middleware.

```rust
let uri: http::Uri = "http://192.168.1.1:50051".parse().unwrap();
let channel = DpdkGrpcChannel::connect(&ctx.reactor, uri).await?;
let mut client = GreeterClient::new(channel);

let response = client.say_hello(HelloRequest { name: "dpdk".into() }).await?;
```

## Tonic 0.14 Notes

- Prost split: `tonic-prost-build` (codegen) + `tonic-prost` (runtime codec)
- `tonic::body::Body` replaces private `BoxBody`
- `Routes::into_axum_router()` replaces deprecated `into_router()`
- Use `build_transport(false)` in codegen to skip transport-dependent `connect()` methods

## Limitations

1. Cannot use `tonic::transport::Server::serve()` or `tonic::transport::Channel` — both require `Send`
2. No TLS — cleartext HTTP/2 (h2c) only
3. Single-threaded per lcore — one slow RPC blocks others on the same lcore
4. `DpdkGrpcChannel` is not `Clone` — create one per tonic client instance
5. Generated clients are `!Send` — cannot be moved between lcores
6. Must use `build_transport(false)` in codegen

## References

- [Axum Integration Design](Axum.md)
- [HTTP Client Design](Client.md)
- [tonic `GrpcService` trait](https://docs.rs/tonic/latest/tonic/client/trait.GrpcService.html)
- [gRPC over HTTP/2 spec](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md)
