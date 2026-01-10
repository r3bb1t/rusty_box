//! Hardware Initialization Example
//!
//! This example demonstrates the complete hardware initialization sequence from
//! Bochs `main.cc:1300-1363`, using the `Emulator` struct which provides a clean
//! API for coordinating all components.
//!
//! ## Initialization Flow (mirrors Bochs)
//!
//! 1. `bx_pc_system.initialize(ips)` - Timer and IPS setup
//! 2. Memory initialization + ROM loading
//! 3. CPU initialization
//! 4. `DEV_init_devices()` - Device initialization with I/O handlers
//! 5. `register_state()` - State registration for save/restore
//! 6. `bx_pc_system.Reset(HARDWARE)` - Full hardware reset
//! 7. `start_timers()` - Activate timing system
//! 8. `cpu_loop()` - Begin execution
//!
//! ## Key Components
//!
//! - **PC System**: Timer management, A20 line control, system reset coordination
//! - **Memory**: Guest RAM, ROM loading, A20 masking
//! - **CPU**: x86-64 processor emulation  
//! - **Devices**: I/O port handlers including Port 0x92 (A20/reset control)

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    Result,
};
use tracing::Level;
use std::time::Instant;

fn main() {
    // Use a larger stack size for debug builds
    const THREAD_STACK_SIZE: usize = if cfg!(debug_assertions) {
        1500 * 1024 * 1024
    } else {
        500 * 1024 * 1024
    };

    std::thread::Builder::new()
        .stack_size(THREAD_STACK_SIZE)
        .name("Emulator".to_string())
        .spawn(|| {
            if let Err(e) = run_emulator() {
                eprintln!("Emulator error: {:?}", e);
                std::process::exit(1);
            }
        })
        .expect("Failed to spawn emulator thread")
        .join()
        .expect("Emulator thread panicked");
}

