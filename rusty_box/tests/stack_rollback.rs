//! What a faulting stack instruction leaves on the stack, run as guest code.
//!
//! Bochs brackets every instruction that moves the stack pointer before it
//! can still fault with `RSP_SPECULATIVE` and `RSP_COMMIT` (cpu.h): the
//! `exception()` a fault-class exception takes restores RSP from `prev_rsp`,
//! so the handler finds the stack exactly as it was before the instruction
//! began, and the faulting instruction can be restarted. Each test here
//! raises #GP after the instruction has already pushed or popped, and reads
//! the stack pointer the processor delivered the fault with.

#![cfg(feature = "std")]

use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

fn on_big_stack(check: fn()) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(check)
        .expect("spawn")
        .join()
        .expect("join");
}

/// What the #GP handler was entered with.
#[derive(Debug, PartialEq, Eq)]
struct Delivered {
    /// The stack pointer the processor had when it delivered the fault — the
    /// one the faulting instruction left behind.
    stack_pointer: u64,
    /// The instruction pointer the fault pushed: the faulting instruction's.
    faulting_ip: u64,
}

/// Run `code` at `entry` and require it to end in the #GP handler at
/// `handler`, whose first instruction is a HLT.
fn run_to_handler(emu: &mut Emulator, entry: u64, code: &[u8], handler: u64) {
    let end = entry + code.len() as u64;
    let image = [code, &[0xEB, 0xFE]].concat();
    emu.mem_write(entry, &image).expect("write code");
    emu.mem_write(handler, &[0xF4]).expect("write handler");
    emu.emu_start(entry, Some(end), None, Some(32))
        .expect("execute");
    let ip = emu.cpu().rip();
    assert!(
        ip == handler || ip == handler + 1,
        "the run stopped at {ip:#x}, not in the #GP handler at {handler:#x}"
    );
}

fn read_u16(emu: &mut Emulator, address: u64) -> u64 {
    let mut bytes = [0u8; 2];
    emu.mem_read(address, &mut bytes).expect("read stack");
    u64::from(u16::from_le_bytes(bytes))
}

fn read_u32(emu: &mut Emulator, address: u64) -> u64 {
    let mut bytes = [0u8; 4];
    emu.mem_read(address, &mut bytes).expect("read stack");
    u64::from(u32::from_le_bytes(bytes))
}

fn read_u64(emu: &mut Emulator, address: u64) -> u64 {
    let mut bytes = [0u8; 8];
    emu.mem_read(address, &mut bytes).expect("read stack");
    u64::from_le_bytes(bytes)
}

// ─────────────────────────── real mode ───────────────────────────

const REAL_CODE: u64 = 0x1000;
const REAL_HANDLER: u64 = 0x5000;
const REAL_STACK: u64 = 0x8000;
/// What a real-mode delivery pushes: FLAGS, CS and IP.
const REAL_FRAME: u64 = 6;
/// An offset one past real mode's 64 KiB CS limit.
const BEYOND_REAL_LIMIT: u32 = 0x0001_0000;

/// Run one real-mode instruction over a stack holding `stack`, through a #GP
/// it must raise.
fn real_mode_fault(stack: &[u8], instruction: &[u8]) -> Delivered {
    let mut emu = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::RealMode)
        .expect("emulator");
    // IVT entry 13 = 0000:REAL_HANDLER.
    let vector = [(REAL_HANDLER as u16).to_le_bytes(), 0u16.to_le_bytes()].concat();
    emu.mem_write(13 * 4, &vector).expect("ivt");
    emu.mem_write(REAL_STACK, stack).expect("stack");
    emu.reg_write(X86Reg::Rsp, REAL_STACK);
    run_to_handler(&mut emu, REAL_CODE, instruction, REAL_HANDLER);
    let sp = emu.reg_read(X86Reg::Rsp) & 0xFFFF;
    Delivered {
        stack_pointer: sp + REAL_FRAME,
        faulting_ip: read_u16(&mut emu, sp),
    }
}

