#![allow(unused_unsafe, dead_code)]

//! Intel CET (Control-flow Enforcement Technology) implementation.
//!
//! Mirrors Bochs cpu/cet.cc — ENDBR32/ENDBR64, shadow stack helpers,
//! indirect branch tracking, and legacy endbranch treatment.

use crate::cpu::{BxCpuC, BxCpuIdTrait};

use super::decoder::{BxSegregs, Instruction};
use super::Result;

// CET control MSR bit constants — matches Bochs cet.cc
pub(super) const CET_SHADOW_STACK_ENABLED: u64 = 1 << 0;
pub(super) const CET_SHADOW_STACK_WRITE_ENABLED: u64 = 1 << 1;
pub(super) const CET_ENDBRANCH_ENABLED: u64 = 1 << 2;
pub(super) const CET_LEGACY_INDIRECT_BRANCH_TREATMENT: u64 = 1 << 3;
pub(super) const CET_ENABLE_NO_TRACK_INDIRECT_BRANCH_PREFIX: u64 = 1 << 4;
pub(super) const CET_SUPPRESS_DIS: u64 = 1 << 5;
// bits 6-9 reserved (0x3c0)
pub(super) const CET_SUPPRESS_INDIRECT_BRANCH_TRACKING: u64 = 1 << 10;
pub(super) const CET_WAIT_FOR_ENBRANCH: u64 = 1 << 11;

// #CP exception error codes — matches Bochs cpu.h BxCPException enum
pub(super) const BX_CP_NEAR_RET: u16 = 1;
pub(super) const BX_CP_FAR_RET_IRET: u16 = 2;
pub(super) const BX_CP_ENDBRANCH: u16 = 3;
pub(super) const BX_CP_RSTORSSP: u16 = 4;
pub(super) const BX_CP_SETSSBSY: u16 = 5;

/// Reserved bits mask for CET control validation — bits 6-9
const CET_RESERVED_BITS: u64 = 0x3c0;

/// Canonicalize a 64-bit address (sign-extend bit 47).
/// Bochs cpu.h CanonicalizeAddress()
#[inline]
pub(super) fn canonicalize_address(addr: u64) -> u64 {
    if addr & 0x0000_8000_0000_0000 != 0 {
        addr | 0xFFFF_0000_0000_0000
    } else {
        addr & 0x0000_FFFF_FFFF_FFFF
    }
}

