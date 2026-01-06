Steps:
```
sudo ip link add xsk_dev1 type veth peer name xsk_dev2
sudo ip link set xsk_dev1 up
sudo ip link set xsk_dev2 up
```

TODO:
This does not work on azure vm.

The libxdp 1.5.6 library includes a capability check that uses BPF_PROG_TEST_RUN to verify kernel BPF features. This check fails on the Azure kernel (6.14.0-1017-azure) with EINVAL, causing the entire socket creation to fail.

Root cause?:
$ ethtool -i enP61732s1
driver: mlx4_en
1. mlx4 VFs do NOT support XDP in native/driver mode
Only mlx5 and mana support full XDP offload and native mode.

See hardware:
$ sudo lspci -nn | grep -i mell

This requires v5 vm:
$ readlink -f /sys/class/net/enP10855s1/device/driver
/sys/bus/pci/drivers/mlx5_core

still does not work:
```
$ sudo target/debug/deps/xdp_test-816494f3b5acdf2d

running 1 test
libbpf: elf: skipping unrecognized data section(8) .xdp_run_config
libbpf: elf: skipping unrecognized data section(9) xdp_metadata
libxdp: Couldn't find xdp program in bpf object
libxdp: Couldn't open BPF file xdp-dispatcher.o
libxdp: Couldn't find xdp program in bpf object
libxdp: Couldn't open BPF file 'xdp-dispatcher.o'
test tests::it_works ... FAILED

failures:

---- tests::it_works stdout ----

thread 'tests::it_works' (2320) panicked at xdp-test/src/lib.rs:31:6:
failed to create dev1 socket: SocketCreateError { reason: "non-zero error code returned when creating AF_XDP socket", err: Os { code: 2, kind: NotFound, message: "No such file or directory" } }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

xdp-socket crate also fails for ponger:
The conclusion from the strace is: veth interfaces don't support AF_XDP sockets properly - the mmap call with the Fill ring offset fails. This is a known limitation of virtual ethernet devices.
[pid 67405] mmap(NULL, 33088, PROT_READ|PROT_WRITE, MAP_SHARED|MAP_POPULATE, 16, 0x100000000) = -1 EINVAL (Invalid argument)
