use std::path::PathBuf;

fn main() {
    // Rebuild if wrapper files change
    println!("cargo:rerun-if-changed=include/wrapper.h");
    println!("cargo:rerun-if-changed=src/wrapper.c");

    // Use pkg-config to find DPDK with static linking preferred
    let cfg = pkg_config::Config::new()
        .atleast_version("25.11.0")
        .statik(true)
        .cargo_metadata(false)
        .probe("libdpdk")
        .unwrap();

    // Use pkgconf to emit cargo metadata.
    pkgconf::PkgConfigParser::new()
        .probe_and_emit(["libdpdk"], None)
        .unwrap();

    generate_bindings(&cfg.include_paths);
}

fn generate_bindings(include_dirs: &[PathBuf]) {
    // Generate bindings using bindgen if needed
    // Generate the dpdk rust bindings.
    let outdir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // Compile wrapper.c with cc
    let mut cc_builder = cc::Build::new();
    cc_builder.file("src/wrapper.c");
    cc_builder.include("include"); // For wrapper.h
    for path in include_dirs {
        cc_builder.include(path);
    }
    // Use corei7/Nehalem for QEMU software emulation compatibility
    // This matches DPDK's cpu_instruction_set=generic setting
    cc_builder.flag("-march=corei7");
    cc_builder.compile("dpdk_wrapper");

    // Start with include paths from pkg-config
    let mut bgbuilder = bindgen::builder();
    for path in include_dirs {
        bgbuilder = bgbuilder.clang_arg(format!("-I{}", path.display()));
    }

    let bgbuilder = bgbuilder
        // generate all the wrapper functions defined in csrc/header.h
        .allowlist_function("rte_.*_")
        // allow our rust wrapper functions
        .allowlist_function("rust_.*")
        .allowlist_function("rte_strerror")
        // generate useful dpdk functions
        .allowlist_function("rte_thread_set_affinity")
        .allowlist_function("rte_thread_register")
        .allowlist_function("rte_thread_unregister")
        .allowlist_function("rte_pktmbuf_pool_create")
        .allowlist_function("rte_mempool_free")
        .allowlist_function("rte_mempool_lookup")
        .allowlist_function("rte_mp_disable")
        .allowlist_function("rte_eal_process_type")
        .allowlist_function("rte_pktmbuf_free_bulk")
        .allowlist_function("rte_mempool_avail_count") // this can be removed
        .allowlist_function("rte_eth_dev_info_get")
        .allowlist_function("rte_eth_dev_count_avail")
        .allowlist_function("rte_eth_macaddr_get")
        .allowlist_function("rte_eth_stats_get")
        .allowlist_function("rte_eth_dev_socket_id")
        .allowlist_function("rte_eth_dev_configure")
        .allowlist_function("rte_eth_dev_start")
        .allowlist_function("rte_eth_dev_stop")
        .allowlist_function("rte_eth_dev_close")
        .allowlist_function("rte_eth_rx_queue_setup")
        .allowlist_function("rte_eth_tx_queue_setup")
        .allowlist_function("rte_eth_promiscuous_enable")
        .allowlist_function("rte_eth_promiscuous_disable")
        .allowlist_function("rte_eth_dev_rss_reta_update")
        .allowlist_function("rte_eth_dev_rss_reta_query")
        .allowlist_function("rte_eth_dev_rss_hash_update")
        .allowlist_function("rte_eth_dev_rss_hash_conf_get")
        .allowlist_function("rte_eal_init")
        .allowlist_function("rte_eal_cleanup")
        // Lcore management functions
        .allowlist_function("rte_eal_remote_launch")
        .allowlist_function("rte_eal_mp_wait_lcore")
        .allowlist_function("rte_eal_wait_lcore")
        .allowlist_function("rte_eal_get_lcore_state")
        .allowlist_function("rte_eal_lcore_role")
        .allowlist_function("rte_lcore_count")
        .allowlist_function("rte_lcore_is_enabled")
        .allowlist_function("rte_lcore_to_socket_id")
        .allowlist_function("rte_lcore_to_cpu_id")
        .allowlist_function("rte_get_next_lcore")
        // Lcore wrapper functions for inlines
        .allowlist_function("rust_rte_lcore_id")
        .allowlist_function("rust_rte_get_main_lcore")
        // generate useful dpdk types
        .allowlist_type("rte_eth_conf")
        .allowlist_type("rte_eth_dev_info")
        .allowlist_type("rte_ether_addr")
        .allowlist_type("rte_mempool")
        .allowlist_type("rte_mbuf")
        .allowlist_type("rte_eth_stats")
        .allowlist_type("rte_proc_type_t")
        // Lcore types
        .allowlist_type("rte_lcore_state_t")
        .allowlist_type("rte_lcore_role_t")
        .allowlist_type("lcore_function_t")
        // generate useful dpdk macros defined in rte_build_config.h.
        .allowlist_var("RTE_MAX_LCORE")
        .allowlist_var("LCORE_ID_ANY")
        .allowlist_var("RTE_MAX_NUMA_NODES")
        .allowlist_var("RTE_MBUF_MAX_NB_SEGS")
        .allowlist_var("RTE_MBUF_DEFAULT_DATAROOM")
        .allowlist_var("RTE_PKTMBUF_HEADROOM")
        .allowlist_var("RTE_ETHDEV_QUEUE_STAT_CNTRS")
        // RSS hash type constants (from wrapper.h static consts)
        .allowlist_var("RUST_RTE_ETH_RSS_.*")
        .header("include/wrapper.h");

    let bindings = bgbuilder
        .generate()
        .expect("Unable to generate DPDK bindings");

    bindings
        .write_to_file(outdir.join("dpdk_bindings.rs"))
        .expect("Couldn't write bindings!");
    cc_builder.compile("dpdk_static_fns");
}
