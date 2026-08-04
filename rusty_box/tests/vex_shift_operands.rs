//! VEX/EVEX shift- and rotate-by-immediate read their source from ModRM.rm.
//!
//! Groups 12-14 (`0F 71/72/73`) are non-destructive three-operand instructions
//! under VEX and EVEX: the destination is VEX.vvvv and the source is the rm
//! operand. Bochs ia_opcodes.def leads `V128_VPSRLD_UdqIb` with `OP_Hdq`
//! (vvvv), followed by `OP_Wdq` (rm) and `OP_Ib`.
//!
//! `e8daedf` moved the decoder onto that convention — `dst = vvvv`,
//! `src1 = rm` — but touched no handler. The handlers had been written against
//! the previous layout, where `dst()` still carried rm, so each of them kept
//! reading its source operand out of `dst()`. Once `dst()` became vvvv that is
//! the *destination* register, and every one of these instructions started
//! computing `dst = dst OP imm`, ignoring its real source.
//!
//! Windows 7 SP1 supports AVX and runs VEX-encoded shifts, so this stalled its
//! boot; Linux guests happened not to hit the affected paths.

#![cfg(feature = "std")]

use rusty_box::cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;

fn avx_emulator() -> Box<Emulator<'static, Corei7SkylakeX>> {
    let cfg = EmulatorConfig::default();
    let mut emu =
        Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64).expect("emulator");
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );
    emu.reg_write(X86Reg::Rax, 0xE7);
    emu.reg_write(X86Reg::Rcx, 0);
    emu.reg_write(X86Reg::Rdx, 0);
    emu.mem_write(CODE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
    emu.emu_start(CODE, Some(CODE + 3), None, Some(1))
        .expect("enable AVX state");
    emu
}

fn run(emu: &mut Emulator<'static, Corei7SkylakeX>, code: &[u8], steps: u64) {
    let park = CODE + code.len() as u64;
    let mut image = code.to_vec();
    image.extend_from_slice(&[0xEB, 0xFE]);
    emu.mem_write(CODE, &image).expect("write code");
    emu.emu_start(CODE, Some(park), None, Some(steps))
        .expect("execute");
    assert_eq!(emu.cpu().rip(), park, "must reach the park jump");
}

/// `VPSRLD xmm0, xmm1, 4` must shift **xmm1** into xmm0 and leave xmm1 alone.
///
/// Reading the source from `dst()` turns this into `xmm0 = xmm0 >> 4`, which
/// with a poisoned destination produces a value bearing no relation to the
/// source at all.
#[test]
fn vex_shift_by_immediate_reads_its_source_from_rm() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = avx_emulator();

            // xmm1 = four dwords of 0x8000_0000; xmm0 poisoned.
            let mut src = [0u8; 64];
            for d in 0..4 {
                src[d * 4..d * 4 + 4].copy_from_slice(&0x8000_0000u32.to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm1, src);
            emu.reg_write_zmm(X86Reg::Zmm0, [0xAA; 64]);

            // vpsrld xmm0, xmm1, 4  =  C5 F9 72 D1 04
            //   C5 F9: 2-byte VEX, vvvv=1111 -> xmm0, L=0, pp=01 (66)
            //   72 /2: PSRLD group; ModRM D1 = mod 11, reg 010 (/2), rm 001 (xmm1)
            run(&mut emu, &[0xC5, 0xF9, 0x72, 0xD1, 0x04], 4);

            let got = emu.reg_read_zmm(X86Reg::Zmm0);
            for d in 0..4usize {
                let v = u32::from_le_bytes(got[d * 4..d * 4 + 4].try_into().unwrap());
                assert_eq!(
                    v, 0x0800_0000,
                    "dword {d}: VPSRLD must shift its rm source (0x80000000 >> 4), \
                     not the destination register"
                );
            }
            assert!(
                got[16..32].iter().all(|&x| x == 0),
                "a VEX.128 result must zero the upper lane"
            );

            // The source register must be untouched.
            let src_after = emu.reg_read_zmm(X86Reg::Zmm1);
            for d in 0..4usize {
                let v = u32::from_le_bytes(src_after[d * 4..d * 4 + 4].try_into().unwrap());
                assert_eq!(v, 0x8000_0000, "dword {d}: the source must not be modified");
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// The same convention holds for the word and qword widths, and for the
/// byte-granular `VPSLLDQ`/`VPSRLDQ` pair that shares the group.
#[test]
fn vex_shift_family_all_read_rm() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            // (encoding, expected first qword of xmm0)
            // xmm1 is always 0x8000_0000_8000_0000 per qword.
            let cases: &[(&[u8], u64, &str)] = &[
                // vpsrlq xmm0, xmm1, 4   C5 F9 73 D1 04  (/2)
                (&[0xC5, 0xF9, 0x73, 0xD1, 0x04], 0x0800_0000_0800_0000, "vpsrlq"),
                // vpsllq xmm0, xmm1, 4   C5 F9 73 F1 04  (/6)
                (&[0xC5, 0xF9, 0x73, 0xF1, 0x04], 0x0000_0008_0000_0000, "vpsllq"),
                // vpslld xmm0, xmm1, 4   C5 F9 72 F1 04  (/6)
                (&[0xC5, 0xF9, 0x72, 0xF1, 0x04], 0x0000_0000_0000_0000, "vpslld"),
                // vpsrldq xmm0, xmm1, 4  C5 F9 73 D9 04  (/3)
                (&[0xC5, 0xF9, 0x73, 0xD9, 0x04], 0x8000_0000_8000_0000, "vpsrldq"),
            ];

            for (code, want_q0, name) in cases {
                let mut emu = avx_emulator();
                let mut src = [0u8; 64];
                for q in 0..2 {
                    src[q * 8..q * 8 + 8]
                        .copy_from_slice(&0x8000_0000_8000_0000u64.to_le_bytes());
                }
                emu.reg_write_zmm(X86Reg::Zmm1, src);
                emu.reg_write_zmm(X86Reg::Zmm0, [0xAA; 64]);

                run(&mut emu, code, 4);

                let got = emu.reg_read_zmm(X86Reg::Zmm0);
                let q0 = u64::from_le_bytes(got[0..8].try_into().unwrap());
                assert_eq!(
                    q0, *want_q0,
                    "{name}: must operate on its rm source, not on the destination"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}
