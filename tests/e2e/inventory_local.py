#!/usr/bin/env python3
"""
Dynamic Ansible inventory for local QEMU/KVM VMs.
Gets VM IPs from libvirt DHCP leases.
"""

import json
import subprocess
import sys
import re

def get_dhcp_leases():
    """Get DHCP leases from libvirt default network."""
    try:
        result = subprocess.run(
            ["virsh", "net-dhcp-leases", "default"],
            capture_output=True, text=True, check=True
        )
        return result.stdout
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        sys.stderr.write(f"Error getting DHCP leases: {e}\n")
        return ""

def parse_leases(output):
    """Parse virsh net-dhcp-leases output."""
    vms = {}
    for line in output.splitlines():
        if "dpdk-vm1" in line:
            match = re.search(r'(\d+\.\d+\.\d+\.\d+)', line)
            if match:
                vms["vm1"] = match.group(1)
        elif "dpdk-vm2" in line:
            match = re.search(r'(\d+\.\d+\.\d+\.\d+)', line)
            if match:
                vms["vm2"] = match.group(1)
    return vms

def load_inventory():
    """Build inventory from local VMs."""
    leases = get_dhcp_leases()
    vms = parse_leases(leases)
    
    inventory = {
        "_meta": {
            "hostvars": {}
        },
        "all": {
            "children": ["vms"]
        },
        "vms": {
            "hosts": []
        }
    }
    
    # VM1
    if "vm1" in vms:
        inventory["vms"]["hosts"].append("vm1")
        inventory["_meta"]["hostvars"]["vm1"] = {
            "ansible_host": vms["vm1"],
            "private_ip": vms["vm1"],  # Management network (DHCP)
            "private_ip2": "10.0.1.4",  # DPDK data network (static)
            "vm_name": "dpdk-vm1",
            "dpdk_interface": "enp0s4",  # Local VM interface name
            "is_local": True,
        }
    
    # VM2
    if "vm2" in vms:
        inventory["vms"]["hosts"].append("vm2")
        inventory["_meta"]["hostvars"]["vm2"] = {
            "ansible_host": vms["vm2"],
            "private_ip": vms["vm2"],  # Management network (DHCP)
            "private_ip2": "10.0.1.6",  # DPDK data network (static)
            "vm_name": "dpdk-vm2",
            "dpdk_interface": "enp0s4",  # Local VM interface name
            "is_local": True,
        }
    
    return inventory

def empty_inventory():
    """Return an empty inventory."""
    return {
        "_meta": {"hostvars": {}},
        "all": {"children": []},
    }

def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--list":
        inventory = load_inventory()
        print(json.dumps(inventory, indent=2))
    elif len(sys.argv) == 3 and sys.argv[1] == "--host":
        # Return empty dict for host-specific queries
        print(json.dumps({}))
    else:
        sys.stderr.write("Usage: inventory_local.py --list | --host <hostname>\n")
        sys.exit(1)

if __name__ == "__main__":
    main()
