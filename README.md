# dpdk-net
[dpdk](https://www.dpdk.org/) for Rust (with tokio).

[dpdk-net](https://github.com/youyuanwu/rust-dpdk-net) aims to provide high level dpdk api like those in [std::net](https://doc.rust-lang.org/std/net/) and [tokio::net](https://docs.rs/tokio/latest/tokio/net/index.html)

Supports `TcpListener` and `TcpStream` with tokio async adaptors. 

# Get Started
1. Install dpdk. (From pkg manager or from [src](https://doc.dpdk.org/guides/linux_gsg/build_dpdk.html))

    This repo install dpdk from source with [cmake commands](CMakeLists.txt):
    ```sh
    # clone this repo.
    cmake -S . -B build
    cmake --build build --target dpdk_configure
    cmake --build build --target dpdk_build --parallel
    sudo cmake --build build --target dpdk_install
    ```
1. Add to cargo.toml
    ```
    dpdk-net = "*"
    ```

1. Run simple code like in the example
    ```rust
    fn main(){
      use dpdk_net::api::rte::eal::EalBuilder;
      // Initialize DPDK EAL with PCI device
      let _eal = EalBuilder::new()
          .init()
          .expect("Failed to initialize EAL");
      // ...
    }
    ```
# Examples: 
* [dpdk_tcp_server](dpdk-net-test/examples/dpdk_tcp_server.rs)
* [dpdk_hyper_test](dpdk-net-test/tests/http_auto_echo_test.rs)

# Reference Projects
[rust-dpdk](https://github.com/ANLAB-KAIST/rust-dpdk) for binding generation.
[rpkt](https://github.com/duanjp8617/rpkt)

# License
MIT
