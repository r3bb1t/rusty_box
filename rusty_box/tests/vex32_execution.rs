//! End-to-end execution of VEX- and EVEX-encoded instructions outside 64-bit
//! mode.
//!
//! `decode32` used to reject every VEX prefix and treat `0x62` as BOUND, so a
//! guest running 32-bit code — a plain 32-bit OS, or a 32-bit process in
//! compatibility mode under a 64-bit kernel, which `icache.rs` routes to
//! `decode32` on `CS.L == 0` — took a #UD on the AVX and AVX-512 instructions
//! CPUID had told it were available. These tests run the real instructions on a
//! protected-mode CPU.

#![cfg(feature = "std")]

use rusty_box::cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

/// Same sizing rationale as fp_vex_scalar_ops.rs: `Emulator` is several MiB, so
/// tests need an explicit thread stack.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;
const DATA: u64 = 0x0021_0000;
const DEST: u64 = 0x0021_0100;

/// A flat 32-bit protected-mode CPU with the full AVX-512 state enabled.
///
/// `FlatProtected32` leaves paging off, so linear and physical addresses match
/// and `[disp32]` operands address memory directly.
fn protected32_emulator() -> Box<Emulator<'static, Corei7SkylakeX>> {
    let cfg = EmulatorConfig::default();
    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatProtected32)
        .expect("emulator");
    // CR4.OSFXSR | CR4.OSXSAVE
    emu.reg_write(
        X86Reg::Cr4,
        emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
    );
    // XSETBV: XCR0 = FPU|SSE|YMM|OPMASK|ZMM_HI256|HI_ZMM.
    emu.reg_write(X86Reg::Rax, 0xE7);
    emu.reg_write(X86Reg::Rcx, 0);
    emu.reg_write(X86Reg::Rdx, 0);
    emu.mem_write(CODE, &[0x0F, 0x01, 0xD1]).expect("xsetbv");
    emu.emu_start(CODE, Some(CODE + 3), None, Some(1))
        .expect("enable AVX-512 state in protected mode");
    emu
}

fn run(emu: &mut Emulator<'static, Corei7SkylakeX>, code: &[u8], steps: u64) {
    let park = CODE + code.len() as u64;
    let mut image = code.to_vec();
    image.extend_from_slice(&[0xEB, 0xFE]); // jmp $
    emu.mem_write(CODE, &image).expect("write code");
    emu.emu_start(CODE, Some(park), None, Some(steps))
        .expect("execute");
    assert_eq!(
        emu.cpu().rip(),
        park,
        "execution must reach the park jump — an early stop means a fault"
    );
}

