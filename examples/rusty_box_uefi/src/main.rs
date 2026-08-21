//! Rusty Box UEFI — boots DLX Linux via full BIOS POST.
//!
//! Everything embedded at compile time: BIOS, VGA BIOS, DLX disk image.
//! Normal BIOS boot path — identical to the desktop dlxlinux example.
//! No Rust allocator required — all large structs placed via UEFI page allocation.

#![no_main]
#![no_std]

use core::mem::MaybeUninit;
use log::{error, info};
use uefi::prelude::*;

use rusty_box::{
    cpu::{builder::BxCpuBuilder, cpu::BxCpuC},
    emulator::{
        AtaSlot, BootDevice, BootOrder, DiskGeometry, Emulator, EmulatorConfig, Ips, MemorySize, MachineBuilder,
    },
    memory::BxMemoryStubC,
};

static BIOS_ROM: &[u8] = include_bytes!("../../../cpp_orig/bochs/bochs/bios/BIOS-bochs-latest");
static VGA_BIOS: &[u8] = include_bytes!(
    "../../../cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"
);
static DLX_DISK: &[u8] = include_bytes!("../../../dlxlinux/hd10meg.img");

const DLX_CYLINDERS: u16 = 306;
const DLX_HEADS: u8 = 4;
const DLX_SPT: u8 = 17;

