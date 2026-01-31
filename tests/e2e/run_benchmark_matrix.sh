#!/bin/bash
# Run full benchmark matrix across different VM1 sizes
#
# This script:
# 1. Deploys 2 VMs (VM1=server, VM2=client)
# 2. Runs all server mode benchmarks
# 3. Resizes VM1 and re-runs benchmarks
# 4. Repeats for each VM1 size in the matrix
#
# Prerequisites:
# - Azure CLI logged in (`az login`)
# - SSH key at ~/.ssh/id_rsa.pub
# - Built binaries: cargo build --release -p dpdk-bench-server -p dpdk-bench-client
#
# Usage:
#   ./run_benchmark_matrix.sh [resource-group-name]
#
# Example:
#   ./run_benchmark_matrix.sh dpdk-bench-test

set -e          # Exit on error
set -o pipefail # Exit on pipeline failure

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
INFRA_DIR="$PROJECT_ROOT/tests/infra"
BUILD_DIR="$PROJECT_ROOT/build"

# Resource group (use argument or default)
RG="${1:-tenant-test}"
LOCATION="westus2"

# VM sizes to test for VM1 (server)
# VM2 (client) stays at D8s to keep load generation consistent
VM1_SIZES=("Standard_D2s_v5" "Standard_D4s_v5" "Standard_D8s_v5")
VM2_SIZE="Standard_D8s_v5"

# VM names follow the pattern: {resource-group}-vm1, {resource-group}-vm2
VM1_NAME="$RG-vm1"
VM2_NAME="$RG-vm2"

log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

error() {
    log "ERROR: $*" >&2
    exit 1
}

# Check prerequisites
check_prerequisites() {
    log "Checking prerequisites..."
    
    command -v az &>/dev/null || error "Azure CLI not installed"
    az account show &>/dev/null || error "Not logged in to Azure (run 'az login')"
    
    [[ -f ~/.ssh/id_rsa.pub ]] || error "SSH public key not found at ~/.ssh/id_rsa.pub"
    
    [[ -f "$PROJECT_ROOT/target/release/dpdk-bench-server" ]] || error "dpdk-bench-server not built (run 'cargo build --release -p dpdk-bench-server')"
    [[ -f "$PROJECT_ROOT/target/release/dpdk-bench-client" ]] || error "dpdk-bench-client not built (run 'cargo build --release -p dpdk-bench-client')"
    
    log "Prerequisites OK"
}

# Deploy VMs
deploy_vms() {
    local vm1_size="$1"
    
    log "Creating resource group: $RG in $LOCATION"
    az group create --name "$RG" --location "$LOCATION" --output none
    
    log "Deploying VMs (VM1=$vm1_size, VM2=$VM2_SIZE)..."
    az deployment group create \
        --resource-group "$RG" \
        --template-file "$INFRA_DIR/main.bicep" \
        --parameters sshPublicKey="$(cat ~/.ssh/id_rsa.pub)" \
                     vmCount=2 \
                     nicsPerVm=2 \
                     vm1Size="$vm1_size" \
                     vm2Size="$VM2_SIZE" \
        --output none
    
    log "VMs deployed successfully"
    
    # Ensure VMs are started (they may be in deallocated state)
    log "Starting VMs..."
    az vm start --resource-group "$RG" --name "$VM1_NAME" --output none || true
    az vm start --resource-group "$RG" --name "$VM2_NAME" --output none || true
    
    # Wait for VMs to be fully ready
    log "Waiting for VMs to be ready..."
    sleep 30s
}

# Resize VM1
resize_vm1() {
    local new_size="$1"
    
    log "Resizing VM1 to $new_size..."
    
    # Deallocate first (required for resize)
    log "Deallocating VM1..."
    az vm deallocate --resource-group "$RG" --name "$VM1_NAME" --output none
    
    # Resize
    log "Applying new size..."
    az vm resize --resource-group "$RG" --name "$VM1_NAME" --size "$new_size" --output none
    
    # Start
    log "Starting VM1..."
    az vm start --resource-group "$RG" --name "$VM1_NAME" --output none
    
    # Wait for VM to be ready
    log "Waiting for VM1 to be ready..."
    sleep 30s
    
    log "VM1 resized to $new_size"
}

# Get short size name for directory (e.g., "Standard_D2s_v5" -> "d2s")
size_to_dir() {
    local size="$1"
    echo "$size" | sed -E 's/Standard_D([0-9]+)s_v5/d\1s/'
}

