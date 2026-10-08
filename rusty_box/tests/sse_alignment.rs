//! Which 128-bit memory operands must be 16-byte aligned, run as guest
//! instructions.
//!
//! Bochs gives every legacy SSE opcode with a 128-bit memory operand a loader
//! in decoder/ia_opcodes.def: `LOAD_Wdq` (load.cc), which raises #GP(0) on a
//! misaligned address unless MXCSR.MM is set, for all of them but the four
//! string compares, which use `LOADU_Wdq` and never check. A VEX encoding
//! loads with `LOAD_Vector`, which never checks either — including the VEX
//! forms this decoder keeps on their legacy opcode, such as VAESENC.

#![cfg(feature = "std")]

use rusty_box::cpu::decoder::X86Feature;
use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};
use rusty_box::params::BxParams;

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;
/// 16-byte aligned, so `DATA + 8` is a misaligned 16-byte operand.
const DATA: u64 = 0x0021_0000;
const MISALIGNED: u64 = DATA + 8;
/// The 32-bit image LDMXCSR loads.
const MXCSR_IMAGE: u64 = 0x0021_1000;
const IDT: u64 = 0x0024_0000;
/// #GP handler: a HLT the run stops on.
const GP_HANDLER: u64 = 0x0025_0000;
const STACK_TOP: u64 = 0x0026_0000;
/// CR4.OSFXSR (legacy SSE) and CR4.OSXSAVE (XSETBV, and with it VEX).
const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXSAVE: u64 = 1 << 18;

