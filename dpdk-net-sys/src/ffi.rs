//! Compatibility shim: re-exports all generated bindings so that
//! downstream code using `dpdk_net_sys::ffi::*` continues to compile.
#![allow(unused_imports)]

pub use super::dpdk::eal::*;
pub use super::dpdk::ethdev::*;
pub use super::dpdk::launch::*;
pub use super::dpdk::lcore::*;
pub use super::dpdk::mbuf::*;
pub use super::dpdk::mempool::*;
pub use super::dpdk::thread::*;
pub use super::dpdk::wrapper::*;

// ── ABI-corrected wrappers ──────────────────────────────────
// The generated binding for rte_eal_remote_launch incorrectly
// takes `*mut lcore_function_t` (pointer-to-function-pointer)
// instead of `lcore_function_t` (function pointer by value).
// Provide a correctly-typed wrapper that shadows the generated one.
/// Launch a function on another lcore.
///
/// # Safety
/// Caller must ensure `arg` points to valid data for the duration
/// of the remote call.
pub unsafe fn rte_eal_remote_launch(
    f: Option<unsafe extern "C" fn(arg: *mut core::ffi::c_void) -> i32>,
    arg: *mut core::ffi::c_void,
    worker_id: u32,
) -> i32 {
    unsafe {
        #[allow(clashing_extern_declarations)]
        unsafe extern "C" {
            fn rte_eal_remote_launch(
                f: Option<unsafe extern "C" fn(*mut core::ffi::c_void) -> i32>,
                arg: *mut core::ffi::c_void,
                worker_id: core::ffi::c_uint,
            ) -> core::ffi::c_int;
        }
        rte_eal_remote_launch(f, arg, worker_id)
    }
}

// ── Build-config constants ──────────────────────────────────
// These come from rte_build_config.h / DPDK headers and cannot
// be extracted by bnd-winmd (they use complex macros).
pub const RTE_MAX_LCORE: u32 = 128;
pub const RTE_MAX_NUMA_NODES: u32 = 32;
pub const RTE_PKTMBUF_HEADROOM: u16 = 128;
pub const RTE_MBUF_DEFAULT_DATAROOM: u16 = 2048;
pub const RTE_MBUF_MAX_NB_SEGS: u16 = u16::MAX;
pub const LCORE_ID_ANY: u32 = u32::MAX;

// ── RSS hash constants ──────────────────────────────────────
// RTE_ETH_RSS_* are defined as RTE_BIT64(n) which bnd-winmd
// cannot evaluate.  Values from rte_ethdev.h.
pub const RUST_RTE_ETH_RSS_IPV4: u64 = 1 << 2;
pub const RUST_RTE_ETH_RSS_FRAG_IPV4: u64 = 1 << 3;
pub const RUST_RTE_ETH_RSS_NONFRAG_IPV4_TCP: u64 = 1 << 4;
pub const RUST_RTE_ETH_RSS_NONFRAG_IPV4_UDP: u64 = 1 << 5;
pub const RUST_RTE_ETH_RSS_NONFRAG_IPV4_SCTP: u64 = 1 << 6;
pub const RUST_RTE_ETH_RSS_NONFRAG_IPV4_OTHER: u64 = 1 << 7;
pub const RUST_RTE_ETH_RSS_IPV6: u64 = 1 << 8;
pub const RUST_RTE_ETH_RSS_FRAG_IPV6: u64 = 1 << 9;
pub const RUST_RTE_ETH_RSS_NONFRAG_IPV6_TCP: u64 = 1 << 10;
pub const RUST_RTE_ETH_RSS_NONFRAG_IPV6_UDP: u64 = 1 << 11;
pub const RUST_RTE_ETH_RSS_NONFRAG_IPV6_SCTP: u64 = 1 << 12;
pub const RUST_RTE_ETH_RSS_NONFRAG_IPV6_OTHER: u64 = 1 << 13;
pub const RUST_RTE_ETH_RSS_IPV6_EX: u64 = 1 << 15;
pub const RUST_RTE_ETH_RSS_IPV6_TCP_EX: u64 = 1 << 16;
pub const RUST_RTE_ETH_RSS_IPV6_UDP_EX: u64 = 1 << 17;
pub const RUST_RTE_ETH_RSS_IP: u64 = RUST_RTE_ETH_RSS_IPV4
    | RUST_RTE_ETH_RSS_FRAG_IPV4
    | RUST_RTE_ETH_RSS_NONFRAG_IPV4_OTHER
    | RUST_RTE_ETH_RSS_IPV6
    | RUST_RTE_ETH_RSS_FRAG_IPV6
    | RUST_RTE_ETH_RSS_NONFRAG_IPV6_OTHER
    | RUST_RTE_ETH_RSS_IPV6_EX;
pub const RUST_RTE_ETH_RSS_TCP: u64 = RUST_RTE_ETH_RSS_NONFRAG_IPV4_TCP
    | RUST_RTE_ETH_RSS_NONFRAG_IPV6_TCP
    | RUST_RTE_ETH_RSS_IPV6_TCP_EX;
pub const RUST_RTE_ETH_RSS_UDP: u64 = RUST_RTE_ETH_RSS_NONFRAG_IPV4_UDP
    | RUST_RTE_ETH_RSS_NONFRAG_IPV6_UDP
    | RUST_RTE_ETH_RSS_IPV6_UDP_EX;