/// Check if a CET control value has invalid bit combinations.
/// Matches Bochs cet.cc is_invalid_cet_control()
pub(super) fn is_invalid_cet_control(val: u64) -> bool {
    // SUPPRESS and WAIT_FOR_ENBRANCH cannot both be set
    if (val & (CET_SUPPRESS_INDIRECT_BRANCH_TRACKING | CET_WAIT_FOR_ENBRANCH))
        == (CET_SUPPRESS_INDIRECT_BRANCH_TRACKING | CET_WAIT_FOR_ENBRANCH)
    {
        return true;
    }
    // Reserved bits 6-9 must be zero
    if val & CET_RESERVED_BITS != 0 {
        return true;
    }
    false
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // CET query helpers — Bochs cet.cc
    // =========================================================================

    /// Check if shadow stack is enabled for the given privilege level.
    /// Bochs cet.cc ShadowStackEnabled()
    pub(super) fn shadow_stack_enabled(&self, cpl: u8) -> bool {
        self.cr4.cet()
            && self.protected_mode()
            && (self.msr.ia32_cet_control[usize::from(cpl == 3)]
                & CET_SHADOW_STACK_ENABLED)
                != 0
    }

    /// Check if shadow stack writes are enabled for the given privilege level.
    /// Bochs cet.cc ShadowStackWriteEnabled()
    pub(super) fn shadow_stack_write_enabled(&self, cpl: u8) -> bool {
        self.cr4.cet()
            && self.protected_mode()
            && (self.msr.ia32_cet_control[usize::from(cpl == 3)]
                & (CET_SHADOW_STACK_ENABLED | CET_SHADOW_STACK_WRITE_ENABLED))
                == (CET_SHADOW_STACK_ENABLED | CET_SHADOW_STACK_WRITE_ENABLED)
    }

    /// Check if indirect branch tracking (ENDBRANCH enforcement) is enabled.
    /// Bochs cet.cc EndbranchEnabled()
    pub(super) fn endbranch_enabled(&self, cpl: u8) -> bool {
        self.cr4.cet()
            && self.protected_mode()
            && (self.msr.ia32_cet_control[usize::from(cpl == 3)]
                & CET_ENDBRANCH_ENABLED)
                != 0
    }

    /// Check if endbranch is enabled and NOT suppressed.
    /// Bochs cet.cc EndbranchEnabledAndNotSuppressed()
    pub(super) fn endbranch_enabled_and_not_suppressed(&self, cpl: u8) -> bool {
        self.cr4.cet()
            && self.protected_mode()
            && (self.msr.ia32_cet_control[usize::from(cpl == 3)]
                & (CET_ENDBRANCH_ENABLED | CET_SUPPRESS_INDIRECT_BRANCH_TRACKING))
                == CET_ENDBRANCH_ENABLED
    }

    /// Check if we are waiting for an ENDBRANCH instruction after an indirect branch.
    /// Bochs cet.cc WaitingForEndbranch()
    pub(super) fn waiting_for_endbranch(&self, cpl: u8) -> bool {
        self.cr4.cet()
            && self.protected_mode()
            && (self.msr.ia32_cet_control[usize::from(cpl == 3)]
                & (CET_ENDBRANCH_ENABLED | CET_WAIT_FOR_ENBRANCH))
                == (CET_ENDBRANCH_ENABLED | CET_WAIT_FOR_ENBRANCH)
    }

    // =========================================================================
    // CET tracking helpers — Bochs cet.cc
    // =========================================================================

    /// Set WAIT_FOR_ENBRANCH flag after an indirect branch.
    /// Bochs cet.cc track_indirect()
    pub(super) fn track_indirect(&mut self, cpl: u8) {
        if self.endbranch_enabled(cpl) {
            let idx = usize::from(cpl == 3);
            self.msr.ia32_cet_control[idx] |= CET_WAIT_FOR_ENBRANCH;
            self.msr.ia32_cet_control[idx] &= !CET_SUPPRESS_INDIRECT_BRANCH_TRACKING;
        }
    }

    /// Track indirect branch if not suppressed (with DS-prefix no-track check).
    /// Bochs cet.cc track_indirect_if_not_suppressed()
    pub(super) fn track_indirect_if_not_suppressed(
        &mut self,
        seg_override_cet: u8,
        cpl: u8,
    ) {
        if self.endbranch_enabled_and_not_suppressed(cpl) {
            // DS segment override acts as no-track prefix when enabled
            if seg_override_cet == BxSegregs::Ds as u8
                && (self.msr.ia32_cet_control[usize::from(cpl == 3)]
                    & CET_ENABLE_NO_TRACK_INDIRECT_BRANCH_PREFIX)
                    != 0
            {
                return;
            }
            self.msr.ia32_cet_control[usize::from(cpl == 3)] |= CET_WAIT_FOR_ENBRANCH;
        }
    }

    /// Reset the ENDBRANCH tracker after executing a valid ENDBRANCH.
    /// Bochs cet.cc reset_endbranch_tracker()
    pub(super) fn reset_endbranch_tracker(&mut self, cpl: u8, suppress: bool) {
        let idx = usize::from(cpl == 3);
        self.msr.ia32_cet_control[idx] &=
            !(CET_WAIT_FOR_ENBRANCH | CET_SUPPRESS_INDIRECT_BRANCH_TRACKING);
        if suppress
            && (self.msr.ia32_cet_control[idx] & CET_SUPPRESS_DIS) == 0
        {
            self.msr.ia32_cet_control[idx] |= CET_SUPPRESS_INDIRECT_BRANCH_TRACKING;
        }
    }

    /// Check legacy endbranch treatment bitmap.
    /// Returns true if the instruction should still raise #CP (legacy check failed).
    /// Returns false if the legacy bitmap indicates this is OK (tracker reset).
    /// Bochs cet.cc LegacyEndbranchTreatment()
    pub(super) fn legacy_endbranch_treatment(&mut self, cpl: u8) -> Result<bool> {
        let idx = usize::from(cpl == 3);
        if self.msr.ia32_cet_control[idx] & CET_LEGACY_INDIRECT_BRANCH_TREATMENT != 0 {
            let lip = if self.long64_mode() {
                self.get_laddr64(BxSegregs::Cs as usize, self.rip())
            } else {
                self.get_laddr32(BxSegregs::Cs as usize, self.rip() as u32) as u64
            };
            let bitmap_addr =
                (self.msr.ia32_cet_control[idx] & !0xFFF) + ((lip & 0xFFFF_FFFF_FFFF) >> 15);
            let bitmap_index = ((lip >> 12) & 0x7) as u32;
            let bitmap = self.system_read_byte(bitmap_addr)?;
            if (bitmap & (1 << bitmap_index)) != 0 {
                self.reset_endbranch_tracker(cpl, true);
                return Ok(false); // legacy bitmap says OK
            }
        }
        Ok(true) // should raise #CP
    }

    // =========================================================================
    // Shadow stack memory operations — Bochs access2.cc
    // =========================================================================

    /// Read a dword from the shadow stack.
    /// Bochs access2.cc shadow_stack_read_dword. The CPL drives the
    /// SS U/S match in the page walker via translate_shadow_stack_read.
    pub(super) fn shadow_stack_read_dword(&mut self, offset: u64, cpl: u8) -> Result<u32> {
        self.shadow_stack_read_linear_dword(offset, cpl)
    }

    /// Read a qword from the shadow stack.
    /// Bochs access2.cc shadow_stack_read_qword.
    pub(super) fn shadow_stack_read_qword(&mut self, offset: u64, cpl: u8) -> Result<u64> {
        self.shadow_stack_read_linear_qword(offset, cpl)
    }

    /// Write a dword to the shadow stack.
    /// Bochs access2.cc shadow_stack_write_dword.
    pub(super) fn shadow_stack_write_dword(&mut self, offset: u64, cpl: u8, data: u32) -> Result<()> {
        self.shadow_stack_write_linear_dword(offset, cpl, data)
    }

    /// Write a qword to the shadow stack.
    /// Bochs access2.cc shadow_stack_write_qword.
    pub(super) fn shadow_stack_write_qword(&mut self, offset: u64, cpl: u8, data: u64) -> Result<()> {
        self.shadow_stack_write_linear_qword(offset, cpl, data)
    }

    /// Pop a qword from the shadow stack: read SSP, then SSP += 8.
    /// Bochs stack.h shadow_stack_pop_64()
    pub(super) fn shadow_stack_pop_64(&mut self) -> Result<u64> {
        let ssp = self.ssp();
        let cpl = self.cs_rpl();
        let val = self.shadow_stack_read_qword(ssp, cpl)?;
        self.set_ssp(ssp + 8);
        Ok(val)
    }

    /// Pop a dword from the shadow stack: read SSP, then SSP += 4.
    /// Bochs stack.h shadow_stack_pop_32()
    pub(super) fn shadow_stack_pop_32(&mut self) -> Result<u32> {
        let ssp = self.ssp();
        let cpl = self.cs_rpl();
        let val = self.shadow_stack_read_dword(ssp, cpl)?;
        self.set_ssp(ssp + 4);
        Ok(val)
    }

    /// Atomic compare-exchange on shadow stack (locked RMW).
    /// Bochs access2.cc shadow_stack_lock_cmpxchg8b()
    /// Returns true if the exchange succeeded.
    fn shadow_stack_lock_cmpxchg8b(
        &mut self,
        offset: u64,
        cpl: u8,
        data: u64,
        expected: u64,
    ) -> Result<bool> {
        let val = self.shadow_stack_read_qword(offset, cpl)?;
        if val == expected {
            self.shadow_stack_write_qword(offset, cpl, data)?;
            Ok(true)
        } else {
            self.shadow_stack_write_qword(offset, cpl, val)?;
            Ok(false)
        }
    }

    /// Atomically set the busy bit on a shadow stack token.
    /// Bochs access2.cc shadow_stack_atomic_set_busy()
    /// Returns true on success.
    pub(super) fn shadow_stack_atomic_set_busy(&mut self, offset: u64, cpl: u8) -> Result<bool> {
        let expected = if self.long64_mode() { offset } else { offset & 0xFFFF_FFFF };
        self.shadow_stack_lock_cmpxchg8b(offset, cpl, offset | 0x1, expected)
    }

    /// Atomically clear the busy bit on a shadow stack token.
    /// Returns the raw cmpxchg result: true if the exchange matched.
    /// Bochs: shadow_stack_atomic_clear_busy (access2.cc)
    pub(super) fn shadow_stack_atomic_clear_busy(&mut self, offset: u64, cpl: u8) -> Result<bool> {
        self.shadow_stack_lock_cmpxchg8b(offset, cpl, offset, offset | 0x1)
    }

    /// Restore shadow stack state from a FRED/IRET shadow stack frame.
    /// Bochs ret_far.cc shadow_stack_restore(raw_cs_selector, return_lip)
    ///
    /// Pops three qwords (prevSSP, shadowLIP, shadowCS), validates them,
    /// and returns prevSSP.
    pub(super) fn shadow_stack_restore_lip(&mut self, raw_cs_selector: u16, return_lip: u64) -> Result<u64> {
        let ssp = self.ssp();
        if ssp & 0x7 != 0 {
            tracing::error!("shadow_stack_restore: SSP must be 8-byte aligned");
            self.exception(super::cpu::Exception::Cp, BX_CP_FAR_RET_IRET)?;
            unreachable!();
        }

        let prev_ssp = self.shadow_stack_pop_64()?;
        let shadow_lip = self.shadow_stack_pop_64()?;
        let shadow_cs = self.shadow_stack_pop_64()?;

        if raw_cs_selector as u64 != shadow_cs {
            tracing::error!("shadow_stack_restore: CS mismatch");
            self.exception(super::cpu::Exception::Cp, BX_CP_FAR_RET_IRET)?;
            unreachable!();
        }
        if return_lip != shadow_lip {
            tracing::error!("shadow_stack_restore: LIP mismatch");
            self.exception(super::cpu::Exception::Cp, BX_CP_FAR_RET_IRET)?;
            unreachable!();
        }
        if prev_ssp & 0x3 != 0 {
            tracing::error!("shadow_stack_restore: prevSSP must be 4-byte aligned");
            self.exception(super::cpu::Exception::Cp, BX_CP_FAR_RET_IRET)?;
            unreachable!();
        }
        if !self.long64_mode() && (prev_ssp >> 32) != 0 {
            tracing::error!("shadow_stack_restore: prevSSP must be 32-bit in 32-bit mode");
            self.exception(super::cpu::Exception::Gp, 0)?;
            unreachable!();
        }

        Ok(prev_ssp)
    }


    // =========================================================================
    // ENDBR32 / ENDBR64 instruction handlers — Bochs cet.cc
    // =========================================================================

    /// ENDBRANCH32 handler.
    /// In non-64-bit mode: resets the endbranch tracker.
    /// In 64-bit mode: acts as NOP (wrong-mode ENDBRANCH is a NOP).
    /// Bochs cet.cc
    pub(super) fn endbranch32(
        &mut self,
        _instr: &Instruction,
    ) -> Result<()> {
        if !self.long64_mode() {
            let cpl = self.cs_rpl();
            self.reset_endbranch_tracker(cpl, false);
        }
        // In 64-bit mode: NOP (BX_NEXT_TRACE)
        Ok(())
    }

    /// ENDBRANCH64 handler.
    /// In 64-bit mode: resets the endbranch tracker.
    /// In non-64-bit mode: acts as NOP (wrong-mode ENDBRANCH is a NOP).
    /// Bochs cet.cc
    pub(super) fn endbranch64(
        &mut self,
        _instr: &Instruction,
    ) -> Result<()> {
        if self.long64_mode() {
            let cpl = self.cs_rpl();
            self.reset_endbranch_tracker(cpl, false);
        }
        // In non-64-bit mode: NOP (BX_NEXT_TRACE)
        Ok(())
    }

    // =========================================================================
    // SETSSBSY / CLRSSBSY instruction handlers — Bochs cet.cc
    // =========================================================================

    /// SETSSBSY handler.
    /// Sets the shadow stack busy flag and loads SSP from IA32_PL0_SSP.
    /// Bochs cet.cc SETSSBSY()
    pub(super) fn setssbsy(
        &mut self,
        _instr: &Instruction,
    ) -> Result<()> {
        // FRED check: SETSSBSY is not supported when FRED is enabled in CR4.
        if self.cr4.fred() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        if !self.shadow_stack_enabled(0) {
            tracing::error!("SETSSBSY: shadow stack not enabled");
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let cpl = self.cs_rpl();
        if cpl > 0 {
            tracing::error!("SETSSBSY: CPL != 0");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let ssp_laddr = self.msr.ia32_pl_ssp[0];
        if ssp_laddr & 0x7 != 0 {
            tracing::error!("SETSSBSY: SSP_LA not aligned to 8 bytes boundary");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        if !self.shadow_stack_atomic_set_busy(ssp_laddr, cpl)? {
            tracing::error!("SETSSBSY: failed to set SSP busy bit");
            return self.exception(super::cpu::Exception::Cp, BX_CP_SETSSBSY);
        }

        self.set_ssp(ssp_laddr);
        Ok(())
    }

    /// CLRSSBSY handler.
    /// Clears the shadow stack busy flag at the address given by the memory operand.
    /// Bochs cet.cc CLRSSBSY()
    pub(super) fn clrssbsy(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        // FRED check: CLRSSBSY is not supported when FRED is enabled in CR4.
        if self.cr4.fred() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        if !self.shadow_stack_enabled(0) {
            tracing::error!("CLRSSBSY: shadow stack not enabled");
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let cpl = self.cs_rpl();
        if cpl > 0 {
            tracing::error!("CLRSSBSY: CPL != 0");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.agen_read32(seg, eaddr as u32, 8)? as u64
        };
        if laddr & 0x7 != 0 {
            tracing::error!("CLRSSBSY: SSP_LA not aligned to 8 bytes boundary");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let invalid_token = self.shadow_stack_atomic_clear_busy(laddr, cpl)?;
        // Bochs cet.cc CLRSSBSY: clearEFlagsOSZAPC(); if (invalid_token) assert_CF();
        self.oszapc.set_oszapc_logic_32(1);
        if invalid_token {
            self.oszapc.set_cf(true);
        }
        self.set_ssp(0);

        Ok(())
    }

    // =========================================================================
    // Shadow-stack pointer manipulation — Bochs cet.cc
    // =========================================================================

    /// INCSSPD — Increment SSP by 32-bit register value (dword stride).
    /// Bochs cet.cc BX_CPU_C::INCSSPD.
    pub(super) fn incsspd(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        let cpl = self.cs_rpl();
        if !self.shadow_stack_enabled(cpl) {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let src = self.get_gpr32(instr.dst() as usize) & 0xff;
        let tmpsrc = if src == 0 { 1 } else { src };

        // Bochs touches the first and last dword of the increment range to
        // trigger any page faults / privilege checks the increment would have.
        let ssp = self.ssp();
        let _ = self.shadow_stack_read_dword(ssp, cpl)?;
        let _ = self
            .shadow_stack_read_dword(ssp + (tmpsrc as u64 - 1) * 4, cpl)?;
        self.set_ssp(ssp + (src as u64) * 4);
        Ok(())
    }

    /// INCSSPQ — Increment SSP by 32-bit register value (qword stride).
    /// Bochs cet.cc BX_CPU_C::INCSSPQ.
    pub(super) fn incsspq(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        let cpl = self.cs_rpl();
        if !self.shadow_stack_enabled(cpl) {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let src = self.get_gpr32(instr.dst() as usize) & 0xff;
        let tmpsrc = if src == 0 { 1 } else { src };

        let ssp = self.ssp();
        let _ = self.shadow_stack_read_qword(ssp, cpl)?;
        let _ = self
            .shadow_stack_read_qword(ssp + (tmpsrc as u64 - 1) * 8, cpl)?;
        self.set_ssp(ssp + (src as u64) * 8);
        Ok(())
    }

    /// RDSSPD — Read SSP into 32-bit destination (zero-extended).
    /// Bochs cet.cc BX_CPU_C::RDSSPD. NOP when shadow stack disabled.
    pub(super) fn rdsspd(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        if self.shadow_stack_enabled(self.cs_rpl()) {
            // Bochs writes BX_READ_32BIT_REG(BX_32BIT_REG_SSP) — low 32 bits of SSP.
            let val = self.ssp() as u32;
            self.set_gpr32(instr.dst() as usize, val);
        }
        Ok(())
    }

    /// RDSSPQ — Read SSP into 64-bit destination.
    /// Bochs cet.cc BX_CPU_C::RDSSPQ. NOP when shadow stack disabled.
    pub(super) fn rdsspq(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        if self.shadow_stack_enabled(self.cs_rpl()) {
            let val = self.ssp();
            self.set_gpr64(instr.dst() as usize, val);
        }
        Ok(())
    }

    /// SAVEPREVSSP — Save previous-SSP token to the previous shadow stack.
    /// Bochs cet.cc BX_CPU_C::SAVEPREVSSP.
    pub(super) fn saveprevssp(
        &mut self,
        _instr: &Instruction,
    ) -> Result<()> {
        let cpl = self.cs_rpl();
        if !self.shadow_stack_enabled(cpl) {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let ssp = self.ssp();
        if ssp & 0x7 != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let previous_ssp_token = self.shadow_stack_read_qword(ssp, cpl)?;

        // Bochs cet.cc — pop alignment hole in legacy/compat mode.
        if self.get_cf() {
            if self.long64_mode() {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            if self.shadow_stack_pop_32()? != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        // Bochs cet.cc — token validity checks.
        if (previous_ssp_token & 0x02) == 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        if !self.long64_mode() && (previous_ssp_token >> 32) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bochs cet.cc — write Prev SSP to old shadow stack.
        let mut old_ssp = previous_ssp_token & !0x03u64;
        let tmp = old_ssp | (self.long64_mode() as u64);
        self.shadow_stack_write_dword(old_ssp - 4, cpl, 0)?;
        old_ssp &= !0x07u64;
        self.shadow_stack_write_qword(old_ssp - 8, cpl, tmp)?;

        self.set_ssp(self.ssp() + 8);
        Ok(())
    }

    /// RSTORSSP — Restore SSP from a shadow-stack restore token.
    /// Bochs cet.cc BX_CPU_C::RSTORSSP.
    pub(super) fn rstorssp(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        let cpl = self.cs_rpl();
        if !self.shadow_stack_enabled(cpl) {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.agen_read32(seg, eaddr as u32, 8)? as u64
        };
        if laddr & 0x7 != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let previous_ssp_token = self.ssp() | (self.long64_mode() as u64) | 0x02;

        // Bochs cet.cc — token validation. Should be atomic RMW per Bochs.
        let ssp_tmp = self.shadow_stack_read_qword(laddr, cpl)?;
        if (ssp_tmp & 0x03) != (self.long64_mode() as u64) {
            return self.exception(super::cpu::Exception::Cp, BX_CP_RSTORSSP);
        }
        if !self.long64_mode() && (ssp_tmp >> 32) != 0 {
            return self.exception(super::cpu::Exception::Cp, BX_CP_RSTORSSP);
        }

        // Bochs cet.cc — derive prior top-of-stack from token, must equal laddr.
        let mut tmp = ssp_tmp & !0x01u64;
        tmp = (tmp - 8) & !0x07u64;
        if tmp != laddr {
            return self.exception(super::cpu::Exception::Cp, BX_CP_RSTORSSP);
        }
        self.shadow_stack_write_qword(laddr, cpl, previous_ssp_token)?;

        self.set_ssp(laddr);

        // Bochs cet.cc — clearEFlagsOSZAPC; set CF if 4-byte alignment hole present.
        self.oszapc.set_oszapc_logic_32(1);
        if ssp_tmp & 0x04 != 0 {
            self.oszapc.set_cf(true);
        }
        Ok(())
    }

    /// WRSSD — Write 32-bit register to shadow stack at memory operand.
    /// Bochs cet.cc BX_CPU_C::WRSSD.
    pub(super) fn wrssd(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        let cpl = self.cs_rpl();
        if !self.shadow_stack_write_enabled(cpl) {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.agen_write32(seg, eaddr as u32, 4)? as u64
        };
        if laddr & 0x3 != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let val = self.get_gpr32(instr.src() as usize);
        self.shadow_stack_write_dword(laddr, cpl, val)?;
        Ok(())
    }

    /// WRSSQ — Write 64-bit register to shadow stack at memory operand.
    /// Bochs cet.cc BX_CPU_C::WRSSQ.
    pub(super) fn wrssq(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        let cpl = self.cs_rpl();
        if !self.shadow_stack_write_enabled(cpl) {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.agen_write32(seg, eaddr as u32, 8)? as u64
        };
        if laddr & 0x7 != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let val = self.get_gpr64(instr.src() as usize);
        self.shadow_stack_write_qword(laddr, cpl, val)?;
        Ok(())
    }

    /// WRUSSD — Write 32-bit register to user shadow stack (CPL=0 only).
    /// Bochs cet.cc BX_CPU_C::WRUSSD.
    pub(super) fn wrussd(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        if !self.cr4.cet() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if self.cs_rpl() > 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.agen_write32(seg, eaddr as u32, 4)? as u64
        };
        if laddr & 0x3 != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        // Bochs cet.cc — writes with cpl=3 (user-mode shadow stack).
        let val = self.get_gpr32(instr.src() as usize);
        self.shadow_stack_write_dword(laddr, 3, val)?;
        Ok(())
    }

    /// WRUSSQ — Write 64-bit register to user shadow stack (CPL=0 only).
    /// Bochs cet.cc BX_CPU_C::WRUSSQ.
    pub(super) fn wrussq(
        &mut self,
        instr: &Instruction,
    ) -> Result<()> {
        if !self.cr4.cet() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if self.cs_rpl() > 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.agen_write32(seg, eaddr as u32, 8)? as u64
        };
        if laddr & 0x7 != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let val = self.get_gpr64(instr.src() as usize);
        self.shadow_stack_write_qword(laddr, 3, val)?;
        Ok(())
    }

    // =========================================================================
    // PAUSE instruction handler — Bochs proc_ctrl.cc
    // =========================================================================

    /// PAUSE handler. Checks VMX/SVM intercepts before executing as no-op hint.
    /// Bochs proc_ctrl.cc
    pub(super) fn pause(
        &mut self,
        _instr: &Instruction,
    ) -> Result<()> {
        // Bochs proc_ctrl.cc — VMX PAUSE exit
        if self.in_vmx_guest {
            self.vmexit_pause()?;
        }

        // Bochs proc_ctrl.cc — SVM PAUSE intercept
        if self.in_svm_guest {
            self.svm_intercept_pause()?;
        }

        // PAUSE is a hint — no architectural state changes
        Ok(())
    }

    /// VMX PAUSE exit handler.
    /// Bochs vmexit.cc VMexit_PAUSE()
    /// Checks PAUSE Exiting and PAUSE Loop Exiting (PLE) controls.
    fn vmexit_pause(&mut self) -> Result<()> {
        // TODO: Implement full VMexit_PAUSE when VMX exit machinery is ported.
        // Bochs checks:
        //   1. vmexec_ctrls1.PAUSE_VMEXIT() → VMexit(VMX_VMEXIT_PAUSE, 0)
        //   2. vmexec_ctrls2.PAUSE_LOOP_VMEXIT() && CPL==0 → PLE timing check
        //      - If gap since last PAUSE > pause_loop_exiting_gap: reset window
        //      - If time in PAUSE loop > pause_loop_exiting_window: VMexit
        Ok(())
    }
}
