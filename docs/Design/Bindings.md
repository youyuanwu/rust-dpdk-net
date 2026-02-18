# Replacing bindgen with bnd-winmd

## Motivation

The current `dpdk-net-sys` crate uses `bindgen` to generate Rust FFI bindings
from DPDK C headers. This works but has notable friction:

- A verbose allowlist in `build.rs` (~80 lines) that must be kept in sync with
  every added or removed FFI symbol.
- `bindgen` emits a single monolithic file with no module structure.
- Struct layout and type fidelity issues with complex DPDK types require
  workarounds.

`bnd-winmd` (+ `windows-bindgen`) replaces the clang-AST-to-Rust pipeline with
a two-stage approach that is declarative, composable, and produces idiomatic
module trees.

## Pipeline

```
                    dpdk-net-sys-gen (binary crate)
                    ┌─────────────────────────────────────────────┐
C headers ──►       │ bnd-winmd ──► .winmd ──► windows-bindgen    │
                ▲   │     ▲                        ▲              │
          bnd-winmd.toml                       --package          │
                    └──────────────────────────────┬──────────────┘
                                                   │
                                                   ▼
                                        dpdk-net-sys/src/dpdk/
                                        (checked into source)
```

1. **`dpdk-net-sys-gen`** — a standalone binary crate that runs the two-stage
   pipeline. It is invoked manually (or in CI) whenever DPDK headers change.
2. **`bnd-winmd`** parses the C headers (via libclang) and encodes every
   declaration into a `.winmd` metadata file.
3. **`windows-bindgen`** reads the `.winmd` and emits a Rust module tree
   with `#[link]` extern blocks, structs, constants, and type aliases.
4. The generated `src/dpdk/` tree is **checked into `dpdk-net-sys`**, so
   building `dpdk-net-sys` requires neither `bnd-winmd` nor `windows-bindgen`.

## Current build.rs (bindgen)

The existing script performs three jobs:

| Step | Tool | Purpose |
|------|------|---------|
| 1 | `pkgconf` | Discover DPDK link flags and include paths |
| 2 | `cc` | Compile `wrapper.c` (non-inline wrappers for macros / static inlines) |
| 3 | `bindgen` | Generate `dpdk_bindings.rs` with an explicit allowlist |

Steps 1 and 2 are **unchanged** by this migration — `pkgconf` and `cc` remain.
Only step 3 is replaced.

## Proposed design

### Crate split

The generation pipeline moves into a **new binary crate** `dpdk-net-sys-gen`.
`dpdk-net-sys` itself becomes a pure `-sys` crate with no code-generation
build dependencies.

### `dpdk-net-sys-gen/Cargo.toml`

```toml
[package]
name = "dpdk-net-sys-gen"
version = "0.1.0"
edition.workspace = true
license.workspace = true
publish = false
description = "Code generator: produces dpdk-net-sys Rust bindings from DPDK C headers"

[dependencies]
bnd-winmd = "0.0.1"             # C header → .winmd
windows-bindgen = "0.66"      # .winmd → Rust module tree
pkgconf.workspace = true      # discover DPDK include paths
```

### `dpdk-net-sys/Cargo.toml` (after migration)

```toml
[package]
name = "dpdk-net-sys"
# ...

[dependencies]
windows-link = "0.2"          # provides #[link] macro used by generated code

[build-dependencies]
cc.workspace = true           # KEEP — compile wrapper.c
pkgconf.workspace = true      # KEEP — link flags
# bindgen — REMOVED
# bnd-winmd — NOT HERE (lives in dpdk-net-sys-gen)
# windows-bindgen — NOT HERE

[features]
# ── generated features (written by dpdk-net-sys-gen) ──
```

### Header dependency map

Before defining partitions, we need to understand the transitive include
structure of the DPDK headers we use:

