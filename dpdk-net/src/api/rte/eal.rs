// rte EAL (Environment Abstraction Layer) API
// See: /usr/local/include/rte_eal.h

use std::ffi::CString;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::api::check_rte_success;

/// Global flag to track if EAL has been initialized
static EAL_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Well-known EAL options as strongly-typed enum variants.
#[derive(Debug, Clone)]
pub enum EalOption {
    /// Program name (required as first argument)
    ProgramName(String),
    /// Don't use hugepages (--no-huge)
    NoHuge,
    /// Don't scan PCI bus (--no-pci)
    NoPci,
    /// Add a virtual device (--vdev=<device>)
    Vdev(String),
    /// Core mask in hex (e.g., "0xf" for cores 0-3)
    CoreMask(String),
    /// Core list (e.g., "0-3" or "0,2,4")
    CoreList(String),
    /// Number of memory channels (-n <num>)
    MemoryChannels(u32),
    /// Process type (--proc-type=<type>)
    ProcessType(ProcessType),
    /// File prefix for multi-process (--file-prefix=<prefix>)
    FilePrefix(String),
    /// Memory per socket in MB (--socket-mem=<amounts>)
    SocketMem(String),
    /// Log level (--log-level=<level>)
    LogLevel(LogLevel),
    /// In-memory mode, no persistent files (--in-memory)
    InMemory,
    /// Base virtual address (--base-virtaddr=<addr>)
    BaseVirtAddr(String),
    /// Allow a PCI device (-a <pci_addr>)
    Allow(String),
    /// Custom argument (pass-through)
    Custom(String),
}

impl EalOption {
    /// Convert to command-line argument strings
    fn to_args(&self) -> Vec<String> {
        match self {
            EalOption::ProgramName(name) => vec![name.clone()],
            EalOption::NoHuge => vec!["--no-huge".to_string()],
            EalOption::NoPci => vec!["--no-pci".to_string()],
            EalOption::Vdev(dev) => vec![format!("--vdev={}", dev)],
            EalOption::CoreMask(mask) => vec!["-c".to_string(), mask.clone()],
            EalOption::CoreList(list) => vec!["-l".to_string(), list.clone()],
            EalOption::MemoryChannels(n) => vec!["-n".to_string(), n.to_string()],
            EalOption::ProcessType(pt) => vec![format!("--proc-type={}", pt.as_str())],
            EalOption::FilePrefix(prefix) => vec![format!("--file-prefix={}", prefix)],
            EalOption::SocketMem(mem) => vec![format!("--socket-mem={}", mem)],
            EalOption::LogLevel(level) => vec![format!("--log-level={}", level.as_str())],
            EalOption::InMemory => vec!["--in-memory".to_string()],
            EalOption::BaseVirtAddr(addr) => vec![format!("--base-virtaddr={}", addr)],
            EalOption::Allow(pci_addr) => vec!["-a".to_string(), pci_addr.clone()],
            EalOption::Custom(arg) => vec![arg.clone()],
        }
    }
}

/// DPDK process type for multi-process support
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessType {
    Primary,
    Secondary,
    Auto,
}

impl ProcessType {
    fn as_str(&self) -> &'static str {
        match self {
            ProcessType::Primary => "primary",
            ProcessType::Secondary => "secondary",
            ProcessType::Auto => "auto",
        }
    }
}

/// DPDK log levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Emergency,
    Alert,
    Critical,
    Error,
    Warning,
    Notice,
    Info,
    Debug,
    /// Custom numeric level (1-8)
    Level(u8),
}

impl LogLevel {
    fn as_str(&self) -> String {
        match self {
            LogLevel::Emergency => "1".to_string(),
            LogLevel::Alert => "2".to_string(),
            LogLevel::Critical => "3".to_string(),
            LogLevel::Error => "4".to_string(),
            LogLevel::Warning => "5".to_string(),
            LogLevel::Notice => "6".to_string(),
            LogLevel::Info => "7".to_string(),
            LogLevel::Debug => "8".to_string(),
            LogLevel::Level(n) => n.to_string(),
        }
    }
}

