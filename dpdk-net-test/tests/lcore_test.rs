//! Lcore API Tests
//!
//! Tests for the DPDK lcore (logical core) APIs.
//! Uses EAL with multiple lcores to test launch/wait functionality.
//!
//! Note: EAL is initialized once globally since it can only be initialized once per process.

use dpdk_net::api::rte::eal::{Eal, EalBuilder};
use dpdk_net::api::rte::lcore::{LaunchBuilder, Lcore, Role, State};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

/// Global EAL context - initialized once for all tests
static GLOBAL_EAL: OnceLock<Eal> = OnceLock::new();

/// Initialize EAL once for all tests
fn init_eal() -> &'static Eal {
    GLOBAL_EAL.get_or_init(|| {
        EalBuilder::new()
            .no_huge()
            .no_pci()
            .core_list("0-3") // 4 lcores: 1 main + 3 workers
            .init()
            .expect("Failed to initialize EAL")
    })
}

/// Test: Main lcore properties
#[test]
#[serial_test::serial]
fn test_main_lcore() {
    let _eal = init_eal();

    // Get main lcore
    let main = Lcore::main();
    println!("Main lcore ID: {}", main.id());

    // Main lcore should have these properties
    assert!(main.is_main());
    assert_eq!(main.role(), Role::Rte);

    // Note: Lcore::current() may return None here because Rust's test harness
    // may run this test on a different thread than the one that initialized EAL.
    // The EAL-initializing thread is the main lcore, not necessarily this thread.
    if let Some(current) = Lcore::current() {
        println!("Current thread is lcore {}", current.id());
    } else {
        println!("Current thread is not an EAL lcore (test harness thread)");
    }

    println!("Main lcore socket: {}", main.socket_id());
    if let Some(cpu) = main.cpu_id() {
        println!("Main lcore CPU: {}", cpu);
    }
}

/// Test: Lcore count and iteration
#[test]
#[serial_test::serial]
fn test_lcore_iteration() {
    let _eal = init_eal();

    let count = Lcore::count();
    println!("Total lcores: {}", count);
    assert!(count >= 2, "Expected at least 2 lcores");

    // Iterate all lcores
    let all: Vec<_> = Lcore::all().collect();
    assert_eq!(all.len() as u32, count);
    println!(
        "All lcores: {:?}",
        all.iter().map(|l| l.id()).collect::<Vec<_>>()
    );

    // Check that main is in the list
    let main = Lcore::main();
    assert!(all.iter().any(|l| l.id() == main.id()));

    // Iterate workers only
    let workers: Vec<_> = Lcore::workers().collect();
    println!(
        "Worker lcores: {:?}",
        workers.iter().map(|l| l.id()).collect::<Vec<_>>()
    );
    assert_eq!(workers.len() as u32, count - 1);

    // Workers should not include main
    assert!(!workers.iter().any(|l| l.id() == main.id()));

    // All workers should not be main
    for worker in &workers {
        assert!(!worker.is_main());
    }
}

/// Test: Launch closure on worker lcore
#[test]
#[serial_test::serial]
fn test_lcore_launch() {
    let _eal = init_eal();

    let worker = Lcore::workers().next().expect("Need at least one worker");
    println!("Launching on worker lcore {}", worker.id());

    // Worker should be available initially
    assert!(worker.is_available());
    assert_eq!(worker.state(), State::Wait);

    // Counter to verify execution
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    // Launch a closure
    worker
        .launch(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            42 // Return value
        })
        .expect("Launch failed");

    // Wait for completion
    let result = worker.wait();
    assert_eq!(result, 42);

    // Verify closure executed
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Worker should be available again
    assert!(worker.is_available());
}

/// Test: Launch and run convenience method
#[test]
#[serial_test::serial]
fn test_lcore_run() {
    let _eal = init_eal();

    let worker = Lcore::workers().next().expect("Need at least one worker");

    // run() combines launch() and wait()
    let result = worker
        .run(|| {
            // Verify we're running on the worker
            let current = Lcore::current().expect("Should be on an lcore");
            assert!(!current.is_main());
            123
        })
        .expect("Run failed");

    assert_eq!(result, 123);
}

/// Test: Launch on all workers
#[test]
#[serial_test::serial]
fn test_launch_on_workers() {
    let _eal = init_eal();

    let worker_count = Lcore::workers().count();
    println!("Worker count: {}", worker_count);

    let counter = Arc::new(AtomicU32::new(0));

    // Launch on all workers
    Lcore::launch_on_workers({
        let counter = counter.clone();
        move |lcore| {
            println!("Worker {} executing", lcore.id());
            counter.fetch_add(1, Ordering::SeqCst);
            0
        }
    })
    .expect("Launch on workers failed");

    // Wait for all
    Lcore::wait_all_workers();

    // All workers should have executed
    assert_eq!(counter.load(Ordering::SeqCst), worker_count as u32);
}

/// Test: LaunchBuilder with socket filtering
#[test]
#[serial_test::serial]
fn test_launch_builder() {
    let _eal = init_eal();

    // Use builder to launch on workers
    let results = LaunchBuilder::workers()
        .run(|lcore| {
            println!("Worker {} via builder", lcore.id());
            lcore.id() as i32
        })
        .expect("Builder run failed");

    println!("Results: {:?}", results);

    // Each result should match the lcore ID
    for (lcore, result) in results {
        assert_eq!(result, lcore.id() as i32);
    }
}

/// Test: LaunchHandle for non-blocking launch
#[test]
#[serial_test::serial]
fn test_launch_handle() {
    let _eal = init_eal();

    // Launch and get handle
    let handle = LaunchBuilder::workers()
        .launch(|lcore| {
            // Simulate some work
            std::thread::sleep(std::time::Duration::from_millis(10));
            lcore.id() as i32 * 2
        })
        .expect("Launch failed");

    // Can check status with is_done()
    println!("Launched, waiting...");

    // Wait and collect results
    let results = handle.wait();

    for (lcore, result) in &results {
        println!("Lcore {} returned {}", lcore.id(), result);
        assert_eq!(*result, lcore.id() as i32 * 2);
    }
}

/// Test: Lcore from_id validation
#[test]
#[serial_test::serial]
fn test_lcore_from_id() {
    let _eal = init_eal();

    // Valid lcore (main/0)
    let lcore = Lcore::from_id(0);
    assert!(lcore.is_some());

    // Invalid lcore (very high ID)
    let invalid = Lcore::from_id(999);
    assert!(invalid.is_none());
}

/// Test: Multiple sequential launches on same lcore
#[test]
#[serial_test::serial]
fn test_sequential_launches() {
    let _eal = init_eal();

    let worker = Lcore::workers().next().expect("Need a worker");

    // Launch multiple times sequentially
    for i in 0..5 {
        let result = worker.run(move || i * 10).expect("Run failed");
        assert_eq!(result, i * 10);
    }

    println!("5 sequential launches completed successfully");
}
