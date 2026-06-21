# CMake script to build DPDK deb package
# Usage: cmake -P dpdk_deb.cmake <deb_path> <pkg_dir> <dpdk_build_dir> <deb_arch> <deb_multiarch>
#
# Skips building if the deb file already exists

cmake_minimum_required(VERSION 3.16)

# Get arguments
set(DEB_PATH "${CMAKE_ARGV3}")
set(PKG_DIR "${CMAKE_ARGV4}")
set(DPDK_BUILD_DIR "${CMAKE_ARGV5}")
set(DEB_ARCH "${CMAKE_ARGV6}")
set(DEB_MULTIARCH "${CMAKE_ARGV7}")
set(DPDK_VERSION "${CMAKE_ARGV8}")

if(NOT DEB_ARCH)
  message(FATAL_ERROR "deb architecture argument is required")
endif()
if(NOT DEB_MULTIARCH)
  message(FATAL_ERROR "deb multiarch triplet argument is required")
endif()
if(NOT DPDK_VERSION)
  message(FATAL_ERROR "dpdk version argument is required")
endif()

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

# Remove example source code (always installed by DPDK regardless of -Dexamples option)
file(REMOVE_RECURSE "${PKG_DIR}/opt/dpdk/share/dpdk/examples")

# Copy DEBIAN package files from cmake directory
# Note: dev packages needed for pkg-config dependencies (Requires.private in libdpdk.pc)
# The control file is a template: substitute @DEB_ARCH@ with the target architecture.
configure_file(
  "${CMAKE_CURRENT_LIST_DIR}/pkg/control.in"
  "${PKG_DIR}/DEBIAN/control"
  @ONLY
)
file(COPY "${CMAKE_CURRENT_LIST_DIR}/pkg/postinst" "${CMAKE_CURRENT_LIST_DIR}/pkg/postrm"
  DESTINATION "${PKG_DIR}/DEBIAN"
  FILE_PERMISSIONS OWNER_READ OWNER_WRITE OWNER_EXECUTE GROUP_READ GROUP_EXECUTE WORLD_READ WORLD_EXECUTE
)

# Install profile.d script to set up PKG_CONFIG_PATH (templated for multiarch)
file(MAKE_DIRECTORY "${PKG_DIR}/etc/profile.d")
configure_file(
  "${CMAKE_CURRENT_LIST_DIR}/pkg/dpdk-net.sh.in"
  "${PKG_DIR}/etc/profile.d/dpdk-net.sh"
  @ONLY
)

# Install ld.so.conf.d config for library path (templated for multiarch)
file(MAKE_DIRECTORY "${PKG_DIR}/etc/ld.so.conf.d")
configure_file(
  "${CMAKE_CURRENT_LIST_DIR}/pkg/dpdk-net.conf.in"
  "${PKG_DIR}/etc/ld.so.conf.d/dpdk-net.conf"
  @ONLY
)

# Build deb package
execute_process(
  COMMAND dpkg-deb --build "${PKG_DIR}" "${DEB_PATH}"
  RESULT_VARIABLE result
)
if(NOT result EQUAL 0)
  message(FATAL_ERROR "Failed to build deb package")
endif()

message(STATUS "Created: ${DEB_PATH}")
