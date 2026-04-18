#![allow(dead_code)]

//! FRED (Flexible Return and Event Delivery) implementation.
//!
//! Mirrors Bochs cpu/fred.cc — FRED event delivery, ERETS/ERETU return
//! instructions, and LKGS helper.

use crate::cpu::{BxCpuC, BxCpuIdTrait};

use super::cpu::Exception;
use super::decoder::{BxSegregs, Instruction};
use super::eflags::EFlags;
use super::exception::InterruptType;
use super::segment_ctrl_pro::parse_selector;
use super::Result;

/// Selector RPL mask: clears RPL bits.
const BX_SELECTOR_RPL_MASK: u64 = 0xFFFC;

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // CSL (Current Stack Level) — low 2 bits of ia32_fred_cfg
    // ========================================================================

    #[inline]
    fn csl(&self) -> u32 {
        (self.msr.ia32_fred_cfg & 0x3) as u32
    }

    #[inline]
    fn set_csl(&mut self, new_csl: u32) {
        self.msr.ia32_fred_cfg = (self.msr.ia32_fred_cfg & !0x3) | (new_csl as u64 & 0x3);
    }

    // ========================================================================
    // Internal swapgs — no instruction check, no CPL/mode validation.
    // Used by FRED event delivery and ERETU which manage CPL transitions
    // themselves.
    // ========================================================================

    fn swapgs_internal(&mut self) {
        let gs_base = self.get_segment_base(BxSegregs::Gs);
        let kernel_gs = self.msr.kernelgsbase;
        self.set_segment_base(BxSegregs::Gs, kernel_gs);
        self.msr.kernelgsbase = gs_base;
    }

    // ========================================================================
    // FRED Event Info / Data
    // ========================================================================

    /// Get FRED event data based on vector and interrupt type.
    /// Mirrors Bochs get_fred_event_data().
    fn get_fred_event_data(&self, vector: u8, int_type: InterruptType) -> u64 {
        if matches!(int_type, InterruptType::HardwareException) {
            if vector == Exception::Pf as u8 {
                return self.cr2;
            }
            if vector == Exception::Nm as u8 {
                return 0; // until MSR_XFD_ERR is implemented
            }
        }

        if vector == Exception::Db as u8 {
            return (self.debug_trap & 0x0000400f) as u64;
        }

        if matches!(int_type, InterruptType::Nmi) {
            return 0; // until NMI source reporting is implemented
        }

        0
    }

    /// Get FRED event info word.
    /// Mirrors Bochs get_fred_event_info().
    ///
    /// Bits: vector[7:0], type[19:16], long_mode[25], nested[26], ilen[31:28]
    fn get_fred_event_info(
        &self,
        vector: u8,
        int_type: InterruptType,
        nested_exception: bool,
        ilen: u16,
    ) -> u32 {
        let mut event_info = vector as u32 | ((int_type as u32) << 16);

        if self.long64_mode() {
            event_info |= 1 << 25;
        }

        if vector != Exception::Df as u8 {
            if nested_exception {
                event_info |= 1 << 26;
            }
        }

        // ilen in bits [31:28] for INTn, INT1, INT3/INTO, SYSCALL, SYSENTER
        event_info |= (ilen as u32) << 28;

        event_info
    }

    /// Set FRED event info and data fields before interrupt/exception delivery.
    /// Mirrors Bochs set_fred_event_info_and_data().
    pub(super) fn set_fred_event_info_and_data(
        &mut self,
        vector: u8,
        int_type: InterruptType,
        nested_exception: bool,
        ilen: u16,
    ) {
        self.fred_event_info = self.get_fred_event_info(vector, int_type, nested_exception, ilen);
        self.fred_event_data = self.get_fred_event_data(vector, int_type);
    }

    /// Returns current fred_event_info.
    #[inline]
    pub(super) fn get_current_fred_event_info(&self) -> u32 {
        self.fred_event_info
    }

    /// Returns current fred_event_data.
    #[inline]
    pub(super) fn get_current_fred_event_data(&self) -> u64 {
        self.fred_event_data
    }

    // ========================================================================
    // FRED Event Delivery
    // Mirrors Bochs FRED_EventDelivery() in fred.cc
    // ========================================================================

    pub(super) fn fred_event_delivery(
        &mut self,
        vector: u8,
        int_type: InterruptType,
        error_code: u16,
    ) -> Result<()> {
        self.in_event = true;

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        let old_csl = if cpl == 3 { 0 } else { self.csl() };

        // Augmented CS: old CS selector | (old_CSL << 16)
        let mut old_cs = self.sregs[BxSegregs::Cs as usize].selector.value as u64;
        old_cs |= (old_csl as u64) << 16;

        // CET: cache shadow stack tracking control in old_CS[18]
        if self.cr4.cet() && self.waiting_for_endbranch(0) {
            old_cs |= 1 << 18;
        }

        // Augmented SS: old SS selector | flags | event_info
        let mut old_ss = self.sregs[BxSegregs::Ss as usize].selector.value as u64;
        if self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS) {
            old_ss |= 1 << 16;
        }
        if matches!(int_type, InterruptType::EventOther | InterruptType::SoftwareInterrupt) {
            old_ss |= 1 << 17;
        } else if matches!(int_type, InterruptType::Nmi) {
            old_ss |= 1 << 18;
        }
        // old_SS[63:32] contains FRED event information
        old_ss |= (self.fred_event_info as u64) << 32;

        let old_rip = self.rip();

        // New RIP from ia32_fred_cfg, page-aligned, +256 if already at CPL 0
        let mut new_rip = self.msr.ia32_fred_cfg & !0xFFF;
        if cpl == 0 {
            new_rip += 256;
        }
        if !self.is_canonical(new_rip) {
            tracing::error!("FRED Event Delivery: non canonical value in IA32_FRED_CONFIG");
            return self.exception(Exception::Gp, 0);
        }

        let nested = (self.fred_event_info & (1 << 26)) != 0;

        // Determine event stack level
        let event_sl = if cpl == 3 && !nested && vector != Exception::Df as u8 {
            0u32
        } else {
            match int_type {
                InterruptType::ExternalInterrupt => {
                    ((self.msr.ia32_fred_cfg >> 9) & 0x3) as u32
                }
                InterruptType::Nmi
                | InterruptType::HardwareException
                | InterruptType::SoftwareException
                | InterruptType::PrivilegedSoftwareInterrupt => {
                    debug_assert!((vector as u32) < 32);
                    ((self.msr.ia32_fred_stack_levels >> (vector as u32 * 2)) & 0x3) as u32
                }
                InterruptType::SoftwareInterrupt | InterruptType::EventOther => 0,
            }
        };
        let new_csl = event_sl.max(old_csl);

        // Determine new RSP
        let old_rsp = self.rsp();
        let new_rsp = if cpl == 3 || new_csl > old_csl {
            self.msr.ia32_fred_rsp[new_csl as usize]
        } else {
            // Decrement RSP by configurable amount and align to 64 bytes
            (old_rsp.wrapping_sub(self.msr.ia32_fred_cfg & 0x1C0)) & !0x3F
        };

        let old_eflags = self.eflags.bits();

        // CET shadow stack: compute old/new SSP before frame push
        let old_ssp = self.ssp();
        let mut new_ssp: u64 = 0;
        if self.shadow_stack_enabled(0) {
            if cpl == 3 || new_csl > old_csl {
                // FRED transitions use IA32_PL0_SSP MSR as IA32_FRED_SSP0
                if new_csl == 0 {
                    new_ssp = self.msr.ia32_pl_ssp[0];
                } else {
                    new_ssp = self.msr.ia32_fred_ssp[new_csl as usize];
                }
                if new_ssp & 0x4 != 0 {
                    tracing::error!("FRED Event Delivery: Shadow Stack not 8-byte aligned");
                    return self.exception(Exception::Gp, 0);
                }
            } else {
                new_ssp = self.ssp().wrapping_sub(self.msr.ia32_fred_cfg & 0x8);
            }
        }

        // ESTABLISH NEW CONTEXT — save state on new regular stack (supervisor privilege)
        self.write_new_stack_qword_64(new_rsp.wrapping_sub(8), 0, 0)?; // first 8 bytes are zeros
        self.write_new_stack_qword_64(new_rsp.wrapping_sub(16), 0, self.fred_event_data)?;
        self.write_new_stack_qword_64(new_rsp.wrapping_sub(24), 0, old_ss)?;
        self.write_new_stack_qword_64(new_rsp.wrapping_sub(32), 0, old_rsp)?;
        self.write_new_stack_qword_64(new_rsp.wrapping_sub(40), 0, old_eflags as u64)?;
        self.write_new_stack_qword_64(new_rsp.wrapping_sub(48), 0, old_cs)?;
        self.write_new_stack_qword_64(new_rsp.wrapping_sub(56), 0, old_rip)?;
        self.write_new_stack_qword_64(new_rsp.wrapping_sub(64), 0, error_code as u64)?;

        // CET: write shadow stack frame after regular stack frame
        if self.shadow_stack_enabled(0) {
            if cpl == 0 {
                // Store 4 bytes of zeros to new_SSP-4
                self.shadow_stack_write_dword(new_ssp.wrapping_sub(4), 0, 0)?;
                new_ssp &= !0x7;
                self.shadow_stack_write_qword(new_ssp.wrapping_sub(8), 0, old_cs)?;
                self.shadow_stack_write_qword(new_ssp.wrapping_sub(16), 0, old_rip)?;
                self.shadow_stack_write_qword(new_ssp.wrapping_sub(24), 0, old_ssp)?;
                new_ssp = new_ssp.wrapping_sub(24);
            }
            self.set_ssp(new_ssp);
        }

        // Update segment registers if event occurred in ring 3
        if cpl == 3 {
            // CS: use STAR MSR selector, flat, 64-bit, DPL=0
            let cs_sel = ((self.msr.star >> 32) & BX_SELECTOR_RPL_MASK) as u16;
            parse_selector(cs_sel, &mut self.sregs[BxSegregs::Cs as usize].selector);
            self.setup_flat_cs(0, true);

            // SS: STAR+8 selector, flat, DPL=0
            let ss_sel = (((self.msr.star >> 32) + 8) & BX_SELECTOR_RPL_MASK) as u16;
            parse_selector(ss_sel, &mut self.sregs[BxSegregs::Ss as usize].selector);
            self.setup_flat_ss(0);

            self.swapgs_internal();
        }

        // Update registers defining context
        self.set_rip(new_rip);
        self.eflags = EFlags::from_bits_retain(0x2); // Clear EFLAGS, bit 1 always set
        self.eflags.remove(EFlags::OSZAPC);
        self.set_rsp(new_rsp.wrapping_sub(64));
        self.set_csl(new_csl);

        // CET: save ring-3 SSP, reset endbranch tracker
        if self.cr4.cet() {
            if self.shadow_stack_enabled(3) && cpl == 3 {
                self.msr.ia32_pl_ssp[3] = super::cet::canonicalize_address(self.msr.ia32_pl_ssp[3]);
            }

            self.reset_endbranch_tracker(0, false);
        }

        // NMI masking
        if matches!(int_type, InterruptType::Nmi) {
            self.mask_event(Self::BX_EVENT_NMI);
        }

        // Final cleanup
        self.sregs[BxSegregs::Cs as usize].selector.rpl = 0; // CPL = 0
        self.user_pl = false;
        self.debug_trap = 0;
        self.fred_event_info = 0;
        self.fred_event_data = 0;

        self.in_event = false;

        self.invalidate_prefetch_q();
        Ok(())
    }

    // ========================================================================
    // ERETS — Return from FRED event to ring 0
    // Mirrors Bochs ERETS() in fred.cc
    // ========================================================================

    pub(super) fn erets(&mut self, _instr: &Instruction) -> Result<()> {
        if !self.cr4.fred() {
            tracing::error!("ERETS: FRED is not enabled in CR4");
            return self.exception(Exception::Ud, 0);
        }

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl > 0 {
            tracing::error!("ERETS: CPL must be 0");
            return self.exception(Exception::Ud, 0);
        }

        // RSP_SPECULATIVE
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp();

        // Skip error code
        self.set_rsp(self.rsp().wrapping_add(8));

        let new_rip = self.pop_64()?;
        let temp_cs = self.pop_64()?;
        let new_rflags = self.pop_64()?;
        let new_rsp = self.pop_64()?;
        let temp_ss = self.pop_64()?;

        // Validate return state
        if !self.is_canonical(new_rip)
            || (temp_cs & 0xFFFF_FFFF_FFF8_FFFF)
                != self.sregs[BxSegregs::Cs as usize].selector.value as u64
            || (new_rflags & 0xFFFF_FFFF_FFC2_802A) != 2
            || (temp_ss & 0xFFF8_FFFF)
                != self.sregs[BxSegregs::Ss as usize].selector.value as u64
        {
            tracing::error!("ERETS: corrupted old state #GP(0)");
            return self.exception(Exception::Gp, 0);
        }

        // ERETS will not numerically increase stack level
        let new_csl = ((temp_cs >> 16) & 0x3) as u32;
        let new_csl = new_csl.min(self.csl());

        // CET shadow stack restore
        if self.shadow_stack_enabled(0) {
            // In FRED, shadow_stack_restore uses temp_cs as CS and new_rip as LIP
            let new_ssp = self.shadow_stack_restore_lip(temp_cs as u16, new_rip)?;
            if !self.is_canonical(new_ssp) {
                tracing::error!("ERETS: new SSP not canonical");
                return self.exception(Exception::Gp, 0);
            }
            if new_csl < self.csl() && self.msr.ia32_fred_ssp[self.csl() as usize] != self.ssp() {
                tracing::error!("ERETS changing stack level: SSP mismatch");
                return self.exception(Exception::Cp, super::cet::BX_CP_FAR_RET_IRET);
            }
            self.set_ssp(new_ssp);
        }

        // CET: IBT restore from saved tracking state in old_CS[18]
        if self.endbranch_enabled_and_not_suppressed(0) {
            let ibt_restore = (temp_cs >> 18) & 0x1 != 0;
            if ibt_restore {
                self.track_indirect(0);
            }
        }

        // RSP_COMMIT
        self.speculative_rsp = false;

        self.set_rip(new_rip);
        self.set_eflags_internal(new_rflags as u32);
        self.set_rsp(new_rsp);
        self.set_csl(new_csl);

        // Update event-related state
        let sti_block = (temp_ss >> 16) & 0x1 != 0;
        if sti_block && self.get_if() != 0 {
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS);
        }

        let pending_db = (temp_ss >> 17) & 0x1 != 0;
        if pending_db && self.eflags.contains(EFlags::TF) {
            self.debug_trap |= Self::BX_DEBUG_SINGLE_STEP_BIT;
            self.async_event = 1;
        }

        let nmi_unblock = (temp_ss >> 18) & 0x1 != 0;
        if nmi_unblock {
            self.unmask_event(Self::BX_EVENT_NMI);
        }

        Ok(())
    }

    // ========================================================================
    // ERETU — Return from FRED event to ring 3
    // Mirrors Bochs ERETU() in fred.cc
    // ========================================================================

    pub(super) fn eretu(&mut self, _instr: &Instruction) -> Result<()> {
        if !self.cr4.fred() {
            tracing::error!("ERETU: FRED is not enabled in CR4");
            return self.exception(Exception::Ud, 0);
        }

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl > 0 {
            tracing::error!("ERETU: CPL must be 0");
            return self.exception(Exception::Ud, 0);
        }

        if self.csl() > 0 {
            tracing::error!("ERETU: CSL must be 0");
            return self.exception(Exception::Gp, 0);
        }

        // RSP_SPECULATIVE
        self.speculative_rsp = true;
        self.prev_rsp = self.rsp();

        // Skip error code
        self.set_rsp(self.rsp().wrapping_add(8));

        let mut new_rip = self.pop_64()?;
        let temp_cs = self.pop_64()?;
        let new_rflags = self.pop_64()?;
        let mut new_rsp = self.pop_64()?;
        let temp_ss = self.pop_64()?;

        // Validate return state for ring 3
        if !self.is_canonical(new_rip)
            || (temp_cs & 0xFFFF_FFFF_FFFF_0003) != 3
            || (new_rflags & 0xFFFF_FFFF_FFC2_B02A) != 2
            || (temp_ss & 0xFFF8_0003) != 3
        {
            tracing::error!("ERETU: corrupted old state #GP(0)");
            return self.exception(Exception::Gp, 0);
        }

        let star_base = self.msr.star >> 48;
        let raw_cs_selector = (temp_cs & 0xFFFF) as u16;
        let raw_ss_selector = (temp_ss & 0xFFFF) as u16;

        let to_long_mode;
        let flat;

        if ((temp_cs & 0x7FFF) == star_base + 16) && ((temp_ss & 0x7FFF) == star_base + 8) {
            // Return to CPL3 in standard 64-bit configuration
            to_long_mode = true;
            flat = true;
        } else if ((temp_cs & 0x7FFF) == star_base) && ((temp_ss & 0x7FFF) == star_base + 8) {
            // Return to CPL3 in standard compatibility mode configuration
            to_long_mode = false;
            flat = true;
        } else {
            flat = false;

            // Load CS descriptor from GDT/LDT
            let mut cs_selector = super::descriptor::BxSelector::default();
            parse_selector(raw_cs_selector, &mut cs_selector);

            if (raw_cs_selector & 0xFFFC) == 0 {
                tracing::error!("ERETU: return CS selector null");
                return self.exception(Exception::Gp, 0);
            }

            let (dword1, dword2) = self.fetch_raw_descriptor(&cs_selector)?;
            let cs_descriptor = self.parse_descriptor(dword1, dword2)?;

            if cs_selector.rpl < cpl {
                tracing::error!("ERETU: return selector RPL < CPL");
                return self.exception(Exception::Gp, (raw_cs_selector & 0xFFFC) as u16);
            }

            self.check_cs(&cs_descriptor, raw_cs_selector, 0, cs_selector.rpl)?;

            let mut ss_selector = super::descriptor::BxSelector::default();
            parse_selector(raw_ss_selector, &mut ss_selector);

            // Load SS descriptor — full fetch_ss_descriptor not available yet,
            // use load_ss which performs equivalent validation
            let (sd1, sd2) = self.fetch_raw_descriptor(&ss_selector)?;
            let mut ss_descriptor = self.parse_descriptor(sd1, sd2)?;

            to_long_mode = cs_descriptor.u.segment_l();

            if to_long_mode {
                if !self.is_canonical(new_rip) {
                    tracing::error!("ERETU: new RIP not canonical");
                    return self.exception(Exception::Gp, 0);
                }
            } else {
                new_rip &= 0xFFFF_FFFF;
                new_rsp &= 0xFFFF_FFFF;

                if new_rip > cs_descriptor.u.segment_limit_scaled() as u64 {
                    tracing::error!("ERETU: RIP > limit");
                    return self.exception(Exception::Gp, 0);
                }
            }

            // CET shadow stack checks for ERETU
            if self.shadow_stack_enabled(3) {
                if !to_long_mode && (self.msr.ia32_pl_ssp[3] >> 32) != 0 {
                    tracing::error!("ERETU: attempt to return to compatibility mode while MSR_IA32_PL3_SSP[63:32] != 0");
                    return self.exception(Exception::Gp, 0);
                }
                self.set_ssp(self.msr.ia32_pl_ssp[3]);
            }
            if self.shadow_stack_enabled(0) && self.msr.ia32_pl_ssp[0] != self.ssp() {
                tracing::error!("ERETU: supervisor shadow stack SSP mismatch");
                return self.exception(Exception::Cp, super::cet::BX_CP_FAR_RET_IRET);
            }

            let dpl = cs_descriptor.dpl;
            let mut cs_desc_mut = cs_descriptor;
            self.load_cs(&mut cs_selector, &mut cs_desc_mut, dpl)?;

            if (raw_ss_selector & 0xFFFC) != 0 {
                self.load_ss(&mut ss_selector, &mut ss_descriptor, cs_selector.rpl)?;
            } else {
                // 64-bit mode with null SS
                self.load_null_selector(BxSegregs::Ss, raw_ss_selector);
            }

            // RSP_COMMIT
            self.speculative_rsp = false;

            self.set_rip(new_rip);
            self.set_eflags_internal(new_rflags as u32);
            self.set_rsp(new_rsp);
            self.sregs[BxSegregs::Cs as usize].selector.rpl = 3; // CPL = 3
            self.user_pl = true;

            self.swapgs_internal();
            self.monitor.reset_umonitor();

            // Event-related state
            let pending_db = (temp_ss >> 17) & 0x1 != 0;
            if pending_db && self.eflags.contains(EFlags::TF) {
                self.debug_trap |= Self::BX_DEBUG_SINGLE_STEP_BIT;
                self.async_event = 1;
            }

            let nmi_unblock = (temp_ss >> 18) & 0x1 != 0;
            if nmi_unblock {
                self.unmask_event(Self::BX_EVENT_NMI);
            }

            return Ok(());
        }

        // Flat path (standard STAR-based selectors)
        if flat {
            parse_selector(
                (temp_cs & 0x7FFF) as u16,
                &mut self.sregs[BxSegregs::Cs as usize].selector,
            );
            parse_selector(
                (temp_ss & 0x7FFF) as u16,
                &mut self.sregs[BxSegregs::Ss as usize].selector,
            );

            self.setup_flat_cs(3, to_long_mode);
            self.setup_flat_ss(3);
        }

        // CET shadow stack checks for ERETU (flat path)
        if self.shadow_stack_enabled(3) {
            if !to_long_mode && (self.msr.ia32_pl_ssp[3] >> 32) != 0 {
                tracing::error!("ERETU: attempt to return to compatibility mode while MSR_IA32_PL3_SSP[63:32] != 0");
                return self.exception(Exception::Gp, 0);
            }
            self.set_ssp(self.msr.ia32_pl_ssp[3]);
        }
        if self.shadow_stack_enabled(0) && self.msr.ia32_pl_ssp[0] != self.ssp() {
            tracing::error!("ERETU: supervisor shadow stack SSP mismatch");
            return self.exception(Exception::Cp, super::cet::BX_CP_FAR_RET_IRET);
        }

        // RSP_COMMIT
        self.speculative_rsp = false;

        self.set_rip(new_rip);
        self.set_eflags_internal(new_rflags as u32);
        self.set_rsp(new_rsp);
        self.sregs[BxSegregs::Cs as usize].selector.rpl = 3; // CPL = 3
        self.user_pl = true;

        self.swapgs_internal();
        self.monitor.reset_umonitor();

        // Event-related state
        let pending_db = (temp_ss >> 17) & 0x1 != 0;
        if pending_db && self.eflags.contains(EFlags::TF) {
            self.debug_trap |= Self::BX_DEBUG_SINGLE_STEP_BIT;
            self.async_event = 1;
        }

        let nmi_unblock = (temp_ss >> 18) & 0x1 != 0;
        if nmi_unblock {
            self.unmask_event(Self::BX_EVENT_NMI);
        }

        Ok(())
    }

    // ========================================================================
    // LKGS — Load Kernel GS Base
    // Mirrors Bochs LKGS_Ew() in fred.cc
    // ========================================================================

    pub(super) fn lkgs_ew(&mut self, instr: &Instruction) -> Result<()> {
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl > 0 {
            tracing::error!("LKGS_Ew: CPL must be 0");
            return self.exception(Exception::Ud, 0);
        }

        let segsel = if instr.mod_c0() {
            self.gen_reg[instr.dst() as usize].rrx() as u16
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            let laddr = self.get_laddr64(seg as usize, eaddr);
            self.read_linear_word(seg, laddr)?
        };

        // Back up current GS segment base into MSR_KERNEL_GS_BASE
        self.swapgs_internal();

        self.load_seg_reg(BxSegregs::Gs, segsel)?;

        // Restore old GS segment base and put new loaded base into MSR_KERNEL_GS_BASE
        self.swapgs_internal();

        Ok(())
    }
}