| Header | Direct includes (relevant) | Notes |
|--------|---------------------------|-------|
| `rte_eal.h` | `rte_config.h`, `rte_per_lcore.h`, `rte_uuid.h` | Self-contained |
| `rte_lcore.h` | `rte_config.h`, `rte_eal.h`, `rte_launch.h`, `rte_thread.h` | Pulls in EAL + launch transitively |
| `rte_launch.h` | *(none)* | Fully standalone — zero includes |
| `rte_ethdev.h` | `rte_config.h`, `rte_errno.h`, `rte_common.h`, … | Large; pulls in rte_eth_ctrl.h, rte_ethdev_core.h internally |
| `rte_mbuf.h` | `rte_config.h`, `rte_mempool.h`, `rte_mbuf_core.h`, … | Transitively includes `rte_mempool.h` |
| `rte_mempool.h` | `rte_config.h`, `rte_lcore.h`, `rte_ring.h`, … | Transitively includes `rte_lcore.h` → `rte_eal.h` |
| `rte_errno.h` | `rte_per_lcore.h` | Does **not** include `rte_config.h` |
| `rte_thread.h` | `rte_os.h` | Does **not** include `rte_config.h` |

Since most headers already include `rte_config.h` directly or transitively,
there is no need to list it in every partition's `headers`. We only list the
header we actually want to parse; transitive dependencies are resolved
automatically by clang.

### TOML config — `dpdk-net-sys-gen/bnd-winmd.toml`

`wrapper.h` is kept as-is (unchanged) and gets its **own partition**. The DPDK
header partitions contain only upstream DPDK headers — no wrapper headers mixed
in.

```toml
# include_paths are injected at runtime from pkgconf (see main.rs below)

[output]
name = "dpdk"
file = "dpdk.winmd"

# ── Wrapper (our C shims for inline/macro functions) ────────
# wrapper.h includes rte_config.h, rte_eal.h, rte_ethdev.h,
# rte_mbuf.h, rte_lcore.h, rte_launch.h transitively.
[[partition]]
namespace = "dpdk.wrapper"
library   = "dpdk_dummy"
headers   = ["wrapper.h"]
traverse  = ["wrapper.h"]

# ── EAL ─────────────────────────────────────────────────────
# rte_eal.h includes rte_config.h transitively.
[[partition]]
namespace = "dpdk.eal"
library   = "dpdk_dummy"
headers   = ["rte_eal.h"]
traverse  = ["rte_eal.h"]

# ── Lcore ────────────────────────────────────────────────────
# rte_lcore.h includes rte_config.h, rte_eal.h, rte_launch.h,
# rte_thread.h transitively.
[[partition]]
namespace = "dpdk.lcore"
library   = "dpdk_dummy"
headers   = ["rte_lcore.h"]
traverse  = ["rte_lcore.h"]

# ── Launch ───────────────────────────────────────────────────
# rte_launch.h has zero includes — fully standalone.
[[partition]]
namespace = "dpdk.launch"
library   = "dpdk_dummy"
headers   = ["rte_launch.h"]
traverse  = ["rte_launch.h"]

# ── Thread ───────────────────────────────────────────────────
# rte_thread.h includes only rte_os.h, rte_compat.h.
[[partition]]
namespace = "dpdk.thread"
library   = "dpdk_dummy"
headers   = ["rte_thread.h"]
traverse  = ["rte_thread.h"]

# ── Mempool ──────────────────────────────────────────────────
# rte_mempool.h includes rte_config.h, rte_lcore.h transitively.
[[partition]]
namespace = "dpdk.mempool"
library   = "dpdk_dummy"
headers   = ["rte_mempool.h"]
traverse  = ["rte_mempool.h"]

# ── Mbuf ─────────────────────────────────────────────────────
# rte_mbuf.h includes rte_mempool.h transitively.
[[partition]]
namespace = "dpdk.mbuf"
library   = "dpdk_dummy"
headers   = ["rte_mbuf.h"]
traverse  = ["rte_mbuf.h"]

# ── Ethernet device ─────────────────────────────────────────
# rte_ethdev.h includes rte_config.h transitively.
[[partition]]
namespace = "dpdk.ethdev"
library   = "dpdk_dummy"
headers   = ["rte_ethdev.h"]
traverse  = ["rte_ethdev.h"]
```

