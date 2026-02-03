# Local VM Testing Infrastructure (QEMU/KVM + Terraform)

This document describes the local VM-based testing framework using Terraform and QEMU/KVM, mirroring the Azure-based infrastructure in `tests/infra/`.

## Current Status

**✅ Implemented:**
- Terraform configuration with libvirt provider (~> 0.9)
- Two Ubuntu 24.04 VMs with dual NICs (virtio-net)
- Dual network topology (management + DPDK)
- Hugepages configuration (10 x 2MB per VM)
- Cloud-init for minimal VM setup (SSH user, hugepages, network)
- Helper scripts (`start-vm.sh`, `destroy-vm.sh`, `ssh-vm.sh`)
- Ansible integration with `--local` flag for `run_tests.sh`
- GitHub Actions E2E job (software emulation with `--no-kvm`)

**VM Defaults:**
- Memory: 4GB per VM
- Disk: 6GB per VM
- vCPUs: 2 per VM

## Quick Start

```bash
cd tests/infra-local

# Start VMs (auto-approve)
./scripts/start-vm.sh -y

# Start VMs without KVM (software emulation, for CI)
./scripts/start-vm.sh --no-kvm -y

# SSH to VMs (waits for VM to be ready)
./scripts/ssh-vm.sh 1
./scripts/ssh-vm.sh 2

# Run Ansible tests on local VMs
cd ../e2e
./run_tests.sh --local playbooks/test_connectivity.yml

# Destroy VMs
cd ../infra-local
./scripts/destroy-vm.sh -y
```

## Network Topology

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Network Topology                             │
│                                                                     │
│   ┌──────────────┐                        ┌──────────────┐         │
│   │     VM1      │                        │     VM2      │         │
│   │              │                        │              │         │
│   │  enp0s3      │                        │  enp0s3      │         │
│   │  DHCP        │                        │  DHCP        │         │
│   │  [kernel]    │                        │  [kernel]    │         │
│   └──────┬───────┘                        └──────┬───────┘         │
│          │           Management Network          │                  │
│          └──────────── default (NAT) ───────────┘                  │
│                192.168.122.0/24 (DHCP)                              │
│                    (SSH, sockets)                                   │
│                                                                     │
│   ┌──────────────┐                        ┌──────────────┐         │
│   │     VM1      │                        │     VM2      │         │
│   │              │                        │              │         │
│   │  enp0s4      │                        │  enp0s4      │         │
│   │  10.0.1.4    │                        │  10.0.1.6    │         │
│   │  [DPDK PMD]  │                        │  [DPDK PMD]  │         │
│   └──────┬───────┘                        └──────┬───────┘         │
│          │             DPDK Network              │                  │
│          └──────────── dpdk-data-net ───────────┘                  │
│                   10.0.1.0/24 (static)                              │
│                   (isolated, DPDK only)                             │
└─────────────────────────────────────────────────────────────────────┘
```

### IP Allocation

| VM | Interface | Network | IP | Driver | Purpose |
|----|-----------|---------|-----|--------|---------|
| VM1 | enp0s3 | default | DHCP (192.168.122.x) | kernel | SSH, management |
| VM1 | enp0s4 | dpdk-data-net | 10.0.1.4 | DPDK virtio PMD | DPDK traffic |
| VM2 | enp0s3 | default | DHCP (192.168.122.x) | kernel | SSH, management |
| VM2 | enp0s4 | dpdk-data-net | 10.0.1.6 | DPDK virtio PMD | DPDK traffic |

## Directory Structure

```
tests/infra-local/
├── Infra.md                    # This document
├── README.md                   # Usage instructions
├── main.tf                     # Main Terraform config (VMs, networks, cloud-init)
├── variables.tf                # Input variables
├── outputs.tf                  # Output values (IPs, etc.)
├── versions.tf                 # Provider versions
├── terraform.tfvars            # Default variable values
├── .terraform.lock.hcl         # Provider lock file (committed)
├── .gitignore                  # Terraform state files
├── scripts/
│   ├── start-vm.sh             # Deploy VMs (terraform apply wrapper)
│   ├── destroy-vm.sh           # Destroy VMs (terraform destroy wrapper)
│   └── ssh-vm.sh               # SSH to VM helper (waits for SSH)
└── CMakeLists.txt              # CMake targets

tests/e2e/
├── inventory_local.py          # Dynamic inventory for local VMs
├── run_tests.sh                # Test runner (--local flag for local VMs)
└── playbooks/
    └── test_connectivity.yml   # Ping test between VMs
