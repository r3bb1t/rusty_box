//! Integer SIMD instructions that share one lane body between their legacy
//! SSE and VEX forms (Bochs cpu/simd_int.h, run through `HANDLE_SSE_2OP` and
//! `HANDLE_AVX_2OP`), run as guest instructions.
//!
//! Every expected value below is written out by hand from the instruction's
//! definition, not computed by the code under test.

#![cfg(feature = "std")]

use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;
/// CR4.OSFXSR (legacy SSE) and CR4.OSXSAVE (XSETBV, and with it VEX).
const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXSAVE: u64 = 1 << 18;

/// XCR0 = x87 | SSE | AVX, so a VEX instruction executes.
const ENABLE_AVX: [u8; 12] = [
    0x31, 0xC9, // xor ecx, ecx
    0xB8, 0x07, 0x00, 0x00, 0x00, // mov eax, 7
    0x31, 0xD2, // xor edx, edx
    0x0F, 0x01, 0xD1, // xsetbv
];

fn emulator() -> Box<Emulator> {
    let mut emu = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
        .expect("emulator");
    emu.reg_write(X86Reg::Cr4, emu.reg_read(X86Reg::Cr4) | CR4_OSFXSR | CR4_OSXSAVE);
    emu
}

/// Run `code` after enabling AVX state, and require it to reach its end.
fn run(emu: &mut Emulator, code: &[u8]) {
    let mut image = [&ENABLE_AVX[..], code].concat();
    let end = CODE + image.len() as u64;
    image.extend_from_slice(&[0xEB, 0xFE]);
    emu.mem_write(CODE, &image).expect("write code");
    emu.emu_start(CODE, Some(end), None, Some(64))
        .expect("execute");
    assert_eq!(emu.cpu().rip(), end, "the sequence must run to its end, not fault");
}

fn on_big_stack(check: fn()) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(check)
        .expect("spawn")
        .join()
        .expect("join");
}

fn from_words(words: [u16; 8]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for (n, word) in words.iter().enumerate() {
        bytes[n * 2..n * 2 + 2].copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn to_words(bytes: [u8; 16]) -> [u16; 8] {
    core::array::from_fn(|n| u16::from_le_bytes([bytes[n * 2], bytes[n * 2 + 1]]))
}

fn from_dwords(dwords: [u32; 4]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for (n, dword) in dwords.iter().enumerate() {
        bytes[n * 4..n * 4 + 4].copy_from_slice(&dword.to_le_bytes());
    }
    bytes
}

fn to_dwords(bytes: [u8; 16]) -> [u32; 4] {
    core::array::from_fn(|n| {
        u32::from_le_bytes([bytes[n * 4], bytes[n * 4 + 1], bytes[n * 4 + 2], bytes[n * 4 + 3]])
    })
}

fn from_qwords(qwords: [u64; 2]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&qwords[0].to_le_bytes());
    bytes[8..16].copy_from_slice(&qwords[1].to_le_bytes());
    bytes
}

fn to_qwords(bytes: [u8; 16]) -> [u64; 2] {
    [
        u64::from_le_bytes(bytes[0..8].try_into().expect("8 bytes")),
        u64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes")),
    ]
}

/// A 256-bit register as its two 128-bit lanes.
struct Lanes {
    low: [u8; 16],
    high: [u8; 16],
}

fn ymm(lanes: &Lanes) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(&lanes.low);
    bytes[16..].copy_from_slice(&lanes.high);
    bytes
}

fn lanes(bytes: [u8; 32]) -> Lanes {
    Lanes {
        low: bytes[..16].try_into().expect("16 bytes"),
        high: bytes[16..].try_into().expect("16 bytes"),
    }
}

/// `op xmm0, xmm1` (ModRM 0xC1) on the given xmm0 and xmm1; xmm0 afterwards.
fn legacy_2op(opcode: &[u8], xmm0: [u8; 16], xmm1: [u8; 16]) -> [u8; 16] {
    let mut emu = emulator();
    emu.reg_write_xmm(X86Reg::Xmm0, xmm0);
    emu.reg_write_xmm(X86Reg::Xmm1, xmm1);
    run(&mut emu, &[opcode, &[0xC1]].concat());
    emu.reg_read_xmm(X86Reg::Xmm0)
}

