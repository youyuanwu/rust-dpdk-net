#!/bin/bash
# Run Ansible E2E tests
# Usage: ./run_tests.sh [playbook]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Make inventory executable
chmod +x inventory.py

# Default playbook
PLAYBOOK="${1:-playbooks/hello_world.yml}"
shift 2>/dev/null || true  # Remove playbook from args, ignore error if no args

echo "=== Running Ansible E2E Tests ==="
echo "Playbook: $PLAYBOOK"
echo "Extra args: $*"
echo ""

# Check inventory
echo "=== Inventory ==="
./inventory.py --list | jq -r '.vms.hosts[]' 2>/dev/null || echo "No VMs found"
echo ""

# Run the playbook with any extra arguments
ansible-playbook "$PLAYBOOK" -v "$@"
