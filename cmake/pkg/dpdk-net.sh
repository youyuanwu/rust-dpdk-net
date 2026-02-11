#!/bin/sh
# Set up environment for DPDK installed in /opt/dpdk
#
# NOTE: Commented out - using default /usr/local prefix which is in default search paths
#
# KNOWN ISSUE WITH CUSTOM INSTALL PATH (/opt/dpdk):
# -------------------------------------------------
# When DPDK is built with --prefix=/opt/dpdk, the Rust pkg-config crate does not
# preserve the correct link order from pkg-config output. DPDK requires:
#   -Wl,--whole-archive <libraries> -Wl,--no-whole-archive
#
# The pkg-config crate emits libraries separately from linker flags, breaking
# the --whole-archive grouping. This causes PMD driver constructors (like net_ring)
# to not be linked into the final binary, resulting in:
#   "EAL: failed to parse device 'net_ring0'"
#
# The RTE_EAL_PMD_PATH is correctly set to /opt/dpdk/lib/x86_64-linux-gnu/dpdk/pmds-26.0
# and the PMD .so files exist, but for static linking the constructor functions
# must be included via --whole-archive.
#
# Workaround: Use default /usr/local prefix until the build.rs is updated to
# manually emit link flags in the correct order.

_dpdk_pkgconfig="/usr/local/lib/x86_64-linux-gnu/pkgconfig"
case ":${PKG_CONFIG_PATH}:" in
    *":${_dpdk_pkgconfig}:"*) ;;
    *) export PKG_CONFIG_PATH="${_dpdk_pkgconfig}${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" ;;
esac
unset _dpdk_pkgconfig

_dpdk_bin="/usr/local/bin"
case ":${PATH}:" in
    *":${_dpdk_bin}:"*) ;;
    *) export PATH="${_dpdk_bin}${PATH:+:$PATH}" ;;
esac
unset _dpdk_bin
