//! SSE/SSE2 data movement instruction handlers
//!
//! Based on Bochs cpu/sse_move.cc
//! Copyright (C) 2003-2018 Stanislav Shwartsman
//!
//! Implements SSE/SSE2 data movement instructions:
//! - Packed loads/stores: MOVUPS, MOVUPD, MOVAPS, MOVAPD, MOVDQA, MOVDQU
//! - Scalar loads/stores: MOVSS, MOVSD
//! - Partial loads/stores: MOVLPS, MOVLPD, MOVHPS, MOVHPD
//! - Register shuffles: MOVLHPS, MOVHLPS
//! - Sign-bit extraction: MOVMSKPS, MOVMSKPD
//! - Integer/XMM transfers: MOVD, MOVQ
//! - Non-temporal stores: MOVNTPS, MOVNTPD, MOVNTDQ, MOVNTI
//! - MXCSR: LDMXCSR, STMXCSR
//!
//! All handlers call `prepare_sse()` first (checks CR0.EM, CR4.OSFXSR, CR0.TS).
//! Legacy SSE (non-VEX) preserves upper bits: uses `write_xmm_reg_lo128`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedXmmRegister,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // MOVUPS / MOVUPD — Unaligned packed single/double (0F 10, 0F 11)
    // MOVDQU          — Unaligned packed integer (F3 0F 6F, F3 0F 7F)
    //
    // In Bochs, all four share the same M handlers (MOVUPS_VpsWpsM /
    // MOVUPS_WpsVpsM) and the same R handler (MOVAPS_VpsWpsR).
    // ========================================================================

    /// MOVUPS/MOVUPD/MOVDQU load — XMM <- M128 (unaligned)
    /// Bochs: MOVUPS_VpsWpsM
    pub(super) fn movups_vps_wps_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.v_read_xmmword(seg, eaddr)?;
        self.write_xmm_reg_lo128(instr.dst(), val);
        Ok(())
    }

    /// MOVUPS/MOVUPD/MOVDQU store — M128 <- XMM (unaligned)
    /// Bochs: MOVUPS_WpsVpsM
    pub(super) fn movups_wps_vps_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.read_xmm_reg(instr.src1());
        self.v_write_xmmword(seg, eaddr, &val)?;
        Ok(())
    }

    // Aliases for MOVUPD / MOVDQU (identical behavior, different opcodes)

    /// MOVUPD load — XMM <- M128 (unaligned)
    #[inline]
    pub(super) fn movupd_vpd_wpd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movups_vps_wps_m(instr)
    }

    /// MOVUPD store — M128 <- XMM (unaligned)
    #[inline]
    pub(super) fn movupd_wpd_vpd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movups_wps_vps_m(instr)
    }

    /// MOVDQU load — XMM <- M128 (unaligned, integer)
    #[inline]
    pub(super) fn movdqu_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movups_vps_wps_m(instr)
    }

    /// MOVDQU store — M128 <- XMM (unaligned, integer)
    #[inline]
    pub(super) fn movdqu_wdq_vdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movups_wps_vps_m(instr)
    }

    // ========================================================================
    // MOVAPS / MOVAPD — Aligned packed single/double (0F 28, 0F 29)
    // MOVDQA          — Aligned packed integer (66 0F 6F, 66 0F 7F)
    //
    // Register form: XMM <- XMM (shared by MOVUPS/MOVAPS/MOVDQA/MOVDQU)
    // Memory form: aligned 16-byte access, #GP if misaligned
    // ========================================================================

    /// MOVAPS/MOVAPD/MOVDQA/MOVUPS/MOVUPD/MOVDQU register form — XMM <- XMM
    /// Bochs: MOVAPS_VpsWpsR
    /// Used for ALL packed 128-bit XMM-to-XMM moves regardless of
    /// aligned/unaligned or float/integer mnemonic.
    pub(super) fn movaps_vps_wps_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val = self.read_xmm_reg(instr.src1());
        self.write_xmm_reg_lo128(instr.dst(), val);
        Ok(())
    }

    /// MOVAPS/MOVAPD/MOVDQA load — XMM <- M128 (aligned, #GP if misaligned)
    /// Bochs: MOVAPS_VpsWpsM
    pub(super) fn movaps_vps_wps_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.v_read_xmmword_aligned(seg, eaddr)?;
        self.write_xmm_reg_lo128(instr.dst(), val);
        Ok(())
    }

    /// MOVAPS/MOVAPD/MOVDQA/MOVNTPS/MOVNTPD/MOVNTDQ store — M128 <- XMM (aligned)
    /// Bochs: MOVAPS_WpsVpsM
    /// Non-temporal hint is ignored in emulation.
    pub(super) fn movaps_wps_vps_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.read_xmm_reg(instr.src1());
        self.v_write_xmmword_aligned(seg, eaddr, &val)?;
        Ok(())
    }

    // Aliases for MOVAPD / MOVDQA / non-temporal stores (all share handlers)

    /// MOVAPD load — XMM <- M128 (aligned)
    #[inline]
    pub(super) fn movapd_vpd_wpd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movaps_vps_wps_m(instr)
    }

    /// MOVAPD store — M128 <- XMM (aligned)
    #[inline]
    pub(super) fn movapd_wpd_vpd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movaps_wps_vps_m(instr)
    }

    /// MOVDQA load — XMM <- M128 (aligned, integer)
    #[inline]
    pub(super) fn movdqa_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movaps_vps_wps_m(instr)
    }

    /// MOVDQA store — M128 <- XMM (aligned, integer)
    #[inline]
    pub(super) fn movdqa_wdq_vdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movaps_wps_vps_m(instr)
    }

    /// MOVNTPS store — M128 <- XMM (aligned, non-temporal hint ignored)
    #[inline]
    pub(super) fn movntps_mps_vps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movaps_wps_vps_m(instr)
    }

    /// MOVNTPD store — M128 <- XMM (aligned, non-temporal hint ignored)
    #[inline]
    pub(super) fn movntpd_mpd_vpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movaps_wps_vps_m(instr)
    }

    /// MOVNTDQ store — M128 <- XMM (aligned, non-temporal hint ignored)
    #[inline]
    pub(super) fn movntdq_mdq_vdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movaps_wps_vps_m(instr)
    }

    // ========================================================================
    // MOVSS — Scalar single-precision (F3 0F 10, F3 0F 11)
    //
    // Register form: dst[31:0] = src[31:0], dst[127:32] PRESERVED
    // Memory load:   dst[31:0] = [mem32], dst[127:32] = 0
    // Memory store:  [mem32] = src[31:0]
    // ========================================================================

    /// MOVSS register form — dst.lo_dword = src.lo_dword, high 96 bits preserved
    /// Bochs: MOVSS_VssWssR
    pub(super) fn movss_vss_wss_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_dword(instr.src1());
        self.write_xmm_lo_dword(instr.dst(), src_lo);
        Ok(())
    }

    /// MOVSS memory load — dst = zero-extend(mem32)
    /// Bochs: MOVSS_VssWssM
    pub(super) fn movss_vss_wss_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val32 = self.v_read_dword(seg, eaddr)?;

        // Memory form: high 96 bits are zeroed
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = val32 as u64;
            op.xmm64u[1] = 0;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVSS register store form — dst.lo_dword = src.lo_dword, high 96 bits preserved
    /// This is the same as movss_vss_wss_r but with src/dst roles swapped in
    /// the opcode encoding. For register form, Bochs reuses MOVSS_VssWssR.
    pub(super) fn movss_wss_vss_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        // For register store (0F 11 /r with mod=11), it's dst.lo = src.lo
        let src_lo = self.xmm_lo_dword(instr.src1());
        self.write_xmm_lo_dword(instr.dst(), src_lo);
        Ok(())
    }

    /// MOVSS memory store — mem32 = src.lo_dword
    /// Bochs: MOVSS_WssVssM
    pub(super) fn movss_wss_vss_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.xmm_lo_dword(instr.src1());
        self.v_write_dword(seg, eaddr, val)?;
        Ok(())
    }

    // ========================================================================
    // MOVSD — Scalar double-precision (F2 0F 10, F2 0F 11)
    //
    // Register form: dst[63:0] = src[63:0], dst[127:64] PRESERVED
    // Memory load:   dst[63:0] = [mem64], dst[127:64] = 0
    // Memory store:  [mem64] = src[63:0]
    // ========================================================================

    /// MOVSD register form — dst.lo_qword = src.lo_qword, high 64 bits preserved
    /// Bochs: MOVSD_VsdWsdR
    pub(super) fn movsd_vsd_wsd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_qword(instr.src1());
        self.write_xmm_lo_qword(instr.dst(), src_lo);
        Ok(())
    }

    /// MOVSD memory load — dst = zero-extend(mem64)
    /// Bochs: MOVSD_VsdWsdM
    pub(super) fn movsd_vsd_wsd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;

        // Memory form: high 64 bits are zeroed
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = val64;
            op.xmm64u[1] = 0;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVSD register store form — dst.lo_qword = src.lo_qword, high 64 preserved
    pub(super) fn movsd_wsd_vsd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_qword(instr.src1());
        self.write_xmm_lo_qword(instr.dst(), src_lo);
        Ok(())
    }

    /// MOVSD memory store — mem64 = src.lo_qword
    /// Bochs: MOVSD_WsdVsdM  (also used for MOVLPS/MOVLPD stores)
    pub(super) fn movsd_wsd_vsd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.xmm_lo_qword(instr.src1());
        self.v_write_qword(seg, eaddr, val)?;
        Ok(())
    }

    // ========================================================================
    // MOVLPS / MOVLPD — Load/store low 64 bits (0F 12, 0F 13, 66 0F 12/13)
    //
    // Load:  dst[63:0] = mem64, dst[127:64] preserved (memory only)
    // Store: mem64 = src[63:0]
    // ========================================================================

    /// MOVLPS/MOVLPD load — dst.lo_qword = mem64, high qword preserved
    /// Bochs: MOVLPS_VpsMq
    pub(super) fn movlps_vps_mq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        self.write_xmm_lo_qword(instr.dst(), val64);
        Ok(())
    }

    /// MOVLPD load — alias for movlps_vps_mq
    #[inline]
    pub(super) fn movlpd_vpd_mq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movlps_vps_mq(instr)
    }

    /// MOVLPS/MOVLPD store — mem64 = src.lo_qword
    /// Bochs: MOVSD_WsdVsdM (same handler — stores low qword)
    pub(super) fn movlps_mq_vps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.xmm_lo_qword(instr.src1());
        self.v_write_qword(seg, eaddr, val)?;
        Ok(())
    }

    // ========================================================================
    // MOVHPS / MOVHPD — Load/store high 64 bits (0F 16, 0F 17, 66 0F 16/17)
    //
    // Load:  dst[127:64] = mem64, dst[63:0] preserved (memory only)
    // Store: mem64 = src[127:64]
    // ========================================================================

    /// MOVHPS/MOVHPD load — dst.hi_qword = mem64, low qword preserved
    /// Bochs: MOVHPS_VpsMq
    pub(super) fn movhps_vps_mq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        self.write_xmm_hi_qword(instr.dst(), val64);
        Ok(())
    }

    /// MOVHPD load — alias for movhps_vps_mq
    #[inline]
    pub(super) fn movhpd_vpd_mq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.movhps_vps_mq(instr)
    }

    /// MOVHPS/MOVHPD store — mem64 = src.hi_qword
    /// Bochs: MOVHPS_MqVps
    pub(super) fn movhps_mq_vps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.xmm_hi_qword(instr.src1());
        self.v_write_qword(seg, eaddr, val)?;
        Ok(())
    }

    // ========================================================================
    // MOVLHPS — Move low qword to high qword (0F 16 register form)
    // MOVHLPS — Move high qword to low qword (0F 12 register form)
    //
    // Both are register-only forms sharing opcodes with MOVHPS/MOVLPS.
    // ========================================================================

    /// MOVLHPS — dst[127:64] = src[63:0], dst[63:0] preserved
    /// Bochs: MOVLHPS_VpsWpsR
    pub(super) fn movlhps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_qword(instr.src1());
        self.write_xmm_hi_qword(instr.dst(), src_lo);
        Ok(())
    }

    /// MOVHLPS — dst[63:0] = src[127:64], dst[127:64] preserved
    /// Bochs: MOVHLPS_VpsWpsR
    pub(super) fn movhlps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_hi = self.xmm_hi_qword(instr.src1());
        self.write_xmm_lo_qword(instr.dst(), src_hi);
        Ok(())
    }

    // ========================================================================
    // MOVMSKPS / MOVMSKPD — Extract sign bits to GPR (0F 50, 66 0F 50)
    //
    // Register-only. Read sign bits from XMM float lanes, write to GPR.
    // ========================================================================

    /// MOVMSKPS — extract 4 sign bits from packed single → GPR
    /// Bochs: MOVMSKPS_GdUps (uses xmm_pmovmskd helper)
    pub(super) fn movmskps_gd_ups(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src = self.read_xmm_reg(instr.src1());
        let mut mask: u32 = 0;
        unsafe {
            if src.xmm32u[0] & 0x8000_0000 != 0 {
                mask |= 1;
            }
            if src.xmm32u[1] & 0x8000_0000 != 0 {
                mask |= 2;
            }
            if src.xmm32u[2] & 0x8000_0000 != 0 {
                mask |= 4;
            }
            if src.xmm32u[3] & 0x8000_0000 != 0 {
                mask |= 8;
            }
        }
        // BX_WRITE_32BIT_REGZ — zero-extends to 64 bits
        self.set_gpr32(instr.dst().into(), mask);
        Ok(())
    }

    /// MOVMSKPD — extract 2 sign bits from packed double → GPR
    /// Bochs: MOVMSKPD_GdUpd (uses xmm_pmovmskq helper)
    pub(super) fn movmskpd_gd_upd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src = self.read_xmm_reg(instr.src1());
        let mut mask: u32 = 0;
        unsafe {
            if src.xmm64u[0] & 0x8000_0000_0000_0000 != 0 {
                mask |= 1;
            }
            if src.xmm64u[1] & 0x8000_0000_0000_0000 != 0 {
                mask |= 2;
            }
        }
        self.set_gpr32(instr.dst().into(), mask);
        Ok(())
    }

    // ========================================================================
    // MOVD — 32-bit transfer between GPR and XMM (66 0F 6E, 66 0F 7E)
    // ========================================================================

    /// MOVD register form — XMM[31:0] = Ed (GPR), XMM[127:32] = 0
    /// Bochs: MOVD_VdqEdR
    pub(super) fn movd_vdq_ed_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val32 = self.get_gpr32(instr.src1().into());
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = val32 as u64;
            op.xmm64u[1] = 0;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVD memory form — XMM[31:0] = mem32, XMM[127:32] = 0
    pub(super) fn movd_vdq_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val32 = self.v_read_dword(seg, eaddr)?;
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = val32 as u64;
            op.xmm64u[1] = 0;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVD register form — Ed (GPR) = XMM[31:0]
    /// Bochs: MOVD_EdVdR
    pub(super) fn movd_ed_vdq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val = self.xmm_lo_dword(instr.src1());
        self.set_gpr32(instr.dst().into(), val);
        Ok(())
    }

    /// MOVD memory form — mem32 = XMM[31:0]
    pub(super) fn movd_ed_vdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.xmm_lo_dword(instr.src1());
        self.v_write_dword(seg, eaddr, val)?;
        Ok(())
    }

    // ========================================================================
    // MOVQ — 64-bit XMM ↔ XMM/M64 (F3 0F 7E, 66 0F D6)
    //
    // F3 0F 7E: MOVQ Vdq, Wq — load/move: dst[63:0]=src[63:0], dst[127:64]=0
    // 66 0F D6: MOVQ Wq, Vdq — store: dst[63:0]=src[63:0]
    // ========================================================================

    /// MOVQ register form (F3 0F 7E) — dst[63:0] = src[63:0], dst[127:64] = 0
    /// Bochs: MOVQ_VqWqR
    pub(super) fn movq_vq_wq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = self.xmm_lo_qword(instr.src1());
            op.xmm64u[1] = 0;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVQ memory load (F3 0F 7E) — dst[63:0] = mem64, dst[127:64] = 0
    pub(super) fn movq_vq_wq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = val64;
            op.xmm64u[1] = 0;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVQ register store (66 0F D6) — dst[63:0] = src[63:0], dst[127:64] preserved
    /// Note: In Bochs for register form this zeros upper. We follow Bochs behavior.
    pub(super) fn movq_wq_vq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_qword(instr.src1());
        // Bochs: for 66 0F D6 register form, writes lo qword and zeros high
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = src_lo;
            op.xmm64u[1] = 0;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVQ memory store (66 0F D6) — mem64 = src[63:0]
    pub(super) fn movq_wq_vq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.xmm_lo_qword(instr.src1());
        self.v_write_qword(seg, eaddr, val)?;
        Ok(())
    }

    // ========================================================================
    // MOVNTI — Non-temporal store 32-bit GPR → memory (0F C3)
    //
    // In Bochs this maps to MOV32_EdGdM (regular store, NT hint ignored).
    // We provide a dedicated handler for clarity.
    // ========================================================================

    /// MOVNTI — mem32 = GPR (non-temporal hint ignored in emulation)
    /// Bochs: MOV32_EdGdM
    pub(super) fn movnti_md_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.get_gpr32(instr.src1().into());
        self.v_write_dword(seg, eaddr, val)?;
        Ok(())
    }

    // ========================================================================
    // MOVNTI — Non-temporal store 64-bit mode variants
    // ========================================================================

    /// MOVNTI Op64 — mem32 = GPR32 (non-temporal hint, 64-bit addressing)
    pub(super) fn movnti_op64_md_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr64(instr);
        let val = self.get_gpr32(instr.src1().into());
        self.write_virtual_dword_64(seg, eaddr, val)?;
        Ok(())
    }

    /// MOVNTI — mem64 = GPR64 (non-temporal hint, 64-bit)
    pub(super) fn movnti_mq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr64(instr);
        let val = self.get_gpr64(instr.src1() as usize);
        self.write_virtual_qword_64(seg, eaddr, val)?;
        Ok(())
    }

    // ========================================================================
    // MOVQ xmm ↔ r/m64 (66 REX.W 0F 6E, 66 REX.W 0F 7E)
    //
    // 64-bit mode only: transfer 64-bit integer between XMM and GPR/memory.
    // ========================================================================

    /// MOVQ xmm, r/m64 — Load 64-bit integer into XMM low qword, zero upper
    /// Bochs: MOVQ_VdqEq (66 REX.W 0F 6E)
    pub(super) fn movq_vdq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val = if instr.mod_c0() {
            self.get_gpr64(instr.src1() as usize)
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)?
        };
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = val;
            op.xmm64u[1] = 0;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVQ r/m64, xmm — Store XMM low qword to GPR or memory
    /// Bochs: MOVQ_EqVq (66 REX.W 0F 7E)
    pub(super) fn movq_eq_vq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val = self.xmm_lo_qword(instr.src1());
        if instr.mod_c0() {
            self.set_gpr64(instr.dst() as usize, val);
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.write_virtual_qword_64(seg, eaddr, val)?;
        }
        Ok(())
    }

    // ========================================================================
    // Note: LDMXCSR and STMXCSR are implemented in proc_ctrl.rs
    // ========================================================================

    // ========================================================================
    // MOVDQ2Q — XMM low qword → MMX register (F2 0F D6)
    // MOVQ2DQ — MMX register → XMM low qword (F3 0F D6)
    //
    // Cross-domain moves between MMX and XMM registers.
    // ========================================================================

    /// MOVDQ2Q — MMX = XMM.lo_qword
    /// Bochs: MOVDQ2Q_PqUdq
    pub(super) fn movdq2q_pq_udq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();

        let val = self.xmm_lo_qword(instr.src1());
        let mmx_reg = super::i387::BxPackedRegister { U64: val };
        self.write_mmx_reg(instr.dst(), mmx_reg);
        Ok(())
    }

    /// MOVQ2DQ — XMM = zero-extend(MMX)
    /// Bochs: MOVQ2DQ_VdqQq
    pub(super) fn movq2dq_vdq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();

        let mmx_val = self.read_mmx_reg(instr.src1());
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = mmx_val.U64;
            op.xmm64u[1] = 0;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    // Note: PMOVMSKB_GdUdq is implemented in sse.rs

    // ========================================================================
    // MOVDDUP — Duplicate low qword (F2 0F 12)
    // MOVSLDUP — Duplicate odd single-precision floats (F3 0F 12)
    // MOVSHDUP — Duplicate even single-precision floats (F3 0F 16)
    // ========================================================================

    /// MOVDDUP register — dst = { src.lo_qword, src.lo_qword }
    /// Bochs: MOVDDUP_VpdWqR
    pub(super) fn movddup_vpd_wq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_qword(instr.src1());
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = src_lo;
            op.xmm64u[1] = src_lo;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVDDUP memory — dst = { mem64, mem64 }
    pub(super) fn movddup_vpd_wq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        let mut op = BxPackedXmmRegister::default();
        unsafe {
            op.xmm64u[0] = val64;
            op.xmm64u[1] = val64;
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVSLDUP register — dst = { src[2], src[2], src[0], src[0] }
    /// Duplicates even-indexed dwords: dst[0]=src[0], dst[1]=src[0], dst[2]=src[2], dst[3]=src[2]
    /// Bochs: MOVSLDUP_VpsWpsR
    pub(super) fn movsldup_vps_wps_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src = self.read_xmm_reg(instr.src1());
        let mut op = src;
        unsafe {
            op.xmm32u[1] = op.xmm32u[0];
            op.xmm32u[3] = op.xmm32u[2];
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVSLDUP memory
    pub(super) fn movsldup_vps_wps_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let mut op = self.v_read_xmmword(seg, eaddr)?;
        unsafe {
            op.xmm32u[1] = op.xmm32u[0];
            op.xmm32u[3] = op.xmm32u[2];
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVSHDUP register — dst = { src[3], src[3], src[1], src[1] }
    /// Duplicates odd-indexed dwords: dst[0]=src[1], dst[1]=src[1], dst[2]=src[3], dst[3]=src[3]
    /// Bochs: MOVSHDUP_VpsWpsR
    pub(super) fn movshdup_vps_wps_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src = self.read_xmm_reg(instr.src1());
        let mut op = src;
        unsafe {
            op.xmm32u[0] = op.xmm32u[1];
            op.xmm32u[2] = op.xmm32u[3];
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MOVSHDUP memory
    pub(super) fn movshdup_vps_wps_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let mut op = self.v_read_xmmword(seg, eaddr)?;
        unsafe {
            op.xmm32u[0] = op.xmm32u[1];
            op.xmm32u[2] = op.xmm32u[3];
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    // ========================================================================
    // SSE2 Pack/Unpack — 128-bit integer forms
    // PUNPCKLBW/WD/DQ, PUNPCKHBW/WD/DQ (66 0F 60-6D)
    //
    // These interleave elements from low or high halves of two XMM registers.
    // ========================================================================

    /// PUNPCKLBW — Unpack and interleave low bytes
    /// Bochs: PUNPCKLBW_VdqWdqR (sse_int.cc)
    pub(super) fn punpcklbw_vdq_wdq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmmubyte[0] = op1.xmmubyte[0];
            result.xmmubyte[1] = op2.xmmubyte[0];
            result.xmmubyte[2] = op1.xmmubyte[1];
            result.xmmubyte[3] = op2.xmmubyte[1];
            result.xmmubyte[4] = op1.xmmubyte[2];
            result.xmmubyte[5] = op2.xmmubyte[2];
            result.xmmubyte[6] = op1.xmmubyte[3];
            result.xmmubyte[7] = op2.xmmubyte[3];
            result.xmmubyte[8] = op1.xmmubyte[4];
            result.xmmubyte[9] = op2.xmmubyte[4];
            result.xmmubyte[10] = op1.xmmubyte[5];
            result.xmmubyte[11] = op2.xmmubyte[5];
            result.xmmubyte[12] = op1.xmmubyte[6];
            result.xmmubyte[13] = op2.xmmubyte[6];
            result.xmmubyte[14] = op1.xmmubyte[7];
            result.xmmubyte[15] = op2.xmmubyte[7];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLBW — memory form
    pub(super) fn punpcklbw_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let op2 = self.v_read_xmmword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmmubyte[0] = op1.xmmubyte[0];
            result.xmmubyte[1] = op2.xmmubyte[0];
            result.xmmubyte[2] = op1.xmmubyte[1];
            result.xmmubyte[3] = op2.xmmubyte[1];
            result.xmmubyte[4] = op1.xmmubyte[2];
            result.xmmubyte[5] = op2.xmmubyte[2];
            result.xmmubyte[6] = op1.xmmubyte[3];
            result.xmmubyte[7] = op2.xmmubyte[3];
            result.xmmubyte[8] = op1.xmmubyte[4];
            result.xmmubyte[9] = op2.xmmubyte[4];
            result.xmmubyte[10] = op1.xmmubyte[5];
            result.xmmubyte[11] = op2.xmmubyte[5];
            result.xmmubyte[12] = op1.xmmubyte[6];
            result.xmmubyte[13] = op2.xmmubyte[6];
            result.xmmubyte[14] = op1.xmmubyte[7];
            result.xmmubyte[15] = op2.xmmubyte[7];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLWD — Unpack and interleave low words
    pub(super) fn punpcklwd_vdq_wdq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm16u[0] = op1.xmm16u[0];
            result.xmm16u[1] = op2.xmm16u[0];
            result.xmm16u[2] = op1.xmm16u[1];
            result.xmm16u[3] = op2.xmm16u[1];
            result.xmm16u[4] = op1.xmm16u[2];
            result.xmm16u[5] = op2.xmm16u[2];
            result.xmm16u[6] = op1.xmm16u[3];
            result.xmm16u[7] = op2.xmm16u[3];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLWD — memory form
    pub(super) fn punpcklwd_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let op2 = self.v_read_xmmword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm16u[0] = op1.xmm16u[0];
            result.xmm16u[1] = op2.xmm16u[0];
            result.xmm16u[2] = op1.xmm16u[1];
            result.xmm16u[3] = op2.xmm16u[1];
            result.xmm16u[4] = op1.xmm16u[2];
            result.xmm16u[5] = op2.xmm16u[2];
            result.xmm16u[6] = op1.xmm16u[3];
            result.xmm16u[7] = op2.xmm16u[3];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLDQ — Unpack and interleave low dwords
    pub(super) fn punpckldq_vdq_wdq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = op1.xmm32u[0];
            result.xmm32u[1] = op2.xmm32u[0];
            result.xmm32u[2] = op1.xmm32u[1];
            result.xmm32u[3] = op2.xmm32u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLDQ — memory form
    pub(super) fn punpckldq_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let op2 = self.v_read_xmmword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = op1.xmm32u[0];
            result.xmm32u[1] = op2.xmm32u[0];
            result.xmm32u[2] = op1.xmm32u[1];
            result.xmm32u[3] = op2.xmm32u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLQDQ — Unpack and interleave low qwords
    pub(super) fn punpcklqdq_vdq_wdq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[0];
            result.xmm64u[1] = op2.xmm64u[0];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLQDQ — memory form
    pub(super) fn punpcklqdq_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let op2 = self.v_read_xmmword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[0];
            result.xmm64u[1] = op2.xmm64u[0];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHBW — Unpack and interleave high bytes
    pub(super) fn punpckhbw_vdq_wdq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmmubyte[0] = op1.xmmubyte[8];
            result.xmmubyte[1] = op2.xmmubyte[8];
            result.xmmubyte[2] = op1.xmmubyte[9];
            result.xmmubyte[3] = op2.xmmubyte[9];
            result.xmmubyte[4] = op1.xmmubyte[10];
            result.xmmubyte[5] = op2.xmmubyte[10];
            result.xmmubyte[6] = op1.xmmubyte[11];
            result.xmmubyte[7] = op2.xmmubyte[11];
            result.xmmubyte[8] = op1.xmmubyte[12];
            result.xmmubyte[9] = op2.xmmubyte[12];
            result.xmmubyte[10] = op1.xmmubyte[13];
            result.xmmubyte[11] = op2.xmmubyte[13];
            result.xmmubyte[12] = op1.xmmubyte[14];
            result.xmmubyte[13] = op2.xmmubyte[14];
            result.xmmubyte[14] = op1.xmmubyte[15];
            result.xmmubyte[15] = op2.xmmubyte[15];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHBW — memory form
    pub(super) fn punpckhbw_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let op2 = self.v_read_xmmword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmmubyte[0] = op1.xmmubyte[8];
            result.xmmubyte[1] = op2.xmmubyte[8];
            result.xmmubyte[2] = op1.xmmubyte[9];
            result.xmmubyte[3] = op2.xmmubyte[9];
            result.xmmubyte[4] = op1.xmmubyte[10];
            result.xmmubyte[5] = op2.xmmubyte[10];
            result.xmmubyte[6] = op1.xmmubyte[11];
            result.xmmubyte[7] = op2.xmmubyte[11];
            result.xmmubyte[8] = op1.xmmubyte[12];
            result.xmmubyte[9] = op2.xmmubyte[12];
            result.xmmubyte[10] = op1.xmmubyte[13];
            result.xmmubyte[11] = op2.xmmubyte[13];
            result.xmmubyte[12] = op1.xmmubyte[14];
            result.xmmubyte[13] = op2.xmmubyte[14];
            result.xmmubyte[14] = op1.xmmubyte[15];
            result.xmmubyte[15] = op2.xmmubyte[15];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHWD — Unpack and interleave high words
    pub(super) fn punpckhwd_vdq_wdq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm16u[0] = op1.xmm16u[4];
            result.xmm16u[1] = op2.xmm16u[4];
            result.xmm16u[2] = op1.xmm16u[5];
            result.xmm16u[3] = op2.xmm16u[5];
            result.xmm16u[4] = op1.xmm16u[6];
            result.xmm16u[5] = op2.xmm16u[6];
            result.xmm16u[6] = op1.xmm16u[7];
            result.xmm16u[7] = op2.xmm16u[7];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHWD — memory form
    pub(super) fn punpckhwd_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let op2 = self.v_read_xmmword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm16u[0] = op1.xmm16u[4];
            result.xmm16u[1] = op2.xmm16u[4];
            result.xmm16u[2] = op1.xmm16u[5];
            result.xmm16u[3] = op2.xmm16u[5];
            result.xmm16u[4] = op1.xmm16u[6];
            result.xmm16u[5] = op2.xmm16u[6];
            result.xmm16u[6] = op1.xmm16u[7];
            result.xmm16u[7] = op2.xmm16u[7];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHDQ — Unpack and interleave high dwords
    pub(super) fn punpckhdq_vdq_wdq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = op1.xmm32u[2];
            result.xmm32u[1] = op2.xmm32u[2];
            result.xmm32u[2] = op1.xmm32u[3];
            result.xmm32u[3] = op2.xmm32u[3];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHDQ — memory form
    pub(super) fn punpckhdq_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let op2 = self.v_read_xmmword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = op1.xmm32u[2];
            result.xmm32u[1] = op2.xmm32u[2];
            result.xmm32u[2] = op1.xmm32u[3];
            result.xmm32u[3] = op2.xmm32u[3];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHQDQ — Unpack and interleave high qwords
    pub(super) fn punpckhqdq_vdq_wdq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[1];
            result.xmm64u[1] = op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHQDQ — memory form
    pub(super) fn punpckhqdq_vdq_wdq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let op2 = self.v_read_xmmword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[1];
            result.xmm64u[1] = op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // PMOVSXBW through PMOVZXDQ — SSE4.1 sign/zero extend (66 0F 38 2x/3x)
    //
    // These are included here because they are data-movement / conversion
    // instructions that live alongside the SSE moves in Bochs sse_move.cc.
    // ========================================================================

    /// PMOVSXBW — Sign-extend 8 packed bytes to 8 packed words
    /// Bochs: PMOVSXBW_VdqWqR
    pub(super) fn pmovsxbw_vdq_wq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_qword(instr.src1());
        let src_bytes = src_lo.to_le_bytes();
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm16u[0] = src_bytes[0] as i8 as i16 as u16;
            result.xmm16u[1] = src_bytes[1] as i8 as i16 as u16;
            result.xmm16u[2] = src_bytes[2] as i8 as i16 as u16;
            result.xmm16u[3] = src_bytes[3] as i8 as i16 as u16;
            result.xmm16u[4] = src_bytes[4] as i8 as i16 as u16;
            result.xmm16u[5] = src_bytes[5] as i8 as i16 as u16;
            result.xmm16u[6] = src_bytes[6] as i8 as i16 as u16;
            result.xmm16u[7] = src_bytes[7] as i8 as i16 as u16;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXBW — memory form
    pub(super) fn pmovsxbw_vdq_wq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        let src_bytes = val64.to_le_bytes();
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm16u[0] = src_bytes[0] as i8 as i16 as u16;
            result.xmm16u[1] = src_bytes[1] as i8 as i16 as u16;
            result.xmm16u[2] = src_bytes[2] as i8 as i16 as u16;
            result.xmm16u[3] = src_bytes[3] as i8 as i16 as u16;
            result.xmm16u[4] = src_bytes[4] as i8 as i16 as u16;
            result.xmm16u[5] = src_bytes[5] as i8 as i16 as u16;
            result.xmm16u[6] = src_bytes[6] as i8 as i16 as u16;
            result.xmm16u[7] = src_bytes[7] as i8 as i16 as u16;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXWD — Sign-extend 4 packed words to 4 packed dwords
    /// Bochs: PMOVSXWD_VdqWqR
    pub(super) fn pmovsxwd_vdq_wq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = src.xmm16u[0] as i16 as i32 as u32;
            result.xmm32u[1] = src.xmm16u[1] as i16 as i32 as u32;
            result.xmm32u[2] = src.xmm16u[2] as i16 as i32 as u32;
            result.xmm32u[3] = src.xmm16u[3] as i16 as i32 as u32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXWD — memory form
    pub(super) fn pmovsxwd_vdq_wq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        let words = [
            val64 as u16,
            (val64 >> 16) as u16,
            (val64 >> 32) as u16,
            (val64 >> 48) as u16,
        ];
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = words[0] as i16 as i32 as u32;
            result.xmm32u[1] = words[1] as i16 as i32 as u32;
            result.xmm32u[2] = words[2] as i16 as i32 as u32;
            result.xmm32u[3] = words[3] as i16 as i32 as u32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXDQ — Sign-extend 2 packed dwords to 2 packed qwords
    /// Bochs: PMOVSXDQ_VdqWqR
    pub(super) fn pmovsxdq_vdq_wq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_qword(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = (src_lo as u32 as i32 as i64) as u64;
            result.xmm64u[1] = ((src_lo >> 32) as u32 as i32 as i64) as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXDQ — memory form
    pub(super) fn pmovsxdq_vdq_wq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = (val64 as u32 as i32 as i64) as u64;
            result.xmm64u[1] = ((val64 >> 32) as u32 as i32 as i64) as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXBW — Zero-extend 8 packed bytes to 8 packed words
    /// Bochs: PMOVZXBW_VdqWqR
    pub(super) fn pmovzxbw_vdq_wq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_qword(instr.src1());
        let src_bytes = src_lo.to_le_bytes();
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm16u[0] = src_bytes[0] as u16;
            result.xmm16u[1] = src_bytes[1] as u16;
            result.xmm16u[2] = src_bytes[2] as u16;
            result.xmm16u[3] = src_bytes[3] as u16;
            result.xmm16u[4] = src_bytes[4] as u16;
            result.xmm16u[5] = src_bytes[5] as u16;
            result.xmm16u[6] = src_bytes[6] as u16;
            result.xmm16u[7] = src_bytes[7] as u16;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXBW — memory form
    pub(super) fn pmovzxbw_vdq_wq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        let src_bytes = val64.to_le_bytes();
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm16u[0] = src_bytes[0] as u16;
            result.xmm16u[1] = src_bytes[1] as u16;
            result.xmm16u[2] = src_bytes[2] as u16;
            result.xmm16u[3] = src_bytes[3] as u16;
            result.xmm16u[4] = src_bytes[4] as u16;
            result.xmm16u[5] = src_bytes[5] as u16;
            result.xmm16u[6] = src_bytes[6] as u16;
            result.xmm16u[7] = src_bytes[7] as u16;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXWD — Zero-extend 4 packed words to 4 packed dwords
    /// Bochs: PMOVZXWD_VdqWqR
    pub(super) fn pmovzxwd_vdq_wq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src = self.read_xmm_reg(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = src.xmm16u[0] as u32;
            result.xmm32u[1] = src.xmm16u[1] as u32;
            result.xmm32u[2] = src.xmm16u[2] as u32;
            result.xmm32u[3] = src.xmm16u[3] as u32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXWD — memory form
    pub(super) fn pmovzxwd_vdq_wq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = val64 as u16 as u32;
            result.xmm32u[1] = (val64 >> 16) as u16 as u32;
            result.xmm32u[2] = (val64 >> 32) as u16 as u32;
            result.xmm32u[3] = (val64 >> 48) as u16 as u32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXDQ — Zero-extend 2 packed dwords to 2 packed qwords
    /// Bochs: PMOVZXDQ_VdqWqR
    pub(super) fn pmovzxdq_vdq_wq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_lo = self.xmm_lo_qword(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = src_lo as u32 as u64;
            result.xmm64u[1] = (src_lo >> 32) as u32 as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXDQ — memory form
    pub(super) fn pmovzxdq_vdq_wq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val64 = self.v_read_qword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = val64 as u32 as u64;
            result.xmm64u[1] = (val64 >> 32) as u32 as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Additional SSE4.1 sign/zero extend variants
    // PMOVSXBD, PMOVSXBQ, PMOVSXWQ
    // PMOVZXBD, PMOVZXBQ, PMOVZXWQ
    // ========================================================================

    /// PMOVSXBD — Sign-extend 4 packed bytes to 4 packed dwords
    /// Bochs: PMOVSXBD_VdqWdR
    pub(super) fn pmovsxbd_vdq_wd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val32 = self.xmm_lo_dword(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = (val32 as u8 as i8 as i32) as u32;
            result.xmm32u[1] = ((val32 >> 8) as u8 as i8 as i32) as u32;
            result.xmm32u[2] = ((val32 >> 16) as u8 as i8 as i32) as u32;
            result.xmm32u[3] = ((val32 >> 24) as u8 as i8 as i32) as u32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXBD — memory form
    pub(super) fn pmovsxbd_vdq_wd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val32 = self.v_read_dword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = (val32 as u8 as i8 as i32) as u32;
            result.xmm32u[1] = ((val32 >> 8) as u8 as i8 as i32) as u32;
            result.xmm32u[2] = ((val32 >> 16) as u8 as i8 as i32) as u32;
            result.xmm32u[3] = ((val32 >> 24) as u8 as i8 as i32) as u32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXBQ — Sign-extend 2 packed bytes to 2 packed qwords
    /// Bochs: PMOVSXBQ_VdqWwR
    pub(super) fn pmovsxbq_vdq_ww_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val32 = self.xmm_lo_dword(instr.src1());
        let val16 = val32 as u16;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = (val16 as u8 as i8 as i64) as u64;
            result.xmm64u[1] = ((val16 >> 8) as u8 as i8 as i64) as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXBQ — memory form
    pub(super) fn pmovsxbq_vdq_ww_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val16 = self.v_read_word(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = (val16 as u8 as i8 as i64) as u64;
            result.xmm64u[1] = ((val16 >> 8) as u8 as i8 as i64) as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXWQ — Sign-extend 2 packed words to 2 packed qwords
    /// Bochs: PMOVSXWQ_VdqWdR
    pub(super) fn pmovsxwq_vdq_wd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val32 = self.xmm_lo_dword(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = (val32 as u16 as i16 as i64) as u64;
            result.xmm64u[1] = ((val32 >> 16) as u16 as i16 as i64) as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVSXWQ — memory form
    pub(super) fn pmovsxwq_vdq_wd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val32 = self.v_read_dword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = (val32 as u16 as i16 as i64) as u64;
            result.xmm64u[1] = ((val32 >> 16) as u16 as i16 as i64) as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXBD — Zero-extend 4 packed bytes to 4 packed dwords
    /// Bochs: PMOVZXBD_VdqWdR
    pub(super) fn pmovzxbd_vdq_wd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val32 = self.xmm_lo_dword(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = val32 as u8 as u32;
            result.xmm32u[1] = (val32 >> 8) as u8 as u32;
            result.xmm32u[2] = (val32 >> 16) as u8 as u32;
            result.xmm32u[3] = (val32 >> 24) as u8 as u32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXBD — memory form
    pub(super) fn pmovzxbd_vdq_wd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val32 = self.v_read_dword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = val32 as u8 as u32;
            result.xmm32u[1] = (val32 >> 8) as u8 as u32;
            result.xmm32u[2] = (val32 >> 16) as u8 as u32;
            result.xmm32u[3] = (val32 >> 24) as u8 as u32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXBQ — Zero-extend 2 packed bytes to 2 packed qwords
    /// Bochs: PMOVZXBQ_VdqWwR
    pub(super) fn pmovzxbq_vdq_ww_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val32 = self.xmm_lo_dword(instr.src1());
        let val16 = val32 as u16;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = val16 as u8 as u64;
            result.xmm64u[1] = (val16 >> 8) as u8 as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXBQ — memory form
    pub(super) fn pmovzxbq_vdq_ww_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val16 = self.v_read_word(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = val16 as u8 as u64;
            result.xmm64u[1] = (val16 >> 8) as u8 as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXWQ — Zero-extend 2 packed words to 2 packed qwords
    /// Bochs: PMOVZXWQ_VdqWdR
    pub(super) fn pmovzxwq_vdq_wd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let val32 = self.xmm_lo_dword(instr.src1());
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = val32 as u16 as u64;
            result.xmm64u[1] = (val32 >> 16) as u16 as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMOVZXWQ — memory form
    pub(super) fn pmovzxwq_vdq_wd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val32 = self.v_read_dword(seg, eaddr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = val32 as u16 as u64;
            result.xmm64u[1] = (val32 >> 16) as u16 as u64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }
}
