# windows-bindgen: `extern "system"` used instead of `extern "C"` on Linux targets

## Summary

windows-bindgen emits `extern "system"` for all function pointer typedefs, regardless of the target platform. On Windows, `extern "system"` maps to `__stdcall`, which is correct for Win32 APIs. On Linux, `extern "system"` and `extern "C"` happen to use the same ABI (System V AMD64), but Rust treats them as **distinct types**, causing type-mismatch errors at call sites.

## Reproducer

C header:

```c
typedef int (lcore_function_t)(void *);
```

Generated Rust:

```rust
pub type lcore_function_t =
    Option<unsafe extern "system" fn(param0: *const core::ffi::c_void) -> i32>;
```

Call site error:

```
expected `Option<unsafe extern "system" fn(*const c_void) -> i32>`
   found `Option<unsafe extern "C" fn(*mut c_void) -> i32>`
```

## Expected

When targeting Linux (or any non-Windows platform), function pointer typedefs should use `extern "C"`:

```rust
pub type lcore_function_t =
    Option<unsafe extern "C" fn(param0: *mut core::ffi::c_void) -> i32>;
```

## Workaround

On Linux, `extern "system"` and `extern "C"` are ABI-compatible, so `transmute` works. We currently provide a manually-typed wrapper in `ffi.rs` that re-declares the function with the correct signature.

## Component

windows-bindgen (winmd-to-Rust stage) — should use `extern "C"` for function pointers when not generating Windows API bindings, or provide a configuration option to control the calling convention.
