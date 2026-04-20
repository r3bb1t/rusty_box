//! Rusty Box UEFI — boots DLX Linux via full BIOS POST.
//!
//! Everything embedded at compile time: BIOS, VGA BIOS, DLX disk image.
//! Normal BIOS boot path — identical to the desktop dlxlinux example.
//! No Rust allocator required — all large structs placed via UEFI page allocation.

#![no_main]
#![no_std]

use log::{error, info};
use uefi::prelude::*;

use rusty_box::{
    cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX, cpu::BxCpuC, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    memory::BxMemoryStubC,
};

static BIOS_ROM: &[u8] = include_bytes!("../../../cpp_orig/bochs/bios/BIOS-bochs-latest");
static VGA_BIOS: &[u8] = include_bytes!("../../../binaries/bios/VGABIOS-lgpl-latest.bin");
static DLX_DISK: &[u8] = include_bytes!("../../../dlxlinux/hd10meg.img");

const DLX_CYLINDERS: u16 = 306;
const DLX_HEADS: u8 = 4;
const DLX_SPT: u8 = 17;

fn print_bytes(bytes: &[u8]) {
    let mut buf = [0u16; 128];
    let mut pos = 0;
    for &b in bytes {
        let ch = match b {
            b'\n' => { buf[pos] = b'\r' as u16; pos += 1; b'\n' as u16 }
            b'\r' => continue,
            0x20..=0x7E => b as u16,
            _ => continue,
        };
        buf[pos] = ch;
        pos += 1;
        if pos >= buf.len() - 2 {
            buf[pos] = 0;
            let s = unsafe { uefi::CStr16::from_u16_with_nul_unchecked(&buf[..=pos]) };
            let _ = uefi::system::with_stdout(|out| { let _ = out.output_string(s); });
            pos = 0;
        }
    }
    if pos > 0 {
        buf[pos] = 0;
        let s = unsafe { uefi::CStr16::from_u16_with_nul_unchecked(&buf[..=pos]) };
        let _ = uefi::system::with_stdout(|out| { let _ = out.output_string(s); });
    }
}

/// Drain an iterator of bytes and print them. Avoids Vec allocation.
fn drain_and_print(iter: impl Iterator<Item = u8>) {
    let mut tmp = [0u8; 256];
    let mut pos = 0;
    for b in iter {
        tmp[pos] = b;
        pos += 1;
        if pos == tmp.len() {
            print_bytes(&tmp[..pos]);
            pos = 0;
        }
    }
    if pos > 0 {
        print_bytes(&tmp[..pos]);
    }
}

macro_rules! bail {
    ($($arg:tt)*) => {{ error!($($arg)*); uefi::boot::stall(10_000_000); return Status::ABORTED; }};
}

/// Allocate `count` zeroed pages via UEFI boot services.
fn alloc_pages(count: usize) -> *mut u8 {
    uefi::boot::allocate_pages(
        uefi::boot::AllocateType::AnyPages,
        uefi::boot::MemoryType::LOADER_DATA,
        count,
    )
    .map_or(core::ptr::null_mut(), |p| p.as_ptr())
}

/// Allocate zeroed memory for a type via UEFI pages.
fn alloc_zeroed_for<T>() -> *mut T {
    let size = core::mem::size_of::<T>();
    let pages = (size + 4095) / 4096;
    let ptr = alloc_pages(pages);
    if ptr.is_null() {
        panic!("UEFI page allocation failed for {} bytes", size);
    }
    // allocate_pages returns zeroed memory (LOADER_DATA from firmware)
    // but let's be safe:
    unsafe { core::ptr::write_bytes(ptr, 0, size); }
    ptr as *mut T
}

const STACK_SIZE: usize = 1024 * 1024; // 1 MB stack