```

## What Cloud-Init Does (Minimal Setup)

Cloud-init performs only essential setup to keep VM boot fast:

1. **User setup** - Creates `azureuser` with SSH key and sudo access
2. **Hugepages** - Configures hugepages for DPDK (default: 10 x 2MB)
3. **DPDK network IP** - Sets static IP (10.0.1.x) on second NIC via netplan

**Not done by cloud-init (deferred to Ansible):**
- Package installation (apt update/install)
- DPDK build dependencies
- DPDK compilation and installation

## Ansible Integration

Run Ansible playbooks against local VMs using the `--local` flag:

```bash
cd tests/e2e
./run_tests.sh --local playbooks/test_connectivity.yml
./run_tests.sh --local playbooks/hello_world.yml
```

The `inventory_local.py` script dynamically discovers VM IPs from libvirt DHCP leases.

## Prerequisites

### Install Required Packages
```bash
# Host requirements (Ubuntu/Debian)
sudo apt install -y qemu-kvm libvirt-daemon-system \
  libvirt-clients libvirt-dev bridge-utils virtinst \
  qemu-utils cloud-image-utils terraform
```

### User Permissions
```bash
# Add user to libvirt group
sudo usermod -aG libvirt $USER
newgrp libvirt

# Verify libvirt access
virsh list --all
```

### Create Default Storage Pool
```bash
# Create storage pool if it doesn't exist
sudo mkdir -p /var/lib/libvirt/images
sudo virsh pool-define-as default dir --target /var/lib/libvirt/images
sudo virsh pool-start default
sudo virsh pool-autostart default

# Verify
virsh pool-info default
```

#### 1.4 Install Terraform
```bash
# Download and install Terraform
cd /tmp
wget https://releases.hashicorp.com/terraform/1.14.4/terraform_1.14.4_linux_amd64.zip
unzip terraform_1.14.4_linux_amd64.zip
sudo mv terraform /usr/local/bin/
terraform --version
```

#### 1.5 Terraform Provider Configuration
Use the `dmacvicar/libvirt` provider for QEMU/KVM:
```hcl
# versions.tf
terraform {
  required_version = ">= 1.0"

  required_providers {
    libvirt = {
      source  = "dmacvicar/libvirt"
      version = "~> 0.9"
    }
    null = {
      source  = "hashicorp/null"
      version = "~> 3.0"
    }
  }
}

provider "libvirt" {
  uri = "qemu:///system"
}
```

## Technical Details

### Network Configuration

Two separate networks ensure normal socket traffic and DPDK traffic don't interfere:

1. **Management Network (`default`)** - libvirt's default NAT network
   - IP range: 192.168.122.0/24 (DHCP)
   - Mode: NAT (VMs can reach host and internet)
   - Purpose: SSH access, normal socket traffic

2. **DPDK Network (`dpdk-data-net`)** - Custom isolated network
   - IP range: 10.0.1.0/24 (static)
   - Mode: Isolated (VM-to-VM only)
   - Purpose: DPDK packet I/O, bypasses kernel

The DPDK network is created via `null_resource` with virsh commands since the libvirt provider has limited network support:

```hcl
resource "null_resource" "dpdk_network" {
  provisioner "local-exec" {
    command = <<-EOT
      export LIBVIRT_DEFAULT_URI="qemu:///system"
      # Create isolated network for DPDK
      virsh net-define /tmp/dpdk-data-net.xml
      virsh net-start dpdk-data-net
      virsh net-autostart dpdk-data-net
    EOT
  }
}
```

### VM Configuration

Each VM is created with:
- **2 vCPUs** (configurable via `vm1_vcpus`, `vm2_vcpus`)
- **4GB RAM** (configurable via `vm1_memory_mb`, `vm2_memory_mb`)
- **10GB disk** (overlay on shared base image)
- **2 NICs** (management + DPDK)
- **512 hugepages** (2MB each = 1GB for DPDK)
- **CPU: host-passthrough** (for KVM acceleration)

### Hugepages

Configured via cloud-init bootcmd:
```yaml
bootcmd:
  - mkdir -p /dev/hugepages
  - mount -t hugetlbfs nodev /dev/hugepages || true
  - echo 512 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
```

Verify inside VM:
```bash
cat /proc/meminfo | grep Huge
# HugePages_Total:     512
# Hugepagesize:       2048 kB
```

### DPDK with virtio PMD

DPDK's **virtio PMD** (Poll Mode Driver) talks directly to QEMU's virtio-net backend:

- No kernel driver binding (like VFIO) needed inside the guest
- Works with standard libvirt networks
- Performance is lower than bare-metal but sufficient for functional testing

**Check virtio devices inside VM:**
```bash
lspci | grep -i virtio
# 00:03.0 Ethernet controller: Red Hat, Inc. Virtio network device (enp0s3 - management)
# 00:04.0 Ethernet controller: Red Hat, Inc. Virtio network device (enp0s4 - DPDK)
```

**Run DPDK app with virtio PMD:**
```bash
# Get PCI address of DPDK NIC
DPDK_NIC="0000:00:04.0"

