use std::path::PathBuf;

fn main() {
    // Rebuild if wrapper files change
    println!("cargo:rerun-if-changed=include/wrapper.h");
    println!("cargo:rerun-if-changed=src/wrapper.c");

    // ── Step 1: pkg-config ──────────────────────────────────
    let pkg = pkgconf::PkgConfigParser::new()
        .probe(["libdpdk"], None)
        .unwrap();
    pkgconf::emit_cargo_metadata(&pkg.libs, true);

    let include_paths: Vec<PathBuf> = pkg
        .cflags
        .iter()
        .filter_map(|flag| {
            if let pkgconf::CompilerFlag::IncludePath(path) = flag {
                Some(path.clone())
            } else {
                None
            }
        })
        .collect();

    // ── Step 2: Compile wrapper.c ───────────────────────────
    let mut cc_builder = cc::Build::new();
    cc_builder.file("src/wrapper.c");
    cc_builder.include("include");
    for path in &include_paths {
        cc_builder.include(path);
    }
    // Use corei7/Nehalem for QEMU software emulation compatibility
    // This matches DPDK's cpu_instruction_set=generic setting
    cc_builder.flag("-march=corei7");
    cc_builder.compile("dpdk_wrapper");
}
