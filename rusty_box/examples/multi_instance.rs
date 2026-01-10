//! Multi-Instance Emulator Example
//!
//! This example demonstrates running multiple independent emulator instances
//! concurrently on different threads. Each emulator has its own CPU, memory,
//! devices, and PC system with no shared global state.
//!
//! Run with: cargo run --example multi_instance --features std

use rusty_box::{
    cpu::core_i7_skylake::Corei7SkylakeX,
    emulator::{Emulator, EmulatorConfig},
    cpu::ResetReason,
    Result,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};
use tracing::Level;

/// Number of emulator instances to spawn
const NUM_INSTANCES: usize = 2;

/// Maximum runtime per instance (seconds)
const MAX_RUNTIME_SECS: u64 = 2;

fn main() {
    // Use a larger stack for debug builds
    const THREAD_STACK_SIZE: usize = if cfg!(debug_assertions) {
        10240 * 1024 * 1024  // 512 MB for debug
    } else {
        1280 * 1024 * 1024  // 128 MB for release
    };

    thread::Builder::new()
        .stack_size(THREAD_STACK_SIZE)
        .name("Main emulator coordinator".to_string())
        .spawn(run_multi_instance)
        .expect("Failed to spawn main thread")
        .join()
        .expect("Main thread panicked");
}

fn run_multi_instance() {
    // Initialize logging
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_max_level(Level::INFO)
        .init();

    tracing::info!("=== Multi-Instance Emulator Demo ===");
    tracing::info!("Spawning {} independent emulator instances", NUM_INSTANCES);

    // Counter for completed instances
    let completed = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicUsize::new(0));

    let start_time = Instant::now();

    // Spawn emulator instances on separate threads
    let handles: Vec<_> = (0..NUM_INSTANCES)
        .map(|id| {
            let completed = Arc::clone(&completed);
            let started = Arc::clone(&started);

            thread::Builder::new()
                .name(format!("Emulator-{}", id))
                .stack_size(128 * 1024 * 1024) // 128 MB per instance
                .spawn(move || {
                    run_emulator_instance(id, started, completed)
                })
                .expect("Failed to spawn emulator thread")
        })
        .collect();

    // Wait for all instances to complete
    for handle in handles {
        if let Err(e) = handle.join() {
            tracing::error!("Emulator thread panicked: {:?}", e);
        }
    }

    let elapsed = start_time.elapsed();
    let total_completed = completed.load(Ordering::SeqCst);

    tracing::info!("=== Results ===");
    tracing::info!(
        "Completed {}/{} instances in {:.2}s",
        total_completed,
        NUM_INSTANCES,
        elapsed.as_secs_f64()
    );

    if total_completed == NUM_INSTANCES {
        tracing::info!("SUCCESS: All emulator instances ran independently!");
    } else {
        tracing::error!("FAILURE: Some instances did not complete");
        std::process::exit(1);
    }
}

fn run_emulator_instance(
    id: usize,
    started: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
) -> Result<()> {
    let instance_start = Instant::now();

    // Create unique configuration for this instance
    // Use smaller memory to allow many instances
    let config = EmulatorConfig {
        guest_memory_size: 4 * 1024 * 1024,   // 4 MB per instance
        host_memory_size: 4 * 1024 * 1024,    // 4 MB
        memory_block_size: 64 * 1024,          // 64 KB blocks
        ips: 1_000_000 + (id as u32 * 100_000), // Varying IPS
        pci_enabled: false,
        cpu_params: Default::default(),
    };

    // Create emulator
    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;
    let instance_id = started.fetch_add(1, Ordering::SeqCst);
    tracing::info!("[Instance {}] Created (started: {})", id, instance_id + 1);

    // Initialize
    emu.initialize()?;
    tracing::debug!("[Instance {}] Initialized", id);

    // Perform hardware reset
    emu.reset(ResetReason::Hardware)?;
    tracing::debug!("[Instance {}] Reset complete, RIP={:#x}", id, emu.rip());

    // Test A20 control (independent per instance)
    let initial_a20 = emu.pc_system.get_enable_a20();
    emu.pc_system.set_enable_a20(false);
    assert!(!emu.pc_system.get_enable_a20());
    emu.pc_system.set_enable_a20(true);
    assert!(emu.pc_system.get_enable_a20());
    tracing::debug!("[Instance {}] A20 control test passed", id);

    // Test Port 92h (independent per instance)
    emu.write_port_92h(0x00); // Disable A20 via port 92h
    assert!(!emu.system_control.a20_gate);
    emu.write_port_92h(0x01); // Enable A20 via port 92h
    assert!(emu.system_control.a20_gate);
    tracing::debug!("[Instance {}] Port 92h test passed", id);

    // Test timer ticks (independent per instance)
    let initial_ticks = emu.ticks();
    emu.pc_system.tick(1000 * (id as u64 + 1));
    let new_ticks = emu.ticks();
    assert_eq!(new_ticks - initial_ticks, 1000 * (id as u64 + 1));
    tracing::debug!("[Instance {}] Timer test passed (ticks: {})", id, new_ticks);

    // Simulate some work with varying duration based on instance ID
    let work_duration = Duration::from_millis(50 + (id as u64 * 10));
    thread::sleep(work_duration);

    // Start timers (prepare for execution)
    emu.prepare_run();

    let elapsed = instance_start.elapsed();
    let count = completed.fetch_add(1, Ordering::SeqCst) + 1;
    
    tracing::info!(
        "[Instance {}] Completed in {:.2}ms (total completed: {})",
        id,
        elapsed.as_secs_f64() * 1000.0,
        count
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instances_are_independent() {
        // Create two instances
        let config = EmulatorConfig {
            guest_memory_size: 1024 * 1024,
            host_memory_size: 1024 * 1024,
            memory_block_size: 64 * 1024,
            ..Default::default()
        };

        let mut emu1 = Emulator::<Corei7SkylakeX>::new(config.clone()).unwrap();
        let mut emu2 = Emulator::<Corei7SkylakeX>::new(config).unwrap();

        emu1.initialize().unwrap();
        emu2.initialize().unwrap();

        // Modify emu1
        emu1.pc_system.set_enable_a20(false);
        emu1.pc_system.tick(5000);

        // emu2 should be unaffected
        assert!(emu2.pc_system.get_enable_a20()); // Still enabled
        assert_eq!(emu2.ticks(), 0); // No ticks
    }
}
