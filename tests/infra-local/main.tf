# Local DPDK Testing Infrastructure
# Creates 1-2 VMs with dual NICs for DPDK testing using QEMU/KVM

locals {
  ssh_public_key = var.ssh_public_key != "" ? var.ssh_public_key : file(pathexpand(var.ssh_key_path))
  
  # IP allocation
  # Management network: uses default libvirt network with DHCP
  # DPDK network: 10.0.1.0/24 (isolated)
  vm1_dpdk_ip = "10.0.1.4"
  vm2_dpdk_ip = "10.0.1.6"
}

# =============================================================================
# Networks
# =============================================================================

# DPDK network - isolated mode for DPDK traffic only
# Created via virsh since libvirt provider has limited network support
resource "null_resource" "dpdk_network" {
  triggers = {
    network_name = "dpdk-data-net"
  }

  provisioner "local-exec" {
    command = <<-EOT
      set -e
      export LIBVIRT_DEFAULT_URI="qemu:///system"
      NETWORK_NAME="dpdk-data-net"
      
      # Check if network exists and is active
      if virsh net-info $NETWORK_NAME 2>/dev/null | grep -q "Active:.*yes"; then
        echo "Network $NETWORK_NAME already exists and is active"
        exit 0
      fi
      
      # If network exists but not active, start it
      if virsh net-info $NETWORK_NAME 2>/dev/null; then
        echo "Network $NETWORK_NAME exists but not active, starting..."
        virsh net-start $NETWORK_NAME || true
        virsh net-autostart $NETWORK_NAME || true
        exit 0
      fi
      
      # Create network XML
      cat > /tmp/$NETWORK_NAME.xml <<EOF
<network>
  <name>$NETWORK_NAME</name>
  <bridge name="virbr-dpdk" stp="on" delay="0"/>
  <ip address="10.0.1.1" netmask="255.255.255.0">
  </ip>
</network>
EOF
      
      # Define and start network
      echo "Creating network $NETWORK_NAME..."
      virsh net-define /tmp/$NETWORK_NAME.xml
      virsh net-start $NETWORK_NAME
      virsh net-autostart $NETWORK_NAME
      rm -f /tmp/$NETWORK_NAME.xml
      
      # Verify network is active
      sleep 1
      virsh net-info $NETWORK_NAME
      echo "Network $NETWORK_NAME created successfully"
    EOT
  }

  provisioner "local-exec" {
    when    = destroy
    command = <<-EOT
      export LIBVIRT_DEFAULT_URI="qemu:///system"
      NETWORK_NAME="dpdk-data-net"
      virsh net-destroy $NETWORK_NAME 2>/dev/null || true
      virsh net-undefine $NETWORK_NAME 2>/dev/null || true
    EOT
  }
}

# =============================================================================
# Base Image
# =============================================================================

# Base OS image volume (downloaded from URL)
resource "libvirt_volume" "base_image" {
  name = "dpdk-base-ubuntu-24.04.qcow2"
  pool = var.storage_pool

  create = {
    content = {
      url = var.base_image_url
    }
  }
}

# =============================================================================
# VM1
# =============================================================================

# VM1 disk volume (overlay on base image)
resource "libvirt_volume" "vm1_disk" {
  name     = "dpdk-vm1-disk.qcow2"
  pool     = var.storage_pool
  capacity = var.disk_size

  target = {
    format = {
      type = "qcow2"
    }
  }

  backing_store = {
    path = libvirt_volume.base_image.path
    format = {
      type = "qcow2"
    }
  }
}