/// Builder for EAL initialization options.
///
/// # Example
/// ```no_run
/// use dpdk_net::api::rte::eal::{EalBuilder, LogLevel};
///
/// fn main() -> Result<(), nix::errno::Errno> {
///     let _eal = EalBuilder::new()
///         .no_huge()
///         .no_pci()
///         .vdev("net_ring0")
///         .log_level(LogLevel::Warning)
///         .init()?;
///     Ok(())
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct EalBuilder {
    program_name: Option<String>,
    options: Vec<EalOption>,
}

impl EalBuilder {
    /// Create a new EAL builder.
    ///
    /// Program name is auto-detected from `std::env::args()`.
    pub fn new() -> Self {
        Self {
            program_name: None,
            options: Vec::new(),
        }
    }

    /// Set the program name (first argument)
    pub fn program_name(mut self, name: impl Into<String>) -> Self {
        self.program_name = Some(name.into());
        self
    }

    /// Add --no-huge option (don't use hugepages)
    pub fn no_huge(mut self) -> Self {
        self.options.push(EalOption::NoHuge);
        self
    }

    /// Add --no-pci option (don't scan PCI bus)
    pub fn no_pci(mut self) -> Self {
        self.options.push(EalOption::NoPci);
        self
    }

    /// Add a virtual device (--vdev=<device>)
    pub fn vdev(mut self, device: impl Into<String>) -> Self {
        self.options.push(EalOption::Vdev(device.into()));
        self
    }

    /// Set core mask in hex (-c <mask>)
    pub fn core_mask(mut self, mask: impl Into<String>) -> Self {
        self.options.push(EalOption::CoreMask(mask.into()));
        self
    }

    /// Set core list (-l <list>)
    pub fn core_list(mut self, list: impl Into<String>) -> Self {
        self.options.push(EalOption::CoreList(list.into()));
        self
    }

    /// Set number of memory channels (-n <num>)
    pub fn memory_channels(mut self, n: u32) -> Self {
        self.options.push(EalOption::MemoryChannels(n));
        self
    }

    /// Set process type for multi-process (--proc-type=<type>)
    pub fn process_type(mut self, pt: ProcessType) -> Self {
        self.options.push(EalOption::ProcessType(pt));
        self
    }

    /// Set file prefix for multi-process (--file-prefix=<prefix>)
    pub fn file_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.options.push(EalOption::FilePrefix(prefix.into()));
        self
    }

    /// Set memory per socket in MB (--socket-mem=<amounts>)
    pub fn socket_mem(mut self, mem: impl Into<String>) -> Self {
        self.options.push(EalOption::SocketMem(mem.into()));
        self
    }

    /// Set log level (--log-level=<level>)
    pub fn log_level(mut self, level: LogLevel) -> Self {
        self.options.push(EalOption::LogLevel(level));
        self
    }

    /// Enable in-memory mode (--in-memory)
    pub fn in_memory(mut self) -> Self {
        self.options.push(EalOption::InMemory);
        self
    }

    /// Allow a PCI device (-a <pci_addr>)
    pub fn allow(mut self, pci_addr: impl Into<String>) -> Self {
        self.options.push(EalOption::Allow(pci_addr.into()));
        self
    }

    /// Add a custom option
    pub fn option(mut self, opt: EalOption) -> Self {
        self.options.push(opt);
        self
    }

    /// Add a custom raw argument
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.options.push(EalOption::Custom(arg.into()));
        self
    }

    /// Build the argument list
    fn build_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        // Program name first - use provided, or auto-detect from env
        let program_name = self.program_name.clone().unwrap_or_else(|| {
            std::env::args()
                .next()
                .unwrap_or_else(|| "dpdk-app".to_string())
        });
        args.push(program_name);

        // Add all options
        for opt in &self.options {
            args.extend(opt.to_args());
        }

        args
    }

    /// Initialize EAL with the configured options.
    ///
    /// Returns an RAII guard that cleans up EAL on drop.
    pub fn init(self) -> crate::api::Result<Eal> {
        let args = self.build_args();
        tracing::info!(args = ?args, "Initializing EAL");
        Eal::init(args)
    }
}

