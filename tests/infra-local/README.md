# Local DPDK Testing Infrastructure

Terraform configuration for provisioning QEMU/KVM virtual machines for DPDK testing.

## Features

- **2 Ubuntu 24.04 VMs** with dual NICs each
- **Management network** (10.0.0.0/24) - SSH and normal socket traffic
- **DPDK network** (10.0.1.0/24) - Isolated network for DPDK virtio PMD
- **Hugepages** pre-configured for DPDK
- **Cloud-init** for automatic SSH key configuration
- **KVM acceleration** with fallback to software emulation

## Prerequisites

1. **Install QEMU/KVM and libvirt:**
   ```bash
   sudo apt install -y qemu-kvm libvirt-daemon-system libvirt-clients libvirt-dev virtinst qemu-utils
   ```

2. **Add user to libvirt group:**
   ```bash
   sudo usermod -aG libvirt $USER
   newgrp libvirt
   ```

3. **Create storage pool:**
   ```bash
   sudo mkdir -p /var/lib/libvirt/images
   sudo virsh pool-define-as default dir --target /var/lib/libvirt/images
   sudo virsh pool-start default
   sudo virsh pool-autostart default
   ```

4. **Install Terraform:**
   ```bash
   wget https://releases.hashicorp.com/terraform/1.14.4/terraform_1.14.4_linux_amd64.zip
   unzip terraform_1.14.4_linux_amd64.zip
   sudo mv terraform /usr/local/bin/
   ```

5. **SSH key** at `~/.ssh/id_rsa.pub`

## Quick Start

### Using helper scripts

```bash
cd tests/infra-local

# Start VMs
./scripts/start-vm.sh

# Start VMs with auto-approve
./scripts/start-vm.sh -y

# Start VMs without KVM (software emulation)
./scripts/start-vm.sh --no-kvm

# Destroy VMs
./scripts/destroy-vm.sh -y
```

### Using CMake targets

```bash
cd build
cmake ..

cmake --build . --target local_vm_deploy   # Deploy VMs
cmake --build . --target local_vm_ssh1     # SSH to VM1
cmake --build . --target local_vm_test     # Run e2e tests
cmake --build . --target local_vm_destroy  # Destroy VMs
```

### Manual Terraform

```bash
cd tests/infra-local
terraform init
terraform apply
terraform destroy
```

## Network Configuration

| Network | Subnet | Mode | Purpose |
|---------|--------|------|---------|
| default | 192.168.122.0/24 | NAT + DHCP | SSH, TCP/UDP sockets |
| dpdk-data-net | 10.0.1.0/24 | Isolated | DPDK traffic |

## IP Allocation

| VM | Management IP | DPDK IP |
|----|---------------|---------|
| VM1 | DHCP (get via virsh) | 10.0.1.4 |
| VM2 | DHCP (get via virsh) | 10.0.1.6 |

## Connecting to VMs

```bash
# Get VM IPs from DHCP leases
virsh net-dhcp-leases default

# SSH to VMs
ssh azureuser@<vm1-ip>  # VM1
ssh azureuser@<vm2-ip>  # VM2

# Or use helper (auto-discovers IP)
./scripts/ssh-vm.sh 1   # VM1
./scripts/ssh-vm.sh 2   # VM2
```

## DPDK Testing

Inside the VMs, the second NIC can be used with DPDK virtio PMD:

```bash
# Check virtio devices
lspci | grep -i virtio

# The second NIC (around 0000:00:06.0) is for DPDK
# No driver binding needed - virtio PMD works directly

# Run DPDK app
./dpdk-app -a 0000:00:06.0 -- <app args>
```

## Configuration

Edit `terraform.tfvars` to customize:

```hcl
vm_count        = 2     # Number of VMs
nics_per_vm     = 2     # NICs per VM
vm1_memory_mb   = 4096  # VM1 RAM
vm1_vcpus       = 2     # VM1 CPUs
hugepages_count = 512   # 2MB hugepages for DPDK
```

## Troubleshooting

### Console access
```bash
virsh console dpdk-vm1
# Ctrl+] to exit
```

### Check VM status
```bash
virsh list --all
virsh dominfo dpdk-vm1
```

### View network info
```bash
virsh net-list
virsh net-info dpdk-mgmt-net
```

### Cloud-init logs (inside VM)
```bash
sudo cat /var/log/cloud-init-output.log
```
