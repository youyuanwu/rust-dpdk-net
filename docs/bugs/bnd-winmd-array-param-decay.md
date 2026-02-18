# bnd-winmd: C array parameters not decayed to pointers

## Summary

bnd-winmd translates C function parameters with array types (e.g. `rte_uuid_t` which is `unsigned char[16]`) as by-value Rust arrays (`[u8; 16]`) instead of pointers (`*mut u8`).

In C, array parameters always decay to pointers — `void f(int a[16])` is identical to `void f(int *a)`. The generated Rust binding passes the array by value, which is not FFI-safe and triggers `improper_ctypes` warnings.

## Reproducer

C header (`rte_uuid.h` / `rte_eal.h`):

```c
typedef unsigned char rte_uuid_t[16];

void rte_eal_vfio_get_vf_token(rte_uuid_t vf_token);
```

Generated Rust (incorrect):

```rust
windows_link::link!("dpdk_dummy" "C" fn rte_eal_vfio_get_vf_token(vf_token: [u8; 16]));
```

```
warning: `extern` block uses type `[u8; 16]`, which is not FFI-safe
  = help: consider passing a pointer to the array
  = note: passing raw arrays by value is not FFI-safe
```

## Expected

```rust
windows_link::link!("dpdk_dummy" "C" fn rte_eal_vfio_get_vf_token(vf_token: *mut u8));
```

## Component

bnd-winmd (C-to-winmd stage) — should recognize array-typed function parameters and emit pointer types instead.
