variable "libvirt_uri" {
  description = "Libvirt connection URI"
  type        = string
  default     = "qemu:///system"
}

variable "ssh_public_key" {
  description = "SSH public key for VM access (if empty, reads from ssh_key_path)"
  type        = string
  default     = ""
}

variable "ssh_key_path" {
  description = "Path to SSH public key file"
  type        = string
  default     = "~/.ssh/id_rsa.pub"
}

variable "ssh_user" {
  description = "SSH username for VMs (matches Azure for Ansible compatibility)"
  type        = string
  default     = "azureuser"
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

variable "base_image_url" {
  description = "URL to Ubuntu cloud image"
  type        = string
  default     = "https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img"
}

variable "vm_count" {
  description = "Number of VMs to deploy (1 or 2)"
  type        = number
  default     = 2

  validation {
    condition     = var.vm_count >= 1 && var.vm_count <= 2
    error_message = "vm_count must be 1 or 2"
  }
}

variable "nics_per_vm" {
  description = "Number of NICs per VM (1 or 2). Use 2 for DPDK scenarios."
  type        = number
  default     = 2

  validation {
    condition     = var.nics_per_vm >= 1 && var.nics_per_vm <= 2
    error_message = "nics_per_vm must be 1 or 2"
  }
}

variable "vm1_memory_mb" {
  description = "Memory for VM1 in MB"
  type        = number
  default     = 4096
}

variable "vm1_vcpus" {
  description = "Number of vCPUs for VM1"
  type        = number
  default     = 2
}

variable "vm2_memory_mb" {
  description = "Memory for VM2 in MB"
  type        = number
  default     = 4096
}

variable "vm2_vcpus" {
  description = "Number of vCPUs for VM2"
  type        = number
  default     = 2
}

variable "disk_size" {
  description = "OS disk size in bytes (default 6GB)"
  type        = number
  default     = 6442450944 # 6 GB
}

variable "hugepages_count" {
  description = "Number of 2MB hugepages to allocate in each VM for DPDK"
  type        = number
  default     = 10
}