/// SSSE3 horizontal adds and subtracts combine adjacent elements, xmm0's
/// pairs into the low half and xmm1's into the high half.
#[test]
fn horizontal_ops_combine_adjacent_elements() {
    on_big_stack(|| {
        let phaddw = legacy_2op(
            &[0x66, 0x0F, 0x38, 0x01],
            from_words([1, 2, 3, 4, 5, 6, 7, 8]),
            from_words([10, 20, 30, 40, 50, 60, 70, 80]),
        );
        assert_eq!(to_words(phaddw), [3, 7, 11, 15, 30, 70, 110, 150], "phaddw");

        let phaddd = legacy_2op(
            &[0x66, 0x0F, 0x38, 0x02],
            from_dwords([1, 2, 3, 4]),
            from_dwords([10, 20, 30, 40]),
        );
        assert_eq!(to_dwords(phaddd), [3, 7, 30, 70], "phaddd");

        // 0x7FFF + 1 and -0x8000 + -1 saturate; the rest are in range.
        let phaddsw = legacy_2op(
            &[0x66, 0x0F, 0x38, 0x03],
            from_words([0x7FFF, 1, 0x8000, 0xFFFF, 2, 3, 0, 0]),
            from_words([0, 0, 0, 0, 0, 0, 5, 0xFFFF]),
        );
        assert_eq!(to_words(phaddsw), [0x7FFF, 0x8000, 5, 0, 0, 0, 0, 4], "phaddsw");

        // Wrapping: 0 - 1 is 0xFFFF.
        let phsubw = legacy_2op(
            &[0x66, 0x0F, 0x38, 0x05],
            from_words([5, 3, 0, 1, 9, 9, 0, 0]),
            from_words([100, 1, 0, 0, 0, 0, 0, 0]),
        );
        assert_eq!(to_words(phsubw), [2, 0xFFFF, 0, 0, 99, 0, 0, 0], "phsubw");

        let phsubd = legacy_2op(
            &[0x66, 0x0F, 0x38, 0x06],
            from_dwords([5, 7, 1, 1]),
            from_dwords([9, 4, 0, 0]),
        );
        assert_eq!(to_dwords(phsubd), [0xFFFF_FFFE, 0, 5, 0], "phsubd");

        // 0x7FFF - (-1) and -0x8000 - 1 saturate.
        let phsubsw = legacy_2op(
            &[0x66, 0x0F, 0x38, 0x07],
            from_words([0x7FFF, 0xFFFF, 0x8000, 1, 0, 0, 0, 0]),
            from_words([0; 8]),
        );
        assert_eq!(to_words(phsubsw), [0x7FFF, 0x8000, 0, 0, 0, 0, 0, 0], "phsubsw");
    });
}

/// SSE4.1's signed and unsigned minimum and maximum per element.
#[test]
fn sse41_min_and_max_compare_by_the_right_signedness() {
    on_big_stack(|| {
        let a = from_dwords([0xFFFF_FFFF, 5, 0x8000_0000, 0x7FFF_FFFF]);
        let b = from_dwords([1, 0xFFFF_FFFB, 0, 0x8000_0000]);
        assert_eq!(
            to_dwords(legacy_2op(&[0x66, 0x0F, 0x38, 0x39], a, b)),
            [0xFFFF_FFFF, 0xFFFF_FFFB, 0x8000_0000, 0x8000_0000],
            "pminsd"
        );
        assert_eq!(
            to_dwords(legacy_2op(&[0x66, 0x0F, 0x38, 0x3D], a, b)),
            [1, 5, 0, 0x7FFF_FFFF],
            "pmaxsd"
        );

        let a = from_words([1, 0xFFFF, 0x8000, 7, 0, 0, 0, 0]);
        let b = from_words([2, 1, 0x7FFF, 7, 0, 0, 0, 0]);
        assert_eq!(
            to_words(legacy_2op(&[0x66, 0x0F, 0x38, 0x3A], a, b)),
            [1, 1, 0x7FFF, 7, 0, 0, 0, 0],
            "pminuw"
        );
        assert_eq!(
            to_words(legacy_2op(&[0x66, 0x0F, 0x38, 0x3E], a, b)),
            [2, 0xFFFF, 0x8000, 7, 0, 0, 0, 0],
            "pmaxuw"
        );

        let mut a = [0u8; 16];
        let mut b = [0u8; 16];
        a[..3].copy_from_slice(&[0xFF, 5, 0x80]); // -1, 5, -128
        b[..3].copy_from_slice(&[1, 0xFB, 0x7F]); // 1, -5, 127
        assert_eq!(
            legacy_2op(&[0x66, 0x0F, 0x38, 0x38], a, b)[..3],
            [0xFF, 0xFB, 0x80],
            "pminsb"
        );
        assert_eq!(
            legacy_2op(&[0x66, 0x0F, 0x38, 0x3C], a, b)[..3],
            [1, 5, 0x7F],
            "pmaxsb"
        );
    });
}

/// PCMPEQQ and PCMPGTQ (signed) set a whole quadword per element.
#[test]
fn quadword_compares_fill_each_element() {
    on_big_stack(|| {
        let a = from_qwords([u64::MAX, 2]);
        let b = from_qwords([1, 2]);
        assert_eq!(to_qwords(legacy_2op(&[0x66, 0x0F, 0x38, 0x29], a, b)), [0, u64::MAX], "pcmpeqq");
        // -1 > 1 is false; 2 > 1 is true.
        let c = from_qwords([1, 1]);
        assert_eq!(
            to_qwords(legacy_2op(&[0x66, 0x0F, 0x38, 0x37], from_qwords([u64::MAX, 2]), c)),
            [0, u64::MAX],
            "pcmpgtq"
        );
    });
}