# Run benchmarks for current VM configuration
run_benchmarks() {
    local vm1_size="$1"
    local size_dir
    size_dir=$(size_to_dir "$vm1_size")
    local output_dir="$BUILD_DIR/benchmarks_$size_dir"
    
    log "Running benchmarks for VM1=$vm1_size (output: $output_dir)"
    
    # Remove any existing output for this size
    rm -rf "$output_dir"
    
    cd "$SCRIPT_DIR"
    
    # Run all modes (outputs to $BUILD_DIR/benchmarks/)
    ./run_all_modes.sh 2>&1 | tee "/tmp/benchmark_run_$size_dir.log"
    
    # Move the entire benchmarks directory to size-specific directory
    if [[ -d "$BUILD_DIR/benchmarks" ]]; then
        mv "$BUILD_DIR/benchmarks" "$output_dir"
        mv "/tmp/benchmark_run_$size_dir.log" "$output_dir/benchmark_run.log"
        
        # Rename the comparison report to include size
        if [[ -f "$output_dir/BENCHMARK_COMPARISON.md" ]]; then
            cp "$output_dir/BENCHMARK_COMPARISON.md" "$output_dir/BENCHMARK_$size_dir.md"
        fi
    fi
    
    log "Benchmarks complete for $vm1_size"
}

# Generate combined report
generate_combined_report() {
    log "Generating combined benchmark report..."
    
    local report_file="$BUILD_DIR/BENCHMARK_MATRIX_REPORT.md"
    
    cat > "$report_file" << 'EOF'
# Benchmark Matrix Report

This report compares benchmark results across different VM1 (server) sizes.

EOF
    
    echo "Generated: $(date -Iseconds)" >> "$report_file"
    echo "" >> "$report_file"
    echo "## VM Configurations Tested" >> "$report_file"
    echo "" >> "$report_file"
    echo "| VM | Role | Sizes Tested |" >> "$report_file"
    echo "|-----|------|--------------|" >> "$report_file"
    echo "| VM1 | Server | ${VM1_SIZES[*]} |" >> "$report_file"
    echo "| VM2 | Client | $VM2_SIZE (fixed) |" >> "$report_file"
    echo "" >> "$report_file"
    
    # Link to individual reports
    echo "## Individual Reports" >> "$report_file"
    echo "" >> "$report_file"
    
    for size in "${VM1_SIZES[@]}"; do
        local size_dir
        size_dir=$(size_to_dir "$size")
        if [[ -f "$BUILD_DIR/benchmarks_$size_dir/BENCHMARK_COMPARISON.md" ]]; then
            echo "- [$size](benchmarks_$size_dir/BENCHMARK_COMPARISON.md)" >> "$report_file"
        fi
    done
    
    log "Combined report: $report_file"
}

# Cleanup (optional)
cleanup() {
    log "Cleaning up resource group: $RG"
    az group delete --name "$RG" --yes --no-wait
    log "Cleanup initiated (running in background)"
}

# Main
main() {
    local start_time
    start_time=$(date +%s)
    
    log "=========================================="
    log "Benchmark Matrix Test"
    log "Resource Group: $RG"
    log "VM1 Sizes: ${VM1_SIZES[*]}"
    log "VM2 Size: $VM2_SIZE"
    log "=========================================="
    
    check_prerequisites
    
    # Deploy with first size
    deploy_vms "${VM1_SIZES[0]}"
    
    # Run benchmarks for each VM1 size
    for i in "${!VM1_SIZES[@]}"; do
        local size="${VM1_SIZES[$i]}"
        
        log ""
        log "=========================================="
        log "Testing VM1 size: $size (${i+1}/${#VM1_SIZES[@]})"
        log "=========================================="
        
        # Resize if not the first iteration
        if [[ $i -gt 0 ]]; then
            resize_vm1 "$size"
        fi
        
        run_benchmarks "$size"
    done
    
    # Generate combined report
    generate_combined_report
    
    local end_time
    end_time=$(date +%s)
    local duration=$((end_time - start_time))
    
    log ""
    log "=========================================="
    log "Benchmark Matrix Complete!"
    log "Duration: $((duration / 60)) minutes $((duration % 60)) seconds"
    log "Results: $BUILD_DIR/benchmarks_*/"
    log "=========================================="
    
    # Optionally cleanup (uncomment to auto-delete)
    # cleanup
    
    log ""
    log "To cleanup Azure resources:"
    log "  az group delete --name $RG --yes"
}

main "$@"
