# Local VM configuration for DPDK testing
# Uncomment and modify as needed

# Number of VMs (1 or 2)
vm_count = 2

# NICs per VM (2 for DPDK testing)
nics_per_vm = 2

# VM1 resources
vm1_memory_mb = 4096
vm1_vcpus     = 2

# VM2 resources
vm2_memory_mb = 4096
vm2_vcpus     = 2

# Hugepages for DPDK (512 x 2MB = 1GB)
hugepages_count = 512

# Use KVM acceleration (set to false for software emulation)
# use_kvm = true

# SSH user (matches Azure for Ansible compatibility)
# ssh_user = "azureuser"

# Custom SSH key path (defaults to ~/.ssh/id_rsa.pub)
# ssh_key_path = "~/.ssh/id_ed25519.pub"
