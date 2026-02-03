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
add_custom_target(dpdk_configure
  COMMAND meson setup --reconfigure ${dpdk_BINARY_DIR} ${dpdk_SOURCE_DIR} -Dexamples=all
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Configuring DPDK with meson"
  VERBATIM
)

# Add custom target to build DPDK with ninja
add_custom_target(dpdk_build
  COMMAND ninja -C ${dpdk_BINARY_DIR}
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Building DPDK with ninja"
  # DEPENDS dpdk_configure
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

# Add custom target to create .deb package without installing to system
# Uses DESTDIR staging with dpkg-deb - no extra tools required
add_custom_target(dpdk_deb
  COMMAND ${CMAKE_COMMAND} -E remove_directory ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg
  COMMAND ${CMAKE_COMMAND} -E make_directory ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg/DEBIAN
  COMMAND ${CMAKE_COMMAND} -E env DESTDIR=${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg ninja -C ${dpdk_BINARY_DIR} install
  COMMAND ${CMAKE_COMMAND} -E echo "Package: dpdk-net" > ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg/DEBIAN/control
  COMMAND ${CMAKE_COMMAND} -E echo "Version: 22.11.11" >> ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg/DEBIAN/control
  COMMAND ${CMAKE_COMMAND} -E echo "Section: libs" >> ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg/DEBIAN/control
  COMMAND ${CMAKE_COMMAND} -E echo "Priority: optional" >> ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg/DEBIAN/control
  COMMAND ${CMAKE_COMMAND} -E echo "Architecture: amd64" >> ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg/DEBIAN/control
  COMMAND ${CMAKE_COMMAND} -E echo "Depends: libc6, libnuma1, libelf1t64 | libelf1, zlib1g, libzstd1, libssl3t64 | libssl3, libatomic1, rdma-core, libibverbs1, libmlx5-1, librdmacm1" >> ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg/DEBIAN/control
  COMMAND ${CMAKE_COMMAND} -E echo "Maintainer: maintainer@example.com" >> ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg/DEBIAN/control
  COMMAND ${CMAKE_COMMAND} -E echo "Description: Data Plane Development Kit with Mellanox mlx5 support (custom build for dpdk-net)" >> ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg/DEBIAN/control
  COMMAND dpkg-deb --build ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg ${CMAKE_CURRENT_BINARY_DIR}/dpdk-net_22.11.11_amd64.deb
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Creating DPDK .deb package (staged, no system install)"
  DEPENDS dpdk_build
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
