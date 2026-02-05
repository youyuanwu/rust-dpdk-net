# CMake script to configure DPDK with meson
# Usage: cmake -P dpdk_configure.cmake <build_dir> <source_dir>
#
# Behavior:
# - First run: uses "meson setup"
# - Already configured: skips (do nothing)
# Use dpdk_reconfigure target to force reconfiguration

cmake_minimum_required(VERSION 3.16)

# Get arguments
set(DPDK_BUILD_DIR "${CMAKE_ARGV3}")
set(DPDK_SOURCE_DIR "${CMAKE_ARGV4}")

# Common meson options
set(MESON_OPTIONS
  -Dexamples=all
  -Dcpu_instruction_set=generic
  --buildtype=release
)

# Check if build directory already has meson configuration
if(EXISTS "${DPDK_BUILD_DIR}/meson-private")
  message(STATUS "DPDK already configured, skipping. Use dpdk_reconfigure to force.")
  return()
endif()

message(STATUS "First time DPDK configuration...")

# Run meson setup
execute_process(
  COMMAND meson setup ${DPDK_BUILD_DIR} ${DPDK_SOURCE_DIR} ${MESON_OPTIONS}
  RESULT_VARIABLE result
)

if(NOT result EQUAL 0)
  message(FATAL_ERROR "Meson configuration failed")
endif()

message(STATUS "DPDK configured successfully")