**Key decisions:**

| Field | Value | Rationale |
|-------|-------|-----------|
| `library` | `"dpdk_dummy"` | A placeholder value — `bnd-winmd` requires this field but `windows-bindgen` does not handle linking on Linux. Actual linking is done by `pkgconf::emit_cargo_metadata` and `cc` in `build.rs`. |
| `headers` | Minimal | Each partition lists only its primary header. Transitive includes (e.g. `rte_config.h`) are resolved by clang automatically — no need to list them. |
| `traverse` | Same as `headers` | We traverse exactly what we list — only declarations from the target header are extracted. Types from transitive includes are available but not emitted. |
| `namespace` | Dotted, 1:1 with DPDK header | `dpdk.lcore` → `rte_lcore.h`, `dpdk.launch` → `rte_launch.h`, etc. |
| `wrapper.h` | Own partition | All our C wrapper functions (`rust_*`) and RSS constants live in a single `dpdk.wrapper` partition. This keeps the upstream DPDK partitions clean and `wrapper.h` unchanged. |

### `dpdk-net-sys-gen/src/main.rs`

```rust
use std::path::{Path, PathBuf};

fn main() {
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

    // ── Step 2: bnd-winmd → .winmd ──────────────────────────
    let extra_includes: Vec<String> = std::iter::once(
            sys_crate_root.join("include").display().to_string()
        )
        .chain(include_paths.iter().map(|p| p.display().to_string()))
        .collect();

    let winmd = bnd_winmd::run_with_includes(
        Path::new("bnd-winmd.toml"),
        &extra_includes,
    )
    .expect("bnd-winmd failed to produce .winmd");

    // ── Step 3: .winmd → Rust module tree (package mode) ────
    //     Writes into dpdk-net-sys/src/dpdk/ and appends
    //     features to dpdk-net-sys/Cargo.toml.
    windows_bindgen::bindgen([
        "--in",      winmd.to_str().unwrap(),
        "--out",     sys_crate_root.to_str().unwrap(),
        "--filter",  "dpdk",
        "--sys",
        "--package",
    ])
    .expect("windows-bindgen failed");

    println!("Generated bindings in {}/src/dpdk/", sys_crate_root.display());
}
```

> **Note:** The exact API for injecting runtime include paths into `bnd-winmd`
> depends on the version. If `run_with_includes` is not available, an
> alternative is to template the TOML at build time with the discovered paths
> and write it to `OUT_DIR` before calling `bnd_winmd::run()`.

### `dpdk-net-sys/build.rs` (simplified)

`build.rs` now only handles pkgconf linking and compiling `wrapper.c` — no
code generation:

```rust
use std::path::PathBuf;

fn main() {
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
    cc_builder.flag("-march=corei7");
    cc_builder.compile("dpdk_wrapper");
}
```

### Source layout after migration

