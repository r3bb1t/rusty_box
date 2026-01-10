//! BIOS Boot Example
//! 
//! This example demonstrates the instance-based initialization of the emulator.
//! Each component is created and owned independently, allowing multiple
//! emulator instances to run concurrently.

use rusty_box::{
    cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX, ResetReason},
    iodev::BxDevicesC,
    memory::{BxMemC, BxMemoryStubC},
    pc_system::BxPcSystemC,
    Result,
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
        .spawn(|| {
            if let Err(e) = inner_main() {
                eprintln!("Error: {:?}", e);
                std::process::exit(1);
            }
        })
        .expect("thread spawn error")
        .join()
        .expect("error while joining spawned thread");
}

fn inner_main() -> Result<()> {
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .init();

    // === Step 1: Create PC System (instance-based, no globals!) ===
    let mut pc_system = BxPcSystemC::new();
    
    // Initialize PC system with instructions-per-second
    const IPS: u32 = 4_000_000; // 4 MIPS
    pc_system.initialize(IPS);

    // === Step 2: Initialize Memory ===
    let guest_mb = 32;
    let host_mb = 32;
    let block_size = 128 * 1024;
    let mem_stub = BxMemoryStubC::create_and_init(
        guest_mb * 1024 * 1024,
        host_mb * 1024 * 1024,
        block_size,
    )?;
    let mut mem = BxMemC::new(mem_stub, false);
    mem.init_memory(guest_mb * 1024 * 1024, host_mb * 1024 * 1024, block_size)?;
    
    // Sync A20 mask from PC system to memory
    mem.set_a20_mask(pc_system.a20_mask());

    // === Step 3: Load BIOS ROM ===
    let bios_paths = [
        "../cpp_orig/bochs/bios/BIOS-bochs-latest",
        "../binaries/bios/BIOS-bochs-latest",
    ];
    let (_path, bios_data) = bios_paths
        .iter()
        .find_map(|p| std::fs::read(p).ok().map(|d| (*p, d)))
        .ok_or_else(|| {
            rusty_box::Error::Memory(rusty_box::memory::MemoryError::RomTooLarge(0))
        })?;
    mem.load_ROM(&bios_data, 0xfffe0000, 0)?;
    tracing::info!("Loaded BIOS from {}", _path);

    // === Step 4: Initialize CPU ===
    let builder: BxCpuBuilder<Corei7SkylakeX> = BxCpuBuilder::new();
    let mut cpu = builder.build()?;
    let config = rusty_box::params::BxParams::default();
    cpu.initialize(config)?;

    // === Step 5: Initialize Devices ===
    let mut devices = BxDevicesC::new();
    devices.init(&mut mem)?;
    devices.register_state()?;

    // === Step 6: Register state for save/restore ===
    pc_system.register_state();

    // === Step 7: Hardware Reset ===
    // This is the full initialization sequence from main.cc:1363
    pc_system.reset(ResetReason::Hardware)?;
    
    // Sync A20 mask after reset (A20 is enabled on reset)
    mem.set_a20_mask(pc_system.a20_mask());
    
    // Reset CPU
    cpu.reset(ResetReason::Hardware);
    
    // Reset devices
    devices.reset(ResetReason::Hardware)?;

    // === Step 8: Start timers ===
    pc_system.start_timers();

    tracing::info!("Hardware initialization complete");
    tracing::info!("Initial RIP after reset: {:#x}", cpu.rip());

    // === Step 9: Run CPU loop ===
    // Add timeout for safety during development
    std::thread::spawn({
        let cpu_rip = cpu.rip();
        move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            eprintln!("TIMEOUT: RIP={:#x}", cpu_rip);
            std::process::exit(1);
        }
    });

    let _ = cpu.cpu_loop(&mut mem, &[]);
    Ok(())
}