# Run DPDK app (no driver binding needed for virtio PMD)
./dpdk-app -a $DPDK_NIC -- <app args>
```

---

## Reference: Terraform Resources

### Base Image Volume
```hcl
resource "libvirt_volume" "base_image" {
  name = "dpdk-base-ubuntu-24.04.qcow2"
  pool = var.storage_pool

  create = {
    content = {
      url = "https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img"
    }
  }
}
```

### VM Disk Volume (Overlay)
```hcl
resource "libvirt_volume" "vm1_disk" {
  name     = "dpdk-vm1-disk.qcow2"
  pool     = var.storage_pool
  capacity = 10737418240  # 10GB

  target = {
    format = { type = "qcow2" }
  }

  backing_store = {
    path = libvirt_volume.base_image.path
    format = { type = "qcow2" }
  }
}
```

#### 3.3 Cloud-Init Configuration
```hcl
# Cloud-init with network config
resource "libvirt_cloudinit_disk" "vm1_cloudinit" {
  name = "dpdk-vm1-cloudinit"

  meta_data = yamlencode({
    instance-id    = "dpdk-vm1-${formatdate("YYYYMMDDhhmmss", timestamp())}"
    local-hostname = "dpdk-vm1"
  })

  user_data = <<-EOF
    #cloud-config
    hostname: dpdk-vm1
    users:
      - name: azureuser
        sudo: ALL=(ALL) NOPASSWD:ALL
        shell: /bin/bash
        ssh_authorized_keys:
          - ${local.ssh_public_key}
    
    package_update: false
    packages:
      - build-essential
      - meson
      - ninja-build
      - python3-pyelftools
      - libnuma-dev
      - pkg-config
    
    # Setup hugepages for DPDK
    bootcmd:
      - mkdir -p /dev/hugepages
      - mount -t hugetlbfs nodev /dev/hugepages
      - echo 512 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
    
    write_files:
      - path: /etc/sysctl.d/99-hugepages.conf
        content: |
          vm.nr_hugepages = 512
    
    runcmd:
      - sysctl -p /etc/sysctl.d/99-hugepages.conf
  EOF

  # Network config for dual NICs
  network_config = yamlencode({
    version = 2
    ethernets = {
      eth0 = {
        match = { name = "en*" }
        addresses = ["10.0.0.4/24"]
        gateway4 = "10.0.0.1"
      }
      # Note: Second NIC (DPDK) doesn't need kernel config
      # It will be managed by DPDK virtio PMD
    }
  })
}
```

#### 3.4 VM Module
```hcl
# modules/vm/main.tf
resource "libvirt_volume" "vm_disk" {
  name     = "${var.vm_name}-disk.qcow2"
  pool     = var.storage_pool
  capacity = var.disk_size

  target = {
    format = { type = "qcow2" }
  }

  backing_store = {
    path   = var.base_image_path
    format = { type = "qcow2" }
  }
}

resource "libvirt_cloudinit_disk" "vm_init" {
  name      = "${var.vm_name}-init.iso"
  user_data = var.user_data
  pool      = "default"
}

resource "libvirt_domain" "vm" {
  name        = var.vm_name
  memory      = var.memory_mb
  memory_unit = "MiB"
  vcpu        = var.vcpus
  type        = var.use_kvm ? "kvm" : "qemu"
  running     = true
  autostart   = true

  os = {
    type      = "hvm"
    type_arch = "x86_64"
    boot_devices = [{ dev = "hd" }]
  }

  # CPU: host-passthrough for KVM, Nehalem for emulation (SSE4.2 for DPDK)
  cpu = {
    mode  = var.use_kvm ? "host-passthrough" : "custom"
    model = var.use_kvm ? null : "Nehalem"
  }

  cloudinit = libvirt_cloudinit_disk.vm_init.id

  # Primary NIC (management - kernel networking)
  network_interface {
    network_id     = var.primary_network_id
    addresses      = [var.primary_ip]
    wait_for_lease = false
  }

  # Secondary NIC (DPDK - virtio PMD)
  dynamic "network_interface" {
    for_each = var.enable_secondary_nic ? [1] : []
    content {
      network_id     = var.secondary_network_id
      addresses      = [var.secondary_ip]
      wait_for_lease = false
    }
  }

  disk {
    volume_id = libvirt_volume.vm_disk.id
  }

  console {
    type        = "pty"
    target_port = "0"
    target_type = "serial"
  }

  graphics {
    type        = "vnc"
    listen_type = "address"
    autoport    = true
  }

  # QEMU guest agent channel for better VM management
  channel {
    type = "unix"
    target_type = "virtio"
    target_name = "org.qemu.guest_agent.0"
  }
}
```

### Phase 4: Helper Scripts

Based on the working scripts from bypass-misc2, create helper scripts for easier management.

#### 4.1 scripts/start-vm.sh
```bash
#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

export LIBVIRT_DEFAULT_URI="qemu:///system"