```
dpdk-net-sys-gen/               # NEW crate
├── Cargo.toml
├── bnd-winmd.toml              # declarative header config
└── src/
    └── main.rs                 # runs bnd-winmd + windows-bindgen

dpdk-net-sys/
├── build.rs                    # SIMPLIFIED — pkgconf + cc only
├── Cargo.toml                  # CHANGED — bindgen removed, generated [features]
├── include/
│   └── wrapper.h               # UNCHANGED
├── src/
│   ├── dpdk/                   # GENERATED (checked in) — module tree
│   │   ├── mod.rs
│   │   ├── wrapper/
│   │   │   └── mod.rs          # rust_* shims, RSS constants
│   │   ├── eal/
│   │   │   └── mod.rs          # rte_eal_init, rte_eal_cleanup, …
│   │   ├── lcore/
│   │   │   └── mod.rs          # rte_lcore_count, rte_lcore_is_enabled, …
│   │   ├── launch/
│   │   │   └── mod.rs          # rte_eal_remote_launch, rte_eal_mp_wait_lcore, …
│   │   ├── thread/
│   │   │   └── mod.rs          # rte_thread_set_affinity, rte_thread_register, …
│   │   ├── mempool/
│   │   │   └── mod.rs          # rte_pktmbuf_pool_create, rte_mempool_free, …
│   │   ├── mbuf/
│   │   │   └── mod.rs          # rte_mbuf, rte_pktmbuf_free_bulk, …
│   │   └── ethdev/
│   │       └── mod.rs          # rte_eth_dev_configure, rte_eth_rx_queue_setup, …
│   ├── wrapper.c               # UNCHANGED
│   └── lib.rs                  # CHANGED — `pub mod dpdk;` (replaces ffi.rs)
```

### Changes to `src/lib.rs`

```rust
// Before (bindgen):
pub mod ffi;

// After (bnd-winmd, package mode):
pub mod dpdk;  // generated sub-modules: dpdk::eal, dpdk::ethdev, dpdk::mbuf
```

`src/ffi.rs` is **deleted** — it is superseded by the generated module tree
under `src/dpdk/`.

## What stays the same

| Component | Why |
|-----------|-----|
| `wrapper.h` / `wrapper.c` | DPDK exposes many hot-path functions as `static inline` or macros (`rte_pktmbuf_alloc`, `rte_eth_rx_burst`, `rte_lcore_id`, etc.). Neither bindgen nor bnd-winmd can generate Rust calls to these directly — the C wrapper functions compiled by `cc` remain necessary. |
| `pkgconf` integration | DPDK installs a `.pc` file; we still need it for link flags and include paths at build time. |
| `cc` compilation | Compiles `wrapper.c` into a static archive linked into the final binary. |
| RSS constant wrappers | The `RUST_RTE_ETH_RSS_*` constants in `wrapper.h` expand `RTE_BIT64()` macros into `static const` values that both bindgen and bnd-winmd can parse. |

## What changes

| Before (bindgen) | After (bnd-winmd) |
|---|---|
| ~80-line allowlist in `build.rs` | Declarative `bnd-winmd.toml` with `headers` + `traverse` |
| Single flat `dpdk_bindings.rs` in `OUT_DIR` | Module tree `src/dpdk/{wrapper,eal,lcore,launch,thread,mempool,mbuf,ethdev}/mod.rs` |
| `bindgen` build-dep in `-sys` crate | `bnd-winmd` + `windows-bindgen` in separate generator crate; `windows-link` runtime dep in `-sys` |
| Code generated on every build | Code generated once, checked in; re-run generator when headers change |
| Symbols opted-in one by one | Entire header is traversed; unwanted symbols are excluded by limiting `traverse` |
| `pub mod ffi` with `include!()` | `pub mod dpdk` with feature-gated sub-modules |
| No Cargo features | Auto-generated features per partition (e.g. `dpdk_eal`, `dpdk_ethdev`, `dpdk_mbuf`) |

## Why package mode

We use `--package` (not `--flat`) because it gives us:

- **Module structure** — symbols are organized into `dpdk::eal`, `dpdk::lcore`,
  `dpdk::launch`, `dpdk::ethdev`, `dpdk::mbuf`, `dpdk::wrapper`, etc., matching
  the DPDK subsystem (or our wrapper layer) they belong to.
- **Feature gates** — each partition generates a Cargo feature. Downstream
  crates can depend on only the subsystems they need.
- **Scalability** — adding a new DPDK subsystem (e.g. `dpdk.crypto`) is a new
  `[[partition]]` entry; no changes to `build.rs`.

### Import path migration