fn run_emulator() -> Result<()> {
    // Initialize tracing for output (INFO level to reduce noise)
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_max_level(Level::INFO)
        .init();

    tracing::info!("=== Hardware Initialization Example ===");
    tracing::info!("Demonstrating Bochs main.cc:1300-1363 sequence");

    // =========================================================================
    // Step 1: Configure and Create Emulator
    // =========================================================================
    // The EmulatorConfig specifies system parameters
    let config = EmulatorConfig {
        guest_memory_size: 32 * 1024 * 1024,  // 32 MB guest memory
        host_memory_size: 32 * 1024 * 1024,   // 32 MB host backing
        memory_block_size: 128 * 1024,         // 128 KB allocation blocks
        ips: 4_000_000,                        // 4 MIPS
        pci_enabled: false,
        ..Default::default()
    };

    tracing::info!("Creating emulator with {} MB memory, {} IPS", 
        config.guest_memory_size / (1024 * 1024),
        config.ips
    );

    // Create the emulator (allocates CPU, Memory, Devices, PC System)
    let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;
    tracing::info!("✓ Emulator created");

    // =========================================================================
    // Step 2: Initialize (calls internal sequence)
    // =========================================================================
    // This performs:
    //   - pc_system.initialize(ips)
    //   - cpu.initialize()
    //   - devices.init()
    //   - register_state() for all components
    emu.initialize()?;
    tracing::info!("✓ Hardware initialized");

    // =========================================================================
    // Step 3: Load BIOS ROM
    // =========================================================================
    let bios_paths = [
        "../cpp_orig/bochs/bios/BIOS-bochs-latest",
        "../binaries/bios/BIOS-bochs-latest",
    ];

    let bios_data = bios_paths
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .expect("Could not find BIOS file");

    // Load BIOS at standard address (128KB BIOS at 0xFFFE0000)
    emu.load_bios(&bios_data, 0xfffe0000)?;
    tracing::info!("✓ Loaded BIOS ({} bytes)", bios_data.len());

    // =========================================================================
    // Step 4: Hardware Reset
    // =========================================================================
    // This performs the full reset sequence:
    //   - pc_system.reset() - enables A20
    //   - Sync A20 mask to memory
    //   - cpu.reset()
    //   - devices.reset() (hardware reset only)
    emu.reset(ResetReason::Hardware)?;
    tracing::info!("✓ Hardware reset complete");

    // =========================================================================
    // Step 5: Show System State and BIOS Reset Vector
    // =========================================================================
    tracing::info!("--- System State After Reset ---");
    tracing::info!("  CS:IP = F000:{:04X} (linear: {:#x})", emu.rip(), 0xFFFF0000u64 + emu.rip());
    tracing::info!("  A20: {}", if emu.pc_system.get_enable_a20() { "enabled" } else { "disabled" });
    
    // Display the bytes at the BIOS reset vector (0xFFFFFFF0)
    // The BIOS is loaded at 0xFFFE0000, so offset 0x1FFF0 = reset vector
    let reset_vector_offset = 0x1FFF0usize; // Offset from BIOS start to 0xFFFFFFF0
    tracing::info!("--- BIOS Reset Vector (F000:FFF0 = FFFFFFF0) ---");
    tracing::info!("  First 16 bytes at reset vector:");
    let first_bytes = &bios_data[reset_vector_offset..reset_vector_offset + 16];
    tracing::info!("  {:02X?}", first_bytes);
    
    // Disassemble the first instruction (typically a far jump)
    // Common pattern: EA xx xx xx xx = JMP FAR segment:offset
    if first_bytes[0] == 0xEA {
        let offset = u16::from_le_bytes([first_bytes[1], first_bytes[2]]);
        let segment = u16::from_le_bytes([first_bytes[3], first_bytes[4]]);
        tracing::info!("  First instruction: JMP FAR {:04X}:{:04X}", segment, offset);
        let linear = ((segment as u32) << 4) + (offset as u32);
        tracing::info!("  Jump target linear address: {:#x}", linear);
    }
    
    // =========================================================================
    // Step 6: Demonstrate I/O Port Access (Port 0x92)
    // =========================================================================
    tracing::info!("--- Port 0x92 (System Control) Demo ---");
    
    // Read Port 92h
    let port92_value = emu.read_port_92h();
    tracing::info!("  Initial Port 92h: {:#04x} (A20={})", 
        port92_value, 
        if port92_value & 0x01 != 0 { "on" } else { "off" }
    );

    // Disable A20 via Port 92h
    let reset_requested = emu.write_port_92h(0x00);
    tracing::info!("  Wrote 0x00 to Port 92h (disable A20)");
    tracing::info!("  A20 now: {}", if emu.pc_system.get_enable_a20() { "enabled" } else { "disabled" });
    tracing::info!("  Reset requested: {}", reset_requested);

    // Re-enable A20
    let _reset_requested = emu.write_port_92h(0x01);
    tracing::info!("  Wrote 0x01 to Port 92h (enable A20)");
    tracing::info!("  A20 now: {}", if emu.pc_system.get_enable_a20() { "enabled" } else { "disabled" });

    // =========================================================================
    // Step 7: Start Timers and Prepare for Execution
    // =========================================================================
    emu.prepare_run();
    tracing::info!("✓ Timers started, ready to run");

    // =========================================================================
    // Step 8: Run CPU Loop with BIOS Execution
    // =========================================================================
    tracing::info!("--- Starting BIOS Execution ---");
    tracing::info!("  Initial CS:IP = F000:{:04X}", emu.rip());
    
    let start_time = Instant::now();
    
    // Run BIOS execution with a large instruction limit (100M instructions)
    const MAX_INSTRUCTIONS: u64 = 100_000_000;
    
    let result = emu.cpu.cpu_loop_n(&mut emu.memory, &[], MAX_INSTRUCTIONS);
    
    let elapsed = start_time.elapsed();
    
    match result {
        Ok(executed) => {
            tracing::info!("--- Execution Complete ---");
            tracing::info!("  Instructions executed: {}", executed);
            tracing::info!("  Execution time: {:?}", elapsed);
            if elapsed.as_secs_f64() > 0.0 {
                tracing::info!("  Average speed: {:.2} MIPS", 
                    executed as f64 / elapsed.as_secs_f64() / 1_000_000.0);
            }
        }
        Err(e) => {
            tracing::warn!("CPU loop ended with error: {:?}", e);
        }
    }

    // =========================================================================
    // Step 9: Show Final CPU State
    // =========================================================================
    tracing::info!("--- Final CPU State ---");
    tracing::info!("  RIP: {:#x}", emu.cpu.rip());
    tracing::info!("  Ticks: {}", emu.pc_system.time_ticks());
    
    // Show general purpose registers
    tracing::info!("  EAX: {:#010x}  EBX: {:#010x}  ECX: {:#010x}  EDX: {:#010x}",
        emu.cpu.eax(), emu.cpu.ebx(), emu.cpu.ecx(), emu.cpu.edx());
    tracing::info!("  ESP: {:#010x}  EBP: {:#010x}  ESI: {:#010x}  EDI: {:#010x}",
        emu.cpu.esp(), emu.cpu.ebp(), emu.cpu.esi(), emu.cpu.edi());

    tracing::info!("=== Hardware Initialization Example Complete ===");
    Ok(())
}