# Parse arguments
USE_KVM=true
AUTO_APPROVE=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --no-kvm) USE_KVM=false; shift ;;
        -y|--yes) AUTO_APPROVE=true; shift ;;
        -h|--help)
            echo "Usage: $0 [--no-kvm] [-y]"
            echo "  --no-kvm  Use QEMU software emulation (slower)"
            echo "  -y        Auto-approve without prompting"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Validate prerequisites
if ! systemctl is-active --quiet libvirtd; then
    echo "Error: libvirtd not running. Start with: sudo systemctl start libvirtd"
    exit 1
fi

if ! virsh list &>/dev/null; then
    echo "Error: Cannot connect to libvirt. Add user to 'libvirt' group."
    exit 1
fi

if ! virsh pool-info default &>/dev/null; then
    echo "Error: Storage pool 'default' not found. See Infra.md for setup."
    exit 1
fi

# Ensure networks exist (create if needed)
for net in dpdk-mgmt-net dpdk-data-net; do
    if ! virsh net-info $net &>/dev/null; then
        echo "Creating network: $net"
        # Networks will be created by Terraform
    fi
done

# Initialize Terraform
[[ -d ".terraform" ]] || terraform init

# Build args
TF_VAR_ARGS=""
[[ "$USE_KVM" == "false" ]] && TF_VAR_ARGS="-var=use_kvm=false"

# Plan
terraform plan $TF_VAR_ARGS -out=tfplan

# Apply
if [[ "$AUTO_APPROVE" == "true" ]]; then
    terraform apply $TF_VAR_ARGS tfplan
else
    read -p "Apply? (y/N) " -n 1 -r
    echo
    [[ $REPLY =~ ^[Yy]$ ]] && terraform apply $TF_VAR_ARGS tfplan
fi
rm -f tfplan

# Wait for VMs and show IPs
echo "Waiting for VMs to boot..."
sleep 10
virsh net-dhcp-leases dpdk-mgmt-net 2>/dev/null || true

echo ""
terraform output
```

#### 4.2 scripts/destroy-vm.sh
```bash
#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

export LIBVIRT_DEFAULT_URI="qemu:///system"

AUTO_APPROVE=false
[[ "$1" == "-y" || "$1" == "--yes" ]] && AUTO_APPROVE=true

if [[ "$AUTO_APPROVE" == "true" ]]; then
    terraform destroy -auto-approve
else
    terraform destroy
fi

echo "VMs destroyed."
```

### Phase 5: Variables and Configuration

#### 5.1 Input Variables (variables.tf)
```hcl
variable "ssh_public_key" {
  description = "SSH public key for VM access"
  type        = string
  default     = ""
}

variable "ssh_key_path" {
  description = "Path to SSH public key file"
  type        = string
  default     = "~/.ssh/id_rsa.pub"
}

variable "use_kvm" {
  description = "Use KVM hardware acceleration (false for software emulation)"
  type        = bool
  default     = true
}

variable "storage_pool" {
  description = "Libvirt storage pool name"
  type        = string
  default     = "default"
}

variable "vm_count" {
  description = "Number of VMs (1 or 2)"
  type        = number
  default     = 2
  validation {
    condition     = var.vm_count >= 1 && var.vm_count <= 2
    error_message = "vm_count must be 1 or 2"
  }
}

variable "nics_per_vm" {
  description = "NICs per VM (1 or 2)"
  type        = number
  default     = 2
}

variable "vm1_vcpus" {
  description = "vCPUs for VM1"
  type        = number
  default     = 2
}

variable "vm1_memory" {
  description = "Memory for VM1 in MB"
  type        = number
  default     = 4096
}

variable "vm2_vcpus" {
  description = "vCPUs for VM2"
  type        = number
  default     = 2
}

variable "vm2_memory" {
  description = "Memory for VM2 in MB"
  type        = number
  default     = 4096
}
```

#### 4.2 VM Size Mapping (Azure → Local)
| Azure Size | vCPUs | RAM | Local Equivalent |
|------------|-------|-----|------------------|
| Standard_D2s_v5 | 2 | 8GB | 2 vCPU, 4-8GB |
| Standard_D4s_v5 | 4 | 16GB | 4 vCPU, 8-16GB |
| Standard_D8s_v5 | 8 | 32GB | 8 vCPU, 16-32GB |

### Phase 5: Outputs for Ansible Integration

#### 5.1 outputs.tf
```hcl
output "vm1_name" {
  value = module.vm1.name
}

output "vm1_private_ip" {
  value = "10.0.0.4"
}

output "vm1_private_ip2" {
  value = var.nics_per_vm == 2 ? "10.0.0.5" : ""
}

output "vm2_name" {
  value = var.vm_count == 2 ? module.vm2[0].name : ""
}

output "vm2_private_ip" {
  value = var.vm_count == 2 ? "10.0.0.6" : ""
}