| Before | After |
|---|---|
| `dpdk_net_sys::ffi::rte_eal_init` | `dpdk_net_sys::dpdk::eal::rte_eal_init` |
| `dpdk_net_sys::ffi::rte_eal_cleanup` | `dpdk_net_sys::dpdk::eal::rte_eal_cleanup` |
| `dpdk_net_sys::ffi::rte_eth_dev_configure` | `dpdk_net_sys::dpdk::ethdev::rte_eth_dev_configure` |
| `dpdk_net_sys::ffi::rte_eth_conf` | `dpdk_net_sys::dpdk::ethdev::rte_eth_conf` |
| `dpdk_net_sys::ffi::rte_lcore_count` | `dpdk_net_sys::dpdk::lcore::rte_lcore_count` |
| `dpdk_net_sys::ffi::rte_eal_lcore_role` | `dpdk_net_sys::dpdk::lcore::rte_eal_lcore_role` |
| `dpdk_net_sys::ffi::rte_eal_remote_launch` | `dpdk_net_sys::dpdk::launch::rte_eal_remote_launch` |
| `dpdk_net_sys::ffi::rte_eal_wait_lcore` | `dpdk_net_sys::dpdk::launch::rte_eal_wait_lcore` |
| `dpdk_net_sys::ffi::rte_eal_get_lcore_state` | `dpdk_net_sys::dpdk::launch::rte_eal_get_lcore_state` |
| `dpdk_net_sys::ffi::rte_thread_register` | `dpdk_net_sys::dpdk::thread::rte_thread_register` |
| `dpdk_net_sys::ffi::rte_pktmbuf_pool_create` | `dpdk_net_sys::dpdk::mbuf::rte_pktmbuf_pool_create` |
| `dpdk_net_sys::ffi::rte_mempool_free` | `dpdk_net_sys::dpdk::mempool::rte_mempool_free` |
| `dpdk_net_sys::ffi::rte_mbuf` | `dpdk_net_sys::dpdk::mbuf::rte_mbuf` |
| `dpdk_net_sys::ffi::rust_pktmbuf_alloc` | `dpdk_net_sys::dpdk::wrapper::rust_pktmbuf_alloc` |
| `dpdk_net_sys::ffi::rust_eth_rx_burst` | `dpdk_net_sys::dpdk::wrapper::rust_eth_rx_burst` |
| `dpdk_net_sys::ffi::rust_rte_lcore_id` | `dpdk_net_sys::dpdk::wrapper::rust_rte_lcore_id` |
| `dpdk_net_sys::ffi::rust_get_rte_errno` | `dpdk_net_sys::dpdk::wrapper::rust_get_rte_errno` |
| `dpdk_net_sys::ffi::RUST_RTE_ETH_RSS_IPV4` | `dpdk_net_sys::dpdk::wrapper::RUST_RTE_ETH_RSS_IPV4` |
| `dpdk_net_sys::ffi::RTE_MAX_LCORE` | See **Preprocessor macros** in open questions |
| `dpdk_net_sys::ffi::RTE_MBUF_DEFAULT_DATAROOM` | See **Preprocessor macros** in open questions |
| `dpdk_net_sys::ffi::RTE_PKTMBUF_HEADROOM` | See **Preprocessor macros** in open questions |

> **Note:** `rte_pktmbuf_pool_create` is declared in `rte_mbuf.h` (not
> `rte_mempool.h`), so it maps to `dpdk::mbuf`. Functions like
> `rte_eal_remote_launch`, `rte_eal_wait_lcore`, and `rte_eal_get_lcore_state`
> are declared in `rte_launch.h` despite their `rte_eal_` prefix, so they
> map to `dpdk::launch`.

## Migration steps

1. Create the `dpdk-net-sys-gen` crate with `Cargo.toml`, `src/main.rs`, and
   `bnd-winmd.toml` as described above.
