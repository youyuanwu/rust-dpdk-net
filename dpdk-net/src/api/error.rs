pub type Errno = nix::errno::Errno;

/// Result type alias using nix's Errno for DPDK operations
pub type Result<T> = std::result::Result<T, Errno>;

pub fn rte_errno() -> Errno {
    let num = unsafe { dpdk_net_sys::ffi::rust_get_rte_errno() };
    Errno::from_raw(num)
}

pub fn check_rte_success(ret: i32) -> Result<()> {
    if ret < 0 { Err(rte_errno()) } else { Ok(()) }
}

// Currently dpdk has only few error codes defined in rte_errno.h
// So we ignore them for now.
// See: /usr/local/include/rte_errno.h
// fn rte_strerror(errnum: i32) -> String {
//     unsafe {
//         let c_str = dpdk_net_sys::ffi::rte_strerror(errnum);
//         if c_str.is_null() {
//             return "Unknown error".to_string();
//         }
//         let rust_str = std::ffi::CStr::from_ptr(c_str).to_string_lossy().into_owned();
//         rust_str
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_rte_success() {
        let e = Errno::EADV;
        println!("Errno EADV: {}", e);
    }
}