/// Flat long mode with SSE and AVX state enabled and a #GP gate, on the
/// default model with `cpu_params` applied.
fn sse_emulator_with(cpu_params: BxParams) -> Box<Emulator> {
    let config = EmulatorConfig {
        cpu_params,
        ..EmulatorConfig::default()
    };
    let mut emu = Emulator::new_with_mode(config, CpuSetupMode::FlatLong64).expect("emulator");
    emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | CR4_OSFXSR | CR4_OSXSAVE);
    emu.reg_write(X86Reg::IdtrBase, IDT);
    emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
    emu.reg_write(X86Reg::Rsp, STACK_TOP);
    let mut gate = [0u8; 16];
    gate[0..2].copy_from_slice(&(GP_HANDLER as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
    gate[5] = 0x8E;
    gate[6..8].copy_from_slice(&((GP_HANDLER >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((GP_HANDLER >> 32) as u32).to_le_bytes());
    emu.mem_write(IDT + 13 * 16, &gate).expect("idt");
    emu.mem_write(GP_HANDLER, &[0xF4]).expect("handler");
    emu.mem_write(DATA, &[0x11; 32]).expect("data");
    emu
}

/// [`sse_emulator_with`] on the default model as it ships.
fn sse_emulator() -> Box<Emulator> {
    sse_emulator_with(BxParams::default())
}

/// A ModRM `[disp32]` operand through SIB 0x25, register field `reg`.
fn absolute(reg: u8, address: u64) -> Vec<u8> {
    let disp = u32::try_from(address).expect("below 4 GiB").to_le_bytes();
    [&[(reg << 3) | 0x04, 0x25][..], &disp].concat()
}

/// XCR0 = x87 | SSE | AVX, so a VEX instruction executes.
const ENABLE_AVX: [u8; 13] = [
    0x31, 0xC9, // xor ecx, ecx
    0xB8, 0x07, 0x00, 0x00, 0x00, // mov eax, 7
    0x31, 0xD2, // xor edx, edx
    0x0F, 0x01, 0xD1, // xsetbv
    0x90, // nop
];

/// How a run of one instruction ended.
#[derive(Debug, PartialEq, Eq)]
enum Ended {
    /// It retired and the run reached the end of the sequence.
    Retired,
    /// It raised #GP and the handler was entered.
    GeneralProtection,
}

/// Run `prologue` and then `instruction` on a fresh [`sse_emulator`], and say
/// how it ended.
fn run(prologue: &[u8], instruction: &[u8]) -> Ended {
    run_on(&mut sse_emulator(), prologue, instruction)
}

/// Run `prologue` and then `instruction` on `emu`, and say how it ended.
fn run_on(emu: &mut Emulator, prologue: &[u8], instruction: &[u8]) -> Ended {
    let mut image = [prologue, instruction].concat();
    let end = CODE + image.len() as u64;
    image.extend_from_slice(&[0xEB, 0xFE]);
    emu.mem_write(CODE, &image).expect("write code");
    emu.emu_start(CODE, Some(end), None, Some(64))
        .expect("execute");
    let rip = emu.cpu().rip();
    if rip == end {
        Ended::Retired
    } else if rip == GP_HANDLER || rip == GP_HANDLER + 1 {
        Ended::GeneralProtection
    } else {
        panic!("the run stopped at {rip:#x}, neither the end nor the #GP handler");
    }
}

fn on_big_stack(check: fn()) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(check)
        .expect("spawn")
        .join()
        .expect("join");
}

/// PADDB's memory operand is aligned: a misaligned one raises #GP(0).
#[test]
fn a_legacy_packed_operand_must_be_aligned() {
    on_big_stack(|| {
        // paddb xmm0, [m128]: 66 0F FC /r
        let paddb = |at| [&[0x66, 0x0F, 0xFC][..], &absolute(0, at)].concat();
        assert_eq!(run(&[], &paddb(DATA)), Ended::Retired);
        assert_eq!(run(&[], &paddb(MISALIGNED)), Ended::GeneralProtection);
    });
}

/// The packed-float forms load the same way: ADDPS and MOVSLDUP.
#[test]
fn legacy_packed_float_operands_must_be_aligned() {
    on_big_stack(|| {
        // addps xmm0, [m128]: 0F 58 /r; movsldup xmm0, [m128]: F3 0F 12 /r
        let addps = [&[0x0F, 0x58][..], &absolute(0, MISALIGNED)].concat();
        let movsldup = [&[0xF3, 0x0F, 0x12][..], &absolute(0, MISALIGNED)].concat();
        assert_eq!(run(&[], &addps), Ended::GeneralProtection);
        assert_eq!(run(&[], &movsldup), Ended::GeneralProtection);
    });
}

/// The string compares load with `LOADU_Wdq`: no alignment rule.
#[test]
fn a_string_compare_operand_may_be_misaligned() {
    on_big_stack(|| {
        // pcmpistri xmm0, [m128], 0: 66 0F 3A 63 /r ib
        let pcmpistri = [&[0x66, 0x0F, 0x3A, 0x63][..], &absolute(0, MISALIGNED), &[0x00]].concat();
        assert_eq!(run(&[], &pcmpistri), Ended::Retired);
    });
}

/// AESENC's legacy form is aligned, and its VEX form is not, though both
/// reach the same handler here.
#[test]
fn aesenc_is_aligned_only_in_its_legacy_encoding() {
    on_big_stack(|| {
        // aesenc xmm0, [m128]: 66 0F 38 DC /r
        let legacy = [&[0x66, 0x0F, 0x38, 0xDC][..], &absolute(0, MISALIGNED)].concat();
        // vaesenc xmm0, xmm0, [m128]: VEX.128.66.0F38.WIG DC /r (C4 E2 79)
        let vex = [&[0xC4, 0xE2, 0x79, 0xDC][..], &absolute(0, MISALIGNED)].concat();
        assert_eq!(run(&[], &legacy), Ended::GeneralProtection);
        assert_eq!(run(&ENABLE_AVX, &vex), Ended::Retired);
    });
}

/// On a processor offered misaligned SSE (Bochs features.h
/// `BX_ISA_MISALIGNED_SSE`), LDMXCSR accepts MXCSR.MM (init.cc widens
/// `mxcsr_mask` by it), and with MM set `LOAD_Wdq` lets a misaligned operand
/// through. MM clear, the same processor still raises #GP.
#[test]
fn mxcsr_mm_lets_a_legacy_operand_be_misaligned() {
    on_big_stack(|| {
        const MXCSR_RESET: u32 = 0x1F80;
        const MXCSR_MM: u32 = 1 << 17;
        let misaligned_sse =
            || sse_emulator_with(BxParams::default().including(X86Feature::IsaMisalignedSse));
        // paddb xmm0, [m128]: 66 0F FC /r
        let paddb = [&[0x66, 0x0F, 0xFC][..], &absolute(0, MISALIGNED)].concat();
        // ldmxcsr [m32]: 0F AE /2
        let ldmxcsr = [&[0x0F, 0xAE][..], &absolute(2, MXCSR_IMAGE)].concat();

        assert_eq!(
            run_on(&mut misaligned_sse(), &[], &paddb),
            Ended::GeneralProtection,
            "MXCSR.MM clear"
        );

        let mut emu = misaligned_sse();
        emu.mem_write(MXCSR_IMAGE, &(MXCSR_RESET | MXCSR_MM).to_le_bytes())
            .expect("mxcsr image");
        assert_eq!(run_on(&mut emu, &ldmxcsr, &paddb), Ended::Retired);
        assert_eq!(
            emu.reg_read(X86Reg::Mxcsr),
            u64::from(MXCSR_RESET | MXCSR_MM),
            "LDMXCSR took MM"
        );
        // XMM0 was zero, so it now holds the misaligned operand's bytes.
        assert_eq!(emu.reg_read_xmm(X86Reg::Xmm0), [0x11; 16]);
    });
}