2. Add `dpdk-net-sys-gen` to the workspace `members` list.
3. In `dpdk-net-sys/Cargo.toml`: remove `bindgen` from `[build-dependencies]`,
   add `windows-link` to `[dependencies]`, add a `[features]` marker section.
4. Simplify `dpdk-net-sys/build.rs` to pkgconf + cc only (remove all bindgen
   code).
5. Run the generator:
   ```sh
   cargo run -p dpdk-net-sys-gen
   ```
   This writes `dpdk-net-sys/src/dpdk/` and appends features to
   `dpdk-net-sys/Cargo.toml`.
6. Delete `dpdk-net-sys/src/ffi.rs`. Change `src/lib.rs` to `pub mod dpdk;`.
7. Check in the generated `src/dpdk/` directory.
8. Update all downstream imports in `dpdk-net`, `dpdk-net-hyper`,
   `dpdk-net-axum`, `dpdk-net-test` from `dpdk_net_sys::ffi::*` to the
   new `dpdk_net_sys::dpdk::{eal,ethdev,mbuf}::*` paths.
9. Build and verify all symbols resolve — use `RUST_LOG=bnd_winmd=debug` for
   diagnostics.
10. Fix any "type not found" errors by adding missing headers to `traverse`
    and re-running step 5.
11. Confirm full workspace compiles and tests pass.

### Re-generating bindings

After upgrading DPDK or modifying `wrapper.h`, re-run:

```sh
cargo run -p dpdk-net-sys-gen
```

Review the diff in `dpdk-net-sys/src/dpdk/` and commit.

## Open questions

- **Preprocessor macros (`#define` constants):** `RTE_MAX_LCORE`,
  `RTE_PKTMBUF_HEADROOM`, and `RTE_MBUF_DEFAULT_DATAROOM` are `#define`
  macros in `rte_build_config.h` / `rte_mbuf_core.h`. `bnd-winmd` may not
  extract them because they are preprocessor definitions, not C declarations.
  **Mitigation:** Add `static const` wrappers for these in `wrapper.h` (the
  same pattern already used for `RUST_RTE_ETH_RSS_*` constants). For example:
  ```c
  static const unsigned int RUST_RTE_MAX_LCORE = RTE_MAX_LCORE;
  static const uint16_t RUST_RTE_MBUF_DEFAULT_DATAROOM = RTE_MBUF_DEFAULT_DATAROOM;
  static const uint16_t RUST_RTE_PKTMBUF_HEADROOM = RTE_PKTMBUF_HEADROOM;
  ```
  Similarly, `LCORE_ID_ANY` (defined in `rte_lcore.h`) and
  `RTE_ETHDEV_QUEUE_STAT_CNTRS` need the same treatment.
- **Cross-partition type references:** Functions in one partition may use
  types defined in another (e.g. `rte_eth_rx_queue_setup` takes
  `rte_mempool*`, `rust_eth_rx_burst` takes `rte_mbuf**`). Verify that
  `windows-bindgen` generates correct cross-module type references or
  re-exports, rather than duplicating struct definitions.
- **Runtime include-path injection:** The `bnd-winmd` API may or may not expose
  a `run_with_includes` helper. If not, we can write a patched TOML to
  `OUT_DIR` at build time with the pkgconf-discovered paths spliced in.
- **Variadic functions:** DPDK has a few (`rte_log`, etc.). bnd-winmd
  auto-skips these — confirm none are in our required set.
- **Anonymous unions in structs:** `rte_mbuf` contains anonymous unions.
  bnd-winmd may need manual workarounds; test and document.
- **`library` field semantics:** We link DPDK via pkgconf, not via
  `#[link(name = "...")]`. Verify that `windows-bindgen` lets us suppress or
  override the emitted `#[link]` attributes so they don't conflict with
  pkgconf's `cargo:rustc-link-lib` directives.
- **Generated code freshness:** Since bindings are checked in, CI should
  verify they are up-to-date by running `cargo run -p dpdk-net-sys-gen` and
  asserting no diff.