output "vm2_private_ip2" {
  value = var.vm_count == 2 && var.nics_per_vm == 2 ? "10.0.0.7" : ""
}

# Generate JSON for Ansible inventory (compatible with e2e/inventory.py)
output "ansible_inventory_json" {
  value = jsonencode({
    vm1_public_ip  = "10.0.0.4"  # Use private IP for local
    vm1_private_ip = "10.0.0.4"
    vm2_public_ip  = var.vm_count == 2 ? "10.0.0.6" : ""
    vm2_private_ip = var.vm_count == 2 ? "10.0.0.6" : ""
  })
}
```

### Phase 6: Integration with Existing E2E Tests

#### 6.1 Modify inventory.py
Update `tests/e2e/inventory.py` to support both Azure and local deployments:
```python
# Check for local deployment outputs
LOCAL_OUTPUTS = "build/docs/local-deployment-outputs.json"
AZURE_OUTPUTS = "build/docs/azure-deployment-outputs.json"

def load_outputs():
    if os.path.exists(LOCAL_OUTPUTS):
        return json.load(open(LOCAL_OUTPUTS))
    return json.load(open(AZURE_OUTPUTS))
```

#### 6.2 CMakeLists.txt
```cmake
# tests/infra-local/CMakeLists.txt

set(TF_DIR ${CMAKE_CURRENT_SOURCE_DIR})
set(SSH_KEY_PATH "$ENV{HOME}/.ssh/id_rsa.pub")
set(LOCAL_OUTPUTS_FILE "${CMAKE_BINARY_DIR}/docs/local-deployment-outputs.json")

# Find terraform executable
find_program(TERRAFORM_EXECUTABLE terraform REQUIRED)

# Download Ubuntu cloud image (one-time)
add_custom_target(local_vm_image
  COMMAND ${CMAKE_CURRENT_SOURCE_DIR}/scripts/download-image.sh
  WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
  COMMENT "Downloading Ubuntu 24.04 cloud image"
)

# Initialize Terraform
add_custom_target(local_vm_init
  COMMAND ${TERRAFORM_EXECUTABLE} -chdir=${TF_DIR} init
  COMMENT "Initializing Terraform for local VMs"
)

# Plan deployment (dry-run)
add_custom_target(local_vm_plan
  COMMAND ${TERRAFORM_EXECUTABLE} -chdir=${TF_DIR} plan
    -var "ssh_public_key=$<SHELL_OUTPUT:cat ${SSH_KEY_PATH}>"
  DEPENDS local_vm_init
  COMMENT "Planning local VM deployment"
)

# Deploy VMs
add_custom_target(local_vm_deploy
  COMMAND ${TERRAFORM_EXECUTABLE} -chdir=${TF_DIR} apply -auto-approve
    -var "ssh_public_key=$<SHELL_OUTPUT:cat ${SSH_KEY_PATH}>"
  COMMAND ${TERRAFORM_EXECUTABLE} -chdir=${TF_DIR} output -json > ${LOCAL_OUTPUTS_FILE}
  DEPENDS local_vm_init
  COMMENT "Deploying local QEMU/KVM VMs"
)

# Destroy VMs
add_custom_target(local_vm_destroy
  COMMAND ${TERRAFORM_EXECUTABLE} -chdir=${TF_DIR} destroy -auto-approve
  COMMENT "Destroying local QEMU/KVM VMs"
)

# Export outputs for Ansible
add_custom_target(local_vm_outputs
  COMMAND ${CMAKE_COMMAND} -E make_directory ${CMAKE_BINARY_DIR}/docs
  COMMAND ${TERRAFORM_EXECUTABLE} -chdir=${TF_DIR} output -json > ${LOCAL_OUTPUTS_FILE}
  COMMENT "Exporting local VM deployment outputs"
)

# SSH to VMs
add_custom_target(local_vm_ssh1
  COMMAND ssh -o StrictHostKeyChecking=no azureuser@10.0.0.4
  COMMENT "SSH to VM1"
  USES_TERMINAL
)

add_custom_target(local_vm_ssh2
  COMMAND ssh -o StrictHostKeyChecking=no azureuser@10.0.0.6
  COMMENT "SSH to VM2"
  USES_TERMINAL
)

# Run e2e tests on local VMs
add_custom_target(local_vm_test
  COMMAND ${CMAKE_SOURCE_DIR}/tests/e2e/run_tests.sh
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}/tests/e2e
  DEPENDS local_vm_deploy
  COMMENT "Running e2e tests on local VMs"
  USES_TERMINAL
)

