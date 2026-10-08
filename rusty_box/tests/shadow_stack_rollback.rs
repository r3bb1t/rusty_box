//! What a fault leaves on the CET shadow stack, run as guest code.
//!
//! Bochs's `RSP_SPECULATIVE` (cpu.h) records SSP alongside RSP
//! (`PRESERVE_SSP`), and the `exception()` a fault-class exception takes
//! (exception.cc) restores both, so a faulting instruction restarts on the
//! shadow stack it began with. Each test here enables a supervisor shadow
//! stack, faults part-way through an instruction that has already pushed or
//! popped it, and compares the SSP the handler is entered with against a
//! control run whose fault touches neither stack. Both deliveries push the
//! same same-privilege frame onto the shadow stack, so the comparison does not
//! depend on how large that frame is: any difference is what the faulting
//! instruction left behind.

#![cfg(feature = "std")]

use rusty_box::cpu::decoder::X86Feature;
use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};
use rusty_box::params::BxParams;

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;
const IDT: u64 = 0x0024_0000;
/// One handler per vector, so where the run stops names the fault.
const NP_HANDLER: u64 = 0x0025_0000;
const GP_HANDLER: u64 = 0x0025_0100;
const CP_HANDLER: u64 = 0x0025_0200;
const STACK_TOP: u64 = 0x0026_0000;
/// The 2 MiB page mapped as the supervisor shadow stack.
const SHADOW_STACK_PAGE: u64 = 0x0040_0000;
/// IA32_PL0_SSP: a supervisor shadow-stack token SETSSBSY marks busy and
/// loads into SSP. Every push goes below it.
const SHADOW_STACK_TOKEN: u64 = 0x005F_F000;
/// The lowest non-canonical address.
const NON_CANONICAL: u64 = 0x8000_0000_0000_0000;

const NP_VECTOR: u64 = 11;
const GP_VECTOR: u64 = 13;
const CP_VECTOR: u64 = 21;
/// A vector whose gate is a 64-bit interrupt gate with P clear.
const NOT_PRESENT_VECTOR: u8 = 0x40;
/// Bochs cpu.h `BX_CP_NEAR_RET`, the #CP error code of a near RET whose
/// return address disagrees with the shadow stack.
const NEAR_RET: u64 = 1;

/// Page-directory entry bits: present, dirty, 2 MiB. A leaf with R/W clear
/// and D set under writable upper levels is a shadow-stack page (Bochs
/// paging.cc), and U/S clear makes it a supervisor one.
const PDE_PRESENT: u64 = 1 << 0;
const PDE_DIRTY: u64 = 1 << 6;
const PDE_LARGE: u64 = 1 << 7;
/// The address bits of a paging-structure entry.
const ENTRY_ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

/// CR0.WP, which CR4.CET requires, then CR4.CET, IA32_S_CET.SH_STK_EN and
/// IA32_PL0_SSP, and SETSSBSY to take the token: a supervisor shadow stack
/// with SSP = `SHADOW_STACK_TOKEN`, set up by the guest's own instructions.
fn enable_shadow_stack() -> Vec<u8> {
    let token = u32::try_from(SHADOW_STACK_TOKEN).expect("below 4 GiB");
    let mov_eax_token = [&[0xB8][..], &token.to_le_bytes()].concat();
    [
        &[0x0F, 0x20, 0xC0][..],         // mov rax, cr0
        &[0x0D, 0x00, 0x00, 0x01, 0x00], // or eax, 1 << 16 (WP)
        &[0x0F, 0x22, 0xC0],             // mov cr0, rax
        &[0x0F, 0x20, 0xE0],             // mov rax, cr4
        &[0x0D, 0x00, 0x00, 0x80, 0x00], // or eax, 1 << 23 (CET)
        &[0x0F, 0x22, 0xE0],             // mov cr4, rax
        &[0xB9, 0xA2, 0x06, 0x00, 0x00], // mov ecx, 0x6A2 (IA32_S_CET)
        &[0xB8, 0x01, 0x00, 0x00, 0x00], // mov eax, 1 (SH_STK_EN)
        &[0x31, 0xD2],                   // xor edx, edx
        &[0x0F, 0x30],                   // wrmsr
        &[0xB9, 0xA4, 0x06, 0x00, 0x00], // mov ecx, 0x6A4 (IA32_PL0_SSP)
        &mov_eax_token,                  // mov eax, SHADOW_STACK_TOKEN
        &[0x0F, 0x30],                   // wrmsr
        &[0xF3, 0x0F, 0x01, 0xE8],       // setssbsy
    ]
    .concat()
}

/// Where the instructions under test begin.
fn body_start() -> u64 {
    CODE + enable_shadow_stack().len() as u64
}

