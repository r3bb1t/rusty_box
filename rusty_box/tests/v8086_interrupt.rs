//! An interrupt out of virtual-8086 mode leaves it.
//!
//! Bochs exception.cc `protected_mode_int` ends with `clear_VM()`, whose
//! setter (cpu.h `IMPLEMENT_EFLAG_SET_ACCESSOR_VM`) recomputes the CPU mode.
//! A handler that kept running in virtual-8086 mode would load segment
//! registers as a real-mode selector times 16 rather than from the GDT, and
//! a Linux kernel's exception entry, which loads its per-CPU segment into FS
//! first thing, would fault on every entry until its stack ran out.

#![cfg(feature = "std")]

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

/// Same sizing rationale as fp_vex_scalar_ops.rs.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

const GDT: u64 = 0x2000;
const IDT: u64 = 0x3000;
const TSS: u64 = 0x4000;
const HANDLER: u64 = 0x5000;
const RING0_STACK_TOP: u64 = 0x6000;
/// Virtual-8086 code at 0700:0000 and stack at 0800:1000.
const V86_CS: u16 = 0x0700;
const V86_SS: u16 = 0x0800;
const V86_SP: u32 = 0x1000;
const CODE: u64 = 0xA000;
/// The stack the ring-0 setup code builds its IRETD frame on.
const SETUP_STACK_TOP: u64 = 0xF000;

const RING0_CODE_SEL: u16 = 0x08;
const RING0_DATA_SEL: u16 = 0x10;
const TSS_SEL: u16 = 0x18;
/// A ring-0 data segment whose base is not its selector times 16.
const BASED_DATA_SEL: u16 = 0x20;
const BASED_DATA_BASE: u64 = 0x0002_0000;
const MARK: u32 = 0x600D_F00D;
const DECOY: u32 = 0xBAD0_BAD0;

/// A flat (limit 4 GiB) 32-bit segment descriptor of access byte `access`
/// at `base`.
fn flat_descriptor(base: u32, access: u8) -> u64 {
    let base = u64::from(base);
    0x000F_FFFF & 0xFFFF
        | (base & 0xFF_FFFF) << 16
        | u64::from(access) << 40
        | 0xC_u64 << 52
        | 0xF_u64 << 48
        | (base >> 24) << 56
}

/// A 32-bit available TSS descriptor at `base` of the minimum size.
fn tss_descriptor(base: u32) -> u64 {
    let base = u64::from(base);
    0x67 | (base & 0xFF_FFFF) << 16 | 0x89_u64 << 40 | (base >> 24) << 56
}

/// A 386 interrupt gate to `selector:offset` callable from privilege 3.
fn ring3_interrupt_gate(selector: u16, offset: u32) -> u64 {
    let offset = u64::from(offset);
    (offset & 0xFFFF) | u64::from(selector) << 16 | 0xEE00_u64 << 32 | (offset >> 16) << 48
}

fn push_imm32(code: &mut Vec<u8>, value: u32) {
    code.push(0x68);
    code.extend_from_slice(&value.to_le_bytes());
}

#[test]
fn a_handler_entered_from_virtual_8086_mode_loads_segments_from_the_gdt() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu =
                Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)
                    .expect("new emulator");
            let write = |emu: &mut Emulator, at: u64, bytes: &[u8]| {
                emu.mem_write(BxPhyAddress::new(at), bytes)
                    .expect("write guest memory");
            };

            let gdt = [
                0,
                flat_descriptor(0, 0x9A),
                flat_descriptor(0, 0x92),
                tss_descriptor(TSS as u32),
                flat_descriptor(BASED_DATA_BASE as u32, 0x92),
            ];
            let gdt_bytes: Vec<u8> = gdt.iter().flat_map(|d| d.to_le_bytes()).collect();
            write(&mut emu, GDT, &gdt_bytes);
            emu.reg_write(X86Reg::GdtrBase, GDT);
            emu.reg_write(X86Reg::GdtrLimit, (gdt_bytes.len() - 1) as u64);

            let gate = ring3_interrupt_gate(RING0_CODE_SEL, HANDLER as u32);
            write(&mut emu, IDT + 0x80 * 8, &gate.to_le_bytes());
            emu.reg_write(X86Reg::IdtrBase, IDT);
            emu.reg_write(X86Reg::IdtrLimit, 256 * 8 - 1);

            // TSS: ESP0 at +4, SS0 at +8.
            write(&mut emu, TSS + 4, &(RING0_STACK_TOP as u32).to_le_bytes());
            write(&mut emu, TSS + 8, &u32::from(RING0_DATA_SEL).to_le_bytes());

            write(&mut emu, BASED_DATA_BASE, &MARK.to_le_bytes());
            write(
                &mut emu,
                u64::from(BASED_DATA_SEL) << 4,
                &DECOY.to_le_bytes(),
            );

            // Ring-0 handler: mov ax, BASED_DATA_SEL; mov fs, ax;
            // mov eax, fs:[0]; hlt.
            let mut handler = vec![0x66, 0xB8];
            handler.extend_from_slice(&BASED_DATA_SEL.to_le_bytes());
            handler.extend_from_slice(&[0x8E, 0xE0, 0x64, 0xA1, 0, 0, 0, 0]);
            let handler_hlt = HANDLER + handler.len() as u64;
            handler.push(0xF4);
            write(&mut emu, HANDLER, &handler);

            // Virtual-8086 code: int 0x80 (IOPL 3, so through the IDT).
            write(&mut emu, u64::from(V86_CS) << 4, &[0xCD, 0x80]);

            // ltr TSS_SEL, then IRETD into virtual-8086 mode with IOPL 3.
            let mut code = vec![0x66, 0xB8];
            code.extend_from_slice(&TSS_SEL.to_le_bytes());
            code.extend_from_slice(&[0x0F, 0x00, 0xD8]);
            for value in [0, 0, 0, 0] {
                push_imm32(&mut code, value); // GS, FS, DS, ES
            }
            push_imm32(&mut code, u32::from(V86_SS));
            push_imm32(&mut code, V86_SP);
            push_imm32(&mut code, (1 << 17) | (3 << 12) | 0x2); // VM, IOPL 3
            push_imm32(&mut code, u32::from(V86_CS));
            push_imm32(&mut code, 0); // IP
            code.push(0xCF);
            write(&mut emu, CODE, &code);
            emu.reg_write(X86Reg::Rsp, SETUP_STACK_TOP);

            emu.emu_start(CODE, Some(handler_hlt), None, Some(1000))
                .expect("run into the handler");

            assert_eq!(emu.cpu().rip(), handler_hlt, "the handler ran to its hlt");
            assert_eq!(
                emu.reg_read(X86Reg::Eflags) & (1 << 17),
                0,
                "the handler runs with VM clear"
            );
            assert_eq!(
                emu.reg_read(X86Reg::Rax) as u32,
                MARK,
                "FS came from the GDT descriptor, not selector * 16"
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}