# Full workflow: image -> init -> deploy -> test
add_custom_target(local_vm_full
  DEPENDS local_vm_image local_vm_deploy local_vm_test
  COMMENT "Full local VM workflow: image + deploy + test"
)
```

**Integration with root CMakeLists.txt:**
```cmake
# In root CMakeLists.txt, add:
add_subdirectory(tests/infra-local)
```

### Phase 7: DPDK with QEMU Emulated Devices

DPDK supports QEMU's **virtio-net** devices through the **virtio PMD** (Poll Mode Driver). This allows DPDK testing without physical hardware or SR-IOV.

#### 7.1 How DPDK Works with QEMU virtio-net

```
┌─────────────────────────────────────────────────────────────┐
│                        QEMU Host                            │
│  ┌─────────────┐                      ┌─────────────┐       │
│  │    VM1      │                      │    VM2      │       │
│  │  ┌───────┐  │                      │  ┌───────┐  │       │
│  │  │ DPDK  │  │                      │  │ DPDK  │  │       │
│  │  │App    │  │                      │  │App    │  │       │
│  │  └───┬───┘  │                      │  └───┬───┘  │       │
│  │      │      │                      │      │      │       │
│  │  ┌───┴───┐  │                      │  ┌───┴───┐  │       │
│  │  │virtio │  │    libvirt bridge    │  │virtio │  │       │
│  │  │PMD    │◄─┼──────────────────────┼─►│PMD    │  │       │
│  │  └───────┘  │     (10.0.0.0/24)    │  └───────┘  │       │
│  └─────────────┘                      └─────────────┘       │
└─────────────────────────────────────────────────────────────┘
```

**Key Points:**
- DPDK's `virtio PMD` talks directly to QEMU's virtio-net backend
- No kernel driver binding (like VFIO) needed inside the guest
- Works with standard libvirt networks (NAT or bridge mode)
- Performance is lower than bare-metal but sufficient for functional testing

#### 7.2 virtio-net Device Configuration

The virtio-net device needs specific features enabled for optimal DPDK performance:

```hcl
# modules/vm/main.tf - Network interface with DPDK-friendly settings
resource "libvirt_domain" "vm" {
  name   = var.vm_name
  memory = var.memory_mb
  vcpu   = var.vcpus

  # Enable KVM acceleration
  type = "kvm"

  # CPU configuration for DPDK performance
  cpu {
    mode = "host-passthrough"
  }

  # Primary NIC - for SSH/management (uses kernel driver)
  network_interface {
    network_id     = var.primary_network_id
    addresses      = [var.primary_ip]
    wait_for_lease = false
  }

  # Secondary NIC - for DPDK (uses virtio PMD)
  dynamic "network_interface" {
    for_each = var.enable_secondary_nic ? [1] : []
    content {
      network_id     = var.secondary_network_id
      addresses      = [var.secondary_ip]
      wait_for_lease = false
    }
  }

  # ... rest of config
}
```

#### 7.3 Advanced virtio-net Options via libvirt XML

For more control, use libvirt XML customization:

```xml
<!-- Enhanced virtio-net for DPDK -->
<interface type='network'>
  <source network='dpdk-secondary-net'/>
  <model type='virtio'/>
  <driver name='vhost' queues='4'/>  <!-- Multi-queue for parallelism -->
  <address type='pci' domain='0x0000' bus='0x00' slot='0x05' function='0x0'/>
</interface>
```

In Terraform, add XML customization:
```hcl
resource "libvirt_domain" "vm" {
  # ... other config ...

  xml {
    xslt = file("${path.module}/dpdk-nic.xslt")
  }
}
```

**dpdk-nic.xslt** - Transform to add multi-queue and vhost:
```xml
<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="xml" indent="yes"/>
  
  <!-- Identity transform -->
  <xsl:template match="@*|node()">
    <xsl:copy>
      <xsl:apply-templates select="@*|node()"/>
    </xsl:copy>
  </xsl:template>
  
  <!-- Add multi-queue driver to virtio interfaces -->
  <xsl:template match="interface[model/@type='virtio' and not(driver)]">
    <xsl:copy>
      <xsl:apply-templates select="@*|node()"/>
      <driver name="vhost" queues="4"/>
    </xsl:copy>
  </xsl:template>
</xsl:stylesheet>
```

#### 7.4 Hugepages Configuration

Hugepages are **required** for DPDK. Configure both host and guest:

**Host Configuration** (one-time setup):
```bash
# Reserve hugepages on host for QEMU
echo 4096 | sudo tee /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages

# Make persistent
echo "vm.nr_hugepages = 4096" | sudo tee -a /etc/sysctl.conf

# Mount hugepages (usually automatic)
sudo mount -t hugetlbfs hugetlbfs /dev/hugepages
```

**Terraform - Enable hugepages backing for VM memory**:
```hcl
resource "libvirt_domain" "vm" {
  # ... other config ...

  memory = var.memory_mb

  # Use hugepages for VM memory (better DPDK performance)
  memorybacking {
    hugetlbfs = true
  }
}
```

**Guest Configuration** (cloud-init):
```yaml
# cloud-init/user-data.yaml
#cloud-config
users:
  - name: azureuser
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    ssh_authorized_keys:
      - ${ssh_public_key}

