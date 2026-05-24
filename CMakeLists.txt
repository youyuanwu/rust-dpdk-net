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
FetchContent_Declare(
  dpdk
  GIT_REPOSITORY https://github.com/DPDK/dpdk.git
  GIT_TAG        ed957165eadbe60a47d5ec223578cdd1c13d0bd9 # v25.11.0
  GIT_SHALLOW    TRUE
)
FetchContent_MakeAvailable(dpdk)

message(STATUS "DPDK source code fetched to: ${dpdk_SOURCE_DIR}")
message(STATUS "DPDK build directory: ${dpdk_BINARY_DIR}")

# Detect Debian package architecture (amd64, arm64, ...) for naming the .deb
# and stamping the Architecture: field in the control file.
find_program(DPKG_EXECUTABLE dpkg)
if(DPKG_EXECUTABLE)
  execute_process(
    COMMAND ${DPKG_EXECUTABLE} --print-architecture
    OUTPUT_VARIABLE DPDK_DEB_ARCH
    OUTPUT_STRIP_TRAILING_WHITESPACE
  )
endif()
if(NOT DPDK_DEB_ARCH)
  # Fallback mapping from CMake host processor to Debian arch
  if(CMAKE_HOST_SYSTEM_PROCESSOR MATCHES "^(x86_64|AMD64)$")
    set(DPDK_DEB_ARCH "amd64")
  elseif(CMAKE_HOST_SYSTEM_PROCESSOR MATCHES "^(aarch64|arm64)$")
    set(DPDK_DEB_ARCH "arm64")
  else()
    message(FATAL_ERROR "Unsupported host architecture for .deb packaging: ${CMAKE_HOST_SYSTEM_PROCESSOR}")
  endif()
endif()

# Detect the GNU multiarch triplet (e.g. x86_64-linux-gnu, aarch64-linux-gnu)
# used in /usr/local/lib/<triplet>/{pkgconfig,...} so the post-install env
# scripts point at the right directory for the host architecture.
find_program(DPKG_ARCHITECTURE_EXECUTABLE dpkg-architecture)
if(DPKG_ARCHITECTURE_EXECUTABLE)
  execute_process(
    COMMAND ${DPKG_ARCHITECTURE_EXECUTABLE} -qDEB_HOST_MULTIARCH
    OUTPUT_VARIABLE DPDK_DEB_MULTIARCH
    OUTPUT_STRIP_TRAILING_WHITESPACE
  )
endif()
if(NOT DPDK_DEB_MULTIARCH)
  if(DPDK_DEB_ARCH STREQUAL "amd64")
    set(DPDK_DEB_MULTIARCH "x86_64-linux-gnu")
  elseif(DPDK_DEB_ARCH STREQUAL "arm64")
    set(DPDK_DEB_MULTIARCH "aarch64-linux-gnu")
  else()
    message(FATAL_ERROR "Cannot determine GNU multiarch triplet for ${DPDK_DEB_ARCH}")
  endif()
endif()

message(STATUS "DPDK .deb architecture: ${DPDK_DEB_ARCH} (multiarch: ${DPDK_DEB_MULTIARCH})")
set(DPDK_DEB_FILE "${CMAKE_CURRENT_BINARY_DIR}/dpdk-net_25.11.0_${DPDK_DEB_ARCH}.deb")

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

# Add custom target to force reconfigure DPDK with meson (wipes existing config)
add_custom_target(dpdk_reconfigure
  COMMAND ${CMAKE_COMMAND} -E rm -rf ${dpdk_BINARY_DIR}
  COMMAND meson setup ${dpdk_BINARY_DIR} ${dpdk_SOURCE_DIR}
    -Dcpu_instruction_set=generic
    --buildtype=release
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Wiping and reconfiguring DPDK with meson"
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
  COMMAND apt install -y ${DPDK_DEB_FILE}
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
  COMMAND ${CMAKE_COMMAND} -E remove -f ${DPDK_DEB_FILE}
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Cleaning DPDK build artifacts and deb package"
  VERBATIM
)

# Add custom target to create .deb package without installing to system
# Uses DESTDIR staging with dpkg-deb - no extra tools required
# Skips if deb file already exists
add_custom_target(dpdk_deb
  COMMAND ${CMAKE_COMMAND} -P ${CMAKE_CURRENT_SOURCE_DIR}/cmake/dpdk_deb.cmake
    ${DPDK_DEB_FILE}
    ${CMAKE_CURRENT_BINARY_DIR}/dpdk-pkg
    ${dpdk_BINARY_DIR}
    ${DPDK_DEB_ARCH}
    ${DPDK_DEB_MULTIARCH}
  WORKING_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}
  COMMENT "Creating DPDK .deb package (staged, no system install)"
  # DEPENDS dpdk_build
  VERBATIM
)

add_subdirectory(tests/infra)
add_subdirectory(tests/infra-local)
