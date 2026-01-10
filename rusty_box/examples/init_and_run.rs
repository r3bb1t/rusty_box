//! Basic initialization and run example
//! 
//! Demonstrates the minimal initialization sequence for the emulator.

use rusty_box::{
    cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX, ResetReason},
    iodev::BxDevicesC,
    memory::{BxMemC, BxMemoryStubC},
    pc_system::BxPcSystemC,
};
use tracing::Level;

fn main() {
    const THREAD_STACK_SIZE: usize = if cfg!(debug_assertions) {
        1500 * 1024 * 1024
    } else {
        500 * 1024 * 1024
    };
    std::thread::Builder::new()
        .stack_size(THREAD_STACK_SIZE)
        .name("Emulator thread".to_string())
        .spawn(inner_main)
        .expect("thread spawn error")
        .join()
        .expect("error while joining spawned thread");
}

fn inner_main() {
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .init();

    // Create PC system instance (no global state!)
    let mut pc_system = BxPcSystemC::new();
    pc_system.initialize(4_000_000); // 4 MIPS

    // Create CPU
    let builder: BxCpuBuilder<Corei7SkylakeX> = BxCpuBuilder::new();
    tracing::info!("Builder: {builder:#?}");
    let mut cpu = builder.build().expect("An error occurred while building cpu");
    tracing::info!("Created cpu");

    tracing::info!(
        "Before setting registers: Rax: {} , Rip: {}",
        cpu.rax(),
        cpu.rip()
    );

    cpu.set_rax(777);
    cpu.set_rip(0);

    tracing::info!("After: Rax: {} , Rip: {}", cpu.rax(), cpu.rip());

    // Create memory
    let guest_mb = 32;
    let host_mb = 32;
    let mem_stub = BxMemoryStubC::create_and_init(
        guest_mb * 1024 * 1024,
        host_mb * 1024 * 1024,
        128 * 1024,
    )
    .unwrap();
    let mut mem = BxMemC::new(mem_stub, false);
    
    // Sync A20 mask
    mem.set_a20_mask(pc_system.a20_mask());

    // Create devices
    let mut devices = BxDevicesC::new();
    let _ = devices.init(&mut mem);

    // Reset sequence
    pc_system.reset(ResetReason::Hardware).unwrap();
    mem.set_a20_mask(pc_system.a20_mask());
    cpu.reset(ResetReason::Hardware);
    devices.reset(ResetReason::Hardware).unwrap();

    tracing::info!("Initial rip after reset: {:x}", cpu.rip());

    // Set a max iteration limit for now to prevent infinite loops
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        eprintln!("TIMEOUT: cpu_loop exceeded 2 seconds, likely stuck");
        std::process::exit(1);
    });

    match cpu.cpu_loop(&mut mem, &[]) {
        Ok(_) => {
            tracing::info!("CPU loop completed successfully");
        }
        Err(e) => {
            tracing::error!("CPU loop encountered error: {:?}", e);
        }
    }
}