#[entry]
fn main() -> Status {
    // UEFI firmware provides a small stack (often 128KB).
    // Allocate a 1MB stack on the heap and switch to it.
    let stack_pages = (STACK_SIZE + 4095) / 4096;
    let stack_base = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::AnyPages,
        uefi::boot::MemoryType::LOADER_DATA,
        stack_pages,
    ).expect("failed to allocate stack");
    let new_sp = stack_base.as_ptr() as usize + STACK_SIZE;
    let result: usize;
    unsafe {
        core::arch::asm!(
            "mov {old_sp}, rsp",
            "mov rsp, {new_sp}",
            "sub rsp, 32",
            "call {func}",
            "mov rsp, {old_sp}",
            func = sym run,
            new_sp = in(reg) new_sp,
            old_sp = out(reg) _,
            lateout("rax") result,
            out("rcx") _, out("rdx") _, out("r8") _, out("r9") _,
            out("r10") _, out("r11") _,
            out("xmm0") _, out("xmm1") _, out("xmm2") _, out("xmm3") _,
            out("xmm4") _, out("xmm5") _,
        );
    }
    let _ = unsafe { uefi::boot::free_pages(stack_base, stack_pages) };
    unsafe { core::mem::transmute::<usize, Status>(result) }
}

/// Actual entry point — runs on a large heap-allocated stack.
fn run() -> Status {
    uefi::helpers::init().unwrap();

    info!("=== Rusty Box UEFI - DLX Linux (no-alloc) ===");
    info!("BIOS: {} KB, VGA: {} KB, Disk: {} MB (all embedded)",
        BIOS_ROM.len() / 1024, VGA_BIOS.len() / 1024, DLX_DISK.len() / (1024 * 1024));

    let config = EmulatorConfig {
        guest_memory_size: 32 * 1024 * 1024,
        host_memory_size: 32 * 1024 * 1024,
        memory_block_size: 128 * 1024,
        ips: 300_000_000,
        pci_enabled: true,
        ..Default::default()
    };

    // --- Allocate large structs via UEFI pages (no Rust allocator) ---

    // 1. CPU (~17-50MB, mostly BxICache fixed arrays)
    info!("Allocating CPU ({} bytes)...", core::mem::size_of::<BxCpuC<Corei7SkylakeX>>());
    let cpu_ptr: *mut BxCpuC<Corei7SkylakeX> = alloc_zeroed_for();
    let cpu = unsafe {
        match BxCpuBuilder::<Corei7SkylakeX>::init_cpu_at(cpu_ptr, ()) {
            Ok(cpu) => cpu,
            Err(e) => bail!("CPU init failed: {:?}", e),
        }
    };

    // 2. Guest RAM buffer (~36MB: 32MB guest + 4MB BIOS ROM + 128KB expansion + pad)
    let mem_buf_size = rusty_box::config::mem_buffer_size(config.guest_memory_size);
    let mem_pages = (mem_buf_size + 4095) / 4096;
    let mem_ptr = alloc_pages(mem_pages);
    if mem_ptr.is_null() {
        bail!("Failed to allocate {} MB for guest RAM", mem_buf_size / (1024 * 1024));
    }
    unsafe { core::ptr::write_bytes(mem_ptr, 0, mem_buf_size); }

    let mem_stub = unsafe {
        match BxMemoryStubC::create_from_raw(
            mem_ptr,
            mem_buf_size,
            config.guest_memory_size,
            config.host_memory_size,
            config.memory_block_size,
        ) {
            Ok(s) => s,
            Err(e) => bail!("Memory stub init failed: {:?}", e),
        }
    };

    // 3. Emulator struct (~2-3MB, embeds DeviceManager with VGA/IDE buffers)
    info!("Allocating Emulator ({} bytes)...", core::mem::size_of::<Emulator<Corei7SkylakeX>>());
    let emu_ptr: *mut Emulator<Corei7SkylakeX> = alloc_zeroed_for();
    let emu = unsafe {
        match Emulator::<Corei7SkylakeX>::init_at(emu_ptr, cpu, mem_stub, config) {
            Ok(e) => e,
            Err(e) => bail!("Emulator init failed: {:?}", e),
        }
    };

    // --- Initialize hardware ---
    emu.init_pc_system();

    let bios_addr = !(BIOS_ROM.len() as u64 - 1);
    if let Err(e) = emu.load_bios(BIOS_ROM, bios_addr) { bail!("BIOS: {:?}", e); }
    let _ = emu.load_optional_rom(VGA_BIOS, 0xC0000);

    if let Err(e) = emu.init_cpu_and_devices() { bail!("CPU init: {:?}", e); }

    emu.configure_memory_in_cmos_from_config();
    emu.configure_disk_geometry_in_cmos(0, DLX_CYLINDERS, DLX_HEADS, DLX_SPT);
    emu.configure_boot_sequence(2, 0, 0);
    emu.attach_disk_data_ref(0, 0, DLX_DISK, DLX_CYLINDERS, DLX_HEADS, DLX_SPT);

    if let Err(e) = emu.reset(ResetReason::Hardware) { bail!("Reset: {:?}", e); }
    emu.start();
    emu.prepare_run();

    info!("Starting BIOS boot...");
    emu.send_scancode(0x3B); // F1 (skip keyboard error)
    emu.send_scancode(0xBB);

    // Main loop — mirrors run_interactive
    let batch: u64 = 100_000;
    let max: u64 = 20_000_000_000;
    let mut total: u64 = 0;
    let mut login_sent = false;
    let mut last_port92: u8 = 0;

    while total < max {
        let n = match unsafe { emu.run_cpu_batch(batch) } {
            Ok(n) => n,
            Err(e) => { error!("CPU error at {}M: {:?}", total / 1_000_000, e); break; }
        };
        total += n;

        // Process PAM register changes (BIOS needs this)
        if emu.device_manager.pam_needs_update {
            emu.device_manager.process_pci_deferred(&mut emu.devices, &mut emu.memory);
        }

        // Drain and print BIOS/serial output (no Vec allocation)
        {
            let mut had_output = false;
            for b in emu.devices.drain_port_e9_output() {
                if !had_output { had_output = true; }
                // Print byte-by-byte through print_bytes
                print_bytes(&[b]);
            }
        }
        drain_and_print(emu.device_manager.drain_serial_tx(0));

        // Tick devices every batch (keyboard, PIT, etc. need periodic updates)
        let dev_usec = (n * 1_000_000 / 300_000_000u64).max(1);
        emu.tick_devices(dev_usec);

        // Sync PIC/LAPIC interrupt flags to CPU async_event
        emu.sync_event_flags();

        // Deliver PIC interrupt between batches if pending.
        // Inhibition window (MOV SS/STI) expired during the batch.
        if emu.device_manager.has_interrupt() && emu.cpu.interrupts_enabled() {
            let vec = emu.iac();
            unsafe { let _ = emu.inject_interrupt(vec); }
        }

        // Port 92h A20 sync
        emu.sync_port92_a20(&mut last_port92);

        // Handle reset requests (keyboard 0xFE, port 92h, PCI CF9)
        match emu.check_and_handle_resets() {
            Ok(true) => {
                info!("RESET at {}k instr, RIP={:#x}", total / 1000, emu.cpu.rip());
                last_port92 = 0;
                continue;
            }
            Err(e) => { error!("Reset failed: {:?}", e); break; }
            _ => {}
        }

        // HLT handling — advance virtual clock to next timer event
        if n == 0 {
            use rusty_box::cpu::cpu::CpuActivityState;
            if matches!(emu.cpu.activity_state,
                CpuActivityState::Hlt | CpuActivityState::Mwait | CpuActivityState::MwaitIf)
            {
                let mut hlt_budget = 0u64;
                while hlt_budget < 10_000_000 {
                    if emu.has_interrupt() && emu.cpu.interrupts_enabled() {
                        break;
                    }
                    let step = emu.pc_system.get_num_ticks_left_next_event().clamp(1, 100_000);
                    emu.pc_system.tickn(step);
                    emu.dispatch_timer_fires();
                    hlt_budget += step as u64;
                    let usec = (step as u64 * 1_000_000 / 300_000_000u64).max(1);
                    emu.tick_devices(usec);
                    emu.sync_event_flags();
                }
            }

            if emu.cpu.is_in_shutdown() {
                info!("SHUTDOWN at {}k instr, RIP={:#x}", total / 1000, emu.cpu.rip());
                break;
            }
        }

        // Auto-login after kernel boots
        if !login_sent && total > 50_000_000 {
            login_sent = true;
            for &sc in &[0x13u8, 0x93, 0x18, 0x98, 0x18, 0x98, 0x14, 0x94, 0x1C, 0x9C] {
                emu.send_scancode(sc);
            }
        }

        #[cfg(feature = "verbose")]
        if total % 500_000 < batch {
            info!("  {}k instr, RIP={:#x}, batch={}, IF={}",
                total / 1000, emu.cpu.rip(), n, emu.cpu.interrupts_enabled());
        }
    }

    info!("Done: {} instructions", total);
    uefi::boot::stall(30_000_000);
    Status::SUCCESS
}
