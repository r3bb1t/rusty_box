//! KSHIFTL / KSHIFTR exist only in the register form.
//!
//! Bochs `cpu/decoder/ia_opcodes.def` defines all eight `BX_IA_KSHIFT*`
//! opcodes with `&BX_CPU_C::BxError` as execute1 — the handler a ModRM memory
//! operand selects — and `fetchdecode_opmap_avx.cc`
//! `BxOpcodeGroup_VEX_0F3A30`..`33` carry no `ATTR_MODC0`. A KSHIFT with
//! mod != 11 therefore decodes, then raises #UD when it executes, before any
//! operand is read or written. The register form shifts the source opmask by
//! the immediate and writes the result, zero-extended, to the destination.

#![cfg(feature = "std")]

use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;
const DATA: u64 = 0x0024_0000;
const IDT_BASE: u64 = 0x0028_0000;
const UD_HANDLER: u64 = 0x0029_0000;
const STACK_TOP: u64 = 0x0030_0000;
const UD_VECTOR: u64 = 6;

/// The source opmask every case shifts. Distinct bits sit at the top of each
/// width (7, 15, 31, 63) and at bit 0, so a wrong width or direction shows.
const K_SRC: u64 = 0x8000_0000_8000_8081;
/// What the destination holds before the instruction runs.
const K_POISON: u64 = 0xDEAD_BEEF_DEAD_BEEF;

/// (name, opcode byte, VEX.W1, destination after a shift of `K_SRC` by 1).
///
/// The opcode byte picks direction and width pair, VEX.W the member of the
/// pair — Bochs `fetchdecode_opmap_avx.cc BxOpcodeGroup_VEX_0F3A30`..`33`.
const CASES: [(&str, u8, bool, u64); 8] = [
    ("kshiftrb", 0x30, false, 0x40),
    ("kshiftrw", 0x30, true, 0x4040),
    ("kshiftrd", 0x31, false, 0x4000_4040),
    ("kshiftrq", 0x31, true, 0x4000_0000_4000_4040),
    ("kshiftlb", 0x32, false, 0x02),
    ("kshiftlw", 0x32, true, 0x0102),
    ("kshiftld", 0x33, false, 0x0001_0102),
    ("kshiftlq", 0x33, true, 0x0000_0001_0001_0102),
];

/// VEX.L0.66.0F3A.W0/W1 `op` /r ib: `C4`, then R̄X̄B̄ = 111 with
/// mmmmm = 00011 (0F3A), then W, v̄v̄v̄v̄ = 1111, L = 0, pp = 01 (66).
fn kshift(op: u8, w1: bool, modrm: u8, imm: u8) -> [u8; 6] {
    let w_vvvv_l_pp = if w1 { 0xF9 } else { 0x79 };
    [0xC4, 0xE3, w_vvvv_l_pp, op, modrm, imm]
}

/// Flat long mode with XCR0 = x87|SSE|YMM|opmask|ZMM_Hi256|Hi16_ZMM, and a
/// #UD gate whose handler is a single HLT.
fn opmask_emulator() -> Box<Emulator> {
    let mut emu = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
        .expect("emulator");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );
    emu.reg_write(X86Reg::Rax, 0xE7);
    emu.reg_write(X86Reg::Rcx, 0);
    emu.reg_write(X86Reg::Rdx, 0);
    emu.mem_write(CODE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
    emu.emu_start(CODE, Some(CODE + 3), None, Some(1))
        .expect("enable AVX-512 state");

    // 16-byte long-mode interrupt gate: selector 0x08, P=1 DPL=0 type 0xE.
    let mut gate = [0u8; 16];
    gate[0..2].copy_from_slice(&(UD_HANDLER as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
    gate[5] = 0x8E;
    gate[6..8].copy_from_slice(&((UD_HANDLER >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((UD_HANDLER >> 32) as u32).to_le_bytes());
    emu.mem_write(IDT_BASE + UD_VECTOR * 16, &gate)
        .expect("#UD gate");
    emu.mem_write(UD_HANDLER, &[0xF4]).expect("#UD handler");
    emu.reg_write(X86Reg::IdtrBase, IDT_BASE);
    emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
    emu.reg_write(X86Reg::Rsp, STACK_TOP);
    emu
}

/// Every register-form KSHIFT executes and writes the shifted source to the
/// destination opmask, zero-extended over all 64 bits.
#[test]
fn kshift_register_form_writes_the_shifted_opmask() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            for (name, op, w1, want) in CASES {
                let mut emu = opmask_emulator();
                emu.reg_write(X86Reg::Opmask2, K_SRC);
                emu.reg_write(X86Reg::Opmask1, K_POISON);

                // ModRM 0xCA: mod = 11, reg = 001 (k1), rm = 010 (k2).
                let code = kshift(op, w1, 0xCA, 1);
                let mut image = code.to_vec();
                image.extend_from_slice(&[0xEB, 0xFE]);
                emu.mem_write(CODE, &image).expect("write code");
                let end = CODE + code.len() as u64;
                emu.emu_start(CODE, Some(end), None, Some(4))
                    .expect("execute");

                assert_eq!(
                    emu.cpu().rip(),
                    end,
                    "{name} k1, k2, 1: the register form must execute, not fault"
                );
                assert_eq!(
                    emu.reg_read(X86Reg::Opmask1),
                    want,
                    "{name} k1, k2, 1: k1 must hold k2 shifted by one at the \
                     instruction's width, zero-extended"
                );
                assert_eq!(
                    emu.reg_read(X86Reg::Opmask2),
                    K_SRC,
                    "{name} k1, k2, 1: the source opmask must be unchanged"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// Every memory-form KSHIFT raises #UD at its own address and leaves the
/// destination opmask untouched — Bochs `ia_opcodes.def` BX_IA_KSHIFT*
/// execute1 = BxError.
#[test]
fn kshift_memory_form_raises_ud() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            for (name, op, w1, _) in CASES {
                let mut emu = opmask_emulator();
                emu.reg_write(X86Reg::Opmask1, K_POISON);
                // A readable, mapped operand holding a shiftable value, so an
                // instruction that loaded it would change k1 visibly.
                emu.mem_write(DATA, &K_SRC.to_le_bytes()).expect("operand");
                emu.reg_write(X86Reg::Rax, DATA);

                // ModRM 0x08: mod = 00, reg = 001 (k1), rm = 000 ([rax]).
                let code = kshift(op, w1, 0x08, 1);
                emu.mem_write(CODE, &code).expect("write code");
                emu.emu_start(CODE, Some(CODE + code.len() as u64), None, Some(1))
                    .expect("execute");

                assert_eq!(
                    emu.cpu().rip(),
                    UD_HANDLER,
                    "{name} k1, [rax], 1: the memory form must be delivered to \
                     the #UD handler"
                );
                assert_eq!(
                    emu.reg_read(X86Reg::Rsp),
                    STACK_TOP - 40,
                    "{name}: a long-mode #UD frame is five qwords, no error code"
                );
                let mut pushed_rip = [0u8; 8];
                emu.mem_read(STACK_TOP - 40, &mut pushed_rip)
                    .expect("read frame");
                assert_eq!(
                    u64::from_le_bytes(pushed_rip),
                    CODE,
                    "{name}: #UD is a fault, so the frame holds the \
                     instruction's own address"
                );
                assert_eq!(
                    emu.reg_read(X86Reg::Opmask1),
                    K_POISON,
                    "{name}: a faulting KSHIFT must not write its destination"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}