/// RAII guard for the EAL environment.
///
/// When dropped, automatically calls `rte_eal_cleanup()`.
/// Note: EAL cannot be reinitialized after cleanup within the same process.
///
/// # Example
/// ```no_run
/// use dpdk_net::api::rte::eal::{Eal, EalBuilder};
///
/// fn main() -> Result<(), nix::errno::Errno> {
///     // Using builder (recommended)
///     let _eal = EalBuilder::new()
///         .no_huge()
///         .no_pci()
///         .vdev("net_ring0")
///         .init()?;
///
///     // Or using init directly
///     // let _eal = Eal::init(["prog", "--no-huge", "--no-pci"])?;
///     Ok(())
/// }
/// ```
pub struct Eal {
    // Unit type () is Send + Sync, so Eal is too.
    // EAL is global state and DPDK functions are internally thread-safe.
    _marker: PhantomData<()>,
}

impl Eal {
    /// Initialize the EAL environment and return an RAII guard.
    ///
    /// The first argument should be the program name (can be anything).
    /// Accepts any iterator of items that can be converted to CString.
    ///
    /// # Examples
    /// ```no_run
    /// use dpdk_net::api::rte::eal::Eal;
    ///
    /// fn main() -> Result<(), nix::errno::Errno> {
    ///     let _eal = Eal::init(["prog", "--no-huge", "--no-pci", "--vdev=net_ring0"])?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    /// Returns an error if EAL initialization fails or if EAL is already initialized.
    pub fn init<I, S>(args: I) -> crate::api::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        // Check if already initialized
        if EAL_INITIALIZED.swap(true, Ordering::SeqCst) {
            return Err(nix::errno::Errno::EALREADY);
        }

        // Convert args to CStrings
        let args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.as_ref()).expect("argument contains null byte"))
            .collect();

        let argc = args.len() as i32;
        let mut argv: Vec<*mut i8> = args.iter().map(|s| s.as_ptr() as *mut i8).collect();
        argv.push(std::ptr::null_mut());

        let ret = unsafe { dpdk_net_sys::ffi::rte_eal_init(argc, argv.as_mut_ptr()) };

        if ret < 0 {
            // Reset flag on failure so user can retry
            EAL_INITIALIZED.store(false, Ordering::SeqCst);
            return Err(crate::api::rte_errno());
        }

        Ok(Eal {
            _marker: PhantomData,
        })
    }

    /// Check if EAL has been initialized.
    pub fn is_initialized() -> bool {
        EAL_INITIALIZED.load(Ordering::SeqCst)
    }
}

impl Drop for Eal {
    fn drop(&mut self) {
        // Best effort cleanup - ignore errors during drop
        let _ = unsafe { dpdk_net_sys::ffi::rte_eal_cleanup() };
        EAL_INITIALIZED.store(false, Ordering::SeqCst);
    }
}

/// Initialize the EAL environment (low-level, no RAII).
///
/// Prefer using `Eal::init()` which provides automatic cleanup.
///
/// # Deprecated
/// Use `Eal::init()` instead for automatic cleanup.
pub fn init<I, S>(args: I) -> crate::api::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<CString> = args
        .into_iter()
        .map(|s| CString::new(s.as_ref()).expect("argument contains null byte"))
        .collect();
    let argc = args.len() as i32;
    let mut argv: Vec<*mut i8> = args.iter().map(|s| s.as_ptr() as *mut i8).collect();
    argv.push(std::ptr::null_mut());
    let ret = unsafe { dpdk_net_sys::ffi::rte_eal_init(argc, argv.as_mut_ptr()) };
    check_rte_success(ret)
}

/// Cleans up the EAL environment (low-level).
///
/// Prefer using `Eal::init()` which provides automatic cleanup via Drop.
/// Cannot reinitialize EAL after this call.
pub fn cleanup() -> crate::api::Result<()> {
    let ret = unsafe { dpdk_net_sys::ffi::rte_eal_cleanup() };
    check_rte_success(ret)
}
