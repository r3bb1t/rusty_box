//! SSE2 PSRLQ and PSLLQ zero every quadword for any count above 63.
//!
//! The SDM clears the destination of a logical quadword shift whenever the
//! count exceeds 63. Bochs `cpu/simd_int.h xmm_psllq` bounds at `> 63`;
//! `xmm_psrlq` bounds at `> 64`, which leaves a count of exactly 64 to a C++
//! shift by the operand's full width — undefined behaviour, not a modelled
//! result. The port answers 64 with the SDM's zero, the same bound as its VEX
//! and EVEX forms; `docs/bochs-parity-divergences.md` D9 registers that
//! choice. A count of 63 is still an ordinary shift.

#![cfg(feature = "std")]

use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;

/// The destination's two quadwords before every shift. Bit 63 and bit 0 are
/// both set in each, so a shift by 63 in either direction leaves one bit.
const DST: [u64; 2] = [0x8000_0000_0000_0001, 0xC000_0000_0000_0003];

/// Flat long mode with CR4.OSFXSR set, so legacy SSE executes.
fn sse_emulator() -> Box<Emulator> {
    let mut emu = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
        .expect("emulator");
    emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | (1 << 9));
    emu
}

fn xmm_bytes(q: [u64; 2]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&q[0].to_le_bytes());
    bytes[8..16].copy_from_slice(&q[1].to_le_bytes());
    bytes
}

/// Run `code` with xmm0 = `DST` and xmm1's low quadword = `count` (its high
/// quadword all ones, which the count must ignore); return xmm0's quadwords.
fn shift_xmm0(code: &[u8], count: u64) -> [u64; 2] {
    let mut emu = sse_emulator();
    emu.reg_write_xmm(X86Reg::Xmm0, xmm_bytes(DST));
    emu.reg_write_xmm(X86Reg::Xmm1, xmm_bytes([count, u64::MAX]));
    let mut image = code.to_vec();
    image.extend_from_slice(&[0xEB, 0xFE]);
    emu.mem_write(CODE, &image).expect("write code");
    let end = CODE + code.len() as u64;
    emu.emu_start(CODE, Some(end), None, Some(4))
        .expect("execute");
    assert_eq!(emu.cpu().rip(), end, "the shift must execute, not fault");
    let got = emu.reg_read_xmm(X86Reg::Xmm0);
    [
        u64::from_le_bytes(got[0..8].try_into().expect("8 bytes")),
        u64::from_le_bytes(got[8..16].try_into().expect("8 bytes")),
    ]
}

/// `psrlq xmm0, imm8` = 66 0F 73 /2 ib, ModRM 0xD0 (mod 11, /2, rm xmm0).
fn psrlq_imm(count: u8) -> [u8; 5] {
    [0x66, 0x0F, 0x73, 0xD0, count]
}

/// `psllq xmm0, imm8` = 66 0F 73 /6 ib, ModRM 0xF0 (mod 11, /6, rm xmm0).
fn psllq_imm(count: u8) -> [u8; 5] {
    [0x66, 0x0F, 0x73, 0xF0, count]
}

/// `psrlq xmm0, xmm1` = 66 0F D3 /r, ModRM 0xC1 (reg xmm0, rm xmm1).
const PSRLQ_XMM: [u8; 4] = [0x66, 0x0F, 0xD3, 0xC1];
/// `psllq xmm0, xmm1` = 66 0F F3 /r, ModRM 0xC1 (reg xmm0, rm xmm1).
const PSLLQ_XMM: [u8; 4] = [0x66, 0x0F, 0xF3, 0xC1];

/// Each quadword's bit 0 after a left shift by 63.
const TOP: u64 = 0x8000_0000_0000_0000;

/// Run `check` on a thread with room for an `Emulator`.
fn on_big_stack(check: fn()) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(check)
        .expect("spawn")
        .join()
        .expect("join");
}

#[test]
fn psrlq_by_immediate_64_zeroes_and_63_shifts() {
    on_big_stack(|| {
        assert_eq!(
            shift_xmm0(&psrlq_imm(64), 0),
            [0, 0],
            "psrlq xmm0, 64 must zero both quadwords"
        );
        assert_eq!(
            shift_xmm0(&psrlq_imm(63), 0),
            [1, 1],
            "psrlq xmm0, 63 must leave each quadword's bit 63 in bit 0"
        );
    });
}

#[test]
fn psrlq_by_register_64_zeroes_and_63_shifts() {
    on_big_stack(|| {
        assert_eq!(
            shift_xmm0(&PSRLQ_XMM, 64),
            [0, 0],
            "psrlq xmm0, xmm1 with a count of 64 must zero both quadwords"
        );
        assert_eq!(
            shift_xmm0(&PSRLQ_XMM, 63),
            [1, 1],
            "psrlq xmm0, xmm1 with a count of 63 must leave each quadword's \
             bit 63 in bit 0"
        );
    });
}

#[test]
fn psllq_by_immediate_64_zeroes_and_63_shifts() {
    on_big_stack(|| {
        assert_eq!(
            shift_xmm0(&psllq_imm(64), 0),
            [0, 0],
            "psllq xmm0, 64 must zero both quadwords"
        );
        assert_eq!(
            shift_xmm0(&psllq_imm(63), 0),
            [TOP, TOP],
            "psllq xmm0, 63 must leave each quadword's bit 0 in bit 63"
        );
    });
}

#[test]
fn psllq_by_register_64_zeroes_and_63_shifts() {
    on_big_stack(|| {
        assert_eq!(
            shift_xmm0(&PSLLQ_XMM, 64),
            [0, 0],
            "psllq xmm0, xmm1 with a count of 64 must zero both quadwords"
        );
        assert_eq!(
            shift_xmm0(&PSLLQ_XMM, 63),
            [TOP, TOP],
            "psllq xmm0, xmm1 with a count of 63 must leave each quadword's \
             bit 0 in bit 63"
        );
    });
}
