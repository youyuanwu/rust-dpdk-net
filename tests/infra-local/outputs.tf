# Outputs for Ansible integration and user convenience

output "vm1_name" {
  description = "Name of VM1"
  value       = libvirt_domain.vm1.name
}

output "vm1_private_ip" {
  description = "Management IP of VM1 (get via: virsh net-dhcp-leases default)"
  value       = "Run: virsh net-dhcp-leases default | grep dpdk-vm1"
}

output "vm1_dpdk_ip" {
  description = "DPDK network IP of VM1"
  value       = var.nics_per_vm >= 2 ? "10.0.1.4" : ""
}

output "vm2_name" {
  description = "Name of VM2"
  value       = var.vm_count >= 2 ? libvirt_domain.vm2[0].name : ""
}

output "vm2_private_ip" {
  description = "Management IP of VM2 (get via: virsh net-dhcp-leases default)"
  value       = var.vm_count >= 2 ? "Run: virsh net-dhcp-leases default | grep dpdk-vm2" : ""
}

output "vm2_dpdk_ip" {
  description = "DPDK network IP of VM2"
  value       = var.vm_count >= 2 && var.nics_per_vm >= 2 ? "10.0.1.6" : ""
}

output "ssh_user" {
  description = "SSH username for VMs"
  value       = var.ssh_user
}

output "management_network" {
  description = "Management network name (DHCP)"
  value       = "default"
}

output "dpdk_network" {
  description = "DPDK network name"
  value       = "dpdk-data-net"
}

output "get_vm_ips" {
  description = "Command to get VM IPs"
  value       = "virsh net-dhcp-leases default"
}
