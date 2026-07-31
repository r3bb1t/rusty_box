//! EVEX memory-operand load phase — port of Bochs `cpu/load.cc`.
//!
//! Bochs splits an EVEX instruction with a memory operand into two calls: a
//! `LOAD_*` function (`execute1`) that resolves the effective address, reads
//! the operand into `BX_VECTOR_TMP_REGISTER`, and then tail-calls the
//! arithmetic handler (`execute2`), which reads that temp register instead of
//! the r/m register. The load function is selected per opcode by
//! `ia_opcodes_evex.def`, and it is where three guest-visible behaviours live:
//!
//! * **Embedded broadcast** — with `EVEX.b` set on a memory operand, the
//!   `LOAD_BROADCAST_*` variants read a *single* element and replicate it
//!   across the vector instead of reading a full vector.
//! * **Masked fault suppression** — the `LOAD_MASK_*` variants read only the
//!   elements whose opmask bit is set, so a masked-off element that lands on an
//!   unmapped page must not raise `#PF`.
//! * **Operand width** — `Half`/`Quarter`/`Eighth` variants read a fraction of
//!   the destination width, for the converting and sign-extending opcodes.
//!
//! rusty_box has no `execute1`/`execute2` split for EVEX, so these are ported
//! as value-returning helpers: each reads the operand and hands back the
//! assembled register for the caller to compute on. That is the same work in
//! the same order with the same faults; only the plumbing differs.
//!
//! Every function here is named after the Bochs symbol it ports, so the
//! `ia_opcodes_evex.def` column that selects a loader maps one-to-one onto a
//! function in this file. Only the loaders some handler actually calls are
//! present — the rest of the `load.cc` family (the quarter/eighth widths, the
//! word-granularity broadcasts, the non-broadcast masked loads) arrives with
//! the handler families that name them, ported from `load.cc` at that point.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedZmmRegister,
};

/// Bochs xmm.h `BYTE_ELEMENTS` — byte lanes for a vector length.
///
/// rusty_box encodes EVEX.L'L as 0/1/2 where Bochs uses its `BX_VL128`=1,
/// `BX_VL256`=2, `BX_VL512`=4 scale, so the multipliers differ from the
/// upstream macro while the counts agree.
#[inline]
pub(super) fn byte_elements(vl: u8) -> usize {
    match vl {
        0 => 16,
        1 => 32,
        _ => 64,
    }
}

/// Bochs xmm.h `WORD_ELEMENTS`.
#[inline]
pub(super) fn word_elements(vl: u8) -> usize {
    match vl {
        0 => 8,
        1 => 16,
        _ => 32,
    }
}

/// Bochs xmm.h `DWORD_ELEMENTS`.
#[inline]
pub(super) fn dword_elements(vl: u8) -> usize {
    match vl {
        0 => 4,
        1 => 8,
        _ => 16,
    }
}

/// Bochs xmm.h `QWORD_ELEMENTS`.
#[inline]
pub(super) fn qword_elements(vl: u8) -> usize {
    match vl {
        0 => 2,
        1 => 4,
        _ => 8,
    }
}

