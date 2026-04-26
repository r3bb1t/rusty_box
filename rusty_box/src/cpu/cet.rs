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

    /// Push a dword on the shadow stack: SSP -= 4; write at new SSP.
    /// Bochs stack.h shadow_stack_push_32().
    pub(super) fn shadow_stack_push_32(&mut self, value: u32) -> Result<()> {
        let new_ssp = self.ssp().wrapping_sub(4);
        let cpl = self.cs_rpl();
        self.shadow_stack_write_dword(new_ssp, cpl, value)?;
        self.set_ssp(new_ssp);
        Ok(())
    }

    /// Push a qword on the shadow stack: SSP -= 8; write at new SSP.
    /// Bochs stack.h shadow_stack_push_64().
    pub(super) fn shadow_stack_push_64(&mut self, value: u64) -> Result<()> {
        let new_ssp = self.ssp().wrapping_sub(8);
        let cpl = self.cs_rpl();
        self.shadow_stack_write_qword(new_ssp, cpl, value)?;
        self.set_ssp(new_ssp);
        Ok(())
    }

    /// Push (CS, LIP, old_SSP) onto the shadow stack for a far CALL / gate
    /// transition. Mirrors Bochs call_far.cc call_far_shadow_stack_push().
    ///
    /// `cs`      — outgoing CS selector (zero-extended to 64 bits in storage)
    /// `lip`     — linear instruction pointer of the return target
    /// `old_ssp` — SSP value before the gate switch (architectural "prevSSP"
    ///             token written as the third qword)
    ///
    /// When SSP is not 8-byte aligned, an alignment hole dword of zeros is
    /// written at SSP-4 and SSP is rounded down to the next 8-byte boundary
    /// before the three pushes — matching the layout the matching
    /// `shadow_stack_restore_lip` pop sequence expects.
    pub(super) fn call_far_shadow_stack_push(
        &mut self,
        cs: u16,
        lip: u64,
        old_ssp: u64,
    ) -> Result<()> {
        // VMX: mark the shadow stack as transiently busy across the multi-step
        // push so any nested intercept observes the in-progress state.
        if self.in_vmx_guest {
            self.vmcs.shadow_stack_prematurely_busy = true;
        }

        if self.ssp() & 0x7 != 0 {
            let cpl = self.cs_rpl();
            let off = self.ssp().wrapping_sub(4);
            self.shadow_stack_write_dword(off, cpl, 0)?;
            self.set_ssp(self.ssp() & !0x7);
        }

        self.shadow_stack_push_64(cs as u64)?;
        self.shadow_stack_push_64(lip)?;
        self.shadow_stack_push_64(old_ssp)?;

        if self.in_vmx_guest {
            self.vmcs.shadow_stack_prematurely_busy = false;
        }
        Ok(())
    }

 pub(super) fn shadow_stack_switch(&mut self, new_ssp: u64) -> Result<()> {
        // Bochs call_far.cc shadow_stack_switch — install the new SSP, then
        // validate alignment, 64-bit residency, and atomically set the busy
        // bit on the new shadow-stack token. On any failure, raise #GP(0).
        self.set_ssp(new_ssp);
        if new_ssp & 0x7 != 0 {
            tracing::error!("shadow_stack_switch: SSP is not aligned to 8 byte boundary");
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        if !self.long64_mode() && (new_ssp >> 32) != 0 {
            tracing::error!("shadow_stack_switch: 64-bit SSP not in 64-bit mode");
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let cpl = self.cs_rpl();
        if !self.shadow_stack_atomic_set_busy(new_ssp, cpl)? {
            tracing::error!("shadow_stack_switch: failure to set busy bit");
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        Ok(())
    }

    /// True atomic compare-exchange on a shadow-stack token.
    ///
    /// Bochs access2.cc shadow_stack_lock_cmpxchg8b does a sequenced read +
    /// conditional write with a "should be atomic RMW" comment — fine under
    /// Bochs' single-threaded-per-CPU model, but not SMP-safe. Per the
    /// CLAUDE.md thread-safety rule we upgrade to a real lock-free CMPXCHG on
    /// the host memory location.
    ///
    /// Returns Ok(true) when the exchange observed `expected` and installed
    /// `new_val`; Ok(false) when `expected` did not match.
    fn shadow_stack_lock_cmpxchg8b(
        &mut self,
        offset: u64,
        _cpl: u8,
        new_val: u64,
        expected: u64,
    ) -> Result<bool> {
        use core::sync::atomic::{AtomicU64, Ordering};

        // Walk the SS-aware page tables. This raises #PF on non-SS pages /
        // U-S mismatch / missing pages — matching Bochs access_write_linear.
        let paddr = self.translate_shadow_stack_write(offset)?;

        // Prefer the TLB-cached host pointer that translate_shadow_stack_write
        // has just populated; fall back to mem_read/mem_write for MMIO-backed
        // SS pages (architecturally unusual but defensible).
        let lpf = offset & super::tlb::LPF_MASK;
        let host_ptr: Option<*mut u64> = {
            let tlb = self.dtlb.get_entry_of(offset, 7);
            if tlb.lpf == lpf && tlb.host_page_addr != 0 {
                let byte_ptr = super::access::host_at_page_offset_mut(
                    tlb.host_page_addr as *mut u8,
                    offset,
                );
                // SSP is architecturally 8-byte aligned on every caller; the
                // raw byte pointer thus aligns for u64/AtomicU64.
                debug_assert_eq!(offset & 0x7, 0, "SS cmpxchg offset must be 8-byte aligned");
                Some(byte_ptr as *mut u64)
            } else {
                None
            }
        };

        if let Some(ptr) = host_ptr {
            // SAFETY: `ptr` was derived from a TLB entry that translate_*
            // validated as a present, writeable, 8-byte-aligned host-backed
            // shadow-stack page. The memory it points at is shared guest
            // RAM — accesses from device threads / other vCPUs must use
            // atomic operations, and that's exactly what `AtomicU64` does
            // at the same address. Lifetime of the underlying page is tied
            // to the emulator's memory backing, which outlives the CPU loop.
            let atomic = unsafe { &*(ptr as *const AtomicU64) };
            let ok = atomic
                .compare_exchange(expected, new_val, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
            self.i_cache.smc_write_check(paddr, 8);
            Ok(ok)
        } else {
            // MMIO-backed SS page (not a real architectural case). Fall back to
            // the Bochs sequenced RMW — no better option for non-RAM targets.
            let val = self.mem_read_qword(paddr);
            if val == expected {
                self.mem_write_qword(paddr, new_val);
                Ok(true)
            } else {
                self.mem_write_qword(paddr, val);
                Ok(false)
            }
        }
    }

    /// Atomically set the busy bit on a shadow stack token.
    /// Bochs access2.cc shadow_stack_atomic_set_busy.
    pub(super) fn shadow_stack_atomic_set_busy(&mut self, offset: u64, cpl: u8) -> Result<bool> {
        let expected = if self.long64_mode() { offset } else { offset & 0xFFFF_FFFF };
        self.shadow_stack_lock_cmpxchg8b(offset, cpl, offset | 0x1, expected)
    }

    /// Atomically clear the busy bit on a shadow stack token.
    /// Bochs access2.cc shadow_stack_atomic_clear_busy — returns true if the
    /// compare matched (busy bit was set as expected).
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
    /// In non-64-bit mode: resets the endbranch tracker and ends the trace
    /// (Bochs BX_NEXT_INSTR — instruction-level barrier).
    /// In 64-bit mode: NOP that continues in the current trace (BX_NEXT_TRACE).
    /// Bochs cet.cc ENDBRANCH32.
    pub(super) fn endbranch32(
        &mut self,
        _instr: &Instruction,
    ) -> Result<()> {
        if !self.long64_mode() {
            let cpl = self.cs_rpl();
            self.reset_endbranch_tracker(cpl, false);
            self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        }
        Ok(())
    }

    /// ENDBRANCH64 handler.
    /// In 64-bit mode: resets the endbranch tracker and ends the trace
    /// (Bochs BX_NEXT_INSTR).
    /// In non-64-bit mode: NOP that continues in the current trace.
    /// Bochs cet.cc ENDBRANCH64.
    pub(super) fn endbranch64(
        &mut self,
        _instr: &Instruction,
    ) -> Result<()> {
        if self.long64_mode() {
            let cpl = self.cs_rpl();
            self.reset_endbranch_tracker(cpl, false);
            self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        }
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

        // Bochs cet.cc INCSSPD: touches the first and last dword of the
        // increment range to surface #PF / shadow-stack page-type checks
        // (the read values are otherwise discarded).
        let ssp = self.ssp();
        self.shadow_stack_read_dword(ssp, cpl)?;
        self.shadow_stack_read_dword(ssp + (tmpsrc as u64 - 1) * 4, cpl)?;
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

        // Bochs cet.cc INCSSPQ: probe first/last qword for #PF / shadow-stack
        // page-type checks; reads are discarded.
        let ssp = self.ssp();
        self.shadow_stack_read_qword(ssp, cpl)?;
        self.shadow_stack_read_qword(ssp + (tmpsrc as u64 - 1) * 8, cpl)?;
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
    /// Bochs vmexit.cc VMexit_PAUSE() — checks PAUSE Exiting control. PAUSE
    /// Loop Exiting (PLE) timing is not modelled (PLE needs the TSC gap tracker).
    fn vmexit_pause(&mut self) -> Result<()> {
        let _ = self.vmexit_check_pause()?;
        Ok(())
    }
}


// ============================================================================
// CET tests \u2014 exercise the helpers wired into CALL/JMP/exception handlers.
// These do not run real instructions; they configure state, invoke helpers,
// and assert observable side-effects (MSR bits, SSP, shadow-stack memory).
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpu::CpuMode;
    use crate::cpu::cpudb::intel::core_i7_skylake::Corei7SkylakeX;
    use crate::cpu::crregs::BxCr4;
    use crate::cpu::decoder::BxSegregs;
    use crate::memory::{BxMemC, BxMemoryStubC};
    use core::ptr::NonNull;

    /// Build a fresh CPU and switch it into protected mode with CET enabled in CR4.
    /// Caller fills in the IA32_S_CET / IA32_U_CET MSR for the specific test.
    fn make_cet_cpu() -> alloc::boxed::Box<BxCpuC<'static, Corei7SkylakeX>> {
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.cpu_mode = CpuMode::Ia32Protected;
        cpu.cr4 = BxCr4::CET;
        // Default to CPL=0 (kernel) by clearing CS RPL.
        cpu.sregs[BxSegregs::Cs as usize].selector.rpl = 0;
        cpu.msr.ia32_cet_control[0] = 0;
        cpu.msr.ia32_cet_control[1] = 0;
        cpu
    }

    #[test]
    fn shadow_stack_enabled_tracks_cr4_and_msr() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut cpu = make_cet_cpu();

                // Disabled out of the gate \u2014 MSR bit clear.
                assert!(!cpu.shadow_stack_enabled(0));
                assert!(!cpu.shadow_stack_enabled(3));

                // S_CET enables CPL<3 only.
                cpu.msr.ia32_cet_control[0] = CET_SHADOW_STACK_ENABLED;
                assert!(cpu.shadow_stack_enabled(0));
                assert!(!cpu.shadow_stack_enabled(3));

                // U_CET enables CPL=3 only.
                cpu.msr.ia32_cet_control[1] = CET_SHADOW_STACK_ENABLED;
                assert!(cpu.shadow_stack_enabled(0));
                assert!(cpu.shadow_stack_enabled(3));

                // CR4.CET clear \u2014 nothing enabled regardless of MSRs.
                cpu.cr4.remove(BxCr4::CET);
                assert!(!cpu.shadow_stack_enabled(0));
                assert!(!cpu.shadow_stack_enabled(3));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn track_indirect_sets_wait_for_endbranch() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut cpu = make_cet_cpu();
                cpu.msr.ia32_cet_control[0] = CET_ENDBRANCH_ENABLED;
                assert_eq!(
                    cpu.msr.ia32_cet_control[0] & CET_WAIT_FOR_ENBRANCH,
                    0,
                    "WAIT_FOR_ENBRANCH must start cleared"
                );

                // Indirect CALL / JMP would emit this after applying the new target.
                // Pass CS as the segment override (i.e. NO no-track DS prefix).
                cpu.track_indirect_if_not_suppressed(BxSegregs::Cs as u8, 0);

                assert_ne!(
                    cpu.msr.ia32_cet_control[0] & CET_WAIT_FOR_ENBRANCH,
                    0,
                    "track_indirect must arm WAIT_FOR_ENBRANCH"
                );
                assert!(cpu.waiting_for_endbranch(0));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn ds_no_track_prefix_suppresses_track_indirect() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut cpu = make_cet_cpu();
                cpu.msr.ia32_cet_control[0] =
                    CET_ENDBRANCH_ENABLED | CET_ENABLE_NO_TRACK_INDIRECT_BRANCH_PREFIX;

                // DS segment override + NO_TRACK feature \u2192 suppress tracking.
                cpu.track_indirect_if_not_suppressed(BxSegregs::Ds as u8, 0);
                assert_eq!(
                    cpu.msr.ia32_cet_control[0] & CET_WAIT_FOR_ENBRANCH,
                    0,
                    "DS no-track prefix must suppress WAIT_FOR_ENBRANCH"
                );

                // Same instruction without the NO_TRACK feature: must track.
                cpu.msr.ia32_cet_control[0] =
                    CET_ENDBRANCH_ENABLED; // drop NO_TRACK feature
                cpu.track_indirect_if_not_suppressed(BxSegregs::Ds as u8, 0);
                assert_ne!(
                    cpu.msr.ia32_cet_control[0] & CET_WAIT_FOR_ENBRANCH,
                    0,
                    "DS without NO_TRACK feature must not suppress"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn reset_endbranch_tracker_clears_wait_bit() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut cpu = make_cet_cpu();
                cpu.msr.ia32_cet_control[0] =
                    CET_ENDBRANCH_ENABLED | CET_WAIT_FOR_ENBRANCH;

                // ENDBR matched: clear WAIT, do not suppress.
                cpu.reset_endbranch_tracker(0, false);
                assert_eq!(
                    cpu.msr.ia32_cet_control[0] & CET_WAIT_FOR_ENBRANCH,
                    0,
                    "reset_endbranch_tracker must clear WAIT_FOR_ENBRANCH"
                );
                assert_eq!(
                    cpu.msr.ia32_cet_control[0] & CET_SUPPRESS_INDIRECT_BRANCH_TRACKING,
                    0,
                    "suppress=false must leave SUPPRESS clear"
                );

                // ENDBR mismatched + SUPPRESS_DIS clear \u2192 set SUPPRESS bit.
                cpu.msr.ia32_cet_control[0] =
                    CET_ENDBRANCH_ENABLED | CET_WAIT_FOR_ENBRANCH;
                cpu.reset_endbranch_tracker(0, true);
                assert_ne!(
                    cpu.msr.ia32_cet_control[0] & CET_SUPPRESS_INDIRECT_BRANCH_TRACKING,
                    0,
                    "suppress=true with SUPPRESS_DIS clear must arm SUPPRESS"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// End-to-end shadow-stack push/pop round-trip with a real memory backing.
    /// CR0.PG=0 so the shadow-stack page-walk skips translation and writes go
    /// straight to physical memory \u2014 enough to verify SSP movement and the
    /// pushed value land at SSP-8.
    #[test]
    fn shadow_stack_push_pop_round_trip() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut cpu = make_cet_cpu();
                cpu.msr.ia32_cet_control[0] =
                    CET_SHADOW_STACK_ENABLED | CET_SHADOW_STACK_WRITE_ENABLED;

                let mem_stub =
                    BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
                let mut mem = BxMemC::new(mem_stub, false);

                // Wire the bus pointers cpu_loop normally sets up.
                cpu.a20_mask = mem.a20_mask();
                let (mem_vector, mem_len) = mem.get_raw_memory_ptr();
                cpu.mem_ptr = Some(mem_vector);
                cpu.mem_len = mem_len;
                let (host_base, host_len) = mem.get_ram_base_ptr();
                cpu.mem_host_base = host_base;
                cpu.mem_host_len = host_len;
                cpu.set_mem_bus_ptr(NonNull::from(&mut mem));

                // Place SSP somewhere inside the 1 MiB RAM region, 16-byte aligned,
                // away from the BIOS shadow region (0xA0000+) and low IVT.
                const INITIAL_SSP: u64 = 0x4_0000;
                cpu.set_ssp(INITIAL_SSP);

                // ---- Push a 64-bit value: SSP retreats by 8, value lands at new SSP.
                let ret_addr: u64 = 0xCAFE_BABE_DEAD_BEEF;
                cpu.shadow_stack_push_64(ret_addr).unwrap();
                assert_eq!(cpu.ssp(), INITIAL_SSP - 8);
                let popped = cpu.shadow_stack_pop_64().unwrap();
                assert_eq!(popped, ret_addr);
                assert_eq!(cpu.ssp(), INITIAL_SSP, "SSP must return to initial after pop");

                // ---- 32-bit push variant.
                cpu.set_ssp(INITIAL_SSP);
                cpu.shadow_stack_push_32(0xDEADBEEF).unwrap();
                assert_eq!(cpu.ssp(), INITIAL_SSP - 4);
                let popped32 = cpu.shadow_stack_pop_32().unwrap();
                assert_eq!(popped32, 0xDEADBEEF);
                assert_eq!(cpu.ssp(), INITIAL_SSP);

                // ---- Far-CALL push triplet: alignment hole + cs + lip + old_ssp.
                cpu.set_ssp(INITIAL_SSP);
                let old_ssp = cpu.ssp();
                cpu.call_far_shadow_stack_push(0x0008, 0x1234_5678, old_ssp).unwrap();
                // After three pushes of 8 bytes each (no alignment hole because
                // INITIAL_SSP is 8-byte aligned), SSP retreated by 24.
                assert_eq!(cpu.ssp(), INITIAL_SSP - 24, "three qword pushes -> SSP-=24");

                // Pop them back in reverse order matching shadow_stack_restore_lip.
                let prev_ssp_token = cpu.shadow_stack_pop_64().unwrap();
                assert_eq!(prev_ssp_token, old_ssp, "first pop yields old SSP");
                let lip_token = cpu.shadow_stack_pop_64().unwrap();
                assert_eq!(lip_token, 0x1234_5678, "second pop yields LIP");
                let cs_token = cpu.shadow_stack_pop_64().unwrap();
                assert_eq!(cs_token, 0x0008, "third pop yields CS");
                assert_eq!(cpu.ssp(), INITIAL_SSP, "SSP back to start after three pops");
            })
            .unwrap()
            .join()
            .unwrap();
    }
}