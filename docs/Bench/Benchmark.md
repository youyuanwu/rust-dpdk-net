# dpdk-net benchmark
Benchmark for different kind of workload, and different setups.

## Single Zone
Machines are located close to each other, in the same data center.
### Http read scenario
DPDK server serves a http html page, and tokio test client try to make as much traffic as possible.
See: 
- [dpdk-bench-server](../../tests/dpdk-bench-server/BenchServer.md)
- [dpdk-bench-client](../../tests/dpdk-bench-client/BenchClient.md)

Driver:
mlx5_core:  `Ethernet controller: Mellanox Technologies MT27800 Family [ConnectX-5 Virtual Function]`

Azure Bench Result:
- [Standard_D2s_v5](./Azure/BENCHMARK_d2s.md): 2 cpu, 2 queue pair.
- [Standard_D4s_v5](./Azure/BENCHMARK_d4s.md): 4 cpu, 4 queue pair.
- [Standard_D8s_v5](./Azure/BENCHMARK_d8s.md): 8 cpu, 4 queue pair.

Note: 
* In some vm queue is less than cpu, so the machine is not fully utilized by the current bench server, which creates 1 thread per queue and process http request on that thread.
But the above vm sizes has 1 queue per cpu.