/// Every handler: `rdsspq rcx`, so RCX holds the SSP it was entered with,
/// then HLT. ModRM 0xC9 names RCX in both its fields.
const HANDLER: [u8; 6] = [0xF3, 0x48, 0x0F, 0x1E, 0xC9, 0xF4];

fn on_big_stack(check: fn()) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(check)
        .expect("spawn")
        .join()
        .expect("join");
}

fn read_u64(emu: &mut Emulator, address: u64) -> u64 {
    let mut bytes = [0u8; 8];
    emu.mem_read(address, &mut bytes).expect("read");
    u64::from_le_bytes(bytes)
}

/// A present 64-bit interrupt gate, DPL 0, to `handler` through the flat
/// code selector.
fn interrupt_gate(handler: u64) -> [u8; 16] {
    let mut gate = [0u8; 16];
    gate[0..2].copy_from_slice(&(handler as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
    gate[5] = 0x8E;
    gate[6..8].copy_from_slice(&((handler >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((handler >> 32) as u32).to_le_bytes());
    gate
}

/// The gate's access byte: P is bit 7.
const GATE_PRESENT: u8 = 0x80;

/// Flat long mode on a model offered CET the way Bochs's `cpuid: cet=1`
/// offers it, with the shadow-stack page mapped, the token in place, and
/// gates for #NP, #GP, #CP and the not-present vector.
fn shadow_stack_machine() -> Box<Emulator> {
    let config = EmulatorConfig {
        cpu_params: BxParams::default().including(X86Feature::IsaCet),
        ..EmulatorConfig::default()
    };
    let mut emu = Emulator::new_with_mode(config, CpuSetupMode::FlatLong64).expect("emulator");

    // The page directory covering the low GiB, found by walking CR3.
    let cr3 = emu.reg_read(X86Reg::Cr3);
    let pdpt = read_u64(&mut emu, cr3 & ENTRY_ADDRESS) & ENTRY_ADDRESS;
    let page_directory = read_u64(&mut emu, pdpt) & ENTRY_ADDRESS;
    let pde = SHADOW_STACK_PAGE | PDE_PRESENT | PDE_DIRTY | PDE_LARGE;
    emu.mem_write(page_directory + (SHADOW_STACK_PAGE >> 21) * 8, &pde.to_le_bytes())
        .expect("pde");
    // Drop every translation cached over the old mapping.
    emu.reg_write(X86Reg::Cr3, cr3);
    // A supervisor token holds its own address with the busy bit clear.
    emu.mem_write(SHADOW_STACK_TOKEN, &SHADOW_STACK_TOKEN.to_le_bytes())
        .expect("token");

    emu.reg_write(X86Reg::IdtrBase, IDT);
    emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
    emu.reg_write(X86Reg::Rsp, STACK_TOP);
    for gate in GATES {
        emu.mem_write(IDT + gate.vector * 16, &interrupt_gate(gate.handler))
            .expect("idt");
        emu.mem_write(gate.handler, &HANDLER).expect("handler");
    }
    let mut not_present = interrupt_gate(GP_HANDLER);
    not_present[5] &= !GATE_PRESENT;
    emu.mem_write(IDT + u64::from(NOT_PRESENT_VECTOR) * 16, &not_present)
        .expect("idt");
    emu
}

/// A vector and the handler its gate leads to.
struct Gate {
    vector: u64,
    handler: u64,
}

const GATES: [Gate; 3] = [
    Gate {
        vector: NP_VECTOR,
        handler: NP_HANDLER,
    },
    Gate {
        vector: GP_VECTOR,
        handler: GP_HANDLER,
    },
    Gate {
        vector: CP_VECTOR,
        handler: CP_HANDLER,
    },
];

/// Which handler the run ended in.
#[derive(Debug, PartialEq, Eq)]
enum Fault {
    NotPresent,
    GeneralProtection,
    ControlProtection,
}

/// What a handler was entered with.
#[derive(Debug, PartialEq, Eq)]
struct Delivered {
    fault: Fault,
    error_code: u64,
    /// The instruction pointer the fault pushed: the faulting instruction's.
    faulting_ip: u64,
    /// The interrupted RSP the 64-bit frame saved.
    stack_pointer: u64,
    /// SSP at handler entry, as the handler's RDSSPQ read it.
    shadow_stack_pointer: u64,
}

/// Enable the shadow stack, run `body`, and require it to end in a handler.
fn run(body: &[u8]) -> Delivered {
    let mut emu = shadow_stack_machine();
    let end = body_start() + body.len() as u64;
    let image = [&enable_shadow_stack()[..], body, &[0xEB, 0xFE]].concat();
    emu.mem_write(CODE, &image).expect("write code");
    emu.emu_start(CODE, Some(end), None, Some(64))
        .expect("execute");
    let rip = emu.cpu().rip();
    let in_handler = |handler: u64| rip == handler + 5 || rip == handler + 6;
    let fault = if in_handler(NP_HANDLER) {
        Fault::NotPresent
    } else if in_handler(GP_HANDLER) {
        Fault::GeneralProtection
    } else if in_handler(CP_HANDLER) {
        Fault::ControlProtection
    } else {
        panic!("the run stopped at {rip:#x}, in none of the handlers");
    };
    // Error code, RIP, CS, RFLAGS, RSP, SS.
    let frame = emu.reg_read(X86Reg::Rsp);
    Delivered {
        fault,
        error_code: read_u64(&mut emu, frame),
        faulting_ip: read_u64(&mut emu, frame + 8),
        stack_pointer: read_u64(&mut emu, frame + 32),
        shadow_stack_pointer: emu.reg_read(X86Reg::Rcx),
    }
}

/// `mov rbx, NON_CANONICAL`: REX.W B8+3 imm64, ten bytes long.
fn load_rbx_non_canonical() -> Vec<u8> {
    [&[0x48, 0xBB][..], &NON_CANONICAL.to_le_bytes()].concat()
}

/// The SSP a fault touching neither stack is delivered with: a load from a
/// non-canonical address, #GP(0).
fn control_shadow_stack_pointer() -> u64 {
    // mov rbx, NON_CANONICAL; mov rax, [rbx]
    let control = run(&[&load_rbx_non_canonical()[..], &[0x48, 0x8B, 0x03]].concat());
    assert_eq!(control.fault, Fault::GeneralProtection);
    assert_eq!(control.faulting_ip, body_start() + 10);
    assert_eq!(control.stack_pointer, STACK_TOP);
    // The handler read a live SSP: the delivery's frame went below the
    // token, onto the shadow-stack page.
    assert!(
        (SHADOW_STACK_PAGE..SHADOW_STACK_TOKEN).contains(&control.shadow_stack_pointer),
        "the handler's SSP {:#x} is not on the shadow stack",
        control.shadow_stack_pointer
    );
    control.shadow_stack_pointer
}

/// A near RET whose return address on the data stack was overwritten after
/// the CALL raises #CP(NEAR_RET), and both its pops are taken back: RSP and
/// SSP are where the RET found them, the CALL's entry still on each (Bochs
/// ctrl_xfer64.cc RETnear64_Iw).
#[test]
fn a_return_to_a_forged_address_raises_cp_with_both_stacks_restored() {
    on_big_stack(|| {
        let control = control_shadow_stack_pointer();
        let body = [
            0xE8, 0x01, 0x00, 0x00, 0x00, // call +1, a non-zero displacement
            0xCC, // the return address, which the CALL skips
            0x48, 0x83, 0x04, 0x24, 0x01, // add qword [rsp], 1
            0xC3, // ret
        ];
        assert_eq!(
            run(&body),
            Delivered {
                fault: Fault::ControlProtection,
                error_code: NEAR_RET,
                faulting_ip: body_start() + 11,
                // The CALL's return address is still on both stacks.
                stack_pointer: STACK_TOP - 8,
                shadow_stack_pointer: control - 8,
            }
        );
    });
}

/// An indirect near CALL to a non-canonical target pushes both stacks before
/// it checks the target; the #GP takes both pushes back (Bochs ctrl_xfer64.cc
/// CALL_EqR).
#[test]
fn a_call_to_a_non_canonical_target_takes_back_its_shadow_push() {
    on_big_stack(|| {
        let control = control_shadow_stack_pointer();
        // mov rbx, NON_CANONICAL; call rbx
        let body = [&load_rbx_non_canonical()[..], &[0xFF, 0xD3]].concat();
        assert_eq!(
            run(&body),
            Delivered {
                fault: Fault::GeneralProtection,
                error_code: 0,
                faulting_ip: body_start() + 10,
                stack_pointer: STACK_TOP,
                shadow_stack_pointer: control,
            }
        );
    });
}

/// INT n through a not-present gate faults during delivery, and the #NP is
/// delivered on the shadow stack INT found (Bochs exception.cc interrupt,
/// RSP_SPECULATIVE).
#[test]
fn a_fault_during_delivery_keeps_the_shadow_stack() {
    on_big_stack(|| {
        let control = control_shadow_stack_pointer();
        let body = [0xCD, NOT_PRESENT_VECTOR]; // int 0x40
        assert_eq!(
            run(&body),
            Delivered {
                fault: Fault::NotPresent,
                // The IDT entry's index, with IDT set and EXT clear.
                error_code: u64::from(NOT_PRESENT_VECTOR) * 8 + 2,
                faulting_ip: body_start(),
                stack_pointer: STACK_TOP,
                shadow_stack_pointer: control,
            }
        );
    });
}
