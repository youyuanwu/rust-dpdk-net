#!/bin/bash
# Run Ansible E2E tests
# Usage: ./run_tests.sh [playbook] [--local] [extra ansible args]
#        ./run_tests.sh --local playbooks/test_connectivity.yml
#        ./tests/e2e/run_tests.sh [playbook]  (from repo root)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Parse arguments
USE_LOCAL=false
PLAYBOOK=""
EXTRA_ARGS=()

for arg in "$@"; do
    case "$arg" in
        --local|-l)
            USE_LOCAL=true
            ;;
        *)
            if [[ -z "$PLAYBOOK" && "$arg" == *.yml ]]; then
                PLAYBOOK="$arg"
            else
                EXTRA_ARGS+=("$arg")
            fi
            ;;
    esac
done

# Default playbook
PLAYBOOK="${PLAYBOOK:-playbooks/hello_world.yml}"
# If playbook path starts with tests/e2e/, strip it (for running from repo root)
PLAYBOOK="${PLAYBOOK#tests/e2e/}"

# Select inventory
if [[ "$USE_LOCAL" == "true" ]]; then
    INVENTORY="inventory_local.py"
    echo "=== Running Ansible E2E Tests (LOCAL VMs) ==="
else
    INVENTORY="inventory.py"
    echo "=== Running Ansible E2E Tests (Azure VMs) ==="
fi

# Make inventory executable
chmod +x "$INVENTORY"

echo "Playbook: $PLAYBOOK"
echo "Inventory: $INVENTORY"
echo "Extra args: ${EXTRA_ARGS[*]}"
echo ""

# Check inventory
echo "=== Inventory ==="
./"$INVENTORY" --list | jq -r '.vms.hosts[]' 2>/dev/null || echo "No VMs found"
echo ""

# Run the playbook with any extra arguments
ansible-playbook -i "$INVENTORY" "$PLAYBOOK" -v "${EXTRA_ARGS[@]}"
