# CMake script to build DPDK deb package
# Usage: cmake -P dpdk_deb.cmake <deb_path> <pkg_dir> <dpdk_build_dir>
#
# Skips building if the deb file already exists

cmake_minimum_required(VERSION 3.16)

# Get arguments
set(DEB_PATH "${CMAKE_ARGV3}")
set(PKG_DIR "${CMAKE_ARGV4}")
set(DPDK_BUILD_DIR "${CMAKE_ARGV5}")

# Check if deb already exists
if(EXISTS "${DEB_PATH}")
  message(STATUS "DPDK .deb package already exists: ${DEB_PATH}")
  message(STATUS "Skipping build. Delete the file to rebuild.")
  return()
endif()

message(STATUS "Building DPDK .deb package...")

# Clean and create package directory
file(REMOVE_RECURSE "${PKG_DIR}")
file(MAKE_DIRECTORY "${PKG_DIR}/DEBIAN")

# Run ninja install with DESTDIR
execute_process(
  COMMAND ${CMAKE_COMMAND} -E env DESTDIR=${PKG_DIR} ninja -C ${DPDK_BUILD_DIR} install
  RESULT_VARIABLE result
)
if(NOT result EQUAL 0)
  message(FATAL_ERROR "Failed to run ninja install")
endif()

# Create control file
file(WRITE "${PKG_DIR}/DEBIAN/control"
"Package: dpdk-net
Version: 22.11.11
Section: libs
Priority: optional
Architecture: amd64
Depends: libc6, libnuma1, libelf1t64 | libelf1, zlib1g, libzstd1, libssl3t64 | libssl3, libatomic1, rdma-core, libibverbs1, libmlx5-1, librdmacm1
Maintainer: maintainer@example.com
Description: Data Plane Development Kit with Mellanox mlx5 support (custom build for dpdk-net)
")

# Build deb package
execute_process(
  COMMAND dpkg-deb --build "${PKG_DIR}" "${DEB_PATH}"
  RESULT_VARIABLE result
)
if(NOT result EQUAL 0)
  message(FATAL_ERROR "Failed to build deb package")
endif()

message(STATUS "Created: ${DEB_PATH}")
