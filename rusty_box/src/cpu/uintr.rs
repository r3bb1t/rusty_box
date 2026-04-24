//! Intel User Interrupts (UINTR) — Bochs cpu/uintr.cc.
//!
//! Scope of this file: state helpers (uintr_masked / uintr_uirr_update /
//! uintr_control) and XSAVE component save/restore for XCR0 bit 14 (UINTR).
//! UINTR instruction handlers (UIRET, STUI, CLUI, TESTUI, SENDUIPI_Eq) and
//! the UINTR delivery path (deliver_UINTR, Process_UINTR_Notification,
//! send_uipi) are deliberately not in this file — they live in their own
//! dedicated UINTR session alongside the LAPIC IPI and VMX intercept wiring
//! they depend on.

use super::cpu::Exception;
use super::decoder::BxSegregs;
use super::instrumentation::Instrumentation;
use super::{BxCpuC, BxCpuIdTrait, Result};

impl<I: BxCpuIdTrait, T: Instrumentation> BxCpuC<'_, I, T> {
    /// Bochs uintr.cc uintr_masked — the user-level interrupt can be delivered
    /// only when running long-64 mode + UIF=1 + CPL=3.
    #[inline]
    pub(super) fn uintr_masked(&self) -> bool {
        !self.long64_mode() || !self.uintr.uif || self.cs_rpl() != 3
    }

    /// Bochs uintr.cc uintr_uirr_update — signal or clear the pending-UINTR
    /// event according to CR4.UINTR and UIRR state.
    pub(super) fn uintr_uirr_update(&mut self) {
        if self.cr4.uintr() && self.uintr.uirr != 0 {
            self.signal_event(Self::BX_EVENT_PENDING_UINTR);
        } else {
            self.clear_event(Self::BX_EVENT_PENDING_UINTR);
        }
    }

    /// Bochs uintr.cc uintr_control — mask or unmask BX_EVENT_PENDING_UINTR
    /// according to the conditions that allow delivery.
    pub(super) fn uintr_control(&mut self) {
        if self.uintr_masked() {
            self.mask_event(Self::BX_EVENT_PENDING_UINTR);
        } else {
            self.unmask_event(Self::BX_EVENT_PENDING_UINTR);
        }
    }

    // =========================================================================
    // UINTR XSAVE state (XCR0 bit 14) — Bochs xsave.cc xsave_uintr_state
    // =========================================================================

    /// Bochs xsave.cc xsave_uintr_state — serialises the UINTR state block
    /// (48 bytes: handler/stack-adjust/misc/PD/RR/TT). Note Bochs zeroes
    /// uintr.uinv after save, preserving the XRSTOR-should-reinit semantic.
    pub(super) fn xsave_uintr_state(
        &mut self,
        seg: BxSegregs,
        base: u64,
    ) -> Result<()> {
        self.v_write_qword(seg, base, self.uintr.ui_handler)?;
        self.v_write_qword(seg, base.wrapping_add(8), self.uintr.stack_adjust)?;
        let uif_bit = if self.uintr.uif { 1u64 << 63 } else { 0 };
        let misc = ((self.uintr.uinv as u64) << 32) | (self.uintr.uitt_size as u64) | uif_bit;
        self.v_write_qword(seg, base.wrapping_add(16), misc)?;
        self.v_write_qword(seg, base.wrapping_add(24), self.uintr.upid_addr)?;
        self.v_write_qword(seg, base.wrapping_add(32), self.uintr.uirr)?;
        self.v_write_qword(seg, base.wrapping_add(40), self.uintr.uitt_addr)?;
        // Bochs clears uinv at end of save so XRSTOR's "uinv must be 0 on
        // restore" guard holds on reload. See xrstor_uintr_state below.
        self.uintr.uinv = 0;
        Ok(())
    }

    /// Bochs xsave.cc xrstor_uintr_state — restores the UINTR state block,
    /// validating every field with the same canonical/reserved-bit checks
    /// that WRMSR on the corresponding IA32_UINTR_* MSR would perform.
    pub(super) fn xrstor_uintr_state(
        &mut self,
        seg: BxSegregs,
        base: u64,
    ) -> Result<()> {
        // Bochs uintr.uinv must be zero on restore; else #GP(0).
        if self.uintr.uinv != 0 {
            tracing::trace!("XRSTOR UINTR: uinv is set, #GP(0)");
            return self.exception(Exception::Gp, 0);
        }
        // Reset uinv immediately so a mid-restore fault leaves uinv=0.
        self.uintr.uinv = 0;

        let ui_handler = self.v_read_qword(seg, base)?;
        let stack_adjust = self.v_read_qword(seg, base.wrapping_add(8))?;
        let mut misc = self.v_read_qword(seg, base.wrapping_add(16))?;
        let uif = (misc >> 63) != 0;
        misc &= !(1u64 << 63);
        let upid_addr = self.v_read_qword(seg, base.wrapping_add(24))?;
        let uirr = self.v_read_qword(seg, base.wrapping_add(32))?;
        let uitt_addr = self.v_read_qword(seg, base.wrapping_add(40))?;

        // Mirror WRMSR semantics — Bochs' xrstor_uintr_state calls wrmsr() for
        // each field. We replicate the validation inline to keep this path
        // independent of the WRMSR dispatch table.
        if !self.is_canonical(ui_handler)
            || !self.is_canonical(stack_adjust)
            || !self.is_canonical(upid_addr)
            || !self.is_canonical(uitt_addr)
        {
            return self.exception(Exception::Gp, 0);
        }
        if (upid_addr & 0x3F) != 0 {
            return self.exception(Exception::Gp, 0);
        }
        if (uitt_addr & 0x0E) != 0 {
            return self.exception(Exception::Gp, 0);
        }
        if misc & 0xffffff0000000000u64 != 0 {
            return self.exception(Exception::Gp, 0);
        }

        self.uintr.ui_handler = ui_handler;
        self.uintr.stack_adjust = stack_adjust;
        self.uintr.uitt_size = misc as u32;
        self.uintr.uinv = (misc >> 32) as u32;
        self.uintr.upid_addr = upid_addr;
        self.uintr.uirr = uirr;
        self.uintr.uitt_addr = uitt_addr;
        self.uintr.uif = uif;

        self.uintr_uirr_update();
        self.uintr_control();
        Ok(())
    }

    /// Bochs xsave.cc xrstor_init_uintr_state — zero the entire UINTR block.
    pub(super) fn xrstor_init_uintr_state(&mut self) {
        self.uintr.ui_handler = 0;
        self.uintr.stack_adjust = 0;
        self.uintr.uitt_size = 0;
        self.uintr.uinv = 0;
        self.uintr.uif = false;
        self.uintr.upid_addr = 0;
        self.uintr.uitt_addr = 0;
        self.uintr.uirr = 0;
    }

    /// Bochs xsave.cc xsave_uintr_state_xinuse — any non-default UINTR field
    /// forces this component to be recorded in xstate_bv.
    #[inline]
    pub(super) fn xsave_uintr_state_xinuse(&self) -> bool {
        self.uintr.ui_handler != 0
            || self.uintr.stack_adjust != 0
            || self.uintr.uitt_size != 0
            || self.uintr.uinv != 0
            || self.uintr.uif
            || self.uintr.upid_addr != 0
            || self.uintr.uitt_addr != 0
            || self.uintr.uirr != 0
    }
}
