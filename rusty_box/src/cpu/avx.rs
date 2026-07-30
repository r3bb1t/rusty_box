#![allow(unused_assignments, dead_code)]
//! AVX/AVX2/AVX-512 instruction handlers for VEX.256 and EVEX operations
//!
//! Implements the subset of VEX.256 and EVEX instructions used by the Linux kernel,
//! primarily from blake2s_compress_avx512 and similar optimized routines.
//!
//! VEX.L=0 (128-bit) instructions are handled by the existing SSE handlers.
//! This file handles VEX.L=1 (256-bit) and EVEX-specific instructions.

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::{features::X86Feature, BxSegregs, Instruction},
    softfloat3e::{
        f128::{SOFTFLOAT_MULADD_SUB_C, SOFTFLOAT_MULADD_SUB_PROD},
        f16_to_f32::f16_to_f32,
        f32_mul_add::f32_mul_add,
        f32_to_f16::f32_to_f16,
        f64_mul_add::f64_mul_add,
        softfloat::{softfloat_getExceptionFlags, FLAG_DENORMAL},
    },
    sse_fp::mxcsr_to_softfloat_status_word,
    xmm::{BxPackedXmmRegister, BxPackedYmmRegister},
};

#[derive(Clone, Copy)]
enum VexFpLogicalOp {
    And,
    AndNot,
    Or,
    Xor,
}

/// AVX2 per-element variable shifts (VEX.66.0F38 45/46/47).
/// Bochs simd_int.h `xmm_psrlvd` / `xmm_psrlvq` / `xmm_psravd` /
/// `xmm_psllvd` / `xmm_psllvq`.
#[derive(Clone, Copy)]
pub(super) enum VexVarShiftOp {
    SrlD,
    SrlQ,
    SraD,
    SllD,
    SllQ,
}

#[derive(Clone, Copy)]
pub(super) enum VexFmaForm {
    F132,
    F213,
    F231,
}

#[derive(Clone, Copy)]
pub(super) enum VexPackedFmaOp {
    Fmadd,
    Fmsub,
    Fnmadd,
    Fnmsub,
    FmaddSub,
    FmsubAdd,
}

#[derive(Clone, Copy)]
pub(super) enum VexScalarFmaOp {
    Fmadd,
    Fmsub,
    Fnmadd,
    Fnmsub,
}

/// Permute the three source operands (as raw bits) into softfloat muladd
/// (a, b, c) order for the given form. Keeping the values as raw bits avoids
/// any f32 round-trip that would canonicalize a NaN before softfloat sees it.
/// Bochs encodes this reorder in the opcode operand tuple; here `v`=DEST,
/// `h`=vvvv, `w`=rm, matching the handler operand sourcing.
#[inline]
fn vex_fma_operands_u32(form: VexFmaForm, v: u32, h: u32, w: u32) -> (u32, u32, u32) {
    match form {
        VexFmaForm::F132 => (v, w, h),
        VexFmaForm::F213 => (h, v, w),
        VexFmaForm::F231 => (h, w, v),
    }
}

#[inline]
fn vex_fma_operands_u64(form: VexFmaForm, v: u64, h: u64, w: u64) -> (u64, u64, u64) {
    match form {
        VexFmaForm::F132 => (v, w, h),
        VexFmaForm::F213 => (h, v, w),
        VexFmaForm::F231 => (h, w, v),
    }
}

/// SoftFloat `f{32,64}_mul_add` op flag for a scalar FMA op.
/// Bochs softfloat.h f32_fmadd/fmsub/fnmadd/fnmsub.
#[inline]
fn scalar_fma_flags(op: VexScalarFmaOp) -> u8 {
    match op {
        VexScalarFmaOp::Fmadd => 0,
        VexScalarFmaOp::Fmsub => SOFTFLOAT_MULADD_SUB_C,
        VexScalarFmaOp::Fnmadd => SOFTFLOAT_MULADD_SUB_PROD,
        VexScalarFmaOp::Fnmsub => SOFTFLOAT_MULADD_SUB_C | SOFTFLOAT_MULADD_SUB_PROD,
    }
}

/// SoftFloat `f{32,64}_mul_add` op flag for a packed FMA op at a given lane.
/// FMADDSUB subtracts on even lanes / adds on odd; FMSUBADD is the inverse
/// (Bochs simd_pfp.h xmm_fmaddsubps / xmm_fmsubaddps).
#[inline]
fn packed_fma_flags(op: VexPackedFmaOp, lane: usize) -> u8 {
    match op {
        VexPackedFmaOp::Fmadd => 0,
        VexPackedFmaOp::Fmsub => SOFTFLOAT_MULADD_SUB_C,
        VexPackedFmaOp::Fnmadd => SOFTFLOAT_MULADD_SUB_PROD,
        VexPackedFmaOp::Fnmsub => SOFTFLOAT_MULADD_SUB_C | SOFTFLOAT_MULADD_SUB_PROD,
        VexPackedFmaOp::FmaddSub if lane & 1 == 0 => SOFTFLOAT_MULADD_SUB_C,
        VexPackedFmaOp::FmaddSub => 0,
        VexPackedFmaOp::FmsubAdd if lane & 1 == 0 => 0,
        VexPackedFmaOp::FmsubAdd => SOFTFLOAT_MULADD_SUB_C,
    }
}

// AMX architectural state — Bochs avx/amx.h BxPackedAmxRegister + amx.cc.
//
// 8 tiles × 16 rows × 64 bytes = 8192 bytes of tile storage (BX_TILE_REGISTERS
// × BX_TILE_MAX_ROWS × 64). Tile config: palette_id, start_row, plus per-tile
// rows/bytes-per-row. `tile_use_tracker` is an 8-bit bitmap of tiles whose row
// data is nonzero — used by xsave_tiledata_state_xinuse.
pub const BX_TILE_REGISTERS: usize = 8;
pub const BX_TILE_MAX_ROWS: usize = 16;
pub const BX_TILE_ROW_BYTES: usize = 64;

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Default, Clone, Copy)]
pub struct TILECFG {
    pub rows: u32,
    pub bytes_per_row: u32,
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug)]
pub struct AMX {
    pub palette_id: u32,
    pub start_row: u32,
    pub tilecfg: [TILECFG; BX_TILE_REGISTERS],
    /// [tile][row] flat byte buffer sized for BX_TILE_MAX_ROWS × BX_TILE_ROW_BYTES.
    pub tile: [[u8; BX_TILE_MAX_ROWS * BX_TILE_ROW_BYTES]; BX_TILE_REGISTERS],
    /// Bitmap: bit i set iff tile `i` has non-initial content — Bochs amx.h
    /// tile_use_tracker (drives xsave_tiledata_state_xinuse).
    pub tile_use_tracker: u8,
}

impl Default for AMX {
    fn default() -> Self {
        Self {
            palette_id: 0,
            start_row: 0,
            tilecfg: [TILECFG::default(); BX_TILE_REGISTERS],
            tile: [[0u8; BX_TILE_MAX_ROWS * BX_TILE_ROW_BYTES]; BX_TILE_REGISTERS],
            tile_use_tracker: 0,
        }
    }
}

impl AMX {
    /// Bochs amx.h tiles_configured: palette_id != 0 indicates config was applied.
    #[inline]
    pub fn tiles_configured(&self) -> bool {
        self.palette_id != 0
    }

    /// Bochs amx.h clear — reset the entire AMX state block.
    pub fn clear(&mut self) {
        *self = AMX::default();
    }

    #[inline]
    pub fn set_tile_used(&mut self, idx: usize) {
        self.tile_use_tracker |= 1 << idx;
    }