# Configure hugepages inside guest
bootcmd:
  - mkdir -p /dev/hugepages
  - mount -t hugetlbfs nodev /dev/hugepages
  - echo 512 > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages

write_files:
  - path: /etc/sysctl.d/99-hugepages.conf
    content: |
      vm.nr_hugepages = 512

packages:
  - build-essential
  - meson
  - ninja-build
  - python3-pyelftools
  - libnuma-dev
  - pkg-config

runcmd:
  - sysctl -p /etc/sysctl.d/99-hugepages.conf
```

#### 7.5 Using virtio PMD in DPDK Application

Inside the VM, bind the secondary NIC to DPDK's virtio PMD:

```bash
# 1. Check available virtio devices
lspci | grep -i virtio
# Output: 00:05.0 Ethernet controller: Red Hat, Inc. Virtio network device

# 2. Get PCI address of secondary NIC
DPDK_NIC="0000:00:05.0"  # Secondary NIC for DPDK

# 3. No driver binding needed! virtio PMD works directly
# The virtio PMD accesses the device via /sys/bus/pci

# 4. Run DPDK app with virtio PMD
./dpdk-app -a $DPDK_NIC -- <app args>

# Or with EAL parameters for hugepages
./dpdk-app --lcores 0-1 -a $DPDK_NIC -- <app args>
```

**Important**: Unlike physical NICs (which need vfio-pci or igb_uio binding), virtio PMD works directly without unbinding from the kernel driver!

#### 7.6 DPDK EAL Parameters for QEMU/virtio

Recommended EAL parameters for virtio in QEMU:

```bash
# Basic virtio PMD usage
./dpdk-app \
  --lcores 0-1 \              # Use cores 0 and 1
  -a 0000:00:05.0 \           # virtio NIC PCI address
  --huge-dir /dev/hugepages \ # Hugepage mount point
  --file-prefix dpdk1 \       # Unique prefix (allows multiple DPDK apps)
  -- <app args>

# With specific memory channels (for NUMA)
./dpdk-app \
  -l 0-1 \
  -n 2 \                      # Memory channels
  -a 0000:00:05.0 \
  -- <app args>
```

#### 7.7 Network Topology for DPDK Testing

```
┌───────────────────────────────────────────────────────────────────────┐
│                      libvirt Network Setup                            │
│                                                                       │
│  ┌─────────────────┐                    ┌─────────────────┐          │
│  │       VM1       │                    │       VM2       │          │
│  │                 │                    │                 │          │
│  │  NIC1 (eth0)    │                    │  NIC1 (eth0)    │          │
│  │  10.0.0.4       │                    │  10.0.0.6       │          │
│  │  [kernel]       │                    │  [kernel]       │          │
│  └────────┬────────┘                    └────────┬────────┘          │
│           │        Management Network            │                    │
│           └─────────── 10.0.0.0/24 ─────────────┘                    │
│                          │                                            │
│                     [NAT to host]                                     │
│                    SSH, TCP sockets                                   │
│                                                                       │
│  ┌─────────────────┐                    ┌─────────────────┐          │
│  │       VM1       │                    │       VM2       │          │
│  │                 │                    │                 │          │
│  │  NIC2           │                    │  NIC2           │          │
│  │  10.0.1.4       │                    │  10.0.1.6       │          │
│  │  [DPDK PMD]     │                    │  [DPDK PMD]     │          │
│  └────────┬────────┘                    └────────┬────────┘          │
│           │           DPDK Network               │                    │
│           └─────────── 10.0.1.0/24 ─────────────┘                    │
│                      (isolated)                                       │
│                   DPDK packet I/O                                     │
└───────────────────────────────────────────────────────────────────────┘
```

**Traffic Flow Examples:**

```bash
# Normal socket traffic (kernel networking)
# VM1 -> VM2 via management network
ssh azureuser@10.0.0.6          # SSH from VM1 to VM2
curl http://10.0.0.6:8080       # HTTP request to kernel TCP server
nc -l 9000                      # Netcat listener on VM2
nc 10.0.0.6 9000                # Connect from VM1

# DPDK traffic (bypasses kernel)
# VM1 DPDK app -> VM2 DPDK app via DPDK network
# On VM2: dpdk-server -a 0000:00:05.0 --ip 10.0.1.6
# On VM1: dpdk-client -a 0000:00:05.0 --server 10.0.1.6
```

#### 7.8 Alternative: vhost-user for Higher Performance

For better performance (closer to bare-metal), use **vhost-user** backend:

```xml
<!-- In libvirt domain XML -->
<interface type='vhostuser'>
  <source type='unix' path='/tmp/vhost-user1.sock' mode='server'/>
  <model type='virtio'/>
  <driver queues='4'/>
