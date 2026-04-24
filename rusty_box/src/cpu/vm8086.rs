//! Virtual 8086 Mode support
//!
//! Based on Bochs cpu/vm8086.cc
//!
//! Implements V8086 mode transitions:
//! - IRET from protected mode to V8086 mode (stack_return_to_v86)
//! - IRET within V8086 mode (iret16/32_stack_return_from_v86)
//! - VME interrupt redirection (v86_redirect_interrupt)
//! - V8086 segment cache initialization (init_v8086_mode)

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::BxSegregs,
    descriptor::{SEG_ACCESS_ROK, SEG_ACCESS_WOK, SEG_VALID_CACHE},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Return from protected mode (CPL=0) to V8086 mode via IRET.
    ///
    /// Bochs: BX_CPU_C::stack_return_to_v86() in vm8086.cc
    ///
    /// Called from iret_protected() when the saved EFLAGS has VM bit set and CPL==0.
    /// Pops SS:ESP, ES, DS, FS, GS from the stack, writes EFLAGS (including VM),
    /// loads CS:EIP, then calls init_v8086_mode() to set up all segment caches.
    pub(super) fn stack_return_to_v86(
        &mut self,
        new_eip: u32,
        raw_cs_selector: u32,
        flags32: u32,
    ) -> super::Result<()> {
        // Must be 32-bit effective opsize, VM is set in upper 16 bits of EFLAGS
        // and CPL == 0 to get here (Bochs vm8086.cc)
        debug_assert_eq!(
            self.sregs[BxSegregs::Cs as usize].selector.rpl,
            0,
            "stack_return_to_v86: CPL must be 0"
        );
        debug_assert!(
            self.protected_mode(),
            "stack_return_to_v86: must be in protected mode"
        );

        // ── Stack layout (Bochs vm8086.cc) ──
        // eSP+32: OLD GS
        // eSP+28: OLD FS
        // eSP+24: OLD DS
        // eSP+20: OLD ES
        // eSP+16: OLD SS
        // eSP+12: OLD ESP
        // eSP+8:  OLD EFLAGS  (already read by caller, passed as flags32)
        // eSP+4:  OLD CS      (already read by caller, passed as raw_cs_selector)
        // eSP+0:  OLD EIP     (already read by caller, passed as new_eip)

        let temp_esp = if self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b() {
            self.esp()
        } else {
            self.sp() as u32
        };

        // Load SS:ESP from stack (vm8086.cc)
        let new_esp = self.stack_read_dword(temp_esp.wrapping_add(12))?;
        let raw_ss_selector = self.stack_read_dword(temp_esp.wrapping_add(16))? as u16;

        // Load ES, DS, FS, GS from stack (vm8086.cc)
        let raw_es_selector = self.stack_read_dword(temp_esp.wrapping_add(20))? as u16;
        let raw_ds_selector = self.stack_read_dword(temp_esp.wrapping_add(24))? as u16;
        let raw_fs_selector = self.stack_read_dword(temp_esp.wrapping_add(28))? as u16;
        let raw_gs_selector = self.stack_read_dword(temp_esp.wrapping_add(32))? as u16;

        // Write EFLAGS with valid mask (vm8086.cc)
        // This sets the VM bit, transitioning to V8086 mode
        self.write_eflags(flags32, EFlags::VALID_MASK.bits());

        // Load CS:EIP (vm8086.cc)
        self.sregs[BxSegregs::Cs as usize].selector.value = raw_cs_selector as u16;
        self.set_eip(new_eip & 0xFFFF); // EIP masked to 16-bit (vm8086.cc)

        // Load remaining segment selectors (vm8086.cc)
        self.sregs[BxSegregs::Es as usize].selector.value = raw_es_selector;
        self.sregs[BxSegregs::Ds as usize].selector.value = raw_ds_selector;
        self.sregs[BxSegregs::Fs as usize].selector.value = raw_fs_selector;
        self.sregs[BxSegregs::Gs as usize].selector.value = raw_gs_selector;
        self.sregs[BxSegregs::Ss as usize].selector.value = raw_ss_selector;
        self.set_esp(new_esp); // Full 32-bit ESP loaded (vm8086.cc)

        // Initialize V8086 segment caches (vm8086.cc)
        self.init_v8086_mode();

        tracing::trace!(
            "stack_return_to_v86: CS:EIP={:04x}:{:04x} SS:ESP={:04x}:{:08x} EFLAGS={:08x}",
            raw_cs_selector as u16,
            new_eip & 0xFFFF,
            raw_ss_selector,
            new_esp,
            flags32
        );

        Ok(())
    }

    /// 16-bit IRET while already in V8086 mode.
    ///
    /// Bochs: BX_CPU_C::iret16_stack_return_from_v86() in vm8086.cc
    ///
    /// Called from iret16() when cpu_mode == Ia32V8086.
    pub(super) fn iret16_stack_return_from_v86(&mut self) -> super::Result<()> {
        let iopl = self.eflags.iopl();

        // IOPL < 3 without VME → trap to V86 monitor (vm8086.cc)
        if iopl < 3 && !self.cr4.vme() {
            tracing::trace!("IRET16 in V86 with IOPL != 3, VME = 0");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let ip = self.pop_16()?;
        let cs_raw = self.pop_16()?;
        let flags16 = self.pop_16()?;

        // VME path: IOPL < 3 but CR4.VME enabled (vm8086.cc)
        if self.cr4.vme() && iopl < 3 {
            // Check VIP+IF or TF → #GP(0) (vm8086.cc)
            if ((flags16 as u32 & EFlags::IF_.bits()) != 0 && self.eflags.contains(EFlags::VIP))
                || (flags16 as u32 & EFlags::TF.bits()) != 0
            {
                tracing::trace!("iret16_stack_return_from_v86(): #GP(0) in VME mode");
                return self.exception(super::cpu::Exception::Gp, 0);
            }

            // Load CS:IP (vm8086.cc)
            self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
            self.set_eip(ip as u32);

            // IF, IOPL unchanged; EFLAGS.VIF = TMP_FLAGS.IF (vm8086.cc)
            let change_mask = EFlags::OSZAPC
                .union(EFlags::TF)
                .union(EFlags::DF)
                .union(EFlags::NT)
                .union(EFlags::VIF);

            let mut flags32 = flags16 as u32;
            if (flags16 as u32 & EFlags::IF_.bits()) != 0 {
                flags32 |= EFlags::VIF.bits();
            }
            self.write_eflags(flags32, change_mask.bits());

            return Ok(());
        }

        // Non-VME path: IOPL == 3 (vm8086.cc)
        self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw);
        self.set_eip(ip as u32);
        // write_flags with change_IOPL=false, change_IF=true (vm8086.cc)
        self.write_flags(flags16, false, true);

        Ok(())
    }

    /// 32-bit IRET while already in V8086 mode.
    ///
    /// Bochs: BX_CPU_C::iret32_stack_return_from_v86() in vm8086.cc
    ///
    /// Called from iret32() when cpu_mode == Ia32V8086.
    pub(super) fn iret32_stack_return_from_v86(&mut self) -> super::Result<()> {
        // IOPL must be 3, else trap to V86 monitor (vm8086.cc)
        if self.eflags.iopl() < 3 {
            tracing::trace!("IRET32 in V86 with IOPL != 3");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Build change mask (vm8086.cc)
        // ID, VIP, VIF, AC, VM, RF, x, NT, IOPL, OF, DF, IF, TF, SF, ZF, x, AF, x, PF, x, CF
        let change_mask = EFlags::OSZAPC
            .union(EFlags::TF)
            .union(EFlags::IF_)
            .union(EFlags::DF)
            .union(EFlags::NT)
            .union(EFlags::RF)
            .union(EFlags::ID)
            .union(EFlags::AC);
        // VIF, VIP, VM, IOPL unchanged

        let eip = self.pop_32()?;
        let cs_raw = self.pop_32()?;
        let flags32 = self.pop_32()?;

        self.load_seg_reg_real_mode(BxSegregs::Cs, cs_raw as u16);
        self.set_eip(eip);
        self.write_eflags(flags32, change_mask.bits());

        Ok(())
    }

    /// VME interrupt redirection check.
    ///
    /// Bochs: BX_CPU_C::v86_redirect_interrupt() in vm8086.cc
    ///
    /// Called from interrupt() for software interrupts in V8086 mode.
    /// Checks the VME redirection bitmap in the TSS to decide whether to
    /// redirect an interrupt through the virtual-mode IVT (real-mode IVT at
    /// linear address vector*4) or let it trap to the V86 monitor.
    ///
    /// Returns true if the interrupt was redirected (handled via virtual IVT),
    /// false if it should go through the IDT (protected_mode_int).
    pub(super) fn v86_redirect_interrupt(&mut self, vector: u8) -> super::Result<bool> {
        if self.cr4.vme() {
            let tr_base = self.tr.cache.u.segment_base();
            let tr_limit = self.tr.cache.u.segment_limit_scaled();

            // TSS must have room for I/O base address field (offset 102-103)
            // (vm8086.cc)
            if tr_limit < 103 {
                tracing::error!("v86_redirect_interrupt(): TR.limit < 103 in VME");
                return self.exception(super::cpu::Exception::Gp, 0).map(|_| false);
            }

            // Read I/O permission bitmap base from TSS offset 102 (vm8086.cc)
            let io_base = self.system_read_word(tr_base + 102)? as u32;
            // Redirection bitmap is 32 bytes before the I/O bitmap (vm8086.cc)
            let offset = io_base.wrapping_sub(32).wrapping_add((vector >> 3) as u32);

            if offset > tr_limit {
                tracing::error!("v86_redirect_interrupt(): failed to fetch VME redirection bitmap");
                return self.exception(super::cpu::Exception::Gp, 0).map(|_| false);
            }

            // Read the redirection bitmap byte (vm8086.cc)
            let vme_redirection_bitmap = self.system_read_byte(tr_base + offset as u64)?;

            // If bit is 0, redirect through virtual-mode IVT (vm8086.cc)
            if (vme_redirection_bitmap & (1 << (vector & 7))) == 0 {
                // Redirect interrupt through virtual-mode IVT (vm8086.cc)
                let mut temp_flags = (self.read_eflags() & 0xFFFF) as u16;

                // Read CS:IP from IVT (real-mode interrupt vector table at address 0)
                let temp_cs = self.system_read_word((vector as u64) * 4 + 2)?;
                let temp_ip = self.system_read_word((vector as u64) * 4)?;

                // IOPL < 3: adjust flags for VME (vm8086.cc)
                if self.eflags.iopl() < 3 {
                    temp_flags |= EFlags::IOPL_MASK.bits() as u16; // show IOPL=3
                    if self.eflags.contains(EFlags::VIF) {
                        temp_flags |= EFlags::IF_.bits() as u16;
                    } else {
                        temp_flags &= !(EFlags::IF_.bits() as u16);
                    }
                }

                let old_ip = self.get_ip();
                let old_cs = self.sregs[BxSegregs::Cs as usize].selector.value;

                // Push flags, CS, IP onto V86 stack (vm8086.cc)
                self.push_16(temp_flags)?;
                self.push_16(old_cs)?;
                self.push_16(old_ip)?;

                // Load new CS:IP from IVT (vm8086.cc)
                self.load_seg_reg_real_mode(BxSegregs::Cs, temp_cs);
                self.set_eip(temp_ip as u32);

                // Bochs vm8086.cc:248-249 — clear_TF(); clear_RF();
                self.eflags.remove(EFlags::TF);
                self.clear_rf();

                // Clear IF or VIF depending on IOPL (vm8086.cc)
                if self.eflags.iopl() == 3 {
                    self.eflags.remove(EFlags::IF_);
                    self.handle_interrupt_mask_change();
                } else {
                    self.eflags.remove(EFlags::VIF);
                }

                return Ok(true);
            }
        }

        // Interrupt is not redirected or VME is OFF (vm8086.cc)
        if self.eflags.iopl() < 3 {
            tracing::trace!(
                "v86_redirect_interrupt(): interrupt cannot be redirected, generate #GP(0)"
            );
            return self.exception(super::cpu::Exception::Gp, 0).map(|_| false);
        }

        Ok(false)
    }

    /// Initialize all segment register caches for V8086 mode.
    ///
    /// Bochs: BX_CPU_C::init_v8086_mode() in vm8086.cc
    ///
    /// Sets all 6 segment registers to V8086 mode: base = selector << 4,
    /// limit = 0xFFFF, DPL = 3, type = data read/write accessed, 16-bit.
    pub(super) fn init_v8086_mode(&mut self) {
        for sreg in 0..6 {
            // Set cache fields (vm8086.cc)
            self.sregs[sreg].cache.valid = SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK;
            self.sregs[sreg].cache.p = true;
            self.sregs[sreg].cache.dpl = 3;
            self.sregs[sreg].cache.segment = true;
            self.sregs[sreg].cache.r#type = 3; // BX_DATA_READ_WRITE_ACCESSED

                self.sregs[sreg].cache.u.set_segment_base(
                    (self.sregs[sreg].selector.value as u64) << 4);
                self.sregs[sreg].cache.u.set_segment_limit_scaled(0xFFFF);
                self.sregs[sreg].cache.u.set_segment_g(false);
                self.sregs[sreg].cache.u.set_segment_d_b(false);
                self.sregs[sreg].cache.u.set_segment_avl(false);
            self.sregs[sreg].selector.rpl = 3;
        }

        // Update CPU mode (VM flag was set in EFLAGS before this call)
        self.handle_cpu_mode_change(); // vm8086.cc

        // Update alignment check state for the CPL change
        self.handle_alignment_check(); // vm8086.cc

        // Invalidate stack cache (vm8086.cc)
        self.invalidate_stack_cache();
    }
}
