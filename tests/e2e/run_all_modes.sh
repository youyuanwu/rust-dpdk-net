#!/bin/bash
# Run HTTP server benchmark with all 5 server modes sequentially

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "=========================================="
echo "Running HTTP Server Benchmark - All Modes"
echo "=========================================="

# Clean up previous benchmark output on VM2
echo "Cleaning up previous benchmark output..."
rm -rf "$SCRIPT_DIR/../../build/benchmarks"

MODES=("dpdk" "tokio" "tokio-local" "kimojio" "kimojio-poll")

for mode in "${MODES[@]}"; do
    echo ""
    echo "=========================================="
    echo "Mode: $mode"
    echo "=========================================="
    echo ""
    
    if [ "$mode" = "dpdk" ]; then
        ./run_tests.sh playbooks/http_server_test.yml
    else
        ./run_tests.sh playbooks/http_server_test.yml -e "server_mode=$mode"
    fi
    
    echo ""
    echo "Completed: $mode"
    echo ""
    
    # Brief pause between modes
    sleep 2
done

echo "=========================================="
echo "All modes completed!"
echo "=========================================="

# Run generate report python script
python3 "$SCRIPT_DIR/generate_benchmark_report.py"