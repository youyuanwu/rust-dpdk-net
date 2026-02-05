cmake_minimum_required(VERSION 3.28)

project(dpdk LANGUAGES)

# Find required tools
find_program(MESON_EXECUTABLE meson REQUIRED)
find_program(NINJA_EXECUTABLE ninja REQUIRED)

message(STATUS "Found meson: ${MESON_EXECUTABLE}")
message(STATUS "Found ninja: ${NINJA_EXECUTABLE}")

# Check for Mellanox/RDMA development packages (required for mlx5 PMD on Azure)
find_program(PKG_CONFIG_EXECUTABLE pkg-config)
if(PKG_CONFIG_EXECUTABLE)
  set(MLX_PACKAGES_MISSING "")
  
  # Check for libibverbs
  execute_process(
    COMMAND ${PKG_CONFIG_EXECUTABLE} --exists libibverbs
    RESULT_VARIABLE LIBIBVERBS_FOUND
  )
  if(NOT LIBIBVERBS_FOUND EQUAL 0)
    list(APPEND MLX_PACKAGES_MISSING "libibverbs-dev")
  endif()
  
  # Check for libmlx5
  execute_process(
    COMMAND ${PKG_CONFIG_EXECUTABLE} --exists libmlx5
    RESULT_VARIABLE LIBMLX5_FOUND
  )
  if(NOT LIBMLX5_FOUND EQUAL 0)
    list(APPEND MLX_PACKAGES_MISSING "libmlx5-1")
  endif()
  
  # Check for librdmacm
  execute_process(
    COMMAND ${PKG_CONFIG_EXECUTABLE} --exists librdmacm
    RESULT_VARIABLE LIBRDMACM_FOUND
  )
  if(NOT LIBRDMACM_FOUND EQUAL 0)
    list(APPEND MLX_PACKAGES_MISSING "librdmacm-dev")
  endif()
  
  if(MLX_PACKAGES_MISSING)
    message(WARNING 
      "Mellanox/RDMA packages not found: ${MLX_PACKAGES_MISSING}\n"
      "DPDK will be built WITHOUT mlx5 support (required for Azure accelerated networking).\n"
      "To enable mlx5 support, install: sudo apt-get install -y rdma-core libibverbs-dev libmlx5-1 librdmacm-dev"
    )
  else()
    message(STATUS "Mellanox/RDMA packages found - mlx5 PMD will be enabled")
  endif()
else()
  message(WARNING "pkg-config not found - cannot check for Mellanox/RDMA packages")
endif()

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
# Uses cpu_instruction_set=generic (corei7/Nehalem) for compatibility with QEMU software emulation
# Uses release build type for optimized binaries
# Skips if already configured (use dpdk_reconfigure to force)
add_custom_target(dpdk_configure
  COMMAND ${CMAKE_COMMAND} -P ${CMAKE_CURRENT_SOURCE_DIR}/cmake/dpdk_configure.cmake
    ${dpdk_BINARY_DIR}
    ${dpdk_SOURCE_DIR}
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Configuring DPDK with meson (release build, generic/corei7 target)"
  VERBATIM
)

# Add custom target to force reconfigure DPDK with meson
add_custom_target(dpdk_reconfigure
  COMMAND meson setup --reconfigure ${dpdk_BINARY_DIR} ${dpdk_SOURCE_DIR}
    -Dexamples=all
    -Dcpu_instruction_set=generic
    --buildtype=release
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Reconfiguring DPDK with meson"
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

# Add custom target to install DPDK from deb package
add_custom_target(dpdk_install
  COMMAND apt install -y ${CMAKE_CURRENT_BINARY_DIR}/dpdk-net_22.11.11_amd64.deb
  # update the shared library cache
  COMMAND ldconfig
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Installing DPDK from .deb package"
  DEPENDS dpdk_deb
  VERBATIM
)

# Add custom target to uninstall DPDK package
add_custom_target(dpdk_uninstall
  COMMAND apt remove -y dpdk-net || true
  COMMAND ldconfig
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Uninstalling DPDK package"
  VERBATIM
)

# Add custom target to clean DPDK build artifacts and deb package
add_custom_target(dpdk_clean
  COMMAND ninja -C ${dpdk_BINARY_DIR} clean
  COMMAND ${CMAKE_COMMAND} -E remove_directory ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg
  COMMAND ${CMAKE_COMMAND} -E remove -f ${CMAKE_CURRENT_BINARY_DIR}/dpdk-net_22.11.11_amd64.deb
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Cleaning DPDK build artifacts and deb package"
  VERBATIM
)

# Add custom target to create .deb package without installing to system
# Uses DESTDIR staging with dpkg-deb - no extra tools required
# Skips if deb file already exists
add_custom_target(dpdk_deb
  COMMAND ${CMAKE_COMMAND} -P ${CMAKE_CURRENT_SOURCE_DIR}/cmake/dpdk_deb.cmake
    ${CMAKE_CURRENT_BINARY_DIR}/dpdk-net_22.11.11_amd64.deb
    ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg
    ${dpdk_BINARY_DIR}
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Creating DPDK .deb package (staged, no system install)"
  # DEPENDS dpdk_build
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

add_subdirectory(tests/infra)
add_subdirectory(tests/infra-local)