/// A 32-bit near RET whose popped EIP is past the CS limit raises #GP with
/// SP back where the RET found it (Bochs ctrl_xfer32.cc RETnear32_Iw).
#[test]
fn a_near_return_past_the_limit_restores_sp() {
    on_big_stack(|| {
        let delivered = real_mode_fault(&BEYOND_REAL_LIMIT.to_le_bytes(), &[0x66, 0xC3]);
        assert_eq!(
            delivered,
            Delivered {
                stack_pointer: REAL_STACK,
                faulting_ip: REAL_CODE
            }
        );
    });
}

/// RETF with no immediate is the immediate form with a count of zero, and it
/// restores SP the same way (Bochs ctrl_xfer32.cc RETfar32_Iw).
#[test]
fn a_far_return_past_the_limit_restores_sp() {
    on_big_stack(|| {
        let stack = [BEYOND_REAL_LIMIT.to_le_bytes(), 0u32.to_le_bytes()].concat();
        let delivered = real_mode_fault(&stack, &[0x66, 0xCB]);
        assert_eq!(
            delivered,
            Delivered {
                stack_pointer: REAL_STACK,
                faulting_ip: REAL_CODE
            }
        );
    });
}

/// A near CALL pushes its return address before the target is checked; the
/// #GP takes the push back (Bochs ctrl_xfer32.cc CALL_Jd).
#[test]
fn a_near_call_past_the_limit_takes_back_its_push() {
    on_big_stack(|| {
        // call rel32: 66 E8 rel32, six bytes long.
        let rel = BEYOND_REAL_LIMIT.wrapping_sub((REAL_CODE + 6) as u32);
        let call = [&[0x66, 0xE8][..], &rel.to_le_bytes()].concat();
        let delivered = real_mode_fault(&[], &call);
        assert_eq!(
            delivered,
            Delivered {
                stack_pointer: REAL_STACK,
                faulting_ip: REAL_CODE
            }
        );
    });
}

/// A far JMP past the CS limit is a guest #GP (Bochs ctrl_xfer32.cc
/// jmp_far32), not a stop of the machine.
#[test]
fn a_far_jump_past_the_limit_raises_gp() {
    on_big_stack(|| {
        // jmp 0000:00010000 — 66 EA ptr16:32.
        let jmp = [&[0x66, 0xEA][..], &BEYOND_REAL_LIMIT.to_le_bytes(), &[0x00, 0x00]].concat();
        let delivered = real_mode_fault(&[], &jmp);
        assert_eq!(
            delivered,
            Delivered {
                stack_pointer: REAL_STACK,
                faulting_ip: REAL_CODE
            }
        );
    });
}

// ─────────────────────── 32-bit protected mode ───────────────────────

const PROTECTED_CODE: u64 = 0x0020_0000;
const PROTECTED_IDT: u64 = 0x0024_0000;
const PROTECTED_HANDLER: u64 = 0x0025_0000;
const PROTECTED_STACK: u64 = 0x0026_0000;
/// What a same-privilege delivery with an error code pushes: EFLAGS, CS,
/// EIP and the error code.
const PROTECTED_FRAME: u64 = 16;

/// A protected-mode #GP: what was delivered and the error code it carried.
#[derive(Debug, PartialEq, Eq)]
struct ProtectedFault {
    delivered: Delivered,
    error_code: u64,
}

