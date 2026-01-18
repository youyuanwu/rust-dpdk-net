cmake_minimum_required(VERSION 3.28)

project(dpdk LANGUAGES)

include(FetchContent)

message(STATUS "Fetching DPDK source code...")
FetchContent_Populate(
  dpdk
  GIT_REPOSITORY https://github.com/DPDK/dpdk.git
  GIT_TAG        v22.11.11  # or specify a specific version tag like v23.11
  GIT_SHALLOW    TRUE
)

message(STATUS "DPDK source code fetched to: ${dpdk_SOURCE_DIR}")
message(STATUS "DPDK build directory: ${dpdk_BINARY_DIR}")

# Add custom target to configure DPDK with meson
add_custom_target(dpdk_configure
  COMMAND meson setup ${dpdk_BINARY_DIR} ${dpdk_SOURCE_DIR} -Dexamples=all
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Configuring DPDK with meson"
  VERBATIM
)

# Add custom target to build DPDK with ninja
add_custom_target(dpdk_build
  COMMAND ninja -C ${dpdk_BINARY_DIR}
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Building DPDK with ninja"
  DEPENDS dpdk_configure
  VERBATIM
)

# Add custom target to install DPDK artifacts
add_custom_target(dpdk_install
  COMMAND ninja -C ${dpdk_BINARY_DIR} install
  # update the shared library cache
  COMMAND ldconfig
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Installing DPDK artifacts"
  DEPENDS dpdk_build
  VERBATIM
)

# Add custom target to uninstall DPDK artifacts
add_custom_target(dpdk_uninstall
  COMMAND ninja -C ${dpdk_BINARY_DIR} uninstall
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Uninstalling DPDK artifacts"
  VERBATIM
)

# Run example
add_custom_target(dpdk_run_example
  COMMAND echo "=== Running helloworld example ==="
  COMMAND sudo ${dpdk_BINARY_DIR}/examples/dpdk-helloworld || true
  COMMAND echo ""
  # This requires 2 devices; uncomment if you have them
  # COMMAND echo "=== Running skeleton example ==="
  # COMMAND sudo ${dpdk_BINARY_DIR}/examples/dpdk-skeleton || true
  # COMMAND echo ""
  # COMMAND echo "=== Checking network devices with dpdk-devbind ==="
  # COMMAND sudo ${dpdk_SOURCE_DIR}/usertools/dpdk-devbind.py --status
  COMMAND echo "=== Running testpmd example ==="
  COMMAND sudo ${dpdk_BINARY_DIR}/app/dpdk-testpmd -l 0-1 -n 4 -- --forward-mode=io --port-topology=chained
  # COMMAND sudo ${dpdk_BINARY_DIR}/examples/dpdk-l2fwd -l 0-1 -n 4 -- -p 0x1 -q 1
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Running DPDK examples"
  DEPENDS dpdk_build
  VERBATIM
)




