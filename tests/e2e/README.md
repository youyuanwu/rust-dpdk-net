# E2E Tests with Ansible

End-to-end tests that run against Azure VMs deployed with the Bicep templates.

## Prerequisites

```sh
pip install ansible
```

## Setup

1. Deploy VMs (from project root):
   ```sh
   cd build && make azure_vm_deploy
   ```

2. Verify inventory:
   ```sh
   cd tests/e2e
   ./inventory.py --list
   ```

## Run Tests

```sh
# Hello world (basic connectivity)
./run_tests.sh

# Or run specific playbook
./run_tests.sh playbooks/hello_world.yml
./run_tests.sh playbooks/test_connectivity.yml

./run_tests.sh playbooks/http_server_test.yml -e server_mode=tokio

# Direct ansible-playbook usage
ansible-playbook playbooks/hello_world.yml -v
```

## Playbooks

| Playbook | Description |
|----------|-------------|
| `hello_world.yml` | Basic connectivity, OS info |
| `test_connectivity.yml` | Private network ping between VMs |

## Dynamic Inventory

The `inventory.py` script reads VM IPs from:
```
build/docs/azure-deployment-outputs.json
```

This file is created by `make azure_vm_deploy` or `make azure_vm_outputs`.

## Adding New Tests

Create a new playbook in `playbooks/` directory:

```yaml
---
- name: My Test
  hosts: vms
  tasks:
    - name: Run something
      ansible.builtin.shell: echo "test"
```