/// Bochs cpu.h `CUT_OPMASK_TO` — low `nelements` bits set.
///
/// Callers must not pass 64: Bochs guards that case explicitly because
/// `1 << 64` wraps to 1 and would erase the mask instead of preserving it.
/// Debug builds assert rather than silently reproducing that overflow.
#[inline]
pub(super) fn cut_opmask_to(nelements: usize) -> u64 {
    debug_assert!(
        nelements < 64,
        "cut_opmask_to({nelements}) overflows; the 64-element case must be skipped as Bochs does"
    );
    (1u64 << nelements) - 1
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // Opmask reads — Bochs cpu.h BX_READ_*_OPMASK plus the `k0 means
    // unmasked` convention every LOAD_MASK_* function open-codes.
    // ========================================================================

    /// Bochs `BX_READ_OPMASK` with the `k0 -> all ones` default.
    #[inline]
    fn load_opmask64(&self, instr: &Instruction) -> u64 {
        let k = instr.opmask();
        if k == 0 {
            u64::MAX
        } else {
            self.opmask_rrx(k as usize)
        }
    }

    /// Bochs `BX_READ_32BIT_OPMASK` with the `k0 -> all ones` default.
    #[inline]
    fn load_opmask32(&self, instr: &Instruction) -> u64 {
        let k = instr.opmask();
        if k == 0 {
            0xffff_ffff
        } else {
            self.opmask_rrx(k as usize) & 0xffff_ffff
        }
    }

    /// Bochs `BX_READ_16BIT_OPMASK` with the `k0 -> all ones` default.
    #[inline]
    fn load_opmask16(&self, instr: &Instruction) -> u64 {
        let k = instr.opmask();
        if k == 0 {
            0xffff
        } else {
            self.opmask_rrx(k as usize) & 0xffff
        }
    }

    /// Bochs `BX_READ_8BIT_OPMASK` with the `k0 -> all ones` default.
    #[inline]
    fn load_opmask8(&self, instr: &Instruction) -> u64 {
        let k = instr.opmask();
        if k == 0 {
            0xff
        } else {
            self.opmask_rrx(k as usize) & 0xff
        }
    }

    /// Bochs cpu.h `BX_SCALAR_ELEMENT_MASK` — a scalar EVEX operand is active
    /// when no opmask is selected or when its low bit is set.
    #[inline]
    pub(super) fn scalar_element_mask(&self, instr: &Instruction) -> bool {
        let k = instr.opmask();
        k == 0 || (self.opmask_rrx(k as usize) & 1) != 0
    }

    // ========================================================================
    // Masked element loads — Bochs cpu/avx/avx512_helpers.cc
    // avx_masked_load8/16/32/64.
    //
    // Each performs, in order:
    //   1. under 64-bit addressing, a canonicality pre-pass over every
    //      *active* element, so a non-canonical lane raises #GP/#SS before any
    //      memory is touched;
    //   2. the element reads, walked high lane to low so the highest faulting
    //      address is the one reported;
    //   3. zero-fill of the inactive lanes.
    //
    // Alignment checking is disabled across the read loop (except for the byte
    // flavour, where no access can be misaligned). As in Bochs, a fault inside
    // the loop leaves it disabled: upstream's `exception()` longjmps past the
    // restore, and the mask is recomputed by `handle_alignment_check` on the
    // CPL change that delivering the fault causes.
    // ========================================================================

    pub(super) fn avx_masked_load8(
        &mut self,
        instr: &Instruction,
        eaddr: u64,
        op: &mut BxPackedZmmRegister,
        mask: u64,
    ) -> super::Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let elements = byte_elements(instr.get_vl());

        if instr.as64_l() != 0 {
            let laddr = self.get_laddr64(seg as usize, eaddr);
            for n in 0..elements {
                if (mask & (1u64 << n)) != 0 && !self.is_canonical(laddr.wrapping_add(n as u64)) {
                    return self.exception(Self::seg_exception(seg), 0);
                }
            }
        }

        for n in (0..elements).rev() {
            if (mask & (1u64 << n)) != 0 {
                let val = self.v_read_byte(seg, eaddr.wrapping_add(n as u64))?;
                op.set_zmmubyte(n, val);
            } else {
                op.set_zmmubyte(n, 0);
            }
        }
        Ok(())
    }

    pub(super) fn avx_masked_load16(
        &mut self,
        instr: &Instruction,
        eaddr: u64,
        op: &mut BxPackedZmmRegister,
        mask: u64,
    ) -> super::Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let elements = word_elements(instr.get_vl());

        if instr.as64_l() != 0 {
            let laddr = self.get_laddr64(seg as usize, eaddr);
            for n in 0..elements {
                if (mask & (1u64 << n)) != 0
                    && !self.is_canonical(laddr.wrapping_add(2 * n as u64))
                {
                    return self.exception(Self::seg_exception(seg), 0);
                }
            }
        }

        let saved_ac = self.alignment_check_mask;
        self.alignment_check_mask = 0;
        for n in (0..elements).rev() {
            if (mask & (1u64 << n)) != 0 {
                let val = self.v_read_word(seg, eaddr.wrapping_add(2 * n as u64))?;
                op.set_zmm16u(n, val);
            } else {
                op.set_zmm16u(n, 0);
            }
        }
        self.alignment_check_mask = saved_ac;
        Ok(())
    }

    pub(super) fn avx_masked_load32(
        &mut self,
        instr: &Instruction,
        eaddr: u64,
        op: &mut BxPackedZmmRegister,
        mask: u64,
    ) -> super::Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let elements = dword_elements(instr.get_vl());

        if instr.as64_l() != 0 {
            let laddr = self.get_laddr64(seg as usize, eaddr);
            for n in 0..elements {
                if (mask & (1u64 << n)) != 0
                    && !self.is_canonical(laddr.wrapping_add(4 * n as u64))
                {
                    return self.exception(Self::seg_exception(seg), 0);
                }
            }
        }

        let saved_ac = self.alignment_check_mask;
        self.alignment_check_mask = 0;
        for n in (0..elements).rev() {
            if (mask & (1u64 << n)) != 0 {
                let val = self.v_read_dword(seg, eaddr.wrapping_add(4 * n as u64))?;
                op.set_zmm32u(n, val);
            } else {
                op.set_zmm32u(n, 0);
            }
        }
        self.alignment_check_mask = saved_ac;
        Ok(())
    }

    pub(super) fn avx_masked_load64(
        &mut self,
        instr: &Instruction,
        eaddr: u64,
        op: &mut BxPackedZmmRegister,
        mask: u64,
    ) -> super::Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let elements = qword_elements(instr.get_vl());

        if instr.as64_l() != 0 {
            let laddr = self.get_laddr64(seg as usize, eaddr);
            for n in 0..elements {
                if (mask & (1u64 << n)) != 0
                    && !self.is_canonical(laddr.wrapping_add(8 * n as u64))
                {
                    return self.exception(Self::seg_exception(seg), 0);
                }
            }
        }

        let saved_ac = self.alignment_check_mask;
        self.alignment_check_mask = 0;
        for n in (0..elements).rev() {
            if (mask & (1u64 << n)) != 0 {
                let val = self.v_read_qword(seg, eaddr.wrapping_add(8 * n as u64))?;
                op.set_zmm64u(n, val);
            } else {
                op.set_zmm64u(n, 0);
            }
        }
        self.alignment_check_mask = saved_ac;
        Ok(())
    }

    // ========================================================================
    // Masked element stores — Bochs cpu/avx/avx512_helpers.cc
    // avx_masked_store8/16/32/64.
    //
    // The ordering matters as much as the masking. Bochs first walks every
    // active element issuing a read-for-ownership (`read_RMW_virtual_*`, no
    // lock) and only then writes any of them, so an element that faults part
    // way through cannot leave the earlier elements already committed to
    // memory. The probe runs high lane to low, the writes low to high.
    // ========================================================================

    pub(super) fn avx_masked_store32(
        &mut self,
        instr: &Instruction,
        eaddr: u64,
        op: &BxPackedZmmRegister,
        mask: u64,
    ) -> super::Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let elements = dword_elements(instr.get_vl());

        if instr.as64_l() != 0 {
            let laddr = self.get_laddr64(seg as usize, eaddr);
            for n in 0..elements {
                if (mask & (1u64 << n)) != 0
                    && !self.is_canonical(laddr.wrapping_add(4 * n as u64))
                {
                    return self.exception(Self::seg_exception(seg), 0);
                }
            }
        }

        let saved_ac = self.alignment_check_mask;
        self.alignment_check_mask = 0;
        // Probe every active element before committing any of them.
        for n in (0..elements).rev() {
            if (mask & (1u64 << n)) != 0 {
                self.v_read_rmw_dword(seg, eaddr.wrapping_add(4 * n as u64))?;
            }
        }
        for n in 0..elements {
            if (mask & (1u64 << n)) != 0 {
                self.v_write_dword(seg, eaddr.wrapping_add(4 * n as u64), op.zmm32u(n))?;
            }
        }
        self.alignment_check_mask = saved_ac;
        Ok(())
    }

    pub(super) fn avx_masked_store64(
        &mut self,
        instr: &Instruction,
        eaddr: u64,
        op: &BxPackedZmmRegister,
        mask: u64,
    ) -> super::Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let elements = qword_elements(instr.get_vl());

        if instr.as64_l() != 0 {
            let laddr = self.get_laddr64(seg as usize, eaddr);
            for n in 0..elements {
                if (mask & (1u64 << n)) != 0
                    && !self.is_canonical(laddr.wrapping_add(8 * n as u64))
                {
                    return self.exception(Self::seg_exception(seg), 0);
                }
            }
        }

        let saved_ac = self.alignment_check_mask;
        self.alignment_check_mask = 0;
        for n in (0..elements).rev() {
            if (mask & (1u64 << n)) != 0 {
                self.v_read_rmw_qword(seg, eaddr.wrapping_add(8 * n as u64))?;
            }
        }
        for n in 0..elements {
            if (mask & (1u64 << n)) != 0 {
                self.v_write_qword(seg, eaddr.wrapping_add(8 * n as u64), op.zmm64u(n))?;
            }
        }
        self.alignment_check_mask = saved_ac;
        Ok(())
    }

    // ========================================================================
    // Full-width vector loads, no masking, no broadcast.
    // ========================================================================

    /// Bochs load.cc `LOAD_Vector` — read a full VL-wide operand.
    pub(super) fn evex_load_vector(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        match instr.get_vl() {
            0 => tmp.set_zmm128(0, self.v_read_xmmword(seg, eaddr)?),
            1 => tmp.set_zmm256(0, self.v_read_ymmword(seg, eaddr)?),
            _ => tmp = self.v_read_zmmword(seg, eaddr)?,
        }
        Ok(tmp)
    }

    /// Bochs load.cc `LOADU_Wdq` — 128-bit operand, alignment never enforced.
    pub(super) fn evex_loadu_wdq(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        tmp.set_zmm128(0, self.v_read_xmmword(seg, eaddr)?);
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_Half_Vector` — 64/128/256-bit operand.
    pub(super) fn evex_load_half_vector(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        match instr.get_vl() {
            0 => tmp.set_zmm64u(0, self.v_read_qword(seg, eaddr)?),
            1 => tmp.set_zmm128(0, self.v_read_xmmword(seg, eaddr)?),
            _ => tmp.set_zmm256(0, self.v_read_ymmword(seg, eaddr)?),
        }
        Ok(tmp)
    }

    // ========================================================================
    // Masked vector loads — fault suppression, no broadcast.
    //
    // When the effective opmask is empty Bochs skips the load entirely and
    // lets the arithmetic handler apply zero/merge masking; the temp register
    // keeps whatever it held. No lane of an all-masked-off operand can reach
    // the destination, so returning a zeroed register here is equivalent and
    // avoids carrying stale state between instructions.
    // ========================================================================

    /// Bochs load.cc `LOAD_MASK_VectorB`.
    pub(super) fn evex_load_mask_vector_b(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let mut opmask = self.load_opmask64(instr);
        // 64 byte lanes at VL512 would overflow the cut; Bochs skips it there.
        if vl != 2 {
            opmask &= cut_opmask_to(byte_elements(vl));
        }
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        self.avx_masked_load8(instr, eaddr, &mut tmp, opmask)?;
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_MASK_VectorW`.
    pub(super) fn evex_load_mask_vector_w(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask32(instr) & cut_opmask_to(word_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        self.avx_masked_load16(instr, eaddr, &mut tmp, opmask)?;
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_MASK_Half_VectorD`.
    /// Bochs load.cc `LOAD_Quarter_Vector` — 32/64/128-bit operand.
    pub(super) fn evex_load_quarter_vector(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        match instr.get_vl() {
            0 => tmp.set_zmm32u(0, self.v_read_dword(seg, eaddr)?),
            1 => tmp.set_zmm64u(0, self.v_read_qword(seg, eaddr)?),
            _ => tmp.set_zmm128(0, self.v_read_xmmword(seg, eaddr)?),
        }
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_Eighth_Vector` — 16/32/64-bit operand.
    pub(super) fn evex_load_eighth_vector(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        match instr.get_vl() {
            0 => tmp.set_zmm16u(0, self.v_read_word(seg, eaddr)?),
            1 => tmp.set_zmm32u(0, self.v_read_dword(seg, eaddr)?),
            _ => tmp.set_zmm64u(0, self.v_read_qword(seg, eaddr)?),
        }
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_MASK_Half_VectorB` — half-width byte operand, so the
    /// mask is cut to the *word* element count.
    pub(super) fn evex_load_mask_half_vector_b(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask32(instr) & cut_opmask_to(word_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        self.avx_masked_load8(instr, eaddr, &mut tmp, opmask)?;
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_MASK_Half_VectorW`.
    pub(super) fn evex_load_mask_half_vector_w(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask16(instr) & cut_opmask_to(dword_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        self.avx_masked_load16(instr, eaddr, &mut tmp, opmask)?;
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_MASK_Quarter_VectorB`.
    pub(super) fn evex_load_mask_quarter_vector_b(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask16(instr) & cut_opmask_to(dword_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        self.avx_masked_load8(instr, eaddr, &mut tmp, opmask)?;
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_MASK_Quarter_VectorW`.
    pub(super) fn evex_load_mask_quarter_vector_w(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask8(instr) & cut_opmask_to(qword_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        self.avx_masked_load16(instr, eaddr, &mut tmp, opmask)?;
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_MASK_Eighth_VectorB`.
    pub(super) fn evex_load_mask_eighth_vector_b(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask8(instr) & cut_opmask_to(qword_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        self.avx_masked_load8(instr, eaddr, &mut tmp, opmask)?;
        Ok(tmp)
    }

    pub(super) fn evex_load_mask_half_vector_d(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask8(instr) & cut_opmask_to(qword_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        self.avx_masked_load32(instr, eaddr, &mut tmp, opmask)?;
        Ok(tmp)
    }

    // ========================================================================
    // Broadcasting vector loads.
    //
    // `EVEX.b` on a memory operand selects embedded broadcast: read one
    // element and replicate it over the whole vector. The `_MASK_` flavours
    // additionally suppress faults on masked-off elements — but only on the
    // non-broadcast path, since a broadcast touches a single element that is
    // always read (Bochs checks `getEvexb()` before consulting the mask).
    // ========================================================================

    /// Bochs load.cc `LOAD_BROADCAST_VectorD`.
    pub(super) fn evex_load_broadcast_vector_d(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let vl = instr.get_vl();
        let mut tmp = BxPackedZmmRegister::default();
        if instr.get_evex_b() != 0 {
            let val = self.v_read_dword(seg, eaddr)?;
            for n in 0..dword_elements(vl) {
                tmp.set_zmm32u(n, val);
            }
        } else {
            tmp = self.evex_load_full_width(instr, eaddr, seg, vl)?;
        }
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_BROADCAST_VectorQ`.
    pub(super) fn evex_load_broadcast_vector_q(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let vl = instr.get_vl();
        let mut tmp = BxPackedZmmRegister::default();
        if instr.get_evex_b() != 0 {
            let val = self.v_read_qword(seg, eaddr)?;
            for n in 0..qword_elements(vl) {
                tmp.set_zmm64u(n, val);
            }
        } else {
            tmp = self.evex_load_full_width(instr, eaddr, seg, vl)?;
        }
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_BROADCAST_MASK_VectorD`.
    pub(super) fn evex_load_broadcast_mask_vector_d(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask16(instr) & cut_opmask_to(dword_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        if instr.get_evex_b() != 0 {
            let val = self.v_read_dword(seg, eaddr)?;
            for n in 0..dword_elements(vl) {
                tmp.set_zmm32u(n, val);
            }
        } else {
            self.avx_masked_load32(instr, eaddr, &mut tmp, opmask)?;
        }
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_BROADCAST_MASK_VectorQ`.
    pub(super) fn evex_load_broadcast_mask_vector_q(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask8(instr) & cut_opmask_to(qword_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        if instr.get_evex_b() != 0 {
            let val = self.v_read_qword(seg, eaddr)?;
            for n in 0..qword_elements(vl) {
                tmp.set_zmm64u(n, val);
            }
        } else {
            self.avx_masked_load64(instr, eaddr, &mut tmp, opmask)?;
        }
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_BROADCAST_Half_VectorD`.
    pub(super) fn evex_load_broadcast_half_vector_d(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let vl = instr.get_vl();
        let mut tmp = BxPackedZmmRegister::default();
        if instr.get_evex_b() != 0 {
            let val = self.v_read_dword(seg, eaddr)?;
            for n in 0..qword_elements(vl) {
                tmp.set_zmm32u(n, val);
            }
        } else {
            tmp = self.evex_load_half_width(instr, eaddr, seg, vl)?;
        }
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_BROADCAST_MASK_Half_VectorD`.
    pub(super) fn evex_load_broadcast_mask_half_vector_d(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let vl = instr.get_vl();
        let opmask = self.load_opmask8(instr) & cut_opmask_to(qword_elements(vl));
        let mut tmp = BxPackedZmmRegister::default();
        if opmask == 0 {
            return Ok(tmp);
        }
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        if instr.get_evex_b() != 0 {
            let val = self.v_read_dword(seg, eaddr)?;
            for n in 0..qword_elements(vl) {
                tmp.set_zmm32u(n, val);
            }
        } else {
            self.avx_masked_load32(instr, eaddr, &mut tmp, opmask)?;
        }
        Ok(tmp)
    }

    // ========================================================================
    // Scalar loads — Bochs load.cc LOAD_Wss / LOAD_Wsd and their masked forms.
    // ========================================================================

    /// Bochs load.cc `LOAD_Wss` — 32-bit scalar into the low dword.
    pub(super) fn evex_load_wss(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        tmp.set_zmm32u(0, self.v_read_dword(seg, eaddr)?);
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_Wsd` — 64-bit scalar into the low qword.
    pub(super) fn evex_load_wsd(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        tmp.set_zmm64u(0, self.v_read_qword(seg, eaddr)?);
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_MASK_Wss` — masked 32-bit scalar; an inactive
    /// element reads as zero and performs no memory access.
    pub(super) fn evex_load_mask_wss(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let mut tmp = BxPackedZmmRegister::default();
        if self.scalar_element_mask(instr) {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            tmp.set_zmm32u(0, self.v_read_dword(seg, eaddr)?);
        }
        Ok(tmp)
    }

    /// Bochs load.cc `LOAD_MASK_Wsd` — masked 64-bit scalar.
    pub(super) fn evex_load_mask_wsd(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        let mut tmp = BxPackedZmmRegister::default();
        if self.scalar_element_mask(instr) {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            tmp.set_zmm64u(0, self.v_read_qword(seg, eaddr)?);
        }
        Ok(tmp)
    }

    // ========================================================================
    // Loader pairs.
    //
    // `ia_opcodes_evex.def` gives an opcode two entries — a base one and a
    // `_Kmask` one — and the decoder picks the `_Kmask` entry exactly when
    // EVEX.aaa is non-zero (Bochs fetchdecode64.cc sets its `MASK_K0`
    // attribute otherwise). rusty_box dispatches both entries to one handler,
    // so the handler needs the same either/or. The pairings are not uniform
    // across opcodes — VPADDB pairs `LOAD_Vector` with `LOAD_MASK_VectorB`
    // while VPSHUFB uses `LOAD_Vector` for both — so each pairing that
    // actually occurs gets its own named function here, and a handler picks
    // the one its def entries specify rather than assuming a rule.
    // ========================================================================

    /// Pair (`LOAD_BROADCAST_VectorD`, `LOAD_BROADCAST_MASK_VectorD`).
    pub(super) fn evex_load_bcst_d_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_broadcast_vector_d(instr)
        } else {
            self.evex_load_broadcast_mask_vector_d(instr)
        }
    }

    /// Pair (`LOAD_BROADCAST_VectorQ`, `LOAD_BROADCAST_MASK_VectorQ`).
    pub(super) fn evex_load_bcst_q_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_broadcast_vector_q(instr)
        } else {
            self.evex_load_broadcast_mask_vector_q(instr)
        }
    }

    /// Pair (`LOAD_Vector`, `LOAD_MASK_VectorB`).
    pub(super) fn evex_load_vec_mask_b_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_vector(instr)
        } else {
            self.evex_load_mask_vector_b(instr)
        }
    }

    /// Pair (`LOAD_Vector`, `LOAD_MASK_VectorW`).
    pub(super) fn evex_load_vec_mask_w_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_vector(instr)
        } else {
            self.evex_load_mask_vector_w(instr)
        }
    }

    /// Pair (`LOAD_BROADCAST_Half_VectorD`, `LOAD_BROADCAST_MASK_Half_VectorD`).
    pub(super) fn evex_load_bcst_half_d_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_broadcast_half_vector_d(instr)
        } else {
            self.evex_load_broadcast_mask_half_vector_d(instr)
        }
    }

    /// Pair (`LOAD_Half_Vector`, `LOAD_MASK_Half_VectorB`) — VPMOVSXBW/ZXBW.
    pub(super) fn evex_load_half_vec_mask_b_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_half_vector(instr)
        } else {
            self.evex_load_mask_half_vector_b(instr)
        }
    }

    /// Pair (`LOAD_Half_Vector`, `LOAD_MASK_Half_VectorW`) — VPMOVSXWD/ZXWD.
    pub(super) fn evex_load_half_vec_mask_w_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_half_vector(instr)
        } else {
            self.evex_load_mask_half_vector_w(instr)
        }
    }

    /// Pair (`LOAD_Quarter_Vector`, `LOAD_MASK_Quarter_VectorB`) — VPMOVSXBD/ZXBD.
    pub(super) fn evex_load_quarter_vec_mask_b_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_quarter_vector(instr)
        } else {
            self.evex_load_mask_quarter_vector_b(instr)
        }
    }

    /// Pair (`LOAD_Quarter_Vector`, `LOAD_MASK_Quarter_VectorW`) — VPMOVSXWQ/ZXWQ.
    pub(super) fn evex_load_quarter_vec_mask_w_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_quarter_vector(instr)
        } else {
            self.evex_load_mask_quarter_vector_w(instr)
        }
    }

    /// Pair (`LOAD_Eighth_Vector`, `LOAD_MASK_Eighth_VectorB`) — VPMOVSXBQ/ZXBQ.
    pub(super) fn evex_load_eighth_vec_mask_b_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_eighth_vector(instr)
        } else {
            self.evex_load_mask_eighth_vector_b(instr)
        }
    }

    /// Pair (`LOAD_Half_Vector`, `LOAD_MASK_Half_VectorD`).
    pub(super) fn evex_load_half_vec_mask_d_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_half_vector(instr)
        } else {
            self.evex_load_mask_half_vector_d(instr)
        }
    }

    /// Pair (`LOAD_Wss`, `LOAD_MASK_Wss`).
    pub(super) fn evex_load_wss_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_wss(instr)
        } else {
            self.evex_load_mask_wss(instr)
        }
    }

    /// Pair (`LOAD_Wsd`, `LOAD_MASK_Wsd`).
    pub(super) fn evex_load_wsd_pair(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.opmask() == 0 {
            self.evex_load_wsd(instr)
        } else {
            self.evex_load_mask_wsd(instr)
        }
    }

    // ========================================================================
    // Shared width helpers for the non-broadcast arms above. Bochs repeats
    // these `getVL()` ladders inline in every LOAD_BROADCAST_* function.
    // ========================================================================

    fn evex_load_full_width(
        &mut self,
        _instr: &Instruction,
        eaddr: u64,
        seg: BxSegregs,
        vl: u8,
    ) -> super::Result<BxPackedZmmRegister> {
        let mut tmp = BxPackedZmmRegister::default();
        match vl {
            0 => tmp.set_zmm128(0, self.v_read_xmmword(seg, eaddr)?),
            1 => tmp.set_zmm256(0, self.v_read_ymmword(seg, eaddr)?),
            _ => tmp = self.v_read_zmmword(seg, eaddr)?,
        }
        Ok(tmp)
    }

    fn evex_load_half_width(
        &mut self,
        _instr: &Instruction,
        eaddr: u64,
        seg: BxSegregs,
        vl: u8,
    ) -> super::Result<BxPackedZmmRegister> {
        let mut tmp = BxPackedZmmRegister::default();
        match vl {
            0 => tmp.set_zmm64u(0, self.v_read_qword(seg, eaddr)?),
            1 => tmp.set_zmm128(0, self.v_read_xmmword(seg, eaddr)?),
            _ => tmp.set_zmm256(0, self.v_read_ymmword(seg, eaddr)?),
        }
        Ok(tmp)
    }

}