</interface>
```

This requires running Open vSwitch with DPDK on the host, which is more complex but offers:
- Near-line-rate performance
- Better suited for performance benchmarking
- Commonly used in NFV/SDN deployments

For functional testing, the standard virtio-net with virtio PMD is sufficient.

#### 7.9 Verifying DPDK Setup Inside VM

```bash
# SSH into VM
ssh azureuser@10.0.0.4

# Check virtio devices
lspci -v | grep -A5 "Virtio network"

# Verify hugepages
cat /proc/meminfo | grep Huge

# Check DPDK can see the device
# (After building DPDK)
dpdk-devbind.py --status

# Expected output:
# Network devices using kernel driver
# ===================================
# 0000:00:03.0 'Virtio network device' if=eth0 drv=virtio-pci ...
# 0000:00:05.0 'Virtio network device' if=eth1 drv=virtio-pci ...  <-- DPDK NIC
```

### Phase 8: Testing Workflow

#### 8.1 Full Workflow (CMake)
```bash
# From build directory
cd build

# 1. Download base image (one-time)
cmake --build . --target local_vm_image

# 2. Initialize Terraform
cmake --build . --target local_vm_init

# 3. Deploy VMs
cmake --build . --target local_vm_deploy

# 4. Run e2e tests
cmake --build . --target local_vm_test

# 5. Cleanup
cmake --build . --target local_vm_destroy

# Or run the full workflow in one command:
cmake --build . --target local_vm_full
```

#### 8.2 Available CMake Targets

| Target | Description |
|--------|-------------|
| `local_vm_image` | Download Ubuntu 24.04 cloud image |
| `local_vm_init` | Initialize Terraform |
| `local_vm_plan` | Plan deployment (dry-run) |
| `local_vm_deploy` | Deploy VMs and export outputs |
| `local_vm_destroy` | Destroy all VMs |
| `local_vm_outputs` | Export Terraform outputs to JSON |
| `local_vm_ssh1` | SSH to VM1 |
| `local_vm_ssh2` | SSH to VM2 |
| `local_vm_test` | Run e2e tests on local VMs |
| `local_vm_full` | Full workflow (image + deploy + test) |

#### 8.3 Integration with Existing Azure Targets

The local VM targets mirror the Azure targets:

| Azure Target | Local Target |
|--------------|---------------|
| `azure_vm_deploy` | `local_vm_deploy` |
| `azure_vm_destroy` | `local_vm_destroy` |
| `azure_vm_outputs` | `local_vm_outputs` |
| `e2e_test` | `local_vm_test` |

## Comparison: Azure vs Local

| Feature | Azure (Bicep) | Local (Terraform + QEMU) |
|---------|---------------|--------------------------|
| Provider | Azure | libvirt/QEMU |
| Network | VNet + NSG | libvirt NAT + isolated |
| VM Image | Ubuntu 24.04 | Ubuntu 24.04 cloud image |
| Auth | SSH keys | SSH keys |
| Management IPs | 10.0.0.0/24 | 192.168.122.0/24 (DHCP) |
| DPDK IPs | 10.0.0.0/24 (same) | 10.0.1.0/24 (isolated) |
| NICs | 1-2 per VM | 2 per VM |
| Auto-shutdown | DevTest Labs | N/A (manual) |
| Cost | Pay per use | Free (local resources) |

## Implementation Status

### ✅ Completed

1. [x] `versions.tf` - Provider requirements (libvirt ~> 0.9, null ~> 3.0)
2. [x] `variables.tf` - Input variables (vm_count, memory, hugepages, etc.)
3. [x] `main.tf` - Full VM configuration:
   - DPDK network via null_resource (dpdk-data-net)
   - Base image volume (auto-download Ubuntu 24.04)
   - VM disk volumes (overlay on base image)
   - Cloud-init with SSH user, hugepages, netplan
   - Two VM domains with dual NICs
4. [x] `outputs.tf` - VM names, IPs, SSH connection info
5. [x] `terraform.tfvars` - Default configuration
6. [x] `scripts/start-vm.sh` - Deploy with prerequisite validation, `--no-kvm` support
7. [x] `scripts/destroy-vm.sh` - Clean teardown
8. [x] `scripts/ssh-vm.sh` - SSH helper with wait-for-ready
9. [x] `CMakeLists.txt` - CMake targets
10. [x] `.gitignore` - Terraform state files
11. [x] `.terraform.lock.hcl` - Provider lock file
12. [x] `tests/e2e/inventory_local.py` - Dynamic Ansible inventory for local VMs
13. [x] `tests/e2e/run_tests.sh --local` - Run Ansible against local VMs
14. [x] `.github/workflows/CI.yml` - E2E job with QEMU software emulation

### 🔄 Future Work

1. [ ] Playbook: Install DPDK build dependencies
2. [ ] Playbook: Build and install DPDK
3. [ ] Playbook: Run DPDK benchmark tests
