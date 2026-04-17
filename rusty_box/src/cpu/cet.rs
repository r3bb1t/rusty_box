#![allow(unused_unsafe, dead_code)]

//! Intel CET (Control-flow Enforcement Technology) implementation.
//!
//! Mirrors Bochs cpu/cet.cc — ENDBR32/ENDBR64, shadow stack helpers,
//! indirect branch tracking, and legacy endbranch treatment.

use crate::cpu::{BxCpuC, BxCpuIdTrait};

use super::decoder::BxSegregs;
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

/// Reserved bits mask for CET control validation — bits 6-9
const CET_RESERVED_BITS: u64 = 0x3c0;

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
    // ENDBR32 / ENDBR64 instruction handlers — Bochs cet.cc
    // =========================================================================

    /// ENDBRANCH32 handler.
    /// In non-64-bit mode: resets the endbranch tracker.
    /// In 64-bit mode: acts as NOP (wrong-mode ENDBRANCH is a NOP).
    /// Bochs cet.cc
    pub(super) fn endbranch32(
        &mut self,
        _instr: &crate::cpu::decoder::Instruction,
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
        _instr: &crate::cpu::decoder::Instruction,
    ) -> Result<()> {
        if self.long64_mode() {
            let cpl = self.cs_rpl();
            self.reset_endbranch_tracker(cpl, false);
        }
        // In non-64-bit mode: NOP (BX_NEXT_TRACE)
        Ok(())
    }

    // =========================================================================
    // PAUSE instruction handler — Bochs proc_ctrl.cc
    // =========================================================================

    /// PAUSE handler. Checks VMX/SVM intercepts before executing as no-op hint.
    /// Bochs proc_ctrl.cc
    pub(super) fn pause(
        &mut self,
        _instr: &crate::cpu::decoder::Instruction,
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

    /// SVM PAUSE intercept handler.
    /// Bochs svm.cc SvmInterceptPAUSE()
    /// Checks SVM_INTERCEPT0_PAUSE and pause filter counter.
    fn svm_intercept_pause(&mut self) -> Result<()> {
        // TODO: Implement full SvmInterceptPAUSE when SVM exit machinery is ported.
        // Bochs checks:
        //   1. SVM_INTERCEPT(SVM_INTERCEPT0_PAUSE) first
        //   2. If pause_filter extension: decrement pause_filter_count, return if >0
        //   3. Otherwise: Svm_Vmexit(SVM_VMEXIT_PAUSE)
        Ok(())
    }
}
