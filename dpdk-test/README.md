# DPDK
build from source
```
sudo apt-get update && sudo apt-get install -y meson ninja-build
```

```
sudo mkdir -p /dev/hugepages
sudo mount -t hugetlbfs none /dev/hugepages
echo 1024 | sudo tee /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages

sudo chmod 666 /dev/vfio/*

sudo ${dpdk_BINARY_DIR}/examples/dpdk-helloworld
```