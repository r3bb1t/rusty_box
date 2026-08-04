//! Bochs-parity regression tests for AVX-512 defects found by the adversarial
//! parity audit. Each test pins one upstream behaviour that rusty_box diverged
//! from; all were red before the accompanying fix.

#![cfg(feature = "std")]

use rusty_box::cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

/// Same sizing rationale as fp_vex_scalar_ops.rs.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;

/// Build an emulator in flat long mode with the full AVX-512 XCR0 enabled.
fn evex_emulator() -> Box<Emulator<'static, Corei7SkylakeX>> {
    let cfg = EmulatorConfig::default();
    let mut emu =
        Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64).expect("emulator");
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
        .expect("enable AVX-512 state");
    emu
}

/// Bochs avx/avx512_broadcast.cc `VPBROADCASTB_MASK_VdqWbM`:
///
/// ```text
/// Bit64u opmask = BX_READ_OPMASK(i->opmask());
/// if (len != BX_VL512) opmask &= CUT_OPMASK_TO(BYTE_ELEMENTS(len));
/// Bit8u val_8 = 0;
/// if (opmask) { eaddr = ...; val_8 = read_virtual_byte(i->seg(), eaddr); }
/// ```
///
/// An all-zero writemask must suppress the memory access entirely. Ours read
/// the operand unconditionally, so a masked-off broadcast took a #PF on an
/// address the instruction never uses.
#[test]
fn evex_vpbroadcastb_suppresses_the_load_under_an_empty_opmask() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = evex_emulator();

            // FlatLong64: PD0 @ 0x3000; entry 3 maps [0x60_0000, 0x80_0000).
            // Unmap it, then point the broadcast's operand inside it.
            const UNMAPPED: u64 = 0x0060_0000;
            emu.mem_write(0x3018, &0u64.to_le_bytes())
                .expect("unmap PD0[3]");

            // kmovw k1, eax                       C5 F8 92 C8   (k1 = 0)
            // vpbroadcastb zmm0 {k1}{z}, byte [UNMAPPED]
            //   EVEX.512.66.0F38.W0 78 /r
            //   62 F2 7D C9 78 04 25 <disp32>
            //   P1=7D: W=0, vvvv=1111, pp=01 (66)
            //   P2=C9: z=1, L'L=10 (512), b=0, V'=1, aaa=001 (k1)
            //   ModRM 04 + SIB 25: absolute disp32
            let mut code: Vec<u8> = vec![
                0xC5, 0xF8, 0x92, 0xC8, //
                0x62, 0xF2, 0x7D, 0xC9, 0x78, 0x04, 0x25,
            ];
            code.extend_from_slice(&(UNMAPPED as u32).to_le_bytes());
            let park = CODE + code.len() as u64;
            code.extend_from_slice(&[0xEB, 0xFE]);
            emu.mem_write(CODE, &code).expect("write code");

            emu.reg_write(X86Reg::Rax, 0);
            match emu.emu_start(CODE, Some(park), None, Some(8)) {
                Ok(_) | Err(_) => {}
            }

            assert_eq!(
                emu.cpu().rip(),
                park,
                "an all-zero opmask must suppress the broadcast's memory \
                 access; taking a #PF on the unused operand means the load \
                 ran unconditionally (Bochs avx512_broadcast.cc \
                 VPBROADCASTB_MASK_VdqWbM guards it with `if (opmask)`)"
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// Bochs avx/avx512_move.cc `VMOVAPD_MASK_VpdWpdR` writes a masked register
/// move through `avx512_write_regq_masked` — one mask bit per QWORD. Our W1
/// register store-form used to delegate straight to the W0 (dword) handler
/// under the comment "register form is identical", so each mask bit gated
/// only 32 bits: with `k1 = 0b0101` a 256-bit `VMOVDQU64` wrote qwords 0 and
/// 2 as half-updated values and left qwords 1 and 3 wrong as well.
///
/// `vmovdqu64 zmm2 {k1}{z}, zmm1` with k1 = 0b0011 must copy qwords 0 and 1
/// whole and zero the rest.
#[test]
fn evex_vmovdqu64_register_store_masks_at_qword_granularity() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = evex_emulator();

            // kmovw k1, eax                      C5 F8 92 C8      (k1 = 0b0011)
            // vmovdqu64 zmm2 {k1}{z}, zmm1       EVEX.512.F3.0F.W1 7F /r
            //   62 F1 FE C9 7F CA
            //   P0=F1: mm=01 (0F map)
            //   P1=FE: W=1, vvvv=1111 (unused), pp=10 (F3)
            //   P2=C9: z=1, L'L=10 (512), b=0, V'=1, aaa=001 (k1)
            //   ModRM CA: reg=zmm1 (source), rm=zmm2 (destination)
            let code: &[u8] = &[
                0xC5, 0xF8, 0x92, 0xC8, //
                0x62, 0xF1, 0xFE, 0xC9, 0x7F, 0xCA, //
                0xEB, 0xFE,
            ];
            emu.mem_write(CODE, code).expect("write code");
            emu.reg_write(X86Reg::Rax, 0b0011);

            // Source qwords are 1,2,3,...; destination is poisoned.
            let mut src = [0u8; 64];
            for q in 0..8 {
                src[q * 8..q * 8 + 8].copy_from_slice(&((q as u64) + 1).to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm1, src);
            emu.reg_write_zmm(X86Reg::Zmm2, [0xAA; 64]);

            emu.emu_start(CODE, Some(CODE + 10), None, Some(8))
                .expect("VMOVDQU64 must execute");

            let got = emu.reg_read_zmm(X86Reg::Zmm2);
            for q in 0..8u64 {
                let v = u64::from_le_bytes(got[q as usize * 8..q as usize * 8 + 8].try_into().unwrap());
                let want = if q < 2 { q + 1 } else { 0 };
                assert_eq!(
                    v, want,
                    "qword {q}: a W1 masked move must apply one mask bit per \
                     QWORD (Bochs avx512_move.cc avx512_write_regq_masked); \
                     dword-granularity masking splits each qword in half"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// Bochs avx/avx512_pfp.cc `VCMPPS_MASK_KGwHpsWpsIbR`:
///
/// ```text
/// BxPackedAvxRegister op1 = BX_READ_AVX_REG(i->src1()), op2 = BX_READ_AVX_REG(i->src2());
/// ... avx_compare32[ib](op1.vmm32u(n), op2.vmm32u(n), &status)
/// ```
///
/// where Bochs `src1` is Hps (EVEX.vvvv) and `src2` is Wps (ModRM.rm).
/// rusty_box's EVEX decoder uses the opposite accessor convention —
/// `src2()` is vvvv and `src1()` is rm — so the compare handlers must read
/// vvvv from `src2()`, exactly as the neighbouring VPTESTMD does. They read
/// them the other way round, which both transposed the comparison and, in
/// the memory form, made op1 an unrelated register instead of vvvv.
///
/// `VCMPPS k1, zmm1, zmm2, LT_OS` with zmm1 = 1.0 and zmm2 = 2.0 must set
/// every lane (1.0 < 2.0); transposed it clears every lane.
#[test]
fn evex_vcmpps_compares_vvvv_against_rm() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = evex_emulator();

            // vcmpps k1, zmm1, zmm2, 1   EVEX.NDS.512.0F.W0 C2 /r ib
            //   62 F1 74 48 C2 CA 01
            // kmovw eax, k1              C5 F8 93 C1
            let code: &[u8] = &[
                0x62, 0xF1, 0x74, 0x48, 0xC2, 0xCA, 0x01, //
                0xC5, 0xF8, 0x93, 0xC1, //
                0xEB, 0xFE,
            ];
            emu.mem_write(CODE, code).expect("write code");

            let mut ones = [0u8; 64];
            let mut twos = [0u8; 64];
            for lane in 0..16 {
                ones[lane * 4..lane * 4 + 4].copy_from_slice(&1.0f32.to_bits().to_le_bytes());
                twos[lane * 4..lane * 4 + 4].copy_from_slice(&2.0f32.to_bits().to_le_bytes());
            }
            emu.reg_write_zmm(X86Reg::Zmm1, ones);
            emu.reg_write_zmm(X86Reg::Zmm2, twos);
            emu.reg_write(X86Reg::Rax, 0);

            emu.emu_start(CODE, Some(CODE + 11), None, Some(8))
                .expect("VCMPPS must execute");

            assert_eq!(
                emu.reg_read(X86Reg::Rax) & 0xFFFF,
                0xFFFF,
                "VCMPPS LT_OS with vvvv=1.0 and rm=2.0 must set every lane; \
                 0 means the operands were transposed (Bochs avx512_pfp.cc \
                 VCMPPS_MASK_KGwHpsWpsIbR compares src1=Hps against src2=Wps)"
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}

/// Bochs avx/avx512_mask64.cc `KTESTQ_KGqKEqR` (and the B/W/D siblings):
///
/// ```text
/// Bit64u op1 = BX_READ_OPMASK(i->src1()), op2 = BX_READ_OPMASK(i->src2());
/// if ((op1 & op2) == 0) assert_ZF();
/// if ((~op1 & op2) == 0) assert_CF();
/// ```
///
/// ia_opcodes.def declares KTEST as `OP_NONE, OP_KGb, OP_KEb` — it has NO
/// destination; the ModRM.reg field is src1 and rm is src2. Our decoder used
/// to route VEX `0F 99` through the SETcc arm (`0F 90..9F`, whose operand IS
/// rm), which transposed the two. `KORTEST` shares that arm but is immune —
/// OR is commutative and both its flag tests are symmetric — so only KTEST's
/// asymmetric CF term exposes the swap.
#[test]
fn ktest_reads_the_reg_field_as_its_first_operand() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let mut emu = evex_emulator();

            // kmovw k1, eax    C5 F8 92 C8   (k1 = 0xFF)
            // kmovw k2, ecx    C5 F8 92 D1   (k2 = 0x0F)
            // ktestb k1, k2    C5 F9 99 CA
            // pushfq ; pop rax ; jmp $
            let code: &[u8] = &[
                0xC5, 0xF8, 0x92, 0xC8, //
                0xC5, 0xF8, 0x92, 0xD1, //
                0xC5, 0xF9, 0x99, 0xCA, //
                0x9C, 0x58, 0xEB, 0xFE,
            ];
            emu.mem_write(CODE, code).expect("write code");
            emu.reg_write(X86Reg::Rax, 0xFF);
            emu.reg_write(X86Reg::Rcx, 0x0F);
            emu.reg_write(X86Reg::Rsp, 0x0050_0000);

            emu.emu_start(CODE, Some(CODE + 14), None, Some(8))
                .expect("KTESTB must execute");

            let flags = emu.reg_read(X86Reg::Rax);
            let cf = flags & 1;
            let zf = (flags >> 6) & 1;

            // op1 = k1 = 0xFF, op2 = k2 = 0x0F:
            //   op1 & op2  = 0x0F  != 0        -> ZF = 0
            //   ~op1 & op2 = 0x00 & 0x0F = 0   -> CF = 1
            // Transposed (op1 = 0x0F, op2 = 0xFF) would give
            //   ~0x0F & 0xFF = 0xF0 != 0       -> CF = 0
            assert_eq!(zf, 0, "ZF: op1 & op2 is nonzero");
            assert_eq!(
                cf, 1,
                "CF must come from (~k1 & k2); CF=0 means the operands were \
                 transposed (Bochs avx512_mask64.cc KTESTB_KGbKEbR reads \
                 src1=ModRM.reg, src2=rm)"
            );
        })
        .expect("spawn")
        .join()
        .expect("join");
}