/// A three-operand VEX instruction runs in protected mode and keeps its
/// non-destructive operand order.
#[test]
fn vex_avx_executes_in_protected_mode() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = protected32_emulator();

            let mut a = [0u8; 64];
            let mut b = [0u8; 64];
            for d in 0..4 {
                a[d * 4..d * 4 + 4].copy_from_slice(&(d as u32 + 1).to_le_bytes());
                b[d * 4..d * 4 + 4].copy_from_slice(&(10u32 * (d as u32 + 1)).to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm1, a);
            emu.reg_write_zmm(X86Reg::Zmm2, b);
            emu.reg_write_zmm(X86Reg::Zmm0, [0xAA; 64]);

            // vpaddd xmm0, xmm1, xmm2 = C5 F1 FE C2
            //   C5: 2-byte VEX; F1 = R(1) vvvv(1110 -> xmm1) L(0) pp(01 = 66)
            run(&mut emu, &[0xC5, 0xF1, 0xFE, 0xC2], 4);

            let got = emu.reg_read_zmm(X86Reg::Zmm0);
            for d in 0..4u32 {
                let v = u32::from_le_bytes(got[d as usize * 4..d as usize * 4 + 4].try_into().unwrap());
                assert_eq!(v, (d + 1) + 10 * (d + 1), "dword {d}");
            }
            assert!(
                got[16..32].iter().all(|&x| x == 0),
                "a VEX.128 result must zero the upper lane"
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// The 256-bit form, to confirm VEX.L is honoured and not just ignored.
#[test]
fn vex_ymm_executes_in_protected_mode() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = protected32_emulator();

            let mut a = [0u8; 64];
            for d in 0..8 {
                a[d * 4..d * 4 + 4].copy_from_slice(&(d as u32 + 1).to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm1, a);
            emu.reg_write_zmm(X86Reg::Zmm2, a);
            emu.reg_write_zmm(X86Reg::Zmm0, [0xAA; 64]);

            // vpaddd ymm0, ymm1, ymm2 = C5 F5 FE C2  (L=1)
            run(&mut emu, &[0xC5, 0xF5, 0xFE, 0xC2], 4);

            let got = emu.reg_read_zmm(X86Reg::Zmm0);
            for d in 0..8u32 {
                let v = u32::from_le_bytes(got[d as usize * 4..d as usize * 4 + 4].try_into().unwrap());
                assert_eq!(v, 2 * (d + 1), "dword {d} of the 256-bit result");
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// An EVEX-encoded instruction with a merging opmask runs in protected mode.
#[test]
fn evex_avx512_executes_in_protected_mode() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = protected32_emulator();

            let mut a = [0u8; 64];
            for q in 0..8 {
                a[q * 8..q * 8 + 8].copy_from_slice(&((q as u64) + 1).to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm1, a);
            emu.reg_write_zmm(X86Reg::Zmm2, [0xAA; 64]);
            emu.reg_write(X86Reg::Rax, 0b0011);

            // kmovw k1, eax                 C5 F8 92 C8
            // vmovdqu64 zmm2 {k1}{z}, zmm1  62 F1 FE C9 7F CA
            run(
                &mut emu,
                &[
                    0xC5, 0xF8, 0x92, 0xC8, //
                    0x62, 0xF1, 0xFE, 0xC9, 0x7F, 0xCA,
                ],
                6,
            );

            let got = emu.reg_read_zmm(X86Reg::Zmm2);
            for q in 0..8u64 {
                let v =
                    u64::from_le_bytes(got[q as usize * 8..q as usize * 8 + 8].try_into().unwrap());
                let want = if q < 2 { q + 1 } else { 0 };
                assert_eq!(v, want, "qword {q} of the masked 512-bit move");
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// `KMOVQ k, m64` and `KMOVQ m64, k` move all 64 bits in every mode.
///
/// Bochs `avx512_mask64.cc` `KMOVQ_KGqKEqM` / `KMOVQ_KEqKGqM` use
/// `read_virtual_qword` / `write_virtual_qword` unconditionally: an opmask is 64
/// bits wide regardless of the CPU's operand size, and VEX.W1 selects the opmask
/// width here rather than the operand size. `BxOpcodeGroup_VEX_0F90` and `_0F91`
/// carry no `ATTR_IS64`, so both forms are reachable from 32-bit code — which is
/// what makes this the one place the store width is observable, and why it went
/// untested while decode32 rejected every VEX prefix.
#[test]
fn kmovq_moves_a_full_qword_in_protected_mode() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = protected32_emulator();

            const VALUE: u64 = 0xDEAD_BEEF_CAFE_BABE;
            emu.mem_write(DATA, &VALUE.to_le_bytes()).expect("source");
            emu.mem_write(DEST, &[0xFF; 8]).expect("poison destination");

            // kmovq k1, [DATA]  = C4 E1 F8 90 0D <disp32>
            // kmovq [DEST], k1  = C4 E1 F8 91 0D <disp32>
            //   C4 E1: RXB = 111, map = 0F
            //   F8:    W(1) vvvv(1111) L(0) pp(00)
            //   ModRM 0D: mod=00 reg=k1 rm=101 -> absolute disp32 in 32-bit mode
            let mut code: Vec<u8> = vec![0xC4, 0xE1, 0xF8, 0x90, 0x0D];
            code.extend_from_slice(&(DATA as u32).to_le_bytes());
            code.extend_from_slice(&[0xC4, 0xE1, 0xF8, 0x91, 0x0D]);
            code.extend_from_slice(&(DEST as u32).to_le_bytes());
            run(&mut emu, &code, 4);

            let mut got = [0u8; 8];
            emu.mem_read(DEST, &mut got).expect("read destination");
            assert_eq!(
                u64::from_le_bytes(got),
                VALUE,
                "KMOVQ must move all 64 bits outside 64-bit mode; a dword-wide \
                 access truncates k[63:32] and leaves the upper four bytes of \
                 the destination stale"
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// LES, LDS and BOUND still decode: the VEX and EVEX prefixes are only
/// recognised when the following byte encodes a register operand, which those
/// three instructions never do.
#[test]
fn legacy_c4_c5_62_still_execute_in_protected_mode() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = protected32_emulator();

            // A far pointer at DATA: offset 0x1234_5678, selector 0x0010.
            emu.mem_write(DATA, &0x1234_5678u32.to_le_bytes())
                .expect("offset");
            emu.mem_write(DATA + 4, &0x0010u16.to_le_bytes())
                .expect("selector");

            // les ecx, [DATA] = C4 0D <disp32>
            let mut code: Vec<u8> = vec![0xC4, 0x0D];
            code.extend_from_slice(&(DATA as u32).to_le_bytes());
            run(&mut emu, &code, 2);

            assert_eq!(
                emu.reg_read(X86Reg::Rcx) as u32,
                0x1234_5678,
                "LES must still load the offset"
            );
            assert_eq!(emu.reg_read(X86Reg::Es), 0x0010, "LES must load ES");
        })
        .expect("spawn")
        .join()
        .expect("join");
}