# VM1 cloud-init
resource "libvirt_cloudinit_disk" "vm1_cloudinit" {
  name = "dpdk-vm1-cloudinit"

  meta_data = yamlencode({
    instance-id    = "dpdk-vm1-${formatdate("YYYYMMDDhhmmss", timestamp())}"
    local-hostname = "dpdk-vm1"
  })

  user_data = <<-EOF
    #cloud-config
    hostname: dpdk-vm1
    fqdn: dpdk-vm1.local
    manage_etc_hosts: true
    
    users:
      - name: ${var.ssh_user}
        sudo: ALL=(ALL) NOPASSWD:ALL
        shell: /bin/bash
        ssh_authorized_keys:
          - ${local.ssh_public_key}
    
    package_update: false
    package_upgrade: false
    
    # Setup hugepages for DPDK
    bootcmd:
      - mkdir -p /dev/hugepages
      - mount -t hugetlbfs nodev /dev/hugepages || true
      - echo ${var.hugepages_count} > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
    
    write_files:
      - path: /etc/sysctl.d/99-hugepages.conf
        content: |
          vm.nr_hugepages = ${var.hugepages_count}
      - path: /etc/netplan/99-dpdk.yaml
        permissions: '0600'
        content: |
          network:
            version: 2
            ethernets:
              enp0s4:
                addresses:
                  - ${local.vm1_dpdk_ip}/24
    
    runcmd:
      - sysctl -p /etc/sysctl.d/99-hugepages.conf
      - netplan apply || true
  EOF

  network_config = yamlencode({
    version = 2
    ethernets = {
      eth0 = {
        match = {
          name = "en*"
        }
        dhcp4 = true
      }
    }
  })
}

# VM1 cloud-init volume
resource "libvirt_volume" "vm1_cloudinit" {
  name = "dpdk-vm1-cloudinit.iso"
  pool = var.storage_pool

  create = {
    content = {
      url = libvirt_cloudinit_disk.vm1_cloudinit.path
    }
  }

  lifecycle {
    replace_triggered_by = [libvirt_cloudinit_disk.vm1_cloudinit]
  }
}

# VM1 domain
resource "libvirt_domain" "vm1" {
  name        = "dpdk-vm1"
  memory      = var.vm1_memory_mb
  memory_unit = "MiB"
  vcpu        = var.vm1_vcpus
  type        = var.use_kvm ? "kvm" : "qemu"
  running     = true
  autostart   = true

  os = {
    type      = "hvm"
    type_arch = "x86_64"
    boot_devices = [
      { dev = "hd" }
    ]
  }

  cpu = {
    mode  = var.use_kvm ? "host-passthrough" : "custom"
    model = var.use_kvm ? null : "Nehalem"
  }

  devices = {
    disks = [
      {
        driver = {
          type = "qcow2"
        }
        source = {
          volume = {
            pool   = var.storage_pool
            volume = libvirt_volume.vm1_disk.name
          }
        }
        target = {
          dev = "vda"
          bus = "virtio"
        }
      },
      {
        device = "cdrom"
        source = {
          volume = {
            pool   = var.storage_pool
            volume = libvirt_volume.vm1_cloudinit.name
          }
        }
        target = {
          dev = "sda"
          bus = "sata"
        }
        readonly = true
      }
    ]

    # Network interfaces
    interfaces = concat(
      # Management NIC (uses default network with DHCP)
      [
        {
          model = {
            type = "virtio"
          }
          source = {
            network = {
              network = "default"
            }
          }
        }
      ],
      # DPDK NIC (uses isolated dpdk network) - only if nics_per_vm >= 2
      var.nics_per_vm >= 2 ? [
        {
          model = {
            type = "virtio"
          }
          source = {
            network = {
              network = "dpdk-data-net"
            }
          }
        }
      ] : []
    )

    consoles = [
      {
        target = {
          type = "serial"
          port = 0
        }
      }
    ]

    graphics = [
      {
        vnc = {
          auto_port = true
        }
      }
    ]

    channels = [
      {
        target = {
          virt_io = {
            name = "org.qemu.guest_agent.0"
          }
        }
      }
    ]
  }

  depends_on = [null_resource.dpdk_network]
}

# =============================================================================
# VM2 (optional, based on vm_count)
# =============================================================================

# VM2 disk volume
resource "libvirt_volume" "vm2_disk" {
  count    = var.vm_count >= 2 ? 1 : 0
  name     = "dpdk-vm2-disk.qcow2"
  pool     = var.storage_pool
  capacity = var.disk_size

  target = {
    format = {
      type = "qcow2"
    }
  }

  backing_store = {
    path = libvirt_volume.base_image.path
    format = {
      type = "qcow2"
    }
  }
}

