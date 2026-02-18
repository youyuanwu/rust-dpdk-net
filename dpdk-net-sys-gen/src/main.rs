use std::path::{Path, PathBuf};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // ── Resolve the target -sys crate root ───────────────────
    let sys_crate_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("dpdk-net-sys");

    // ── Step 1: Discover DPDK include paths via pkgconf ─────
    let pkg = pkgconf::PkgConfigParser::new()
        .probe(["libdpdk"], None)
        .unwrap();

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

    // ── Step 2: Load config and inject pkgconf include paths ─
    let config_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("bnd-winmd.toml");
    let mut cfg =
        bnd_winmd::config::load_config(&config_path).expect("failed to load bnd-winmd.toml");

    let base_dir = config_path.parent().unwrap();

    // Resolve relative include_paths from the TOML against base_dir so
    // that `resolve_header` (which calls `.exists()` directly on the
    // PathBuf) works regardless of the process's CWD.
    cfg.include_paths = cfg
        .include_paths
        .iter()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                base_dir.join(p)
            }
        })
        .collect();

    // Append DPDK system include paths discovered at runtime (already absolute)
    cfg.include_paths.extend(include_paths);
    let winmd_bytes = bnd_winmd::generate_from_config(&cfg, base_dir)
        .expect("bnd-winmd failed to generate .winmd");

    let winmd_path = base_dir.join(&cfg.output.file);
    std::fs::write(&winmd_path, &winmd_bytes).expect("failed to write .winmd file");

    // ── Step 4: .winmd → Rust module tree (package mode) ────
    //     Writes into dpdk-net-sys/src/dpdk/ and appends
    //     features to dpdk-net-sys/Cargo.toml.
    let _warnings = windows_bindgen::bindgen([
        "--in",
        winmd_path.to_str().unwrap(),
        "--out",
        sys_crate_root.to_str().unwrap(),
        "--filter",
        "dpdk",
        "--sys",
        "--package",
    ]);

    println!(
        "Generated bindings in {}/src/dpdk/",
        sys_crate_root.display()
    );
}