/// PACKUSDW clamps signed dwords to unsigned words, xmm0's into the low half.
#[test]
fn packusdw_clamps_to_unsigned_words() {
    on_big_stack(|| {
        let packed = legacy_2op(
            &[0x66, 0x0F, 0x38, 0x2B],
            from_dwords([0xFFFF_FFFB, 70_000, 100, 65_535]),
            from_dwords([0, 1, 0xFFFF_FFFF, 0x1_0000]),
        );
        assert_eq!(to_words(packed), [0, 0xFFFF, 100, 0xFFFF, 0, 1, 0, 0xFFFF]);
    });
}

/// EXTRACTPS writes the selected dword of an XMM register to a GPR.
#[test]
fn extractps_writes_the_selected_dword() {
    on_big_stack(|| {
        let mut emu = emulator();
        emu.reg_write_xmm(X86Reg::Xmm0, from_dwords([0x1111, 0x2222, 0x3333, 0x4444]));
        emu.reg_write(X86Reg::Rax, u64::MAX);
        // extractps eax, xmm0, 2: 66 0F 3A 17 /r ib, ModRM 0xC0.
        run(&mut emu, &[0x66, 0x0F, 0x3A, 0x17, 0xC0, 0x02]);
        assert_eq!(emu.reg_read(X86Reg::Rax), 0x3333);
    });
}

/// The VEX forms take their first source from VEX.vvvv, so the destination
/// can be a third register, and clear every bit above the vector length.
#[test]
fn vex_forms_read_vvvv_and_clear_above_the_vector_length() {
    on_big_stack(|| {
        let mut emu = emulator();
        emu.reg_write_xmm(X86Reg::Xmm0, from_words([1, 3, 0xFFFF, 0, 0, 0, 0, 0]));
        emu.reg_write_xmm(X86Reg::Xmm1, from_words([2, 3, 0xFFFF, 1, 0, 0, 0, 0]));
        emu.reg_write_ymm(X86Reg::Ymm2, [0xAA; 32]);
        // vpavgw xmm2, xmm0, xmm1: VEX.128.66.0F E3 /r (C5 F9), ModRM 0xD1.
        run(&mut emu, &[0xC5, 0xF9, 0xE3, 0xD1]);
        let result = lanes(emu.reg_read_ymm(X86Reg::Ymm2));
        assert_eq!(to_words(result.low), [2, 3, 0xFFFF, 1, 0, 0, 0, 0], "rounded averages");
        assert_eq!(result.high, [0; 16], "VEX.128 clears bits 255:128");
    });
}

/// At VL256 each 128-bit lane is combined on its own: the high lane's
/// horizontal sums come from the high lanes of both sources.
#[test]
fn vex_256_operates_on_each_lane_separately() {
    on_big_stack(|| {
        let mut emu = emulator();
        emu.reg_write_ymm(
            X86Reg::Ymm0,
            ymm(&Lanes {
                low: from_words([1, 1, 2, 2, 3, 3, 4, 4]),
                high: from_words([10, 10, 20, 20, 30, 30, 40, 40]),
            }),
        );
        emu.reg_write_ymm(
            X86Reg::Ymm1,
            ymm(&Lanes {
                low: from_words([5, 5, 6, 6, 7, 7, 8, 8]),
                high: from_words([50, 50, 60, 60, 70, 70, 80, 80]),
            }),
        );
        // vphaddw ymm2, ymm0, ymm1: VEX.256.66.0F38 01 /r (C4 E2 7D), ModRM 0xD1.
        run(&mut emu, &[0xC4, 0xE2, 0x7D, 0x01, 0xD1]);
        let result = lanes(emu.reg_read_ymm(X86Reg::Ymm2));
        assert_eq!(to_words(result.low), [2, 4, 6, 8, 10, 12, 14, 16]);
        assert_eq!(to_words(result.high), [20, 40, 60, 80, 100, 120, 140, 160]);

        // vpackusdw ymm2, ymm0, ymm1: VEX.256.66.0F38 2B /r.
        emu.reg_write_ymm(
            X86Reg::Ymm0,
            ymm(&Lanes {
                low: from_dwords([1, 2, 3, 4]),
                high: from_dwords([0xFFFF_FFFF, 0x2_0000, 9, 10]),
            }),
        );
        emu.reg_write_ymm(
            X86Reg::Ymm1,
            ymm(&Lanes { low: from_dwords([5, 6, 7, 8]), high: from_dwords([11, 12, 13, 14]) }),
        );
        run(&mut emu, &[0xC4, 0xE2, 0x7D, 0x2B, 0xD1]);
        let result = lanes(emu.reg_read_ymm(X86Reg::Ymm2));
        assert_eq!(to_words(result.low), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(to_words(result.high), [0, 0xFFFF, 9, 10, 11, 12, 13, 14]);
    });
}
