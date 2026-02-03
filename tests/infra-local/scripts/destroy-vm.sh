#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

# Use system libvirt connection
export LIBVIRT_DEFAULT_URI="qemu:///system"

# Parse arguments
AUTO_APPROVE=false
NO_COLOR=false

while [[ $# -gt 0 ]]; do
    case $1 in
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
echo "  DPDK Local VM Destruction"
echo "========================================"
echo ""

# Check if terraform state exists
if [[ ! -f "terraform.tfstate" ]]; then
    echo "No terraform state found. Nothing to destroy."
    exit 0
fi

# Destroy
if [[ "$AUTO_APPROVE" == "true" ]]; then
    terraform destroy $TF_COLOR_ARG -auto-approve
else
    terraform destroy $TF_COLOR_ARG
fi

echo ""
echo "VMs destroyed successfully."