/// Run `code` in flat 32-bit protected mode through a #GP it must raise.
fn protected_mode_fault(code: &[u8]) -> ProtectedFault {
    let mut emu =
        Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)
            .expect("emulator");
    let mut gate = [0u8; 8];
    gate[0..2].copy_from_slice(&(PROTECTED_HANDLER as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
    gate[5] = 0x8E;
    gate[6..8].copy_from_slice(&((PROTECTED_HANDLER >> 16) as u16).to_le_bytes());
    emu.mem_write(PROTECTED_IDT + 13 * 8, &gate).expect("idt");
    emu.reg_write(X86Reg::IdtrBase, PROTECTED_IDT);
    emu.reg_write(X86Reg::IdtrLimit, 256 * 8 - 1);
    emu.reg_write(X86Reg::Rsp, PROTECTED_STACK);
    run_to_handler(&mut emu, PROTECTED_CODE, code, PROTECTED_HANDLER);
    let esp = emu.reg_read(X86Reg::Rsp) & 0xFFFF_FFFF;
    ProtectedFault {
        delivered: Delivered {
            stack_pointer: esp + PROTECTED_FRAME,
            faulting_ip: read_u32(&mut emu, esp + 4),
        },
        error_code: read_u32(&mut emu, esp),
    }
}

/// POP SS of a null selector raises #GP(0) with the selector still on the
/// stack: Bochs stack32.cc POP32_Sw reads it, loads it, and only then
/// releases it.
#[test]
fn popping_a_null_stack_selector_leaves_it_on_the_stack() {
    on_big_stack(|| {
        // push 0; pop ss
        assert_eq!(
            protected_mode_fault(&[0x6A, 0x00, 0x17]),
            ProtectedFault {
                delivered: Delivered {
                    stack_pointer: PROTECTED_STACK - 4,
                    faulting_ip: PROTECTED_CODE + 2
                },
                error_code: 0,
            }
        );
    });
}

/// MOV SS of a null selector is a guest #GP(0) (Bochs segment_ctrl_pro.cc
/// load_seg_reg), not a stop of the machine.
#[test]
fn moving_a_null_selector_into_ss_raises_gp() {
    on_big_stack(|| {
        // xor eax, eax; mov ss, ax
        assert_eq!(
            protected_mode_fault(&[0x31, 0xC0, 0x8E, 0xD0]),
            ProtectedFault {
                delivered: Delivered {
                    stack_pointer: PROTECTED_STACK,
                    faulting_ip: PROTECTED_CODE + 2
                },
                error_code: 0,
            }
        );
    });
}

// ─────────────────────── virtual-8086 mode ───────────────────────

const V86_GDT: u64 = 0x1000;
const V86_TSS: u64 = 0x2000;
const V86_IDT: u64 = 0x3000;
const V86_HANDLER: u64 = 0x4000;
/// The ring-0 stack the IRETD into virtual-8086 mode pops its frame from.
const V86_ENTRY_STACK: u64 = 0x8000;
/// TSS.ESP0: where a fault in virtual-8086 mode delivers to.
const V86_RING0_STACK: u64 = 0x9000;
/// The virtual-8086 code and stack segments, and SP.
const V86_CS: u64 = 0x0500;
const V86_SS: u64 = 0x0700;
const V86_SP: u64 = 0x0100;
/// The offset in CS the virtual-8086 IRET returns to: a HLT, which faults
/// at CPL 3.
const V86_HLT_IP: u64 = 0x0010;
const RING0_CODE: u16 = 0x08;
const RING0_DATA: u16 = 0x10;
const TSS_SELECTOR: u16 = 0x18;

/// Enter virtual-8086 mode at IOPL 3, run `iret` there over `iret_frame` on
/// the virtual-8086 stack, back to a HLT at `V86_HLT_IP`, and require the
/// HLT's #GP to reach the ring-0 handler.
fn v86_iret_then_fault(iret: &[u8], iret_frame: &[u8]) -> ProtectedFault {
    let mut emu =
        Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)
            .expect("emulator");
    // Null, flat ring-0 code and data, and a 32-bit TSS (limit 0x67).
    let gdt = [
        0u64,
        0x00CF_9A00_0000_FFFF,
        0x00CF_9200_0000_FFFF,
        0x0000_8900_0000_0067 | (V86_TSS << 16),
    ];
    let gdt_bytes: Vec<u8> = gdt.iter().flat_map(|entry| entry.to_le_bytes()).collect();
    emu.mem_write(V86_GDT, &gdt_bytes).expect("gdt");
    emu.reg_write(X86Reg::GdtrBase, V86_GDT);
    emu.reg_write(X86Reg::GdtrLimit, 4 * 8 - 1);
    // TSS.ESP0 and TSS.SS0.
    emu.mem_write(V86_TSS + 4, &(V86_RING0_STACK as u32).to_le_bytes())
        .expect("tss");
    emu.mem_write(V86_TSS + 8, &u32::from(RING0_DATA).to_le_bytes())
        .expect("tss");
    // #GP: a 32-bit interrupt gate, DPL 0, to a ring-0 HLT.
    let mut gate = [0u8; 8];
    gate[0..2].copy_from_slice(&(V86_HANDLER as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&RING0_CODE.to_le_bytes());
    gate[5] = 0x8E;
    gate[6..8].copy_from_slice(&((V86_HANDLER >> 16) as u16).to_le_bytes());
    emu.mem_write(V86_IDT + 13 * 8, &gate).expect("idt");
    emu.reg_write(X86Reg::IdtrBase, V86_IDT);
    emu.reg_write(X86Reg::IdtrLimit, 256 * 8 - 1);
    emu.mem_write(V86_HANDLER, &[0xF4]).expect("handler");

    // The IRETD frame into virtual-8086 mode at V86_CS:0, IOPL 3:
    // EIP, CS, EFLAGS (VM, IOPL 3, IF), ESP, SS, ES, DS, FS, GS.
    let entry: Vec<u8> = [0, V86_CS, 0x0002_3202, V86_SP, V86_SS, 0, 0, 0, 0]
        .iter()
        .flat_map(|dword: &u64| (*dword as u32).to_le_bytes())
        .collect();
    emu.mem_write(V86_ENTRY_STACK, &entry).expect("entry frame");
    emu.reg_write(X86Reg::Rsp, V86_ENTRY_STACK);

    // The virtual-8086 code: `iret`, then a HLT at V86_HLT_IP.
    let mut v86_image = vec![0x90u8; V86_HLT_IP as usize + 1];
    v86_image[..iret.len()].copy_from_slice(iret);
    v86_image[V86_HLT_IP as usize] = 0xF4;
    emu.mem_write(V86_CS << 4, &v86_image).expect("v86 code");
    emu.mem_write((V86_SS << 4) + V86_SP, iret_frame)
        .expect("v86 stack");

    // mov eax, TSS_SELECTOR; ltr ax; iretd
    let code = [
        &[0xB8][..],
        &u32::from(TSS_SELECTOR).to_le_bytes(),
        &[0x0F, 0x00, 0xD8, 0xCF],
    ]
    .concat();
    emu.mem_write(PROTECTED_CODE, &code).expect("write code");
    emu.emu_start(PROTECTED_CODE, None, None, Some(32))
        .expect("execute");
    let ip = emu.cpu().rip();
    assert!(
        ip == V86_HANDLER || ip == V86_HANDLER + 1,
        "the run stopped at {ip:#x}, not in the #GP handler at {V86_HANDLER:#x}"
    );

    // Error code, EIP, CS, EFLAGS, ESP, SS, then the data segments.
    let frame = emu.reg_read(X86Reg::Rsp) & 0xFFFF_FFFF;
    ProtectedFault {
        delivered: Delivered {
            stack_pointer: read_u32(&mut emu, frame + 16),
            faulting_ip: read_u32(&mut emu, frame + 4),
        },
        error_code: read_u32(&mut emu, frame),
    }
}

/// An IRET in virtual-8086 mode commits the pops it made (Bochs
/// ctrl_xfer16.cc IRET16 reaches RSP_COMMIT after
/// iret16_stack_return_from_v86), so a later fault — the HLT it returns to,
/// at CPL 3 — is delivered with the SP the IRET left, not the SP it began
/// with.
#[test]
fn an_iret_in_virtual_8086_mode_commits_its_pops() {
    on_big_stack(|| {
        // IP, CS, FLAGS.
        let frame: Vec<u8> = [V86_HLT_IP as u16, V86_CS as u16, 0x0202]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        assert_eq!(
            v86_iret_then_fault(&[0xCF], &frame),
            ProtectedFault {
                delivered: Delivered {
                    stack_pointer: V86_SP + 6,
                    faulting_ip: V86_HLT_IP,
                },
                error_code: 0,
            }
        );
    });
}

/// The same for IRETD (Bochs ctrl_xfer32.cc IRET32 after
/// iret32_stack_return_from_v86).
#[test]
fn an_iretd_in_virtual_8086_mode_commits_its_pops() {
    on_big_stack(|| {
        // EIP, CS, EFLAGS.
        let frame: Vec<u8> = [V86_HLT_IP as u32, V86_CS as u32, 0x0202]
            .iter()
            .flat_map(|dword| dword.to_le_bytes())
            .collect();
        assert_eq!(
            v86_iret_then_fault(&[0x66, 0xCF], &frame),
            ProtectedFault {
                delivered: Delivered {
                    stack_pointer: V86_SP + 12,
                    faulting_ip: V86_HLT_IP,
                },
                error_code: 0,
            }
        );
    });
}

// ─────────────────────────── long mode ───────────────────────────

const LONG_CODE: u64 = 0x0020_0000;
const LONG_IDT: u64 = 0x0024_0000;
const LONG_HANDLER: u64 = 0x0025_0000;
/// 16-byte aligned, so delivery's alignment of RSP is visible only where the
/// faulting instruction left RSP misaligned.
const LONG_STACK: u64 = 0x0026_0000;
/// The lowest non-canonical address.
const NON_CANONICAL: u64 = 0x8000_0000_0000_0000;

/// Run `code` in flat long mode through a #GP it must raise. A 64-bit
/// delivery saves the interrupted RSP in its frame, so that is read back
/// rather than inferred from the handler's RSP, which delivery aligns.
fn long_mode_fault(code: &[u8]) -> Delivered {
    let mut emu = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
        .expect("emulator");
    let mut gate = [0u8; 16];
    gate[0..2].copy_from_slice(&(LONG_HANDLER as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
    gate[5] = 0x8E;
    gate[6..8].copy_from_slice(&((LONG_HANDLER >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((LONG_HANDLER >> 32) as u32).to_le_bytes());
    emu.mem_write(LONG_IDT + 13 * 16, &gate).expect("idt");
    emu.reg_write(X86Reg::IdtrBase, LONG_IDT);
    emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
    emu.reg_write(X86Reg::Rsp, LONG_STACK);
    run_to_handler(&mut emu, LONG_CODE, code, LONG_HANDLER);
    // Error code, RIP, CS, RFLAGS, RSP, SS.
    let frame = emu.reg_read(X86Reg::Rsp);
    Delivered {
        stack_pointer: read_u64(&mut emu, frame + 32),
        faulting_ip: read_u64(&mut emu, frame + 8),
    }
}

/// `mov rbx, NON_CANONICAL`: REX.W B8+3 imm64, ten bytes long.
fn load_rbx_non_canonical() -> Vec<u8> {
    [&[0x48, 0xBB][..], &NON_CANONICAL.to_le_bytes()].concat()
}

/// POP m64 pops before it writes; a write that faults puts the qword back
/// (Bochs stack64.cc POP_EqM).
#[test]
fn a_pop_to_a_faulting_destination_restores_rsp() {
    on_big_stack(|| {
        // push rax; mov rbx, NON_CANONICAL; pop qword [rbx]
        let code = [&[0x50][..], &load_rbx_non_canonical(), &[0x8F, 0x03]].concat();
        assert_eq!(
            long_mode_fault(&code),
            Delivered {
                stack_pointer: LONG_STACK - 8,
                faulting_ip: LONG_CODE + 11
            }
        );
    });
}

/// An indirect near CALL to a non-canonical target pushes its return address
/// first; the #GP takes the push back (Bochs ctrl_xfer64.cc CALL_EqR).
#[test]
fn an_indirect_call_to_a_non_canonical_target_takes_back_its_push() {
    on_big_stack(|| {
        // mov rbx, NON_CANONICAL; call rbx
        let code = [&load_rbx_non_canonical()[..], &[0xFF, 0xD3]].concat();
        assert_eq!(
            long_mode_fault(&code),
            Delivered {
                stack_pointer: LONG_STACK,
                faulting_ip: LONG_CODE + 10
            }
        );
    });
}
