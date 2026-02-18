# bnd-winmd: `void *` mapped to `*const c_void` in callback typedefs

## Summary

bnd-winmd translates `void *` parameters in C function pointer typedefs as `*const core::ffi::c_void` instead of `*mut core::ffi::c_void`.

In C, `void *` is an unqualified (mutable) pointer. The Rust equivalent is `*mut c_void`. Using `*const` prevents the callback from mutating through the pointer and causes type mismatches when callers pass `*mut c_void`.

## Reproducer

C header (`rte_launch.h`):

```c
typedef int (lcore_function_t)(void *);
```

Generated Rust (incorrect):

```rust
pub type lcore_function_t =
    Option<unsafe extern "system" fn(param0: *const core::ffi::c_void) -> i32>;
```

Similarly for `rte_thread.h`:

```c
typedef uint32_t (*rte_thread_func) (void *arg);
```

Generated:

```rust
pub type rte_thread_func_t =
    Option<unsafe extern "system" fn(param0: *const core::ffi::c_void) -> u32>;
```

## Expected

```rust
pub type lcore_function_t =
    Option<unsafe extern "C" fn(param0: *mut core::ffi::c_void) -> i32>;
```

(The `extern "system"` vs `extern "C"` issue is tracked separately.)

## Impact

Callers must cast `*mut c_void` to `*const c_void` or transmute when passing arguments to callbacks, which defeats the purpose of having typed bindings.

## Component

bnd-winmd (C-to-winmd stage) — should map unqualified `void *` to `*mut c_void`, reserving `*const c_void` for `const void *`.