# VM2 cloud-init
resource "libvirt_cloudinit_disk" "vm2_cloudinit" {
  count = var.vm_count >= 2 ? 1 : 0
  name  = "dpdk-vm2-cloudinit"

  meta_data = yamlencode({
    instance-id    = "dpdk-vm2-${formatdate("YYYYMMDDhhmmss", timestamp())}"
    local-hostname = "dpdk-vm2"
  })

  user_data = <<-EOF
    #cloud-config
    hostname: dpdk-vm2
    fqdn: dpdk-vm2.local
    manage_etc_hosts: true
    
    users:
      - name: ${var.ssh_user}
        sudo: ALL=(ALL) NOPASSWD:ALL
        shell: /bin/bash
        ssh_authorized_keys:
          - ${local.ssh_public_key}
    
    package_update: false
    package_upgrade: false
    
    # Setup hugepages for DPDK
    bootcmd:
      - mkdir -p /dev/hugepages
      - mount -t hugetlbfs nodev /dev/hugepages || true
      - echo ${var.hugepages_count} > /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages
    
    write_files:
      - path: /etc/sysctl.d/99-hugepages.conf
        content: |
          vm.nr_hugepages = ${var.hugepages_count}
      - path: /etc/netplan/99-dpdk.yaml
        permissions: '0600'
        content: |
          network:
            version: 2
            ethernets:
              enp0s4:
                addresses:
                  - ${local.vm2_dpdk_ip}/24
    
    runcmd:
      - sysctl -p /etc/sysctl.d/99-hugepages.conf
      - netplan apply || true
  EOF

  network_config = yamlencode({
    version = 2
    ethernets = {
      eth0 = {
        match = {
          name = "en*"
        }
        dhcp4 = true
      }
    }
  })
}

# VM2 cloud-init volume
resource "libvirt_volume" "vm2_cloudinit" {
  count = var.vm_count >= 2 ? 1 : 0
  name  = "dpdk-vm2-cloudinit.iso"
  pool  = var.storage_pool

  create = {
    content = {
      url = libvirt_cloudinit_disk.vm2_cloudinit[0].path
    }
  }

  lifecycle {
    replace_triggered_by = [libvirt_cloudinit_disk.vm2_cloudinit]
  }
}

# VM2 domain
resource "libvirt_domain" "vm2" {
  count       = var.vm_count >= 2 ? 1 : 0
  name        = "dpdk-vm2"
  memory      = var.vm2_memory_mb
  memory_unit = "MiB"
  vcpu        = var.vm2_vcpus
  type        = var.use_kvm ? "kvm" : "qemu"
  running     = true
  autostart   = true

  os = {
    type      = "hvm"
    type_arch = "x86_64"
    boot_devices = [
      { dev = "hd" }
    ]
  }

  cpu = {
    mode  = var.use_kvm ? "host-passthrough" : "custom"
    model = var.use_kvm ? null : "Nehalem"
  }

  devices = {
    disks = [
      {
        driver = {
          type = "qcow2"
        }
        source = {
          volume = {
            pool   = var.storage_pool
            volume = libvirt_volume.vm2_disk[0].name
          }
        }
        target = {
          dev = "vda"
          bus = "virtio"
        }
      },
      {
        device = "cdrom"
        source = {
          volume = {
            pool   = var.storage_pool
            volume = libvirt_volume.vm2_cloudinit[0].name
          }
        }
        target = {
          dev = "sda"
          bus = "sata"
        }
        readonly = true
      }
    ]

    # Network interfaces
    interfaces = concat(
      # Management NIC (uses default network with DHCP)
      [
        {
          model = {
            type = "virtio"
          }
          source = {
            network = {
              network = "default"
            }
          }
        }
      ],
      # DPDK NIC (uses isolated dpdk network) - only if nics_per_vm >= 2
      var.nics_per_vm >= 2 ? [
        {
          model = {
            type = "virtio"
          }
          source = {
            network = {
              network = "dpdk-data-net"
            }
          }
        }
      ] : []
    )

    consoles = [
      {
        target = {
          type = "serial"
          port = 0
        }
      }
    ]

    graphics = [
      {
        vnc = {
          auto_port = true
        }
      }
    ]

    channels = [
      {
        target = {
          virt_io = {
            name = "org.qemu.guest_agent.0"
          }
        }
      }
    ]
  }

  depends_on = [null_resource.dpdk_network]
}