    #[inline]
    pub fn clear_tile_used(&mut self, idx: usize) {
        self.tile_use_tracker &= !(1 << idx);
        // Also zero the tile storage so xinuse reports consistently.
        self.tile[idx] = [0u8; BX_TILE_MAX_ROWS * BX_TILE_ROW_BYTES];
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // VZEROUPPER / VZEROALL (VEX.0F 77)
    // ========================================================================

    /// VZEROUPPER — Zero upper 128 bits of all YMM registers.
    /// Bochs avx.cc: for i in 0..nregs { vmm[i].set_ymm128(1, 0); }
    pub(super) fn vzeroupper(&mut self, _instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let nregs = if self.long64_mode() { 16 } else { 8 };
        for i in 0..nregs {
            // Clear upper 128 bits (ymm128[1]) and ZMM upper 256 bits
            self.vmm[i].set_zmm128(1, BxPackedXmmRegister::default());
            self.vmm[i].set_zmm128(2, BxPackedXmmRegister::default());
            self.vmm[i].set_zmm128(3, BxPackedXmmRegister::default());
        }
        Ok(())
    }

    /// VZEROALL — Zero all YMM registers (all 256 bits).
    /// Bochs avx.cc: for i in 0..nregs { vmm[i] = 0; }
    pub(super) fn vzeroall(&mut self, _instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let nregs = if self.long64_mode() { 16 } else { 8 };
        for i in 0..nregs {
            self.vmm[i].clear();
        }
        Ok(())
    }

    // ========================================================================
    // VEX.L-aware dispatch wrappers
    // These check VEX.L and dispatch to 128-bit (SSE) or 256-bit (AVX) handlers
    // ========================================================================

    /// VMOVDQU load — VEX.L=0: XMM <- M128, VEX.L=1: YMM <- M256
    /// Also handles register form (mod=11): dst_reg <- src_reg
    pub(super) fn vmovdqu_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            // Register form: copy src1 (rm) to dst (nnn)
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.write_xmm_reg(instr.dst(), val);
            }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                let val = self.v_read_ymmword(seg, eaddr)?;
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.v_read_xmmword(seg, eaddr)?;
                self.write_xmm_reg(instr.dst(), val);
            }
        }
        Ok(())
    }

    /// VMOVDQU store — VEX.L=0: M128 <- XMM, VEX.L=1: M256 <- YMM
    /// Also handles register form (mod=11): dst_reg <- src_reg
    pub(super) fn vmovdqu_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            // Register form: copy src1 (nnn) to dst (rm)
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.write_xmm_reg(instr.dst(), val);
            }
        } else {
            // Memory form: store to memory
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.v_write_ymmword(seg, eaddr, &val)?;
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.v_write_xmmword(seg, eaddr, &val)?;
            }
        }
        Ok(())
    }

    /// VMOVDQA/VMOVAPS/VMOVAPD register-to-register — VEX.L aware
    pub(super) fn vmovdqa_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let val = self.read_ymm_reg(instr.src1());
            self.write_ymm_reg(instr.dst(), val);
        } else {
            let val = self.read_xmm_reg(instr.src1());
            self.write_xmm_reg(instr.dst(), val);
        }
        Ok(())
    }

    /// VMOVDQA/VMOVAPS load — VEX.L=0: XMM <- M128, VEX.L=1: YMM <- M256 (aligned)
    /// Also handles register form (mod=11): dst_reg <- src_reg
    pub(super) fn vmovdqa_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            // Register form: copy src1 (rm) to dst (nnn)
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.write_xmm_reg(instr.dst(), val);
            }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                let val = self.v_read_ymmword(seg, eaddr)?;
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.v_read_xmmword_aligned(seg, eaddr)?;
                self.write_xmm_reg(instr.dst(), val);
            }
        }
        Ok(())
    }

    /// VMOVDQA store — VEX.L=0: M128 <- XMM, VEX.L=1: M256 <- YMM (aligned)
    /// Also handles register form (mod=11): dst_reg <- src_reg
    pub(super) fn vmovdqa_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            // Register form: copy src1 (nnn) to dst (rm)
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.write_xmm_reg(instr.dst(), val);
            }
        } else {
            // Memory form: store to memory
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.v_write_ymmword(seg, eaddr, &val)?;
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.v_write_xmmword_aligned(seg, eaddr, &val)?;
            }
        }
        Ok(())
    }

    /// VMOVUPS/VMOVUPD load — VEX.L aware unaligned
    pub(super) fn vmovups_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vmovdqu_load(instr) // same behavior
    }

    /// VMOVUPS/VMOVUPD store — VEX.L aware unaligned
    pub(super) fn vmovups_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vmovdqu_store(instr) // same behavior
    }

    // ========================================================================
    // VEX.L-aware packed integer arithmetic
    // ========================================================================

    /// VPADDD — Packed Add Dwords (VEX.L aware)
    /// dst = src1 + src2 (element-wise 32-bit add)
    pub(super) fn vpaddd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            // 256-bit: 8 dwords
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, src1.ymm32u(i).wrapping_add(src2.ymm32u(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            // 128-bit: 4 dwords
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, src1.xmm32u(i).wrapping_add(src2.xmm32u(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPXOR / VPXORD — Packed XOR (VEX.L aware)
    /// dst = src1 ^ src2
    pub(super) fn vpxor(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, src1.ymm64u(i) ^ src2.ymm64u(i));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, src1.xmm64u(i) ^ src2.xmm64u(i));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPAND / VPANDD — Packed AND (VEX.L aware)
    pub(super) fn vpand(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, src1.ymm64u(i) & src2.ymm64u(i));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, src1.xmm64u(i) & src2.xmm64u(i));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPEQD — Packed Compare Equal Dwords (VEX.L aware)
    /// dst[i] = (src1[i] == src2[i]) ? 0xFFFFFFFF : 0
    pub(super) fn vpcmpeqd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(
                    i,
                    if src1.ymm32u(i) == src2.ymm32u(i) {
                        0xFFFF_FFFF
                    } else {
                        0
                    },
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(
                    i,
                    if src1.xmm32u(i) == src2.xmm32u(i) {
                        0xFFFF_FFFF
                    } else {
                        0
                    },
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSHUFD — Packed Shuffle Dwords (VEX.L aware)
    /// dst[i] = src[imm8[i*2+1:i*2]] for each dword lane
    pub(super) fn vpshufd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let imm = instr.ib();
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane
            for i in 0..4 {
                let sel = ((imm >> (i * 2)) & 0x3) as usize;
                result.set_ymm32u(i, src.ymm32u(sel));
            }
            // Upper 128-bit lane (operates independently)
            for i in 0..4 {
                let sel = ((imm >> (i * 2)) & 0x3) as usize;
                result.set_ymm32u(4 + i, src.ymm32u(4 + sel));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                let sel = ((imm >> (i * 2)) & 0x3) as usize;
                result.set_xmm32u(i, src.xmm32u(sel));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // AVX-512 specific instructions (EVEX only)
    // ========================================================================

    /// VPERMI2D — Full Permute of Dwords from Two Sources
    /// EVEX.66.0F38.W0 76 /r
    /// For each dword element i in dest:
    ///   index = dest[i] (low bits select from concatenation of src1:src2)
    ///   result[i] = (src1:src2)[index]
    /// where src1 = VEX.vvvv, src2 = r/m
    pub(super) fn vpermi2d(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv

        if instr.get_vl() >= 1 {
            // 256-bit: 8 dwords, index bits 2:0 select from 16-element pool (8+8)
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let indices = self.read_ymm_reg(dst_idx);
            let mut result = BxPackedYmmRegister::default();

            // Concatenate src1 (elements 0-7) and src2 (elements 8-15)
            let num_elements = 8usize; // 256-bit / 32-bit = 8 elements
            let index_mask = (num_elements * 2 - 1) as u32; // 0xF for 16-element pool
            for i in 0..num_elements {
                let idx = (indices.ymm32u(i) & index_mask) as usize;
                if idx < num_elements {
                    result.set_ymm32u(i, src1.ymm32u(idx));
                } else {
                    result.set_ymm32u(i, src2.ymm32u(idx - num_elements));
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            // 128-bit: 4 dwords, index bits 2:0 select from 8-element pool (4+4)
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let indices = self.read_xmm_reg(dst_idx);
            let mut result = BxPackedXmmRegister::default();

            let num_elements = 4usize;
            let index_mask = (num_elements * 2 - 1) as u32; // 0x7 for 8-element pool
            for i in 0..num_elements {
                let idx = (indices.xmm32u(i) & index_mask) as usize;
                if idx < num_elements {
                    result.set_xmm32u(i, src1.xmm32u(idx));
                } else {
                    result.set_xmm32u(i, src2.xmm32u(idx - num_elements));
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPRORD — Packed Rotate Right Dwords by immediate
    /// EVEX.66.0F.W0 72 /0 ib
    /// Operands: dst=VEX.vvvv (src2), src=rm (src1), imm8
    pub(super) fn vprord(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv — for EVEX group opcodes, dst is in vvvv
        let count = (instr.ib() & 31) as u32; // rotate count mod 32

        // For EVEX group opcodes, rm (source) is in dst(), nnn is opcode extension
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, src.ymm32u(i).rotate_right(count));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, src.xmm32u(i).rotate_right(count));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPROLD — Packed Rotate Left Dwords by immediate
    /// EVEX.66.0F.W0 72 /1 ib
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vprold(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv — for EVEX group opcodes, dst is in vvvv
        let count = (instr.ib() & 31) as u32; // rotate count mod 32

        // For EVEX group opcodes, rm (source) is in dst(), nnn is opcode extension
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, src.ymm32u(i).rotate_left(count));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, src.xmm32u(i).rotate_left(count));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // VPMOVSX/VPMOVZX — sign/zero extension (Bochs avx2.cc VPMOVSXBW_VdqWdqR
    // et al.). Each form reads a fixed source width (VL256 doubles the VL128
    // width — Bochs fetchdecode_opmap_avx.cc BxOpcodeGroup_VEX_0F3820..25 /
    // 30..35) and widens into the full destination, zeroing above VL.
    // ========================================================================

    /// Read the widening source for VPMOVSX/VPMOVZX: `bytes` source bytes
    /// (2/4/8/16) from the low bytes of the rm register or from memory,
    /// zero-padded to 16 bytes.
    fn vex_pmov_read(&mut self, instr: &Instruction, bytes: usize) -> super::Result<[u8; 16]> {
        let mut out = [0u8; 16];
        if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            out[..bytes].copy_from_slice(&src.raw()[..bytes]);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            match bytes {
                2 => out[..2].copy_from_slice(&self.v_read_word(seg, eaddr)?.to_le_bytes()),
                4 => out[..4].copy_from_slice(&self.v_read_dword(seg, eaddr)?.to_le_bytes()),
                8 => out[..8].copy_from_slice(&self.v_read_qword(seg, eaddr)?.to_le_bytes()),
                _ => out.copy_from_slice(self.v_read_xmmword(seg, eaddr)?.raw()),
            }
        }
        Ok(out)
    }

    /// VPMOVSXBW — sign-extend 8 (VL128) / 16 (VL256) bytes to words.
    pub(super) fn vpmovsxbw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 16)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(i, src[i] as i8 as i16 as u16);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(i, src[i] as i8 as i16 as u16);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVSXBD — sign-extend 4 (VL128) / 8 (VL256) bytes to dwords.
    pub(super) fn vpmovsxbd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, src[i] as i8 as i32 as u32);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 4)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, src[i] as i8 as i32 as u32);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVSXBQ — sign-extend 2 (VL128) / 4 (VL256) bytes to qwords.
    pub(super) fn vpmovsxbq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 4)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, src[i] as i8 as i64 as u64);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 2)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, src[i] as i8 as i64 as u64);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVSXWD — sign-extend 4 (VL128) / 8 (VL256) words to dwords.
    pub(super) fn vpmovsxwd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 16)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                let w = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                result.set_ymm32u(i, w as i16 as i32 as u32);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                let w = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                result.set_xmm32u(i, w as i16 as i32 as u32);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVSXWQ — sign-extend 2 (VL128) / 4 (VL256) words to qwords.
    pub(super) fn vpmovsxwq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                let w = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                result.set_ymm64u(i, w as i16 as i64 as u64);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 4)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                let w = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                result.set_xmm64u(i, w as i16 as i64 as u64);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVSXDQ — sign-extend 2 (VL128) / 4 (VL256) dwords to qwords.
    pub(super) fn vpmovsxdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 16)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                let d = u32::from_le_bytes([
                    src[i * 4],
                    src[i * 4 + 1],
                    src[i * 4 + 2],
                    src[i * 4 + 3],
                ]);
                result.set_ymm64u(i, d as i32 as i64 as u64);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                let d = u32::from_le_bytes([
                    src[i * 4],
                    src[i * 4 + 1],
                    src[i * 4 + 2],
                    src[i * 4 + 3],
                ]);
                result.set_xmm64u(i, d as i32 as i64 as u64);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVZXBW — zero-extend 8 (VL128) / 16 (VL256) bytes to words.
    pub(super) fn vpmovzxbw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 16)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(i, src[i] as u16);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(i, src[i] as u16);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVZXBD — zero-extend 4 (VL128) / 8 (VL256) bytes to dwords.
    pub(super) fn vpmovzxbd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, src[i] as u32);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 4)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, src[i] as u32);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVZXBQ — zero-extend 2 (VL128) / 4 (VL256) bytes to qwords.
    pub(super) fn vpmovzxbq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 4)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, src[i] as u64);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 2)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, src[i] as u64);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVZXWD — zero-extend 4 (VL128) / 8 (VL256) words to dwords.
    pub(super) fn vpmovzxwd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 16)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                let w = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                result.set_ymm32u(i, w as u32);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                let w = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                result.set_xmm32u(i, w as u32);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVZXWQ — zero-extend 2 (VL128) / 4 (VL256) words to qwords.
    pub(super) fn vpmovzxwq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                let w = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                result.set_ymm64u(i, w as u64);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 4)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                let w = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                result.set_xmm64u(i, w as u64);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPMOVZXDQ — zero-extend 2 (VL128) / 4 (VL256) dwords to qwords.
    pub(super) fn vpmovzxdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let src = self.vex_pmov_read(instr, 16)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                let d = u32::from_le_bytes([
                    src[i * 4],
                    src[i * 4 + 1],
                    src[i * 4 + 2],
                    src[i * 4 + 3],
                ]);
                result.set_ymm64u(i, d as u64);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_pmov_read(instr, 8)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                let d = u32::from_le_bytes([
                    src[i * 4],
                    src[i * 4 + 1],
                    src[i * 4 + 2],
                    src[i * 4 + 3],
                ]);
                result.set_xmm64u(i, d as u64);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ========================================================================
    // VPTEST / VMOVMSKPS / VMOVMSKPD / VPABS / VLDDQU-adjacent VEX forms
    // ========================================================================

    /// VPTEST — logical compare over the full VL (Bochs avx.cc VPTEST_VdqWdqR):
    /// ZF = ((rm AND dst) == 0), CF = ((rm AND NOT dst) == 0).
    pub(super) fn vptest(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        // Bochs avx.cc VPTEST_VdqWdqR: clearEFlagsOSZAPC()
        self.oszapc.set_oszapc_logic_32(1);
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.dst());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut zf = true;
            let mut cf = true;
            for i in 0..4 {
                if op2.ymm64u(i) & op1.ymm64u(i) != 0 {
                    zf = false;
                }
                if op2.ymm64u(i) & !op1.ymm64u(i) != 0 {
                    cf = false;
                }
            }
            self.oszapc.set_zf(zf);
            self.oszapc.set_cf(cf);
        } else {
            let op1 = self.read_xmm_reg(instr.dst());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut zf = true;
            let mut cf = true;
            for i in 0..2 {
                if op2.xmm64u(i) & op1.xmm64u(i) != 0 {
                    zf = false;
                }
                if op2.xmm64u(i) & !op1.xmm64u(i) != 0 {
                    cf = false;
                }
            }
            self.oszapc.set_zf(zf);
            self.oszapc.set_cf(cf);
        }
        Ok(())
    }

    /// VTESTPS / VTESTPD — like VPTEST but restricted to the packed sign bits
    /// (Bochs avx_pfp.cc VTESTPS_VpsWpsR / VTESTPD_VpdWpdR):
    /// ZF = ((rm AND dst AND signmask) == 0), CF = ((rm AND NOT dst AND
    /// signmask) == 0). No destination is written; OF/SF/AF/PF always clear.
    ///
    /// Bochs reads both operands as full YMM registers and iterates
    /// `QWORD_ELEMENTS(len)`, so VL128 sees only the low two qwords.
    pub(super) fn vtest(&mut self, instr: &Instruction, qword_elements: bool) -> super::Result<()> {
        self.prepare_sse()?;
        // Bochs seeds `result` with ZF|CF and clears the rest via
        // setEFlagsOSZAPC, i.e. OF/SF/AF/PF are unconditionally cleared.
        self.oszapc.set_oszapc_logic_32(1);
        let sign_mask: u64 = if qword_elements {
            0x8000_0000_0000_0000
        } else {
            0x8000_0000_8000_0000
        };
        // The memory form must touch exactly VL bytes (Bochs LOAD_Vector), so
        // the two lengths take different read paths — a blanket ymmword read
        // would fault on a VL128 operand at the end of a page.
        let mut op1 = [0u64; 4];
        let mut op2 = [0u64; 4];
        let elements = if instr.get_vl() >= 1 {
            let a = self.read_ymm_reg(instr.dst());
            let b = self.vex_read_src2_ymm(instr)?;
            for n in 0..4 {
                op1[n] = a.ymm64u(n);
                op2[n] = b.ymm64u(n);
            }
            4
        } else {
            let a = self.read_xmm_reg(instr.dst());
            let b = self.vex_read_src2_xmm(instr)?;
            for n in 0..2 {
                op1[n] = a.xmm64u(n);
                op2[n] = b.xmm64u(n);
            }
            2
        };
        let mut zf = true;
        let mut cf = true;
        for n in 0..elements {
            if (op2[n] & op1[n] & sign_mask) != 0 {
                zf = false;
            }
            if (op2[n] & !op1[n] & sign_mask) != 0 {
                cf = false;
            }
        }
        self.oszapc.set_zf(zf);
        self.oszapc.set_cf(cf);
        Ok(())
    }

    /// VMOVMSKPS — dword sign bits over VL (Bochs avx.cc VMOVMSKPS_GdUps):
    /// 4-bit (VL128) or 8-bit (VL256) mask, zero-extended into the GPR.
    pub(super) fn vmovmskps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut mask: u32 = 0;
        if instr.get_vl() >= 1 {
            let src = self.read_ymm_reg(instr.src1());
            for i in 0..8 {
                if src.ymm32u(i) & 0x8000_0000 != 0 {
                    mask |= 1 << i;
                }
            }
        } else {
            let src = self.read_xmm_reg(instr.src1());
            for i in 0..4 {
                if src.xmm32u(i) & 0x8000_0000 != 0 {
                    mask |= 1 << i;
                }
            }
        }
        self.set_gpr32(instr.dst().into(), mask);
        Ok(())
    }

    /// VMOVMSKPD — qword sign bits over VL (Bochs avx.cc VMOVMSKPD_GdUpd):
    /// 2-bit (VL128) or 4-bit (VL256) mask, zero-extended into the GPR.
    pub(super) fn vmovmskpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut mask: u32 = 0;
        if instr.get_vl() >= 1 {
            let src = self.read_ymm_reg(instr.src1());
            for i in 0..4 {
                if src.ymm64u(i) & 0x8000_0000_0000_0000 != 0 {
                    mask |= 1 << i;
                }
            }
        } else {
            let src = self.read_xmm_reg(instr.src1());
            for i in 0..2 {
                if src.xmm64u(i) & 0x8000_0000_0000_0000 != 0 {
                    mask |= 1 << i;
                }
            }
        }
        self.set_gpr32(instr.dst().into(), mask);
        Ok(())
    }

    /// VPABSB — per-byte absolute value over VL (Bochs HANDLE_AVX_1OP<xmm_pabsb>).
    pub(super) fn vpabsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let mut op = self.vex_read_src2_ymm(instr)?;
            for lane in 0..2 {
                let mut x = op.ymm128(lane);
                super::sse::pabsb_lane(&mut x);
                op.set_ymm128(lane, x);
            }
            self.write_ymm_reg(instr.dst(), op);
        } else {
            let mut op = self.vex_read_src2_xmm(instr)?;
            super::sse::pabsb_lane(&mut op);
            self.write_xmm_reg(instr.dst(), op);
        }
        Ok(())
    }

    /// VPABSW — per-word absolute value over VL (Bochs HANDLE_AVX_1OP<xmm_pabsw>).
    pub(super) fn vpabsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let mut op = self.vex_read_src2_ymm(instr)?;
            for lane in 0..2 {
                let mut x = op.ymm128(lane);
                super::sse::pabsw_lane(&mut x);
                op.set_ymm128(lane, x);
            }
            self.write_ymm_reg(instr.dst(), op);
        } else {
            let mut op = self.vex_read_src2_xmm(instr)?;
            super::sse::pabsw_lane(&mut op);
            self.write_xmm_reg(instr.dst(), op);
        }
        Ok(())
    }

    /// VPABSD — per-dword absolute value over VL (Bochs HANDLE_AVX_1OP<xmm_pabsd>).
    pub(super) fn vpabsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let mut op = self.vex_read_src2_ymm(instr)?;
            for lane in 0..2 {
                let mut x = op.ymm128(lane);
                super::sse::pabsd_lane(&mut x);
                op.set_ymm128(lane, x);
            }
            self.write_ymm_reg(instr.dst(), op);
        } else {
            let mut op = self.vex_read_src2_xmm(instr)?;
            super::sse::pabsd_lane(&mut op);
            self.write_xmm_reg(instr.dst(), op);
        }
        Ok(())
    }

    /// VINSERTPS — insert dword with zero mask (Bochs avx.cc
    /// VINSERTPS_VpsHpsWssIbR/M); vvvv is the first source, upper bits zeroed.
    pub(super) fn vinsertps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let control = instr.ib();
        let op2 = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1())
                .xmm32u(((control >> 6) & 3) as usize)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_dword(seg, eaddr)?
        };
        let mut op1 = self.read_xmm_reg(instr.src2());
        super::sse::insertps_core(&mut op1, op2, control);
        self.write_xmm_reg(instr.dst(), op1);
        Ok(())
    }

    /// VPINSRB — insert byte from GPR/memory into a copy of the vvvv source.
    /// VEX.128.66.0F3A.W0 20 /r ib. Bochs avx.cc `VPINSRB_VdqHdqEbIbR/M`.
    /// 3-operand: dst = src1(vvvv) with byte at imm8[3:0] replaced by src2 (r/m).
    /// Decoder maps src2()=vvvv (base), src1()=rm (value), dst()=destination.
    pub(super) fn vpinsrb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_avx()?;
        let mut op1 = self.read_xmm_reg(instr.src2()); // vvvv = base vector
        let op2 = if instr.mod_c0() {
            // BX_READ_8BIT_REGL — always low byte, never AH/CH/DH/BH
            self.gen_reg[instr.src1() as usize].rl()
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_byte(seg, eaddr)?
        };
        op1.set_xmmubyte((instr.ib() & 0xF) as usize, op2);
        self.write_xmm_reg(instr.dst(), op1); // VEX-128 clears bits [255:128]
        Ok(())
    }

    /// VPINSRW — insert word from GPR/memory into a copy of the vvvv source.
    /// VEX.128.66.0F.W0 C4 /r ib. Bochs avx.cc `VPINSRW_VdqHdqEwIbR/M`.
    pub(super) fn vpinsrw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_avx()?;
        let mut op1 = self.read_xmm_reg(instr.src2()); // vvvv = base vector
        let op2 = if instr.mod_c0() {
            self.get_gpr16(instr.src1().into())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_word(seg, eaddr)?
        };
        op1.set_xmm16u((instr.ib() & 7) as usize, op2);
        self.write_xmm_reg(instr.dst(), op1); // VEX-128 clears bits [255:128]
        Ok(())
    }

    /// VPINSRD — insert dword from GPR/memory into a copy of the vvvv source.
    /// VEX.128.66.0F3A.W0 22 /r ib. Bochs avx.cc `VPINSRD_VdqHdqEdIbR/M`.
    pub(super) fn vpinsrd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_avx()?;
        let mut op1 = self.read_xmm_reg(instr.src2()); // vvvv = base vector
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src1().into())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_dword(seg, eaddr)?
        };
        op1.set_xmm32u((instr.ib() & 3) as usize, op2);
        self.write_xmm_reg(instr.dst(), op1); // VEX-128 clears bits [255:128]
        Ok(())
    }

    /// VPINSRQ — insert qword from GPR/memory into a copy of the vvvv source.
    /// VEX.128.66.0F3A.W1 22 /r ib (64-bit mode only). Bochs `VPINSRQ_VdqHdqEqIbR/M`.
    pub(super) fn vpinsrq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_avx()?;
        let mut op1 = self.read_xmm_reg(instr.src2()); // vvvv = base vector
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src1().into())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        op1.set_xmm64u((instr.ib() & 1) as usize, op2);
        self.write_xmm_reg(instr.dst(), op1); // VEX-128 clears bits [255:128]
        Ok(())
    }

    /// VMPSADBW — multiple SADs, per-128-bit lane (Bochs avx2.cc
    /// VMPSADBW_VdqHdqWdqIbR); the upper lane consumes imm8 bits [5:3].
    pub(super) fn vmpsadbw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let ib = instr.ib();
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let control = if lane == 0 { ib } else { ib >> 3 };
                result.set_ymm128(
                    lane,
                    super::sse::mpsadbw_lane(&op1.ymm128(lane), &op2.ymm128(lane), control),
                );
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let result = super::sse::mpsadbw_lane(&op1, &op2, ib);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPHMINPOSUW — horizontal minimum unsigned word (Bochs sse.cc
    /// PHMINPOSUW_VdqWdqR shared by the V128 form), upper bits zeroed.
    pub(super) fn vphminposuw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.vex_read_src2_xmm(instr)?;
        let result = super::sse::phminposuw_core(&op);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// VMOVQ VqWq (VEX.128.F3.0F 7E) — load/move low qword, zero the rest
    /// (Bochs ia_opcodes.def BX_IA_VMOVQ_VqWq → MOVQ_VqWqR / MOVSD_VsdWsdM).
    pub(super) fn vmovq_vq_wq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val = if instr.mod_c0() {
            self.xmm_lo_qword(instr.src1())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        let mut op = BxPackedXmmRegister::default();
        op.set_xmm64u(0, val);
        self.write_xmm_reg(instr.dst(), op);
        Ok(())
    }

    /// VMOVQ WqVq (VEX.128.66.0F D6) — store low qword; the register form
    /// zeroes the destination above bit 63 (Bochs ia_opcodes.def
    /// BX_IA_VMOVQ_WqVq → MOVQ_VqWqR / MOVSD_WsdVsdM).
    pub(super) fn vmovq_wq_vq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            let mut op = BxPackedXmmRegister::default();
            op.set_xmm64u(0, self.xmm_lo_qword(instr.src1()));
            self.write_xmm_reg(instr.dst(), op);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            let val = self.xmm_lo_qword(instr.src1());
            self.v_write_qword(seg, eaddr, val)?;
        }
        Ok(())
    }

    /// VPBLENDVB — variable byte blend (Bochs avx.cc VPBLENDVB_VdqHdqWdqIbR
    /// via simd_int.h xmm_pblendvb). The mask register is is4 (imm8[7:4],
    /// decoded into src3); blending is per-byte sign bit, per 128-bit lane.
    pub(super) fn vpblendvb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let mut result = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mask = self.read_ymm_reg(instr.src3());
            for lane in 0..2 {
                let mut r = result.ymm128(lane);
                super::sse::pblendvb_lane(&mut r, &op2.ymm128(lane), &mask.ymm128(lane));
                result.set_ymm128(lane, r);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mask = self.read_xmm_reg(instr.src3());
            super::sse::pblendvb_lane(&mut result, &op2, &mask);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPUNPCKLDQ — Unpack and Interleave Low Dwords (VEX.L aware)
    /// 128-bit: dst = [src1[0], src2[0], src1[1], src2[1]]
    /// 256-bit: same per 128-bit lane
    pub(super) fn vpunpckldq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            // Lower lane
            result.set_ymm32u(0, src1.ymm32u(0));
            result.set_ymm32u(1, src2.ymm32u(0));
            result.set_ymm32u(2, src1.ymm32u(1));
            result.set_ymm32u(3, src2.ymm32u(1));
            // Upper lane
            result.set_ymm32u(4, src1.ymm32u(4));
            result.set_ymm32u(5, src2.ymm32u(4));
            result.set_ymm32u(6, src1.ymm32u(5));
            result.set_ymm32u(7, src2.ymm32u(5));
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, src1.xmm32u(0));
            result.set_xmm32u(1, src2.xmm32u(0));
            result.set_xmm32u(2, src1.xmm32u(1));
            result.set_xmm32u(3, src2.xmm32u(1));
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKHDQ — Unpack and Interleave High Dwords (VEX.L aware)
    pub(super) fn vpunpckhdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            // Lower lane: high dwords
            result.set_ymm32u(0, src1.ymm32u(2));
            result.set_ymm32u(1, src2.ymm32u(2));
            result.set_ymm32u(2, src1.ymm32u(3));
            result.set_ymm32u(3, src2.ymm32u(3));
            // Upper lane
            result.set_ymm32u(4, src1.ymm32u(6));
            result.set_ymm32u(5, src2.ymm32u(6));
            result.set_ymm32u(6, src1.ymm32u(7));
            result.set_ymm32u(7, src2.ymm32u(7));
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, src1.xmm32u(2));
            result.set_xmm32u(1, src2.xmm32u(2));
            result.set_xmm32u(2, src1.xmm32u(3));
            result.set_xmm32u(3, src2.xmm32u(3));
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBD — Packed Subtract Dwords (VEX.L aware)
    pub(super) fn vpsubd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, src1.ymm32u(i).wrapping_sub(src2.ymm32u(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, src1.xmm32u(i).wrapping_sub(src2.xmm32u(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLD — Packed Shift Left Logical Dwords by immediate (VEX.L aware)
    /// Used in EVEX as EVEX.66.0F.W0 72 /6 ib
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpslld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv — for EVEX group opcodes, dst is in vvvv
        let count = instr.ib() as u32;

        // For EVEX group opcodes, rm (source) is in dst(), nnn is opcode extension
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            if count < 32 {
                for i in 0..8 {
                    result.set_ymm32u(i, src.ymm32u(i) << count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            if count < 32 {
                for i in 0..4 {
                    result.set_xmm32u(i, src.xmm32u(i) << count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLD — Packed Shift Right Logical Dwords by immediate (VEX.L aware)
    /// Used in EVEX as EVEX.66.0F.W0 72 /2 ib
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsrld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv — for EVEX group opcodes, dst is in vvvv
        let count = instr.ib() as u32;

        // For EVEX group opcodes, rm (source) is in dst(), nnn is opcode extension
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            if count < 32 {
                for i in 0..8 {
                    result.set_ymm32u(i, src.ymm32u(i) >> count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            if count < 32 {
                for i in 0..4 {
                    result.set_xmm32u(i, src.xmm32u(i) >> count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPOR / VPORD — Packed OR (VEX.L aware)
    pub(super) fn vpor(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, src1.ymm64u(i) | src2.ymm64u(i));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, src1.xmm64u(i) | src2.xmm64u(i));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VINSERTF128 / VINSERTI128 — Insert 128-bit value into 256-bit register
    /// VEX.256.66.0F3A.W0 18 /r ib (VINSERTF128)
    /// VEX.256.66.0F3A.W0 38 /r ib (VINSERTI128)
    /// Matches Bochs VINSERTF128_VdqHdqWdqIbR (avx.cc)
    /// Both instructions perform the identical operation — integer vs float is
    /// only a naming distinction.
    /// dst = src1 (VEX.vvvv) with 128-bit lane[imm8[0]] replaced by src2 (rm)
    pub(super) fn vinsert_f128_i128(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        // Read the full 256-bit source (VEX.vvvv)
        let mut result = self.read_ymm_reg(instr.src2());
        let imm = instr.ib();

        // Read the 128-bit value to insert (rm — register or memory)
        let src2 = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_xmmword(seg, eaddr)?
        };

        // Insert into the selected 128-bit lane
        // For VEX.256: offset = imm8 & 1 (only 2 lanes)
        let offset = (imm & 1) as usize;
        let base = offset * 2; // index into ymm64u array
        result.set_ymm64u(base, src2.xmm64u(0));
        result.set_ymm64u(base + 1, src2.xmm64u(1));

        self.write_ymm_reg(instr.dst(), result);
        Ok(())
    }

    /// VEXTRACTF128 / VEXTRACTI128 — Extract 128-bit lane from 256-bit register.
    /// VEX.256.66.0F3A.W0 19 /r ib (F128) and 39 /r ib (I128).
    /// The two forms are bit-identical lane moves; Bochs `avx.cc` routes both
    /// `BX_IA_V256_VEXTRACTF128_WdqVdqIb` and `..VEXTRACTI128..` to the single
    /// `VEXTRACTF128_WdqVdqIb` handler.
    /// If imm8[0]=0: dst = src[127:0]; if imm8[0]=1: dst = src[255:128]
    /// Our decoder: dst() = nnn (source YMM), src1() = rm (destination XMM)
    pub(super) fn vextracti128(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_avx()?;
        let src_idx = instr.dst(); // nnn — source YMM register
        let imm = instr.ib();
        let src = self.read_ymm_reg(src_idx);
        let mut result = BxPackedXmmRegister::default();
        if (imm & 1) != 0 {
            // Extract upper 128 bits
            result.set_xmm64u(0, src.ymm64u(2));
            result.set_xmm64u(1, src.ymm64u(3));
        } else {
            // Extract lower 128 bits
            result.set_xmm64u(0, src.ymm64u(0));
            result.set_xmm64u(1, src.ymm64u(1));
        }

        if instr.mod_c0() {
            self.write_xmm_reg(instr.src1(), result); // rm = destination
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_write_xmmword(seg, eaddr, &result)?;
        }
        Ok(())
    }

    /// VPERM2F128 — Permute 128-bit lanes from two 256-bit sources.
    ///
    /// Bochs `VPERM2F128_VdqHdqWdqIbR` implements both:
    /// - VEX.256.66.0F3A.W0 06 /r ib (`VPERM2F128`, AVX)
    /// - VEX.256.66.0F3A.W0 46 /r ib (`VPERM2I128`, AVX2)
    ///
    /// For each 128-bit half (n=0,1): select from imm8 bits [n*4+3:n*4].
    /// bit 3 zeros that half; bit 1 selects op2 instead of op1; bit 0 selects
    /// the 128-bit half of the chosen source.
    pub(super) fn vperm2f128(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_ymm_reg(instr.src2()); // VEX.vvvv
        let op2 = if instr.mod_c0() {
            self.read_ymm_reg(instr.src1()) // rm
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_ymmword(seg, eaddr)?
        };
        let mut order = instr.ib();
        let mut result = BxPackedYmmRegister::default();

        for n in 0..2u8 {
            let base = (n as usize) * 2; // index into ymm64u (0 or 2)
            if (order & 0x8) != 0 {
                // Zero this 128-bit half
                result.set_ymm64u(base, 0);
                result.set_ymm64u(base + 1, 0);
            } else {
                let src = if (order & 0x2) != 0 { &op2 } else { &op1 };
                let half = (order & 0x1) as usize; // which 128-bit half of source
                let src_base = half * 2;
                result.set_ymm64u(base, src.ymm64u(src_base));
                result.set_ymm64u(base + 1, src.ymm64u(src_base + 1));
            }
            order >>= 4;
        }

        self.write_ymm_reg(instr.dst(), result);
        Ok(())
    }

    /// VPSHUFB — Packed Shuffle Bytes (VEX.L aware, 3-operand VEX encoding)
    /// VEX.128/256.66.0F38 00 /r
    /// Matches Bochs VPSHUFB (avx512.cc) — per-lane byte shuffle
    /// dst[i] = (mask[i] & 0x80) ? 0 : data[mask[i] & 0xF]  (within each 128-bit lane)
    pub(super) fn vpshufb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let data_idx = instr.src2(); // VEX.vvvv — data source

        if instr.get_vl() >= 1 {
            // 256-bit: two independent 128-bit lane shuffles
            let data = self.read_ymm_reg(data_idx);
            let mask = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane (bytes 0-15)
            for i in 0..16usize {
                let m = mask.ymmubyte(i);
                if (m & 0x80) != 0 {
                    result.set_ymmubyte(i, 0);
                } else {
                    result.set_ymmubyte(i, data.ymmubyte((m & 0xf) as usize));
                }
            }
            // Upper 128-bit lane (bytes 16-31) — shuffles within upper lane only
            for i in 16..32usize {
                let m = mask.ymmubyte(i);
                if (m & 0x80) != 0 {
                    result.set_ymmubyte(i, 0);
                } else {
                    result.set_ymmubyte(i, data.ymmubyte(16 + (m & 0xf) as usize));
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            // 128-bit: single lane shuffle
            let data = self.read_xmm_reg(data_idx);
            let mask = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16usize {
                let m = mask.xmmubyte(i);
                if (m & 0x80) != 0 {
                    result.set_xmmubyte(i, 0);
                } else {
                    result.set_xmmubyte(i, data.xmmubyte((m & 0xf) as usize));
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // =========================================================================
    // VPALIGNR — Packed Align Right (AVX/AVX2)
    // Bochs: avx2.cc VPALIGNR_VdqHdqWdqIbR
    // =========================================================================

    /// VPALIGNR — Packed Align Right (VEX.L aware, 3-operand)
    /// VEX.128/256.66.0F3A 0F /r ib
    /// Per 128-bit lane: result = [src1:src2] >> (imm8 * 8), where src1 is high.
    /// Bochs: op1 = src1 (vvv), op2 = src2 (rm); xmm_palignr(&op2, &op1, imm8);
    ///        write op2 to dst.
    pub(super) fn vpalignr(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let shift = instr.ib() as usize;

        if instr.get_vl() >= 1 {
            // 256-bit: two independent 128-bit lane align-right operations
            let op1 = self.read_ymm_reg(instr.src2()); // VEX.vvvv = high part
            let op2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            // Process each 128-bit lane independently
            for lane in 0..2usize {
                let base = lane * 16;
                Self::palignr_lane(&op1, &op2, base, shift, &mut result);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            // 128-bit: single lane
            let op1 = self.read_xmm_reg(instr.src2()); // VEX.vvvv = high part
            let op2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            // Concatenate [op1:op2] (32 bytes, but only 16 bytes each)
            // and extract 16 bytes starting at byte offset `shift`
            if shift >= 32 {
                // All zeros — result already zeroed
            } else if shift >= 16 {
                // Only op1 bytes contribute, shifted right
                let s = shift - 16;
                for i in 0..(16 - s) {
                    result.set_xmmubyte(i, op1.xmmubyte(i + s));
                }
            } else {
                // Both op2 and op1 contribute
                for i in 0..16usize {
                    let src_idx = i + shift;
                    if src_idx < 16 {
                        result.set_xmmubyte(i, op2.xmmubyte(src_idx));
                    } else {
                        result.set_xmmubyte(i, op1.xmmubyte(src_idx - 16));
                    }
                }
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// Helper: PALIGNR for one 128-bit lane within a YMM register.
    /// op1[base..base+16] is high, op2[base..base+16] is low.
    fn palignr_lane(
        op1: &BxPackedYmmRegister,
        op2: &BxPackedYmmRegister,
        base: usize,
        shift: usize,
        result: &mut BxPackedYmmRegister,
    ) {
        if shift >= 32 {
            // All zeros — result bytes already 0
        } else if shift >= 16 {
            let s = shift - 16;
            for i in 0..(16 - s) {
                result.set_ymmubyte(base + i, op1.ymmubyte(base + i + s));
            }
        } else {
            for i in 0..16usize {
                let src_idx = i + shift;
                if src_idx < 16 {
                    result.set_ymmubyte(base + i, op2.ymmubyte(base + src_idx));
                } else {
                    result.set_ymmubyte(base + i, op1.ymmubyte(base + src_idx - 16));
                }
            }
        }
    }

    // =========================================================================
    // VPBLENDD — Blend Packed Dwords (AVX2)
    // Bochs: VPBLENDD_VdqHdqWdqIbR → uses same logic as VBLENDPS
    // =========================================================================

    /// VPBLENDD — Blend packed dwords by immediate mask (VEX.L aware)
    /// VEX.128/256.66.0F3A.W0 02 /r ib
    /// For each dword lane i: dst[i] = (imm8 & (1<<i)) ? src2[i] : src1[i]
    /// src1 = VEX.vvvv (src2()), src2 = rm (src1())
    pub(super) fn vpblendd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let imm8 = instr.ib();
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv

        if instr.get_vl() >= 1 {
            // 256-bit: 8 dwords
            let src1 = self.read_ymm_reg(src1_idx);
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8usize {
                if (imm8 & (1 << i)) != 0 {
                    result.set_ymm32u(i, src2.ymm32u(i));
                } else {
                    result.set_ymm32u(i, src1.ymm32u(i));
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            // 128-bit: 4 dwords (only bits 0-3 of imm8 matter)
            let src1 = self.read_xmm_reg(src1_idx);
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4usize {
                if (imm8 & (1 << i)) != 0 {
                    result.set_xmm32u(i, src2.xmm32u(i));
                } else {
                    result.set_xmm32u(i, src1.xmm32u(i));
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // =========================================================================
    // VPBROADCAST — Broadcast scalar to all elements (AVX2)
    // Bochs: avx2.cc VPBROADCASTB/W/D/Q
    // =========================================================================

    /// VPBROADCASTB — broadcast byte from XMM[0] to all bytes of dst
    pub(super) fn vpbroadcastb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_byte = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmmubyte(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_byte(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                result.set_ymmubyte(i, src_byte);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                result.set_xmmubyte(i, src_byte);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPBROADCASTW — broadcast word from XMM[0] to all words of dst
    pub(super) fn vpbroadcastw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_word = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm16u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_word(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(i, src_word);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(i, src_word);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPBROADCASTD — broadcast dword from XMM[0] to all dwords of dst
    pub(super) fn vpbroadcastd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_dword = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm32u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_dword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, src_dword);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, src_dword);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPBROADCASTQ — broadcast qword from XMM[0] to all qwords of dst
    pub(super) fn vpbroadcastq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_qword = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, src_qword);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, src_qword);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VBROADCASTF128 / VBROADCASTI128 — load 128-bit from memory, copy to both YMM lanes
    /// Bochs: avx.cc VBROADCASTF128_VdqMdq (shared handler for both F128 and I128)
    pub(super) fn vbroadcast_f128_i128(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let src = self.v_read_xmmword(seg, eaddr)?;

        let mut result = BxPackedYmmRegister::default();
        result.set_ymm128(0, src);
        result.set_ymm128(1, src);
        self.write_ymm_reg(instr.dst(), result);
        Ok(())
    }

    /// VPERMD — Permute dwords in YMM using index from another YMM (AVX2)
    /// Bochs: avx2.cc V256_VPERMD_VdqHdqWdq
    pub(super) fn vpermd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let idx = self.read_ymm_reg(instr.src2()); // VEX.vvvv = index
        let src = if instr.mod_c0() {
            self.read_ymm_reg(instr.src1())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_ymmword(seg, eaddr)?
        };
        let mut result = BxPackedYmmRegister::default();
        for i in 0..8 {
            let sel = (idx.ymm32u(i) & 7) as usize;
            result.set_ymm32u(i, src.ymm32u(sel));
        }
        self.write_ymm_reg(instr.dst(), result);
        Ok(())
    }

    /// VPERMQ — Permute qwords in YMM by immediate (AVX2)
    /// Bochs: avx2.cc VPERMQ_VdqWdqIbR / simd_int.h ymm_vpermq.
    /// Operand W is the ModRM r/m source; VEX.vvvv is unused for this form.
    pub(super) fn vpermq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src = if instr.mod_c0() {
            self.read_ymm_reg(instr.src1())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_ymmword(seg, eaddr)?
        };
        let imm = instr.ib();
        let mut result = BxPackedYmmRegister::default();
        for qword in 0..4 {
            let sel = ((imm >> (qword * 2)) & 0x03) as usize;
            result.set_ymm64u(qword, src.ymm64u(sel));
        }
        self.write_ymm_reg(instr.dst(), result);
        Ok(())
    }

    /// VPERMILPS — per-128-bit-lane single-precision permute with a variable
    /// control vector (Bochs `HANDLE_AVX_3OP<xmm_permilps>`, simd_int.h
    /// `xmm_permilps`). VEX.vvvv supplies the data, ModRM.rm the selectors;
    /// each selector uses only its own lane, so no data crosses lanes.
    pub(super) fn vpermilps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let data = self.read_ymm_reg(instr.src2());
            let ctl = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                for n in 0..4 {
                    let sel = (ctl.ymm32u(lane * 4 + n) & 0x3) as usize;
                    result.set_ymm32u(lane * 4 + n, data.ymm32u(lane * 4 + sel));
                }
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let data = self.read_xmm_reg(instr.src2());
            let ctl = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for n in 0..4 {
                let sel = (ctl.xmm32u(n) & 0x3) as usize;
                result.set_xmm32u(n, data.xmm32u(sel));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPERMILPD — per-128-bit-lane double-precision permute with a variable
    /// control vector (Bochs `HANDLE_AVX_3OP<xmm_permilpd>`, simd_int.h
    /// `xmm_permilpd`). The selector for each qword is bit 1 of the *even*
    /// dword of that qword pair — dwords 0 and 2 within the lane.
    pub(super) fn vpermilpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let data = self.read_ymm_reg(instr.src2());
            let ctl = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base32 = lane * 4;
                let base64 = lane * 2;
                let s0 = ((ctl.ymm32u(base32) >> 1) & 0x1) as usize;
                let s1 = ((ctl.ymm32u(base32 + 2) >> 1) & 0x1) as usize;
                result.set_ymm64u(base64, data.ymm64u(base64 + s0));
                result.set_ymm64u(base64 + 1, data.ymm64u(base64 + s1));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let data = self.read_xmm_reg(instr.src2());
            let ctl = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            let s0 = ((ctl.xmm32u(0) >> 1) & 0x1) as usize;
            let s1 = ((ctl.xmm32u(2) >> 1) & 0x1) as usize;
            result.set_xmm64u(0, data.xmm64u(s0));
            result.set_xmm64u(1, data.xmm64u(s1));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPERMILPS with an immediate control (Bochs avx.cc
    /// `VPERMILPS_VpsWpsIbR`, which applies the same imm8 to every lane via
    /// `xmm_shufps(result, op1, op1, Ib)`). Source is ModRM.rm; no vvvv.
    pub(super) fn vpermilps_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let order = instr.ib();
        if instr.get_vl() >= 1 {
            let src = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 4;
                for n in 0..4 {
                    let sel = ((order >> (n * 2)) & 0x3) as usize;
                    result.set_ymm32u(base + n, src.ymm32u(base + sel));
                }
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for n in 0..4 {
                let sel = ((order >> (n * 2)) & 0x3) as usize;
                result.set_xmm32u(n, src.xmm32u(sel));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPERMILPD with an immediate control (Bochs avx.cc
    /// `VPERMILPD_VpdWpdIbR`). Each lane consumes two imm8 bits — Bochs shifts
    /// `order >>= 2` after every 128-bit lane, so the upper lane uses bits
    /// [3:2] even though only one bit per qword is significant.
    pub(super) fn vpermilpd_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let order = instr.ib();
        if instr.get_vl() >= 1 {
            let src = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 2;
                let bits = order >> (lane * 2);
                result.set_ymm64u(base, src.ymm64u(base + (bits & 0x1) as usize));
                result.set_ymm64u(base + 1, src.ymm64u(base + ((bits >> 1) & 0x1) as usize));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let src = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, src.xmm64u((order & 0x1) as usize));
            result.set_xmm64u(1, src.xmm64u(((order >> 1) & 0x1) as usize));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// AVX2 per-element variable shifts — VPSRLVD/Q, VPSRAVD, VPSLLVD/Q
    /// (Bochs `HANDLE_AVX_2OP<xmm_psrlvd>` and friends). VEX.vvvv holds the
    /// values, ModRM.rm the per-element counts; a count wider than the element
    /// yields 0 for the logical shifts and the replicated sign for VPSRAVD.
    pub(super) fn vex_var_shift(
        &mut self,
        instr: &Instruction,
        op: VexVarShiftOp,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let (values, counts) = if instr.get_vl() >= 1 {
            (
                self.read_ymm_reg(instr.src2()),
                self.vex_read_src2_ymm(instr)?,
            )
        } else {
            let v = self.read_xmm_reg(instr.src2());
            let c = self.vex_read_src2_xmm(instr)?;
            let (mut vy, mut cy) = (
                BxPackedYmmRegister::default(),
                BxPackedYmmRegister::default(),
            );
            vy.set_ymm128(0, v);
            cy.set_ymm128(0, c);
            (vy, cy)
        };
        let dwords = if instr.get_vl() >= 1 { 8 } else { 4 };
        let qwords = dwords / 2;
        let mut result = BxPackedYmmRegister::default();
        match op {
            VexVarShiftOp::SrlD => {
                for n in 0..dwords {
                    let shift = counts.ymm32u(n);
                    result.set_ymm32u(n, if shift > 31 { 0 } else { values.ymm32u(n) >> shift });
                }
            }
            VexVarShiftOp::SllD => {
                for n in 0..dwords {
                    let shift = counts.ymm32u(n);
                    result.set_ymm32u(n, if shift > 31 { 0 } else { values.ymm32u(n) << shift });
                }
            }
            VexVarShiftOp::SraD => {
                for n in 0..dwords {
                    let shift = counts.ymm32u(n);
                    let v = values.ymm32s(n);
                    let r = if shift > 31 {
                        if v < 0 {
                            -1i32
                        } else {
                            0
                        }
                    } else {
                        v >> shift
                    };
                    result.set_ymm32u(n, r as u32);
                }
            }
            VexVarShiftOp::SrlQ => {
                for n in 0..qwords {
                    let shift = counts.ymm64u(n);
                    result.set_ymm64u(n, if shift > 63 { 0 } else { values.ymm64u(n) >> shift });
                }
            }
            VexVarShiftOp::SllQ => {
                for n in 0..qwords {
                    let shift = counts.ymm64u(n);
                    result.set_ymm64u(n, if shift > 63 { 0 } else { values.ymm64u(n) << shift });
                }
            }
        }
        if instr.get_vl() >= 1 {
            self.write_ymm_reg(instr.dst(), result);
        } else {
            self.write_xmm_reg(instr.dst(), result.ymm128(0));
        }
        Ok(())
    }

    /// Collapse the element sign bits of the `vvvv` mask register into a bit
    /// per element. Bochs builds the same value with `xmm_pmovmskd` /
    /// `xmm_pmovmskq` over the two 128-bit lanes (avx.cc VMASKMOVPS_VpsHpsMps).
    fn vex_mask_bits(mask: &BxPackedYmmRegister, qword: bool, elements: usize) -> u32 {
        let mut bits = 0u32;
        for n in 0..elements {
            let signed = if qword {
                (mask.ymm64u(n) >> 63) != 0
            } else {
                (mask.ymm32u(n) >> 31) != 0
            };
            if signed {
                bits |= 1 << n;
            }
        }
        bits
    }

    /// Element count for a masked move at the instruction's vector length.
    fn vex_mask_elements(instr: &Instruction, qword: bool) -> usize {
        match (instr.get_vl() >= 1, qword) {
            (true, false) => 8,
            (true, true) => 4,
            (false, false) => 4,
            (false, true) => 2,
        }
    }

    /// VMASKMOVPS / VMASKMOVPD / VPMASKMOVD / VPMASKMOVQ, load direction.
    /// Bochs avx.cc `VMASKMOVPS_VpsHpsMps` via avx512_helpers.cc
    /// `avx_masked_load32` / `avx_masked_load64`.
    ///
    /// Masked-off elements are read as zero and — critically — are never
    /// accessed, so they cannot fault. The accesses stay element-at-a-time for
    /// exactly that reason; a single wide load would turn suppressed faults
    /// into real ones.
    pub(super) fn vmaskmov_load(&mut self, instr: &Instruction, qword: bool) -> super::Result<()> {
        self.prepare_avx()?;
        let elements = Self::vex_mask_elements(instr, qword);
        let mask = self.read_ymm_reg(instr.src2()); // vvvv
        let bits = Self::vex_mask_bits(&mask, qword, elements);
        let esize = if qword { 8u64 } else { 4u64 };
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);

        // Bochs checks every masked-in element for canonicality up front, so a
        // non-canonical element raises #GP/#SS before any element is read.
        if instr.as64_l() != 0 {
            let laddr = self.get_laddr64(seg as usize, eaddr);
            for n in 0..elements {
                if (bits & (1 << n)) != 0 && !self.is_canonical(laddr.wrapping_add(esize * n as u64))
                {
                    return self.exception(Self::seg_exception(seg), 0);
                }
            }
        }

        let mut result = BxPackedYmmRegister::default();
        // Bochs walks high element to low so the highest address faults first.
        for n in (0..elements).rev() {
            if (bits & (1 << n)) == 0 {
                continue; // masked off: stays zero, no memory access
            }
            let addr = eaddr.wrapping_add(esize * n as u64);
            if qword {
                let v = self.v_read_qword(seg, addr)?;
                result.set_ymm64u(n, v);
            } else {
                let v = self.v_read_dword(seg, addr)?;
                result.set_ymm32u(n, v);
            }
        }

        if instr.get_vl() >= 1 {
            self.write_ymm_reg(instr.dst(), result);
        } else {
            self.write_xmm_reg(instr.dst(), result.ymm128(0));
        }
        Ok(())
    }

    /// VMASKMOVPS / VMASKMOVPD / VPMASKMOVD / VPMASKMOVQ, store direction.
    /// Bochs avx.cc `VMASKMOVPS_MpsHpsVps` via avx512_helpers.cc
    /// `avx_masked_store32` / `avx_masked_store64`.
    ///
    /// Bochs probes every masked-in element with an unlocked RMW read before
    /// writing any of them, so the store either faults with memory untouched
    /// or completes in full. Masked-off elements are left alone.
    pub(super) fn vmaskmov_store(&mut self, instr: &Instruction, qword: bool) -> super::Result<()> {
        self.prepare_avx()?;
        let elements = Self::vex_mask_elements(instr, qword);
        let mask = self.read_ymm_reg(instr.src2()); // vvvv
        let data = self.read_ymm_reg(instr.dst()); // nnn
        let bits = Self::vex_mask_bits(&mask, qword, elements);
        let esize = if qword { 8u64 } else { 4u64 };
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);

        if instr.as64_l() != 0 {
            let laddr = self.get_laddr64(seg as usize, eaddr);
            for n in 0..elements {
                if (bits & (1 << n)) != 0 && !self.is_canonical(laddr.wrapping_add(esize * n as u64))
                {
                    return self.exception(Self::seg_exception(seg), 0);
                }
            }
        }

        // Probe pass — the read value is deliberately unused; this exists only
        // so a fault on any element happens before the first write.
        for n in (0..elements).rev() {
            if (bits & (1 << n)) == 0 {
                continue;
            }
            let addr = eaddr.wrapping_add(esize * n as u64);
            if qword {
                self.v_read_rmw_qword(seg, addr)?;
            } else {
                self.v_read_rmw_dword(seg, addr)?;
            }
        }

        for n in 0..elements {
            if (bits & (1 << n)) == 0 {
                continue;
            }
            let addr = eaddr.wrapping_add(esize * n as u64);
            if qword {
                self.v_write_qword(seg, addr, data.ymm64u(n))?;
            } else {
                self.v_write_dword(seg, addr, data.ymm32u(n))?;
            }
        }
        Ok(())
    }

    /// VPCLMULQDQ — carry-less multiply, per 128-bit lane
    /// (Bochs avx_pclmul.cc `VPCLMULQDQ_VdqHdqWdqIbR`). Unlike the legacy
    /// 3-operand form this sources its first operand from VEX.vvvv, and the
    /// 256-bit form (the VPCLMULQDQ extension) applies the same imm8 selectors
    /// independently to each lane.
    pub(super) fn vpclmulqdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_avx()?;
        let imm8 = instr.ib();
        let sel1 = (imm8 & 1) as usize;
        let sel2 = ((imm8 >> 4) & 1) as usize;

        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2()); // vvvv
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let a = op1.ymm64u(lane * 2 + sel1);
                let b = op2.ymm64u(lane * 2 + sel2);
                result.set_ymm128(lane, super::aes::xmm_pclmulqdq(a, b));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2()); // vvvv
            let op2 = self.vex_read_src2_xmm(instr)?;
            let result = super::aes::xmm_pclmulqdq(op1.xmm64u(sel1), op2.xmm64u(sel2));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VCVTPH2PS — widen packed half-precision to single-precision
    /// (Bochs avx_cvt.cc `VCVTPH2PS_VpsWpsR`). The source is half the
    /// destination width, so the memory form touches VL/2 bytes
    /// (Bochs `LOAD_Half_Vector`).
    pub(super) fn vcvtph2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_avx()?;
        let mut status = mxcsr_to_softfloat_status_word(self.mxcsr);
        // Bochs: ignore MXCSR.DAZ and never report #D for this conversion.
        status.softfloat_denormals_are_zeros = false;
        status.softfloat_suppressException = FLAG_DENORMAL;

        let elements = if instr.get_vl() >= 1 { 8 } else { 4 };
        let src = self.vex_pmov_read(instr, elements * 2)?;
        let mut result = BxPackedYmmRegister::default();
        for n in 0..elements {
            let half = u16::from_le_bytes([src[n * 2], src[n * 2 + 1]]);
            result.set_ymm32u(n, f16_to_f32(half, &mut status));
        }
        if instr.get_vl() >= 1 {
            self.write_ymm_reg(instr.dst(), result);
        } else {
            self.write_xmm_reg(instr.dst(), result.ymm128(0));
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))
    }

    /// VCVTPS2PH — narrow packed single-precision to half-precision
    /// (Bochs avx_cvt.cc `VCVTPS2PH_WpsVpsIb`). The destination is ModRM.rm
    /// and is half the source width; nnn is the source. imm8[2] selects
    /// MXCSR.RC, otherwise imm8[1:0] overrides the rounding mode.
    pub(super) fn vcvtps2ph(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_avx()?;
        let mut status = mxcsr_to_softfloat_status_word(self.mxcsr);
        let control = instr.ib();
        // Bochs: ignore MXCSR.FUZ; imm8 may override the rounding mode.
        status.softfloat_flush_underflow_to_zero = false;
        if (control & 0x4) == 0 {
            status.softfloat_roundingMode = control & 0x3;
        }

        let src = self.read_ymm_reg(instr.dst()); // nnn = source
        let elements = if instr.get_vl() >= 1 { 8 } else { 4 };
        let mut packed = BxPackedXmmRegister::default();
        for n in 0..elements {
            packed.set_xmm16u(n, f32_to_f16(src.ymm32u(n), &mut status));
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;

        if instr.mod_c0() {
            // Register destination: VL256 writes the full low xmm, VL128 only
            // the low qword. Both clear everything above.
            let mut out = BxPackedXmmRegister::default();
            if instr.get_vl() >= 1 {
                out = packed;
            } else {
                out.set_xmm64u(0, packed.xmm64u(0));
            }
            self.write_xmm_reg(instr.src1(), out); // rm = destination
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                self.v_write_xmmword(seg, eaddr, &packed)?;
            } else {
                self.v_write_qword(seg, eaddr, packed.xmm64u(0))?;
            }
        }
        Ok(())
    }

    /// VEX FMA packed single-precision helper.
    pub(super) fn vex_fma_packed_ps(
        &mut self,
        instr: &Instruction,
        form: VexFmaForm,
        op: VexPackedFmaOp,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        self.require_fma()?;
        let mut status = mxcsr_to_softfloat_status_word(self.mxcsr);
        let dst_idx = instr.dst();
        if instr.get_vl() >= 1 {
            let v = self.read_ymm_reg(dst_idx);
            let h = self.read_ymm_reg(instr.src2());
            let w = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..8 {
                let (a, b, c) =
                    vex_fma_operands_u32(form, v.ymm32u(lane), h.ymm32u(lane), w.ymm32u(lane));
                result.set_ymm32u(
                    lane,
                    f32_mul_add(a, b, c, packed_fma_flags(op, lane), &mut status),
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let v = self.read_xmm_reg(dst_idx);
            let h = self.read_xmm_reg(instr.src2());
            let w = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for lane in 0..4 {
                let (a, b, c) =
                    vex_fma_operands_u32(form, v.xmm32u(lane), h.xmm32u(lane), w.xmm32u(lane));
                result.set_xmm32u(
                    lane,
                    f32_mul_add(a, b, c, packed_fma_flags(op, lane), &mut status),
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))
    }

    /// VEX FMA packed double-precision helper.
    pub(super) fn vex_fma_packed_pd(
        &mut self,
        instr: &Instruction,
        form: VexFmaForm,
        op: VexPackedFmaOp,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        self.require_fma()?;
        let mut status = mxcsr_to_softfloat_status_word(self.mxcsr);
        let dst_idx = instr.dst();
        if instr.get_vl() >= 1 {
            let v = self.read_ymm_reg(dst_idx);
            let h = self.read_ymm_reg(instr.src2());
            let w = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..4 {
                let (a, b, c) =
                    vex_fma_operands_u64(form, v.ymm64u(lane), h.ymm64u(lane), w.ymm64u(lane));
                result.set_ymm64u(
                    lane,
                    f64_mul_add(a, b, c, packed_fma_flags(op, lane), &mut status),
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let v = self.read_xmm_reg(dst_idx);
            let h = self.read_xmm_reg(instr.src2());
            let w = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for lane in 0..2 {
                let (a, b, c) =
                    vex_fma_operands_u64(form, v.xmm64u(lane), h.xmm64u(lane), w.xmm64u(lane));
                result.set_xmm64u(
                    lane,
                    f64_mul_add(a, b, c, packed_fma_flags(op, lane), &mut status),
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))
    }

    /// VEX FMA scalar single-precision helper.
    pub(super) fn vex_fma_scalar_ss(
        &mut self,
        instr: &Instruction,
        form: VexFmaForm,
        op: VexScalarFmaOp,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        self.require_fma()?;
        let mut status = mxcsr_to_softfloat_status_word(self.mxcsr);
        let dst_idx = instr.dst();
        let mut result = self.read_xmm_reg(dst_idx);
        let v = result.xmm32u(0);
        let h = self.read_xmm_reg(instr.src2()).xmm32u(0);
        let w = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1()).xmm32u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_dword(seg, eaddr)?
        };
        let (a, b, c) = vex_fma_operands_u32(form, v, h, w);
        result.set_xmm32u(0, f32_mul_add(a, b, c, scalar_fma_flags(op), &mut status));
        self.write_xmm_reg(dst_idx, result);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))
    }

    /// VEX FMA scalar double-precision helper.
    pub(super) fn vex_fma_scalar_sd(
        &mut self,
        instr: &Instruction,
        form: VexFmaForm,
        op: VexScalarFmaOp,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        self.require_fma()?;
        let mut status = mxcsr_to_softfloat_status_word(self.mxcsr);
        let dst_idx = instr.dst();
        let mut result = self.read_xmm_reg(dst_idx);
        let v = result.xmm64u(0);
        let h = self.read_xmm_reg(instr.src2()).xmm64u(0);
        let w = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1()).xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        let (a, b, c) = vex_fma_operands_u64(form, v, h, w);
        result.set_xmm64u(0, f64_mul_add(a, b, c, scalar_fma_flags(op), &mut status));
        self.write_xmm_reg(dst_idx, result);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))
    }

    /// #UD when the CPU model does not advertise FMA (Bochs `BX_ISA_AVX_FMA`).
    #[inline]
    fn require_fma(&mut self) -> super::Result<()> {
        if !self.bx_cpuid_support_isa_extension(X86Feature::IsaAvxFma) {
            return self.exception(Exception::Ud, 0);
        }
        Ok(())
    }

    /// VFMADD132PS — V * W + H, packed single-precision (VEX FMA)
    pub(super) fn vfmadd132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vex_fma_packed_ps(instr, VexFmaForm::F132, VexPackedFmaOp::Fmadd)
    }

    /// VFMADD132PD — V * W + H, packed double-precision (VEX FMA)
    pub(super) fn vfmadd132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vex_fma_packed_pd(instr, VexFmaForm::F132, VexPackedFmaOp::Fmadd)
    }

    /// VFMADD132SS — scalar single: low f32 = V * W + H.
    pub(super) fn vfmadd132ss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vex_fma_scalar_ss(instr, VexFmaForm::F132, VexScalarFmaOp::Fmadd)
    }

    /// VFMADD132SD — scalar double: low f64 = V * W + H.
    pub(super) fn vfmadd132sd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vex_fma_scalar_sd(instr, VexFmaForm::F132, VexScalarFmaOp::Fmadd)
    }

    // ========================================================================
    // Unpack byte/word/qword variants
    // ========================================================================

    /// VPUNPCKLBW — Unpack and Interleave Low Bytes (VEX.L aware)
    /// Per 128-bit lane: result[2i] = src1[i], result[2i+1] = src2[i] for i in 0..8
    pub(super) fn vpunpcklbw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane
            for i in 0..8usize {
                result.set_ymmubyte(i * 2, src1.ymmubyte(i));
                result.set_ymmubyte(i * 2 + 1, src2.ymmubyte(i));
            }
            // Upper 128-bit lane
            for i in 0..8usize {
                result.set_ymmubyte(16 + i * 2, src1.ymmubyte(16 + i));
                result.set_ymmubyte(16 + i * 2 + 1, src2.ymmubyte(16 + i));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8usize {
                result.set_xmmubyte(i * 2, src1.xmmubyte(i));
                result.set_xmmubyte(i * 2 + 1, src2.xmmubyte(i));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKHBW — Unpack and Interleave High Bytes (VEX.L aware)
    /// Per 128-bit lane: result[2i] = src1[8+i], result[2i+1] = src2[8+i] for i in 0..8
    pub(super) fn vpunpckhbw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane (high bytes 8..16)
            for i in 0..8usize {
                result.set_ymmubyte(i * 2, src1.ymmubyte(8 + i));
                result.set_ymmubyte(i * 2 + 1, src2.ymmubyte(8 + i));
            }
            // Upper 128-bit lane (high bytes 24..32)
            for i in 0..8usize {
                result.set_ymmubyte(16 + i * 2, src1.ymmubyte(24 + i));
                result.set_ymmubyte(16 + i * 2 + 1, src2.ymmubyte(24 + i));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8usize {
                result.set_xmmubyte(i * 2, src1.xmmubyte(8 + i));
                result.set_xmmubyte(i * 2 + 1, src2.xmmubyte(8 + i));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKLWD — Unpack and Interleave Low Words (VEX.L aware)
    /// Per 128-bit lane: result[2i] = src1[i], result[2i+1] = src2[i] for i in 0..4
    pub(super) fn vpunpcklwd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane
            for i in 0..4usize {
                result.set_ymm16u(i * 2, src1.ymm16u(i));
                result.set_ymm16u(i * 2 + 1, src2.ymm16u(i));
            }
            // Upper 128-bit lane
            for i in 0..4usize {
                result.set_ymm16u(8 + i * 2, src1.ymm16u(8 + i));
                result.set_ymm16u(8 + i * 2 + 1, src2.ymm16u(8 + i));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4usize {
                result.set_xmm16u(i * 2, src1.xmm16u(i));
                result.set_xmm16u(i * 2 + 1, src2.xmm16u(i));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKHWD — Unpack and Interleave High Words (VEX.L aware)
    /// Per 128-bit lane: result[2i] = src1[4+i], result[2i+1] = src2[4+i] for i in 0..4
    pub(super) fn vpunpckhwd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane (high words 4..8)
            for i in 0..4usize {
                result.set_ymm16u(i * 2, src1.ymm16u(4 + i));
                result.set_ymm16u(i * 2 + 1, src2.ymm16u(4 + i));
            }
            // Upper 128-bit lane (high words 12..16)
            for i in 0..4usize {
                result.set_ymm16u(8 + i * 2, src1.ymm16u(12 + i));
                result.set_ymm16u(8 + i * 2 + 1, src2.ymm16u(12 + i));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4usize {
                result.set_xmm16u(i * 2, src1.xmm16u(4 + i));
                result.set_xmm16u(i * 2 + 1, src2.xmm16u(4 + i));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKLQDQ — Unpack and Interleave Low Qwords (VEX.L aware)
    /// Per 128-bit lane: result[0] = src1[0], result[1] = src2[0]
    pub(super) fn vpunpcklqdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            // Lower lane
            result.set_ymm64u(0, src1.ymm64u(0));
            result.set_ymm64u(1, src2.ymm64u(0));
            // Upper lane
            result.set_ymm64u(2, src1.ymm64u(2));
            result.set_ymm64u(3, src2.ymm64u(2));
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, src1.xmm64u(0));
            result.set_xmm64u(1, src2.xmm64u(0));
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKHQDQ — Unpack and Interleave High Qwords (VEX.L aware)
    /// Per 128-bit lane: result[0] = src1[1], result[1] = src2[1]
    pub(super) fn vpunpckhqdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            // Lower lane
            result.set_ymm64u(0, src1.ymm64u(1));
            result.set_ymm64u(1, src2.ymm64u(1));
            // Upper lane
            result.set_ymm64u(2, src1.ymm64u(3));
            result.set_ymm64u(3, src2.ymm64u(3));
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, src1.xmm64u(1));
            result.set_xmm64u(1, src2.xmm64u(1));
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed integer add/sub (byte, word, qword widths)
    // ========================================================================

    /// VPADDQ — Packed Add Qwords (VEX.L aware)
    /// dst[i] = vvvv[i] + src[i] (element-wise 64-bit wrapping add)
    pub(super) fn vpaddq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, src1.ymm64u(i).wrapping_add(src2.ymm64u(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, src1.xmm64u(i).wrapping_add(src2.xmm64u(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDW — Packed Add Words (VEX.L aware)
    /// dst[i] = vvvv[i] + src[i] (element-wise 16-bit wrapping add)
    pub(super) fn vpaddw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(i, src1.ymm16u(i).wrapping_add(src2.ymm16u(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(i, src1.xmm16u(i).wrapping_add(src2.xmm16u(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDB — Packed Add Bytes (VEX.L aware)
    /// dst[i] = vvvv[i] + src[i] (element-wise 8-bit wrapping add)
    pub(super) fn vpaddb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                result.set_ymmubyte(i, src1.ymmubyte(i).wrapping_add(src2.ymmubyte(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                result.set_xmmubyte(i, src1.xmmubyte(i).wrapping_add(src2.xmmubyte(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBQ — Packed Subtract Qwords (VEX.L aware)
    /// dst[i] = vvvv[i] - src[i] (element-wise 64-bit wrapping sub)
    pub(super) fn vpsubq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, src1.ymm64u(i).wrapping_sub(src2.ymm64u(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, src1.xmm64u(i).wrapping_sub(src2.xmm64u(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBW — Packed Subtract Words (VEX.L aware)
    /// dst[i] = vvvv[i] - src[i] (element-wise 16-bit wrapping sub)
    pub(super) fn vpsubw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(i, src1.ymm16u(i).wrapping_sub(src2.ymm16u(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(i, src1.xmm16u(i).wrapping_sub(src2.xmm16u(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBB — Packed Subtract Bytes (VEX.L aware)
    /// dst[i] = vvvv[i] - src[i] (element-wise 8-bit wrapping sub)
    pub(super) fn vpsubb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                result.set_ymmubyte(i, src1.ymmubyte(i).wrapping_sub(src2.ymmubyte(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                result.set_xmmubyte(i, src1.xmmubyte(i).wrapping_sub(src2.xmmubyte(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed logical: VPANDN
    // ========================================================================

    /// VPANDN — Packed AND NOT (VEX.L aware)
    /// dst[i] = NOT(vvvv[i]) AND src[i]
    pub(super) fn vpandn(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, !src1.ymm64u(i) & src2.ymm64u(i));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, !src1.xmm64u(i) & src2.xmm64u(i));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed multiply
    // ========================================================================

    /// VPMULUDQ — Unsigned Multiply Dwords to Qwords (VEX.L aware)
    /// dst_q[i] = (vvvv_d[i*2] as u64) * (src_d[i*2] as u64)
    /// Uses even-numbered dwords only, produces qword results
    pub(super) fn vpmuludq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, (src1.ymm32u(i * 2) as u64) * (src2.ymm32u(i * 2) as u64));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(i, (src1.xmm32u(i * 2) as u64) * (src2.xmm32u(i * 2) as u64));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULDQ — Signed Multiply Dwords to Qwords (VEX.L aware)
    /// dst_q[i] = (vvvv_d[i*2] as i32 as i64) * (src_d[i*2] as i32 as i64)
    /// Uses even-numbered dwords only (signed), produces qword results
    pub(super) fn vpmuldq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                let a = src1.ymm32s(i * 2) as i64;
                let b = src2.ymm32s(i * 2) as i64;
                result.set_ymm64u(i, (a * b) as u64);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                let a = src1.xmm32s(i * 2) as i64;
                let b = src2.xmm32s(i * 2) as i64;
                result.set_xmm64u(i, (a * b) as u64);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULLD — Packed Multiply Low Dwords (VEX.L aware)
    /// dst[i] = (vvvv[i] as i32).wrapping_mul(src[i] as i32) as u32
    pub(super) fn vpmulld(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, (src1.ymm32s(i) as i64 * src2.ymm32s(i) as i64) as u32);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, (src1.xmm32s(i) as i64 * src2.xmm32s(i) as i64) as u32);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULLW — Packed Multiply Low Words (VEX.L aware)
    /// dst[i] = low 16 bits of (vvvv[i] * src[i])
    pub(super) fn vpmullw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                let prod = (src1.ymm16s(i) as i32) * (src2.ymm16s(i) as i32);
                result.set_ymm16u(i, prod as u16);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                let prod = (src1.xmm16s(i) as i32) * (src2.xmm16s(i) as i32);
                result.set_xmm16u(i, prod as u16);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULHW — Packed Multiply High Words Signed (VEX.L aware)
    /// dst[i] = high 16 bits of (vvvv[i] as i16 * src[i] as i16)
    pub(super) fn vpmulhw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                let prod = (src1.ymm16s(i) as i32) * (src2.ymm16s(i) as i32);
                result.set_ymm16u(i, (prod >> 16) as u16);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                let prod = (src1.xmm16s(i) as i32) * (src2.xmm16s(i) as i32);
                result.set_xmm16u(i, (prod >> 16) as u16);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULHUW — Packed Multiply High Words Unsigned (VEX.L aware)
    /// dst[i] = high 16 bits of (vvvv[i] as u16 * src[i] as u16)
    pub(super) fn vpmulhuw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                let prod = (src1.ymm16u(i) as u32) * (src2.ymm16u(i) as u32);
                result.set_ymm16u(i, (prod >> 16) as u16);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                let prod = (src1.xmm16u(i) as u32) * (src2.xmm16u(i) as u32);
                result.set_xmm16u(i, (prod >> 16) as u16);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULHRSW — Packed Multiply High with Round and Scale (VEX.L aware)
    /// Bochs simd_int.h: result[i] = (((src1[i] * src2[i]) >> 14) + 1) >> 1
    pub(super) fn vpmulhrsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                let t = ((src1.ymm16s(i) as i32 * src2.ymm16s(i) as i32) >> 14) + 1;
                result.set_ymm16u(i, (t >> 1) as u16);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                let t = ((src1.xmm16s(i) as i32 * src2.xmm16s(i) as i32) >> 14) + 1;
                result.set_xmm16u(i, (t >> 1) as u16);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    #[inline]
    pub(super) fn vex_read_src2_xmm(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src1()))
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_xmmword(seg, eaddr)
        }
    }

    #[inline]
    pub(super) fn vex_read_src2_ymm(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedYmmRegister> {
        if instr.mod_c0() {
            Ok(self.read_ymm_reg(instr.src1()))
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_ymmword(seg, eaddr)
        }
    }

    #[inline]
    fn vex_fp_logical_qword(src1: u64, src2: u64, op: VexFpLogicalOp) -> u64 {
        match op {
            VexFpLogicalOp::And => src1 & src2,
            VexFpLogicalOp::AndNot => !src1 & src2,
            VexFpLogicalOp::Or => src1 | src2,
            VexFpLogicalOp::Xor => src1 ^ src2,
        }
    }

    fn vex_fp_logical(&mut self, instr: &Instruction, op: VexFpLogicalOp) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        // Bochs HANDLE_AVX_2OP<xmm_{andps,andnps,orps,xorps}>.
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(instr.src2());
            let src2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for qword in 0..4 {
                result.set_ymm64u(
                    qword,
                    Self::vex_fp_logical_qword(src1.ymm64u(qword), src2.ymm64u(qword), op),
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(instr.src2());
            let src2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for qword in 0..2 {
                result.set_xmm64u(
                    qword,
                    Self::vex_fp_logical_qword(src1.xmm64u(qword), src2.xmm64u(qword), op),
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    pub(super) fn vandps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vex_fp_logical(instr, VexFpLogicalOp::And)
    }

    pub(super) fn vandnps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vex_fp_logical(instr, VexFpLogicalOp::AndNot)
    }

    pub(super) fn vorps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vex_fp_logical(instr, VexFpLogicalOp::Or)
    }

    pub(super) fn vxorps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vex_fp_logical(instr, VexFpLogicalOp::Xor)
    }

    pub(super) fn vpsadbw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        // Bochs simd_int.h xmm_psadbw.
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(instr.src2());
            let src2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for qword in 0..4 {
                let base = qword * 8;
                let mut sum = 0u64;
                for j in 0..8 {
                    sum += (src1.ymmubyte(base + j) as i16 - src2.ymmubyte(base + j) as i16)
                        .unsigned_abs() as u64;
                }
                result.set_ymm64u(qword, sum);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(instr.src2());
            let src2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for qword in 0..2 {
                let base = qword * 8;
                let mut sum = 0u64;
                for j in 0..8 {
                    sum += (src1.xmmubyte(base + j) as i16 - src2.xmmubyte(base + j) as i16)
                        .unsigned_abs() as u64;
                }
                result.set_xmm64u(qword, sum);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    fn vpminmax_bytes(
        &mut self,
        instr: &Instruction,
        signed: bool,
        take_max: bool,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(instr.src2());
            let src2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                let src1_raw = src1.ymmubyte(i);
                let src2_raw = src2.ymmubyte(i);
                let take_src2 = if signed {
                    let a = src1.ymm_sbyte(i);
                    let b = src2.ymm_sbyte(i);
                    if take_max {
                        b > a
                    } else {
                        b < a
                    }
                } else if take_max {
                    src2_raw > src1_raw
                } else {
                    src2_raw < src1_raw
                };
                result.set_ymmubyte(i, if take_src2 { src2_raw } else { src1_raw });
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(instr.src2());
            let src2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                let src1_raw = src1.xmmubyte(i);
                let src2_raw = src2.xmmubyte(i);
                let take_src2 = if signed {
                    let a = src1.xmm_sbyte(i);
                    let b = src2.xmm_sbyte(i);
                    if take_max {
                        b > a
                    } else {
                        b < a
                    }
                } else if take_max {
                    src2_raw > src1_raw
                } else {
                    src2_raw < src1_raw
                };
                result.set_xmmubyte(i, if take_src2 { src2_raw } else { src1_raw });
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    fn vpminmax_words(
        &mut self,
        instr: &Instruction,
        signed: bool,
        take_max: bool,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(instr.src2());
            let src2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                let src1_raw = src1.ymm16u(i);
                let src2_raw = src2.ymm16u(i);
                let take_src2 = if signed {
                    let a = src1.ymm16s(i);
                    let b = src2.ymm16s(i);
                    if take_max {
                        b > a
                    } else {
                        b < a
                    }
                } else if take_max {
                    src2_raw > src1_raw
                } else {
                    src2_raw < src1_raw
                };
                result.set_ymm16u(i, if take_src2 { src2_raw } else { src1_raw });
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(instr.src2());
            let src2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                let src1_raw = src1.xmm16u(i);
                let src2_raw = src2.xmm16u(i);
                let take_src2 = if signed {
                    let a = src1.xmm16s(i);
                    let b = src2.xmm16s(i);
                    if take_max {
                        b > a
                    } else {
                        b < a
                    }
                } else if take_max {
                    src2_raw > src1_raw
                } else {
                    src2_raw < src1_raw
                };
                result.set_xmm16u(i, if take_src2 { src2_raw } else { src1_raw });
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    fn vpminmax_dwords(
        &mut self,
        instr: &Instruction,
        signed: bool,
        take_max: bool,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(instr.src2());
            let src2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                let src1_raw = src1.ymm32u(i);
                let src2_raw = src2.ymm32u(i);
                let take_src2 = if signed {
                    let a = src1.ymm32s(i);
                    let b = src2.ymm32s(i);
                    if take_max {
                        b > a
                    } else {
                        b < a
                    }
                } else if take_max {
                    src2_raw > src1_raw
                } else {
                    src2_raw < src1_raw
                };
                result.set_ymm32u(i, if take_src2 { src2_raw } else { src1_raw });
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(instr.src2());
            let src2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                let src1_raw = src1.xmm32u(i);
                let src2_raw = src2.xmm32u(i);
                let take_src2 = if signed {
                    let a = src1.xmm32s(i);
                    let b = src2.xmm32s(i);
                    if take_max {
                        b > a
                    } else {
                        b < a
                    }
                } else if take_max {
                    src2_raw > src1_raw
                } else {
                    src2_raw < src1_raw
                };
                result.set_xmm32u(i, if take_src2 { src2_raw } else { src1_raw });
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    pub(super) fn vpminsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_bytes(instr, true, false)
    }

    pub(super) fn vpminsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_words(instr, true, false)
    }

    pub(super) fn vpminsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_dwords(instr, true, false)
    }

    pub(super) fn vpminub(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_bytes(instr, false, false)
    }

    pub(super) fn vpminuw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_words(instr, false, false)
    }

    pub(super) fn vpminud(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_dwords(instr, false, false)
    }

    pub(super) fn vpmaxsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_bytes(instr, true, true)
    }

    pub(super) fn vpmaxsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_words(instr, true, true)
    }

    pub(super) fn vpmaxsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_dwords(instr, true, true)
    }

    pub(super) fn vpmaxub(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_bytes(instr, false, true)
    }

    pub(super) fn vpmaxuw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_words(instr, false, true)
    }

    pub(super) fn vpmaxud(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vpminmax_dwords(instr, false, true)
    }

    // ========================================================================
    // Packed shift by register (count from low 64 bits of XMM source)
    // ========================================================================

    /// VPSRLQ — Packed Shift Right Logical Qwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 63, result is zero.
    pub(super) fn vpsrlq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
                                     // Shift count from ModRM source (register or memory)
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            if count < 64 {
                let count = count as u32;
                for i in 0..4 {
                    result.set_ymm64u(i, src1.ymm64u(i) >> count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            if count < 64 {
                let count = count as u32;
                for i in 0..2 {
                    result.set_xmm64u(i, src1.xmm64u(i) >> count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLD — Packed Shift Left Logical Dwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 31, result is zero.
    pub(super) fn vpslld_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            if count < 32 {
                let count = count as u32;
                for i in 0..8 {
                    result.set_ymm32u(i, src1.ymm32u(i) << count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            if count < 32 {
                let count = count as u32;
                for i in 0..4 {
                    result.set_xmm32u(i, src1.xmm32u(i) << count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLQ — Packed Shift Left Logical Qwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 63, result is zero.
    pub(super) fn vpsllq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            if count < 64 {
                let count = count as u32;
                for i in 0..4 {
                    result.set_ymm64u(i, src1.ymm64u(i) << count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            if count < 64 {
                let count = count as u32;
                for i in 0..2 {
                    result.set_xmm64u(i, src1.xmm64u(i) << count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLW — Packed Shift Right Logical Words by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 15, result is zero.
    pub(super) fn vpsrlw_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            if count < 16 {
                let count = count as u32;
                for i in 0..16 {
                    result.set_ymm16u(i, src1.ymm16u(i) >> count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            if count < 16 {
                let count = count as u32;
                for i in 0..8 {
                    result.set_xmm16u(i, src1.xmm16u(i) >> count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLD — Packed Shift Right Logical Dwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 31, result is zero.
    pub(super) fn vpsrld_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            if count < 32 {
                let count = count as u32;
                for i in 0..8 {
                    result.set_ymm32u(i, src1.ymm32u(i) >> count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            if count < 32 {
                let count = count as u32;
                for i in 0..4 {
                    result.set_xmm32u(i, src1.xmm32u(i) >> count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRAW — Packed Shift Right Arithmetic Words by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 15, count is clamped to 15.
    pub(super) fn vpsraw_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count_raw = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        let count = if count_raw > 15 {
            15u32
        } else {
            count_raw as u32
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(i, (src1.ymm16s(i) >> count) as u16);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(i, (src1.xmm16s(i) >> count) as u16);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRAD — Packed Shift Right Arithmetic Dwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 31, count is clamped to 31.
    pub(super) fn vpsrad_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count_raw = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        let count = if count_raw > 31 {
            31u32
        } else {
            count_raw as u32
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, (src1.ymm32s(i) >> count) as u32);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, (src1.xmm32s(i) >> count) as u32);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLW — Packed Shift Left Logical Words by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 15, result is zero.
    pub(super) fn vpsllw_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm64u(0)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            if count < 16 {
                let count = count as u32;
                for i in 0..16 {
                    result.set_ymm16u(i, src1.ymm16u(i) << count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            if count < 16 {
                let count = count as u32;
                for i in 0..8 {
                    result.set_xmm16u(i, src1.xmm16u(i) << count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed shift by immediate
    // ========================================================================

    /// VPSRLQ — Packed Shift Right Logical Qwords by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsrlq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count = instr.ib() as u32;
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            if count < 64 {
                for i in 0..4 {
                    result.set_ymm64u(i, src.ymm64u(i) >> count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            if count < 64 {
                for i in 0..2 {
                    result.set_xmm64u(i, src.xmm64u(i) >> count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLQ — Packed Shift Left Logical Qwords by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsllq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count = instr.ib() as u32;
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            if count < 64 {
                for i in 0..4 {
                    result.set_ymm64u(i, src.ymm64u(i) << count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            if count < 64 {
                for i in 0..2 {
                    result.set_xmm64u(i, src.xmm64u(i) << count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLW — Packed Shift Right Logical Words by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsrlw_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count = instr.ib() as u32;
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            if count < 16 {
                for i in 0..16 {
                    result.set_ymm16u(i, src.ymm16u(i) >> count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            if count < 16 {
                for i in 0..8 {
                    result.set_xmm16u(i, src.xmm16u(i) >> count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLW — Packed Shift Left Logical Words by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsllw_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count = instr.ib() as u32;
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            if count < 16 {
                for i in 0..16 {
                    result.set_ymm16u(i, src.ymm16u(i) << count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            if count < 16 {
                for i in 0..8 {
                    result.set_xmm16u(i, src.xmm16u(i) << count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRAW — Packed Shift Right Arithmetic Words by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    /// Arithmetic shift sign-extends; count clamped to 15 if > 15.
    pub(super) fn vpsraw_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count_raw = instr.ib() as u32;
        let count = if count_raw > 15 { 15 } else { count_raw };
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(i, (src.ymm16s(i) >> count) as u16);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(i, (src.xmm16s(i) >> count) as u16);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRAD — Packed Shift Right Arithmetic Dwords by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    /// Arithmetic shift sign-extends; count clamped to 31 if > 31.
    pub(super) fn vpsrad_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count_raw = instr.ib() as u32;
        let count = if count_raw > 31 { 31 } else { count_raw };
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(i, (src.ymm32s(i) >> count) as u32);
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(i, (src.xmm32s(i) >> count) as u32);
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLDQ — Packed Shift Left Double Quadword by immediate (VEX.L aware)
    /// Byte-granularity left shift of each 128-bit lane. Immediate = byte count (0-15).
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpslldq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let shift = instr.ib() as usize;
        let shift = if shift > 15 { 16 } else { shift };
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane
            for i in 0..16usize {
                if i >= shift {
                    result.set_ymmubyte(i, src.ymmubyte(i - shift));
                }
                // else remains 0 (zero-fill from the right)
            }
            // Upper 128-bit lane
            for i in 0..16usize {
                if i >= shift {
                    result.set_ymmubyte(16 + i, src.ymmubyte(16 + i - shift));
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16usize {
                if i >= shift {
                    result.set_xmmubyte(i, src.xmmubyte(i - shift));
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLDQ — Packed Shift Right Double Quadword by immediate (VEX.L aware)
    /// Byte-granularity right shift of each 128-bit lane. Immediate = byte count (0-15).
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsrldq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let shift = instr.ib() as usize;
        let shift = if shift > 15 { 16 } else { shift };
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane
            for i in 0..16usize {
                if i + shift < 16 {
                    result.set_ymmubyte(i, src.ymmubyte(i + shift));
                }
                // else remains 0 (zero-fill from the left)
            }
            // Upper 128-bit lane
            for i in 0..16usize {
                if i + shift < 16 {
                    result.set_ymmubyte(16 + i, src.ymmubyte(16 + i + shift));
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16usize {
                if i + shift < 16 {
                    result.set_xmmubyte(i, src.xmmubyte(i + shift));
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed compare
    // ========================================================================

    /// VPCMPEQB — Packed Compare Equal Bytes (VEX.L aware)
    /// dst[i] = (vvvv[i] == src[i]) ? 0xFF : 0x00
    pub(super) fn vpcmpeqb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                result.set_ymmubyte(
                    i,
                    if src1.ymmubyte(i) == src2.ymmubyte(i) {
                        0xFF
                    } else {
                        0x00
                    },
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                result.set_xmmubyte(
                    i,
                    if src1.xmmubyte(i) == src2.xmmubyte(i) {
                        0xFF
                    } else {
                        0x00
                    },
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPEQW — Packed Compare Equal Words (VEX.L aware)
    /// dst[i] = (vvvv[i] == src[i]) ? 0xFFFF : 0x0000
    pub(super) fn vpcmpeqw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(
                    i,
                    if src1.ymm16u(i) == src2.ymm16u(i) {
                        0xFFFF
                    } else {
                        0x0000
                    },
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(
                    i,
                    if src1.xmm16u(i) == src2.xmm16u(i) {
                        0xFFFF
                    } else {
                        0x0000
                    },
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPEQQ — Packed Compare Equal Qwords (VEX.L aware)
    /// dst[i] = (vvvv[i] == src[i]) ? 0xFFFF_FFFF_FFFF_FFFF : 0
    pub(super) fn vpcmpeqq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(
                    i,
                    if src1.ymm64u(i) == src2.ymm64u(i) {
                        0xFFFF_FFFF_FFFF_FFFF
                    } else {
                        0
                    },
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(
                    i,
                    if src1.xmm64u(i) == src2.xmm64u(i) {
                        0xFFFF_FFFF_FFFF_FFFF
                    } else {
                        0
                    },
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPGTB — Packed Compare Greater Than Bytes, signed (VEX.L aware)
    /// dst[i] = ((vvvv[i] as i8) > (src[i] as i8)) ? 0xFF : 0x00
    pub(super) fn vpcmpgtb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                result.set_ymmubyte(
                    i,
                    if src1.ymm_sbyte(i) > src2.ymm_sbyte(i) {
                        0xFF
                    } else {
                        0x00
                    },
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                result.set_xmmubyte(
                    i,
                    if src1.xmm_sbyte(i) > src2.xmm_sbyte(i) {
                        0xFF
                    } else {
                        0x00
                    },
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPGTW — Packed Compare Greater Than Words, signed (VEX.L aware)
    /// dst[i] = ((vvvv[i] as i16) > (src[i] as i16)) ? 0xFFFF : 0x0000
    pub(super) fn vpcmpgtw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(
                    i,
                    if src1.ymm16s(i) > src2.ymm16s(i) {
                        0xFFFF
                    } else {
                        0x0000
                    },
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(
                    i,
                    if src1.xmm16s(i) > src2.xmm16s(i) {
                        0xFFFF
                    } else {
                        0x0000
                    },
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPGTD — Packed Compare Greater Than Dwords, signed (VEX.L aware)
    /// dst[i] = ((vvvv[i] as i32) > (src[i] as i32)) ? 0xFFFFFFFF : 0
    pub(super) fn vpcmpgtd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32u(
                    i,
                    if src1.ymm32s(i) > src2.ymm32s(i) {
                        0xFFFF_FFFF
                    } else {
                        0
                    },
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32u(
                    i,
                    if src1.xmm32s(i) > src2.xmm32s(i) {
                        0xFFFF_FFFF
                    } else {
                        0
                    },
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPGTQ — Packed Compare Greater Than Qwords, signed (VEX.L aware)
    /// dst[i] = ((vvvv[i] as i64) > (src[i] as i64)) ? 0xFFFF_FFFF_FFFF_FFFF : 0
    pub(super) fn vpcmpgtq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(
                    i,
                    if src1.ymm64s(i) > src2.ymm64s(i) {
                        0xFFFF_FFFF_FFFF_FFFF
                    } else {
                        0
                    },
                );
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64u(
                    i,
                    if src1.xmm64s(i) > src2.xmm64s(i) {
                        0xFFFF_FFFF_FFFF_FFFF
                    } else {
                        0
                    },
                );
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // VPMOVMSKB — Move Byte Mask to GPR
    // ========================================================================

    /// VPMOVMSKB — Extract MSB of each byte, packed into GPR (VEX.L aware)
    /// Result is a bitmask: bit i = MSB of byte i in source XMM/YMM
    pub(super) fn vpmovmskb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_gpr = instr.dst() as usize;
        if instr.get_vl() >= 1 {
            // 256-bit: 32 bytes -> 32-bit mask
            let src = self.read_ymm_reg(instr.src1());
            let mut mask: u32 = 0;
            for i in 0..32 {
                if (src.ymmubyte(i) & 0x80) != 0 {
                    mask |= 1u32 << i;
                }
            }
            self.set_gpr32(dst_gpr, mask);
        } else {
            // 128-bit: 16 bytes -> 16-bit mask (zero-extended to 32/64)
            let src = self.read_xmm_reg(instr.src1());
            let mut mask: u32 = 0;
            for i in 0..16 {
                if (src.xmmubyte(i) & 0x80) != 0 {
                    mask |= 1u32 << i;
                }
            }
            self.set_gpr32(dst_gpr, mask);
        }
        Ok(())
    }

    // ========================================================================
    // VPSHUFHW / VPSHUFLW — Shuffle high/low words within 64-bit lanes
    // ========================================================================

    /// VPSHUFHW — Shuffle High Words within 64-bit lanes (VEX.L aware)
    /// In each 128-bit lane: words 0-3 are copied unchanged, words 4-7 are shuffled
    /// by imm8[1:0], imm8[3:2], imm8[5:4], imm8[7:6]
    pub(super) fn vpshufhw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let imm = instr.ib();
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane
            // Words 0-3 copied unchanged
            for i in 0..4 {
                result.set_ymm16u(i, src.ymm16u(i));
            }
            // Words 4-7 shuffled from high half of lower lane (words 4-7)
            for i in 0..4 {
                let sel = ((imm >> (i * 2)) & 0x3) as usize;
                result.set_ymm16u(4 + i, src.ymm16u(4 + sel));
            }
            // Upper 128-bit lane
            // Words 8-11 copied unchanged
            for i in 0..4 {
                result.set_ymm16u(8 + i, src.ymm16u(8 + i));
            }
            // Words 12-15 shuffled from high half of upper lane (words 12-15)
            for i in 0..4 {
                let sel = ((imm >> (i * 2)) & 0x3) as usize;
                result.set_ymm16u(12 + i, src.ymm16u(12 + sel));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            // Words 0-3 copied unchanged
            for i in 0..4 {
                result.set_xmm16u(i, src.xmm16u(i));
            }
            // Words 4-7 shuffled
            for i in 0..4 {
                let sel = ((imm >> (i * 2)) & 0x3) as usize;
                result.set_xmm16u(4 + i, src.xmm16u(4 + sel));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSHUFLW — Shuffle Low Words within 64-bit lanes (VEX.L aware)
    /// In each 128-bit lane: words 4-7 are copied unchanged, words 0-3 are shuffled
    /// by imm8[1:0], imm8[3:2], imm8[5:4], imm8[7:6]
    pub(super) fn vpshuflw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let imm = instr.ib();
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister::default();
            // Lower 128-bit lane
            // Words 0-3 shuffled from low half of lower lane (words 0-3)
            for i in 0..4 {
                let sel = ((imm >> (i * 2)) & 0x3) as usize;
                result.set_ymm16u(i, src.ymm16u(sel));
            }
            // Words 4-7 copied unchanged
            for i in 0..4 {
                result.set_ymm16u(4 + i, src.ymm16u(4 + i));
            }
            // Upper 128-bit lane
            // Words 8-11 shuffled from low half of upper lane (words 8-11)
            for i in 0..4 {
                let sel = ((imm >> (i * 2)) & 0x3) as usize;
                result.set_ymm16u(8 + i, src.ymm16u(8 + sel));
            }
            // Words 12-15 copied unchanged
            for i in 0..4 {
                result.set_ymm16u(12 + i, src.ymm16u(12 + i));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            // Words 0-3 shuffled
            for i in 0..4 {
                let sel = ((imm >> (i * 2)) & 0x3) as usize;
                result.set_xmm16u(i, src.xmm16u(sel));
            }
            // Words 4-7 copied unchanged
            for i in 0..4 {
                result.set_xmm16u(4 + i, src.xmm16u(4 + i));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Saturating packed add/sub
    // ========================================================================

    /// VPADDSB — Packed Add Signed Saturating Bytes (VEX.L aware)
    pub(super) fn vpaddsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                result.set_ymm_sbyte(i, src1.ymm_sbyte(i).saturating_add(src2.ymm_sbyte(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                result.set_xmm_sbyte(i, src1.xmm_sbyte(i).saturating_add(src2.xmm_sbyte(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDSW — Packed Add Signed Saturating Words (VEX.L aware)
    pub(super) fn vpaddsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16s(i, src1.ymm16s(i).saturating_add(src2.ymm16s(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16s(i, src1.xmm16s(i).saturating_add(src2.xmm16s(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBSB — Packed Subtract Signed Saturating Bytes (VEX.L aware)
    pub(super) fn vpsubsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                result.set_ymm_sbyte(i, src1.ymm_sbyte(i).saturating_sub(src2.ymm_sbyte(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                result.set_xmm_sbyte(i, src1.xmm_sbyte(i).saturating_sub(src2.xmm_sbyte(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBSW — Packed Subtract Signed Saturating Words (VEX.L aware)
    pub(super) fn vpsubsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16s(i, src1.ymm16s(i).saturating_sub(src2.ymm16s(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16s(i, src1.xmm16s(i).saturating_sub(src2.xmm16s(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDUSB — Packed Add Unsigned Saturating Bytes (VEX.L aware)
    pub(super) fn vpaddusb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                result.set_ymmubyte(i, src1.ymmubyte(i).saturating_add(src2.ymmubyte(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                result.set_xmmubyte(i, src1.xmmubyte(i).saturating_add(src2.xmmubyte(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDUSW — Packed Add Unsigned Saturating Words (VEX.L aware)
    pub(super) fn vpaddusw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(i, src1.ymm16u(i).saturating_add(src2.ymm16u(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(i, src1.xmm16u(i).saturating_add(src2.xmm16u(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBUSB — Packed Subtract Unsigned Saturating Bytes (VEX.L aware)
    pub(super) fn vpsubusb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..32 {
                result.set_ymmubyte(i, src1.ymmubyte(i).saturating_sub(src2.ymmubyte(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..16 {
                result.set_xmmubyte(i, src1.xmmubyte(i).saturating_sub(src2.xmmubyte(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBUSW — Packed Subtract Unsigned Saturating Words (VEX.L aware)
    pub(super) fn vpsubusw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister::default();
            for i in 0..16 {
                result.set_ymm16u(i, src1.ymm16u(i).saturating_sub(src2.ymm16u(i)));
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister::default();
            for i in 0..8 {
                result.set_xmm16u(i, src1.xmm16u(i).saturating_sub(src2.xmm16u(i)));
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }
}
