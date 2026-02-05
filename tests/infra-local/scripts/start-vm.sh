#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

# Use system libvirt connection
export LIBVIRT_DEFAULT_URI="qemu:///system"

# Parse arguments
USE_KVM=true
AUTO_APPROVE=false
NO_COLOR=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --no-kvm)
            USE_KVM=false
            shift
            ;;
        -y|--yes)
            AUTO_APPROVE=true
            shift
            ;;
        --no-color)
            NO_COLOR=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --no-kvm    Use QEMU software emulation instead of KVM (slower)"
            echo "  -y, --yes   Auto-approve without prompting"
            echo "  --no-color  Disable colored output"
            echo "  -h, --help  Show this help"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Color output
if [[ "$NO_COLOR" == "true" ]]; then
    TF_COLOR_ARG="-no-color"
else
    TF_COLOR_ARG=""
fi

echo "========================================"
if [[ "$USE_KVM" == "true" ]]; then
    echo "  DPDK Local VM Provisioning (KVM)"
else
    echo "  DPDK Local VM Provisioning (QEMU)"
    echo "  Warning: Software emulation is slower"
fi
echo "========================================"
echo ""

# Validate prerequisites
echo "Checking prerequisites..."

# Check libvirtd
if ! systemctl is-active --quiet libvirtd; then
    echo "Error: libvirtd is not running."
    echo "Start it with: sudo systemctl start libvirtd"
    exit 1
fi
echo "  ✓ libvirtd is running"

# Check libvirt access
if ! virsh list &>/dev/null; then
    echo "Error: Cannot connect to libvirt."
    echo "Make sure your user is in the 'libvirt' group:"
    echo "  sudo usermod -aG libvirt $USER && newgrp libvirt"
    exit 1
fi
echo "  ✓ libvirt access OK"

# Check storage pool
if ! virsh pool-info default &>/dev/null; then
    echo "Error: Storage pool 'default' not found."
    echo "Create it with:"
    echo "  sudo mkdir -p /var/lib/libvirt/images"
    echo "  sudo virsh pool-define-as default dir --target /var/lib/libvirt/images"
    echo "  sudo virsh pool-start default"
    echo "  sudo virsh pool-autostart default"
    exit 1
fi
echo "  ✓ Storage pool 'default' exists"

# Check SSH key
SSH_KEY_PATH="${SSH_KEY_PATH:-$HOME/.ssh/id_rsa.pub}"
if [[ ! -f "$SSH_KEY_PATH" ]]; then
    echo "Error: SSH public key not found at $SSH_KEY_PATH"
    echo "Generate one with: ssh-keygen -t ed25519"
    exit 1
fi
echo "  ✓ SSH key found"

echo ""

# Build terraform args
TF_VAR_ARGS=""
if [[ "$USE_KVM" == "false" ]]; then
    TF_VAR_ARGS="-var=use_kvm=false"
fi

# Initialize Terraform if needed
if [[ ! -d ".terraform" ]]; then
    echo "Initializing Terraform..."
    terraform init $TF_COLOR_ARG
    echo ""
fi

# Plan
echo "Planning infrastructure..."
terraform plan $TF_COLOR_ARG $TF_VAR_ARGS -out=tfplan

echo ""

# Apply
if [[ "$AUTO_APPROVE" == "true" ]]; then
    REPLY=y
else
    read -p "Apply this plan? (y/N) " -n 1 -r
    echo ""
fi

if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "Applying configuration..."
    terraform apply $TF_COLOR_ARG $TF_VAR_ARGS tfplan
    rm -f tfplan

    echo ""
    echo "========================================"
    echo "  VMs Created Successfully!"
    echo "========================================"
    echo ""
    echo "DPDK Network IPs (static):"
    echo "  VM1: 10.0.1.4"
    echo "  VM2: 10.0.1.6"
    echo ""
    echo "Connect with (waits for VM to be ready):"
    echo "  ./tests/infra-local/scripts/ssh-vm.sh 1   # SSH to VM1"
    echo "  ./tests/infra-local/scripts/ssh-vm.sh 2   # SSH to VM2"
    echo ""
    echo "Or get IPs manually:"
    echo "  virsh net-dhcp-leases default"
else
    echo "Aborted."
    rm -f tfplan
    exit 0
fi
