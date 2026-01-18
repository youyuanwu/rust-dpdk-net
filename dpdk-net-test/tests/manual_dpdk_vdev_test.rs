/// Use DPDK vdev for test
#[test]
fn test_smoltcp_on_dpdk_vdev() {
    dpdk_net_test::manual::tcp::tcp_echo_test(false);
}
