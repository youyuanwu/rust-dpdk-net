# bnd-winmd: C function typedefs generate extra pointer indirection

## Summary

When a C function takes a parameter of type `function_typedef *f` where the typedef is a function type (not a function pointer type), bnd-winmd generates `*mut function_typedef_t` in the Rust binding. Since `function_typedef_t` is already translated as `Option<fn(...)>` (i.e., a function pointer), the result is a **pointer to a function pointer** — one level of indirection too many.

## Reproducer

C header (`rte_launch.h`):

```c
// Note: lcore_function_t is a function TYPE, not a function pointer type
typedef int (lcore_function_t)(void *);

// f is a pointer-to-function, i.e. a function pointer passed by value
int rte_eal_remote_launch(lcore_function_t *f, void *arg, unsigned worker_id);
```

Generated Rust (incorrect):

```rust
pub type lcore_function_t =
    Option<unsafe extern "system" fn(param0: *const core::ffi::c_void) -> i32>;

// *mut lcore_function_t = *mut Option<fn(...)> — pointer to a function pointer!
windows_link::link!("dpdk_dummy" "C" fn rte_eal_remote_launch(
    f: *mut lcore_function_t, arg: *mut core::ffi::c_void, worker_id: u32) -> i32);
```

## Expected

`lcore_function_t *f` in C is just a function pointer passed by value. The correct Rust:

```rust
pub type lcore_function_t =
    Option<unsafe extern "C" fn(param0: *mut core::ffi::c_void) -> i32>;

windows_link::link!("dpdk_dummy" "C" fn rte_eal_remote_launch(
    f: lcore_function_t, arg: *mut core::ffi::c_void, worker_id: u32) -> i32);
```

## Root Cause

In C, `typedef int (func_t)(void *)` defines a **function type**. A parameter `func_t *f` is a pointer to that function type, i.e., a regular function pointer. bnd-winmd appears to first translate `func_t` to `Option<fn(...)>` (already a pointer), then applies `*` from the parameter declaration, producing double indirection.

## Impact

Callers cannot pass function pointers directly — they get type errors. Requires manual wrapper functions to work around.

## Component

bnd-winmd (C-to-winmd stage) — should recognize that `function_type *` in a parameter position is a function pointer, not a pointer-to-function-pointer.
