#!/usr/bin/env python3
"""
Dynamic Ansible inventory that reads VM IPs from Azure deployment outputs.
Reads from: build/tests/infra/azure-deployment-outputs.json
"""

import json
import os
import sys
from pathlib import Path

def get_outputs_file():
    """Find the Azure deployment outputs file."""
    # Try relative to this script (tests/e2e -> project root -> build/tests/infra)
    script_dir = Path(__file__).parent
    project_root = script_dir.parent.parent
    
    # Check common locations
    candidates = [
        project_root / "build" / "tests" / "infra" / "azure-deployment-outputs.json",
        project_root / "build" / "azure-deployment-outputs.json",
        Path(os.environ.get("OUTPUTS_FILE", "")),
    ]
    
    for path in candidates:
        if path and path.exists():
            return path
    
    # Default path for error message
    return candidates[0]

def load_inventory():
    """Load inventory from Azure deployment outputs."""
    outputs_file = get_outputs_file()
    
    if not outputs_file.exists():
        sys.stderr.write(f"Error: Outputs file not found: {outputs_file}\n")
        sys.stderr.write("Run 'make azure_vm_deploy' or 'make azure_vm_outputs' first.\n")
        return empty_inventory()
    
    try:
        with open(outputs_file) as f:
            outputs = json.load(f)
    except json.JSONDecodeError as e:
        sys.stderr.write(f"Error parsing {outputs_file}: {e}\n")
        return empty_inventory()
    
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
    
    # VM1 (always present)
    vm1_public_ip = outputs.get("vm1PublicIp", {}).get("value", "")
    vm1_private_ip = outputs.get("vm1PrivateIp", {}).get("value", "")
    vm1_private_ip2 = outputs.get("vm1PrivateIp2", {}).get("value", "")
    vm1_name = outputs.get("vm1Name", {}).get("value", "vm1")
    
    if vm1_public_ip:
        inventory["vms"]["hosts"].append("vm1")
        inventory["_meta"]["hostvars"]["vm1"] = {
            "ansible_host": vm1_public_ip,
            "private_ip": vm1_private_ip,
            "private_ip2": vm1_private_ip2,
            "vm_name": vm1_name,
            "dpdk_interface": "eth1",  # Azure VM interface name
            "is_local": False,
        }
    
    # VM2 (optional)
    vm2_public_ip = outputs.get("vm2PublicIp", {}).get("value", "")
    vm2_private_ip = outputs.get("vm2PrivateIp", {}).get("value", "")
    vm2_private_ip2 = outputs.get("vm2PrivateIp2", {}).get("value", "")
    vm2_name = outputs.get("vm2Name", {}).get("value", "")
    
    if vm2_public_ip:
        inventory["vms"]["hosts"].append("vm2")
        inventory["_meta"]["hostvars"]["vm2"] = {
            "ansible_host": vm2_public_ip,
            "private_ip": vm2_private_ip,
            "private_ip2": vm2_private_ip2,
            "vm_name": vm2_name,
            "dpdk_interface": "eth1",  # Azure VM interface name
            "is_local": False,
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
        # Return empty dict for host vars (already in _meta)
        print(json.dumps({}))
    else:
        sys.stderr.write("Usage: inventory.py --list | --host <hostname>\n")
        sys.exit(1)

if __name__ == "__main__":
    main()