fn print_bytes(bytes: &[u8]) {
    let mut buf = [0u16; 128];
    let mut pos = 0;
    for &b in bytes {
        let ch = match b {
            b'\n' => {
                buf[pos] = b'\r' as u16;
                pos += 1;
                b'\n' as u16
            }
            b'\r' => continue,
            0x20..=0x7E => b as u16,
            _ => continue,
        };
        buf[pos] = ch;
        pos += 1;
        if pos >= buf.len() - 2 {
            buf[pos] = 0;
            let s = unsafe { uefi::CStr16::from_u16_with_nul_unchecked(&buf[..=pos]) };
            let _ = uefi::system::with_stdout(|out| {
                let _ = out.output_string(s);
            });
            pos = 0;
        }
    }
    if pos > 0 {
        buf[pos] = 0;
        let s = unsafe { uefi::CStr16::from_u16_with_nul_unchecked(&buf[..=pos]) };
        let _ = uefi::system::with_stdout(|out| {
            let _ = out.output_string(s);
        });
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
    unsafe {
        core::ptr::write_bytes(ptr, 0, size);
    }
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
    )
    .expect("failed to allocate stack");
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
    info!(
        "BIOS: {} KB, VGA: {} KB, Disk: {} MB (all embedded)",
        BIOS_ROM.len() / 1024,
        VGA_BIOS.len() / 1024,
        DLX_DISK.len() / (1024 * 1024)
    );

    let config = EmulatorConfig {
        memory: MemorySize::bytes(32 * 1024 * 1024),
        memory_block_size: 128 * 1024,
        ips: Ips::new(300_000_000),
        pci_enabled: true,
        ..Default::default()
    };

    // --- Allocate large structs via UEFI pages (no Rust allocator) ---

    // 1. CPU (~17-50MB, mostly BxICache fixed arrays)
    info!(
        "Allocating CPU ({} bytes)...",
        core::mem::size_of::<BxCpuC>()
    );
    let cpu_ptr: *mut BxCpuC = alloc_zeroed_for();
    let cpu: &'static mut BxCpuC = unsafe {
        match BxCpuBuilder::new().init_cpu_at(cpu_ptr, ()) {
            Ok(cpu) => cpu,
            Err(e) => bail!("CPU init failed: {:?}", e),
        }
    };

    // The machine borrows its CPU set as a slice, so the slice itself needs
    // somewhere to live that outlasts the machine. UEFI pages are never freed
    // here, which is what makes the `'static` borrow honest — the same reason
    // the CPU above can be `&'static mut`.
    let cpu_handles: *mut &'static mut BxCpuC = alloc_zeroed_for();
    let cpus: &'static mut [&'static mut BxCpuC] = unsafe {
        cpu_handles.write(cpu);
        core::slice::from_raw_parts_mut(cpu_handles, 1)
    };

    // 2. Guest RAM buffer (~36MB: 32MB guest + 4MB BIOS ROM + 128KB expansion + pad)
    let mem_buf_size = rusty_box::config::mem_buffer_size(config.memory.guest_bytes());
    let mem_pages = (mem_buf_size + 4095) / 4096;
    let mem_ptr = alloc_pages(mem_pages);
    if mem_ptr.is_null() {
        bail!(
            "Failed to allocate {} MB for guest RAM",
            mem_buf_size / (1024 * 1024)
        );
    }
    unsafe {
        core::ptr::write_bytes(mem_ptr, 0, mem_buf_size);
    }

    let mem_stub = unsafe {
        match BxMemoryStubC::create_from_raw(
            mem_ptr,
            mem_buf_size,
            config.memory.guest_bytes(),
            config.memory.host_bytes(),
            config.memory_block_size,
        ) {
            Ok(s) => s,
            Err(e) => bail!("Memory stub init failed: {:?}", e),
        }
    };

    // 3. Emulator struct (~2-3MB, embeds DeviceManager with VGA/IDE buffers)
    info!(
        "Allocating Emulator ({} bytes)...",
        core::mem::size_of::<Emulator>()
    );
    let emu_ptr: *mut MaybeUninit<Emulator> = alloc_zeroed_for();
    // SAFETY: `alloc_zeroed_for` returned firmware pages sized and aligned for
    // the machine, and nothing else ever borrows them.
    let emu_storage: &'static mut MaybeUninit<Emulator> = unsafe { &mut *emu_ptr };
    let emu = match MachineBuilder::new(config)
        .bios(BIOS_ROM)
        .vga_bios(VGA_BIOS)
        .boot_order(BootOrder::just(BootDevice::Disk))
        .disk_static(
            AtaSlot::PRIMARY_MASTER,
            DLX_DISK,
            DiskGeometry::new(DLX_CYLINDERS.into(), DLX_HEADS, DLX_SPT),
        )
        .build_at(emu_storage, cpus, mem_stub)
    {
        Ok(e) => e,
        Err(e) => bail!("Machine build failed: {:?}", e),
    };

    emu.prepare_run();

    info!("Starting BIOS boot...");
    // F1, to skip the BIOS keyboard-error prompt.
    let queued = emu.keyboard().scancodes(&[0x3B, 0xBB]);
    if queued != 2 {
        info!("keyboard ring was full; F1 not queued");
    }

    // Main loop — mirrors run_interactive
    let batch: u64 = 100_000;
    let max: u64 = 20_000_000_000;
    let mut total: u64 = 0;
    let mut login_sent = false;

    while total < max {
        let outcome = match emu.step_batch(batch) {
            Ok(result) => result,
            Err(e) => {
                error!("CPU error at {}M: {:?}", total / 1_000_000, e);
                break;
            }
        };
        total += outcome.executed;


        // Drain and print BIOS/serial output (no Vec allocation)
        {
            let mut had_output = false;
            for b in emu.debug_port().take_output() {
                if !had_output {
                    had_output = true;
                }
                // Print byte-by-byte through print_bytes
                print_bytes(&[b]);
            }
        }
        drain_and_print(emu.serial(0).expect("COM1 is always modelled").take_output());

        // Every terminal cause, not just the CPU shutdown state: a guest that
        // powers off through ACPI S5 leaves the CPU perfectly healthy, so
        // testing the CPU alone would keep stepping a machine that asked to be
        // off until this loop hit its own instruction cap.
        if outcome.is_terminal() {
            info!(
                "STOP ({:?}) at {}k instr, RIP={:#x}",
                outcome.stop,
                total / 1000,
                emu.cpu().rip()
            );
            break;
        }



        // Auto-login after kernel boots
        if !login_sent && total > 50_000_000 {
            login_sent = true;
            let login = [0x13u8, 0x93, 0x18, 0x98, 0x18, 0x98, 0x14, 0x94, 0x1C, 0x9C];
            let sent = emu.keyboard().scancodes(&login);
            if sent != login.len() {
                info!("login sequence truncated at {sent} of {}", login.len());
            }
        }

        #[cfg(feature = "verbose")]
        if total % 500_000 < batch {
            info!(
                "  {}k instr, RIP={:#x}, batch={}, IF={}",
                total / 1000,
                emu.cpu().rip(),
                n,
                emu.cpu().interrupts_enabled()
            );
        }
    }

    info!("Done: {} instructions", total);
    uefi::boot::stall(30_000_000);
    Status::SUCCESS
}
