#!/bin/bash
# SSH helper for local DPDK VMs

VM="${1:-1}"
USER="${SSH_USER:-azureuser}"
WAIT="${2:-}"  # Optional: pass "wait" to wait for SSH

case "$VM" in
    1|vm1)
        VM_NAME="dpdk-vm1"
        ;;
    2|vm2)
        VM_NAME="dpdk-vm2"
        ;;
    *)
        echo "Usage: $0 [1|2|vm1|vm2] [wait]"
        echo ""
        echo "Examples:"
        echo "  $0 1        # SSH to VM1"
        echo "  $0 vm2      # SSH to VM2"
        echo "  $0 1 wait   # Wait for VM1 to be ready, then SSH"
        exit 1
        ;;
esac

# Get IP from DHCP leases (with retry)
get_vm_ip() {
    virsh net-dhcp-leases default 2>/dev/null | grep "$VM_NAME" | awk '{print $5}' | cut -d'/' -f1
}

IP=$(get_vm_ip)

# If no IP yet, wait for it
if [[ -z "$IP" ]]; then
    echo "Waiting for $VM_NAME to get IP address..."
    for i in {1..30}; do
        IP=$(get_vm_ip)
        if [[ -n "$IP" ]]; then
            break
        fi
        sleep 2
    done
fi

if [[ -z "$IP" ]]; then
    echo "Error: Could not find IP for $VM_NAME"
    echo "Check if VM is running: virsh list"
    echo "Check DHCP leases: virsh net-dhcp-leases default"
    exit 1
fi

# Wait for SSH to be ready
echo "Waiting for SSH on $VM_NAME ($IP)..."
for i in {1..60}; do
    if ssh -o ConnectTimeout=2 -o StrictHostKeyChecking=no -o BatchMode=yes \
           -o UserKnownHostsFile=/dev/null "$USER@$IP" "exit 0" 2>/dev/null; then
        break
    fi
    sleep 2
done

echo "Connecting to $VM_NAME ($IP)..."
exec ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "$USER@$IP"
