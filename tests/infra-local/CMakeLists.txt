# Local VM Infrastructure Targets
# Terraform-based QEMU/KVM VMs for DPDK testing

set(INFRA_LOCAL_DIR ${CMAKE_CURRENT_SOURCE_DIR})
set(LOCAL_OUTPUTS_FILE "${CMAKE_BINARY_DIR}/docs/local-deployment-outputs.json")

# Find terraform executable
find_program(TERRAFORM_EXECUTABLE terraform)

if(NOT TERRAFORM_EXECUTABLE)
    message(STATUS "Terraform not found - local VM targets will not be available")
    return()
endif()

message(STATUS "Terraform found: ${TERRAFORM_EXECUTABLE}")

# Initialize Terraform
add_custom_target(local_vm_init
    COMMAND ${TERRAFORM_EXECUTABLE} init
    WORKING_DIRECTORY ${INFRA_LOCAL_DIR}
    COMMENT "Initializing Terraform for local VMs"
)

# Plan deployment (dry-run)
add_custom_target(local_vm_plan
    COMMAND ${INFRA_LOCAL_DIR}/scripts/start-vm.sh --help > /dev/null  # Validate script exists
    COMMAND ${TERRAFORM_EXECUTABLE} plan
    WORKING_DIRECTORY ${INFRA_LOCAL_DIR}
    COMMENT "Planning local VM deployment"
)

# Deploy VMs using helper script
add_custom_target(local_vm_deploy
    COMMAND ${INFRA_LOCAL_DIR}/scripts/start-vm.sh -y --no-color
    COMMAND ${CMAKE_COMMAND} -E make_directory ${CMAKE_BINARY_DIR}/docs
    COMMAND ${TERRAFORM_EXECUTABLE} output -json > ${LOCAL_OUTPUTS_FILE}
    WORKING_DIRECTORY ${INFRA_LOCAL_DIR}
    COMMENT "Deploying local QEMU/KVM VMs"
)

# Deploy VMs without KVM (software emulation)
add_custom_target(local_vm_deploy_no_kvm
    COMMAND ${INFRA_LOCAL_DIR}/scripts/start-vm.sh -y --no-kvm --no-color
    COMMAND ${CMAKE_COMMAND} -E make_directory ${CMAKE_BINARY_DIR}/docs
    COMMAND ${TERRAFORM_EXECUTABLE} output -json > ${LOCAL_OUTPUTS_FILE}
    WORKING_DIRECTORY ${INFRA_LOCAL_DIR}
    COMMENT "Deploying local QEMU VMs (no KVM, software emulation)"
)

# Destroy VMs
add_custom_target(local_vm_destroy
    COMMAND ${INFRA_LOCAL_DIR}/scripts/destroy-vm.sh -y --no-color
    WORKING_DIRECTORY ${INFRA_LOCAL_DIR}
    COMMENT "Destroying local QEMU/KVM VMs"
)

# Export outputs for Ansible
add_custom_target(local_vm_outputs
    COMMAND ${CMAKE_COMMAND} -E make_directory ${CMAKE_BINARY_DIR}/docs
    COMMAND ${TERRAFORM_EXECUTABLE} output -json > ${LOCAL_OUTPUTS_FILE}
    WORKING_DIRECTORY ${INFRA_LOCAL_DIR}
    COMMENT "Exporting local VM deployment outputs"
)

# SSH to VMs
add_custom_target(local_vm_ssh1
    COMMAND ${INFRA_LOCAL_DIR}/scripts/ssh-vm.sh 1
    WORKING_DIRECTORY ${INFRA_LOCAL_DIR}
    COMMENT "SSH to VM1"
    USES_TERMINAL
)

add_custom_target(local_vm_ssh2
    COMMAND ${INFRA_LOCAL_DIR}/scripts/ssh-vm.sh 2
    WORKING_DIRECTORY ${INFRA_LOCAL_DIR}
    COMMENT "SSH to VM2"
    USES_TERMINAL
)

# Run e2e tests on local VMs
add_custom_target(local_vm_test
    COMMAND ${CMAKE_SOURCE_DIR}/tests/e2e/run_tests.sh
    WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}/tests/e2e
    COMMENT "Running e2e tests on local VMs"
    USES_TERMINAL
)

# Print available targets
add_custom_target(local_vm_help
    COMMAND ${CMAKE_COMMAND} -E echo "Local VM targets:"
    COMMAND ${CMAKE_COMMAND} -E echo "  local_vm_init         - Initialize Terraform"
    COMMAND ${CMAKE_COMMAND} -E echo "  local_vm_plan         - Plan deployment (dry-run)"
    COMMAND ${CMAKE_COMMAND} -E echo "  local_vm_deploy       - Deploy VMs with KVM"
    COMMAND ${CMAKE_COMMAND} -E echo "  local_vm_deploy_no_kvm - Deploy VMs without KVM"
    COMMAND ${CMAKE_COMMAND} -E echo "  local_vm_destroy      - Destroy VMs"
    COMMAND ${CMAKE_COMMAND} -E echo "  local_vm_outputs      - Export outputs to JSON"
    COMMAND ${CMAKE_COMMAND} -E echo "  local_vm_ssh1         - SSH to VM1"
    COMMAND ${CMAKE_COMMAND} -E echo "  local_vm_ssh2         - SSH to VM2"
    COMMAND ${CMAKE_COMMAND} -E echo "  local_vm_test         - Run e2e tests"
    COMMENT "Showing local VM targets"
)
