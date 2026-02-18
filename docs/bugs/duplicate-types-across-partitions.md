# bnd-winmd / windows-bindgen: Duplicate types emitted across partitions

## Summary

When multiple partitions each traverse a shared header (e.g., `bits/types/struct_FILE.h`), each partition gets its own copy of the types and constants from that header. This produces duplicate struct definitions and constant declarations across generated modules.

While the generated code compiles (each module is self-contained), re-exporting all modules via glob (`pub use module::*`) causes `ambiguous_glob_reexports` warnings because the same symbol exists in multiple modules.

## Reproducer

`bnd-winmd.toml`:

```toml
[[partition]]
name = "ethdev"
traverse = ["rte_ethdev.h", "bits/types/struct_FILE.h"]

[[partition]]
name = "mempool"
traverse = ["rte_mempool.h", "bits/types/struct_FILE.h"]

[[partition]]
name = "mbuf"
traverse = ["rte_mbuf.h", "bits/types/struct_FILE.h"]
```

Result: `_IO_FILE` struct, `_IO_EOF_SEEN`, `_IO_ERR_SEEN`, `_IO_USER_LOCK`, `__struct_FILE_defined` constants are all emitted in ethdev, mempool, and mbuf modules.

```
warning: ambiguous glob re-exports
  --> ffi.rs:6:9
   |
 6 | pub use super::dpdk::ethdev::*;
   |         ^^^^^^^^^^^^^^^^^^^^^^ the name `_IO_FILE` is first re-exported here
10 | pub use super::dpdk::mempool::*;
   |         ----------------------- but `_IO_FILE` is also re-exported here
```

## Expected

Shared types should be emitted once — either in a common/foundation module or in whichever partition "owns" the type, with other partitions referencing it via cross-module paths. Windows-bindgen already supports cross-module references (`super::ethdev::_IO_FILE`), but only for some references, not all.

## Workaround

Suppress the warning with `#[allow(ambiguous_glob_reexports)]` on the re-export module, or use explicit named re-exports instead of globs.

## Component

This spans both bnd-winmd (which decides type placement in the .winmd) and windows-bindgen (which decides whether to emit a local copy or a cross-module reference). Ideally bnd-winmd would place shared types in a single partition and other partitions would reference them.
