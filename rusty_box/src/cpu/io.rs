//! I/O Port Instructions
//!
//! Implements IN and OUT instructions for port I/O.
//! Mirrors `io.cc` from Bochs.

use super::{
    decoder::{BxSegregs, Instruction},
    BxCpuC, BxCpuIdTrait,
};
use crate::cpu::rusty_box::MemoryAccessType;

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // I/O Privilege Check — Bochs io.cc
    // ========================================================================

    /// Check I/O port permission based on IOPL and TSS I/O permission bitmap.
    /// Returns true if access is allowed, false if #GP(0) should be raised.
    /// Based on Bochs io.cc allow_io().
    fn allow_io(&mut self, port: u16, len: u32) -> super::Result<bool> {
        // If not in protected mode, or CPL <= IOPL and not V8086, allow
        if !self.cr0.pe() {
            return Ok(true);
        }

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        let iopl = self.eflags.iopl();
        let vm = self.v8086_mode();

        // In PM: check if we need I/O permission bitmap
        // Bochs: if (PE && (VM || CPL > IOPL))
        if vm || cpl > iopl {
            // Must consult TSS I/O permission bitmap
            // Check TR points to a valid 386 TSS
            if self.tr.cache.valid == 0
                || (self.tr.cache.r#type != 0x9 && self.tr.cache.r#type != 0xB)
            {
                // TR doesn't point to available/busy 386 TSS
                return Ok(false);
            }

            let tr_limit = self.tr.cache.u.segment_limit_scaled();
            if tr_limit < 103 {
                return Ok(false);
            }

            let tr_base = self.tr.cache.u.segment_base();
            let io_base = self.system_read_word(tr_base + 102)? as u32;

            if (io_base + (port as u32) / 8) >= tr_limit {
                return Ok(false);
            }

            let permission16 =
                self.system_read_word(tr_base + io_base as u64 + (port as u64) / 8)?;

            let bit_index = (port & 7) as u32;
            let mask = (1u32 << len) - 1;
            if ((permission16 as u32) >> bit_index) & mask != 0 {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// IN AL, imm8 - Input byte from immediate port to AL
    /// Bochs io.cc
    pub fn in_al_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        self.svm_intercept_io(port, 1, true, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 1, true, true)? {
            return Ok(());
        }
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 1) as u8;
        self.set_al(value);
        Ok(())
    }

    /// IN AX, imm8 - Input word from immediate port to AX
    /// Bochs io.cc
    pub fn in_ax_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        self.svm_intercept_io(port, 2, true, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 2, true, true)? {
            return Ok(());
        }
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 2) as u16;
        self.set_ax(value);
        Ok(())
    }

    /// IN EAX, imm8 - Input dword from immediate port to EAX
    /// Bochs io.cc — writes RAX (zero-extends to 64-bit)
    pub fn in_eax_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        self.svm_intercept_io(port, 4, true, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 4, true, true)? {
            return Ok(());
        }
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 4);
        self.set_rax(value as u64);
        Ok(())
    }

    /// OUT imm8, AL - Output byte from AL to immediate port
    /// Bochs io.cc
    pub fn out_ib_al(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        self.svm_intercept_io(port, 1, false, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 1, false, true)? {
            return Ok(());
        }
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.al();
        self.port_out(port, value as u32, 1);
        Ok(())
    }

    /// OUT imm8, AX - Output word from AX to immediate port
    /// Bochs io.cc
    pub fn out_ib_ax(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        self.svm_intercept_io(port, 2, false, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 2, false, true)? {
            return Ok(());
        }
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.ax();
        self.port_out(port, value as u32, 2);
        Ok(())
    }

    /// OUT imm8, EAX - Output dword from EAX to immediate port
    /// Bochs io.cc
    pub fn out_ib_eax(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        self.svm_intercept_io(port, 4, false, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 4, false, true)? {
            return Ok(());
        }
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.eax();
        self.port_out(port, value, 4);
        Ok(())
    }

    /// IN AL, DX - Input byte from port DX to AL
    /// Bochs io.cc
    pub fn in_al_dx(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        self.svm_intercept_io(port, 1, true, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 1, true, false)? {
            return Ok(());
        }
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 1) as u8;
        self.set_al(value);
        Ok(())
    }

    /// IN AX, DX - Input word from port DX to AX
    /// Bochs io.cc
    pub fn in_ax_dx(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        self.svm_intercept_io(port, 2, true, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 2, true, false)? {
            return Ok(());
        }
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 2) as u16;
        self.set_ax(value);
        Ok(())
    }

    /// IN EAX, DX - Input dword from port DX to EAX
    /// Bochs io.cc — writes RAX (zero-extends to 64-bit)
    pub fn in_eax_dx(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        self.svm_intercept_io(port, 4, true, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 4, true, false)? {
            return Ok(());
        }
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 4);
        self.set_rax(value as u64);
        Ok(())
    }

    /// OUT DX, AL - Output byte from AL to port DX
    /// Bochs io.cc
    pub fn out_dx_al(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        self.svm_intercept_io(port, 1, false, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 1, false, false)? {
            return Ok(());
        }
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.al();
        self.port_out(port, value as u32, 1);
        Ok(())
    }

    /// OUT DX, AX - Output word from AX to port DX
    /// Bochs io.cc
    pub fn out_dx_ax(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        self.svm_intercept_io(port, 2, false, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 2, false, false)? {
            return Ok(());
        }
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.ax();
        self.port_out(port, value as u32, 2);
        Ok(())
    }

    /// OUT DX, EAX - Output dword from EAX to port DX
    /// Bochs io.cc
    pub fn out_dx_eax(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        self.svm_intercept_io(port, 4, false, false, false, 0)?;
        if self.in_vmx_guest && self.vmexit_check_io(port, 4, false, false)? {
            return Ok(());
        }
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.eax();
        self.port_out(port, value, 4);
        Ok(())
    }

    // ========================================================================
    // INS/OUTS - String I/O instructions
    // ========================================================================

    // ---- INS: 16-bit address mode (DI/CX, ES segment) ----
    // Bochs io.cc — INS uses ES:DI, no segment override allowed

    /// INSB - Input byte from port DX to ES:DI (16-bit address mode)
    fn insb16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let di = self.di() as u32;
        let laddr = self.prepare_rmw_virtual_byte(BxSegregs::Es, di)?;
        self.check_rmw_write_permissions(laddr, 1)?;
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        let old_value = self.read_prepared_rmw_byte();
        #[cfg(feature = "instrumentation")]
        self.report_ins_rmw_access(laddr, &[old_value]);
        let value = self.port_in(port, 1) as u8;
        self.write_rmw_linear_byte(value);
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_di(self.di().wrapping_add(1));
        }
        Ok(())
    }

    #[inline]
    fn commit_insw_rmw(&mut self, value: u16) {
        self.write_rmw_linear_word(value);
    }

    #[cfg(feature = "instrumentation")]
    #[inline]
    fn report_ins_rmw_access(&mut self, laddr: u64, bytes: &[u8]) {
        let xlation = self.address_xlation;
        if xlation.pages == 2 {
            let len1 = xlation.len1 as usize;
            self.on_lin_access(
                laddr,
                xlation.paddress1,
                &bytes[..len1],
                super::instrumentation::MemAccessRW::RW,
            );
            self.on_lin_access(
                (laddr | 0x0fff).wrapping_add(1),
                xlation.paddress2,
                &bytes[len1..],
                super::instrumentation::MemAccessRW::RW,
            );
        } else {
            self.on_lin_access(
                laddr,
                xlation.paddress1,
                bytes,
                super::instrumentation::MemAccessRW::RW,
            );
        }
    }

    /// Direct bulk execution must not bypass active instrumentation hooks.
    #[inline]
    pub(super) fn direct_rep_bulk_allowed(&self, includes_io: bool) -> bool {
        #[cfg(feature = "instrumentation")]
        {
            self.page_permissions.is_none()
                && !self.instrumentation.active.has_exec()
                && !self.instrumentation.active.has_mem()
                && (!includes_io || !self.instrumentation.active.has_io())
        }
        #[cfg(not(feature = "instrumentation"))]
        {
            let _ = includes_io;
            true
        }
    }

    /// INSW - Input word from port DX to ES:DI (16-bit address mode)
    fn insw16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let di = self.di() as u32;
        // Prepare all segment/page translations first. Instrumentation
        // permissions are checked before a potentially destructive MMIO read,
        // then the port input commits through that same RMW translation.
        let laddr = self.prepare_rmw_virtual_word(BxSegregs::Es, di)?;
        self.check_rmw_word_write_permissions(laddr)?;
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        let old_value = self.read_prepared_rmw_word();
        #[cfg(feature = "instrumentation")]
        self.report_ins_rmw_access(laddr, &old_value.to_le_bytes());
        let value = self.port_in(port, 2) as u16;
        self.commit_insw_rmw(value);
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_di(self.di().wrapping_add(2));
        }
        Ok(())
    }

    /// INSD - Input dword from port DX to ES:DI (16-bit address mode)
    fn insd16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let di = self.di() as u32;
        let laddr = self.prepare_rmw_virtual_dword(BxSegregs::Es, di)?;
        self.check_rmw_write_permissions(laddr, 4)?;
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        let old_value = self.read_prepared_rmw_dword();
        #[cfg(feature = "instrumentation")]
        self.report_ins_rmw_access(laddr, &old_value.to_le_bytes());
        let value = self.port_in(port, 4);
        self.write_rmw_linear_dword(value);
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_di(self.di().wrapping_add(4));
        }
        Ok(())
    }

    // ---- INS: 32-bit address mode (EDI/ECX, ES segment) ----

    /// INSB - Input byte from port DX to ES:EDI (32-bit address mode)
    /// Bochs io.cc INSB32_YbDX: writes RDI = EDI ± 1 (clears upper 32 bits)
    fn insb32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let edi = self.edi();
        let laddr = self.prepare_rmw_virtual_byte(BxSegregs::Es, edi)?;
        self.check_rmw_write_permissions(laddr, 1)?;
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        let old_value = self.read_prepared_rmw_byte();
        #[cfg(feature = "instrumentation")]
        self.report_ins_rmw_access(laddr, &[old_value]);
        let value = self.port_in(port, 1) as u8;
        self.write_rmw_linear_byte(value);
        if self.get_df() {
            self.set_rdi(edi.wrapping_sub(1) as u64);
        } else {
            self.set_rdi(edi.wrapping_add(1) as u64);
        }
        Ok(())
    }

    /// INSW - Input word from port DX to ES:EDI (32-bit address mode)
    /// Bochs io.cc INSW32_YwDX (lines 325-373): RMW pattern triggers page faults
    /// before I/O read. port_in does not touch address_xlation so RMW is safe.
    fn insw32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let edi = self.edi();
        // Translate before the physical RMW read so instrumentation faults do
        // not consume destructive MMIO state.
        let laddr = self.prepare_rmw_virtual_word(BxSegregs::Es, edi)?;
        self.check_rmw_word_write_permissions(laddr)?;
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        let old_value = self.read_prepared_rmw_word();
        #[cfg(feature = "instrumentation")]
        self.report_ins_rmw_access(laddr, &old_value.to_le_bytes());
        let value = self.port_in(port, 2) as u16;
        self.commit_insw_rmw(value);
        if self.get_df() {
            self.set_rdi(edi.wrapping_sub(2) as u64);
        } else {
            self.set_rdi(edi.wrapping_add(2) as u64);
        }
        Ok(())
    }

    /// INSD - Input dword from port DX to ES:EDI (32-bit address mode)
    /// Bochs io.cc INSD32_YdDX (lines 436-449): RMW pattern triggers page faults
    /// before I/O read. port_in does not touch address_xlation so RMW is safe.
    fn insd32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let edi = self.edi();
        let laddr = self.prepare_rmw_virtual_dword(BxSegregs::Es, edi)?;
        self.check_rmw_write_permissions(laddr, 4)?;
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        let old_value = self.read_prepared_rmw_dword();
        #[cfg(feature = "instrumentation")]
        self.report_ins_rmw_access(laddr, &old_value.to_le_bytes());
        let value = self.port_in(port, 4);
        self.write_rmw_linear_dword(value);
        if self.get_df() {
            self.set_rdi(edi.wrapping_sub(4) as u64);
        } else {
            self.set_rdi(edi.wrapping_add(4) as u64);
        }
        Ok(())
    }

    // ---- OUTS: 16-bit address mode (SI/CX, segment-overridable) ----
    // Bochs io.cc — OUTS uses seg:SI, segment override IS allowed

    /// OUTSB - Output byte from seg:SI to port DX (16-bit address mode)
    fn outsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let seg = BxSegregs::from(instr.seg());
        let si = self.si() as u32;
        let value = self.v_read_byte(seg, si)?;
        self.port_out(port, value as u32, 1);
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(1));
        } else {
            self.set_si(self.si().wrapping_add(1));
        }
        Ok(())
    }

    /// OUTSW - Output word from seg:SI to port DX (16-bit address mode)
    fn outsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let seg = BxSegregs::from(instr.seg());
        let si = self.si() as u32;
        let value = self.v_read_word(seg, si)?;
        self.port_out(port, value as u32, 2);
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(2));
        } else {
            self.set_si(self.si().wrapping_add(2));
        }
        Ok(())
    }

    /// OUTSD - Output dword from seg:SI to port DX (16-bit address mode)
    fn outsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let seg = BxSegregs::from(instr.seg());
        let si = self.si() as u32;
        let value = self.v_read_dword(seg, si)?;
        self.port_out(port, value, 4);
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(4));
        } else {
            self.set_si(self.si().wrapping_add(4));
        }
        Ok(())
    }

    // ---- OUTS: 32-bit address mode (ESI/ECX, segment-overridable) ----

    /// OUTSB - Output byte from seg:ESI to port DX (32-bit address mode)
    /// Bochs io.cc OUTSB32_DXXb: writes RSI = ESI ± 1 (clears upper 32 bits)
    fn outsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let seg = BxSegregs::from(instr.seg());
        let esi = self.esi();
        let value = self.v_read_byte(seg, esi)?;
        self.port_out(port, value as u32, 1);
        if self.get_df() {
            self.set_rsi(esi.wrapping_sub(1) as u64);
        } else {
            self.set_rsi(esi.wrapping_add(1) as u64);
        }
        Ok(())
    }

    /// OUTSW - Output word from seg:ESI to port DX (32-bit address mode)
    /// Bochs io.cc OUTSW32_DXXw: writes RSI = ESI ± 2
    fn outsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let seg = BxSegregs::from(instr.seg());
        let esi = self.esi();
        let value = self.v_read_word(seg, esi)?;
        self.port_out(port, value as u32, 2);
        if self.get_df() {
            self.set_rsi(esi.wrapping_sub(2) as u64);
        } else {
            self.set_rsi(esi.wrapping_add(2) as u64);
        }
        Ok(())
    }

    /// OUTSD - Output dword from seg:ESI to port DX (32-bit address mode)
    /// Bochs io.cc OUTSD32_DXXd: writes RSI = ESI ± 4
    fn outsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let seg = BxSegregs::from(instr.seg());
        let esi = self.esi();
        let value = self.v_read_dword(seg, esi)?;
        self.port_out(port, value, 4);
        if self.get_df() {
            self.set_rsi(esi.wrapping_sub(4) as u64);
        } else {
            self.set_rsi(esi.wrapping_add(4) as u64);
        }
        Ok(())
    }

    // ---- REP INS: 16-bit address mode ----

    fn rep_insb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.insb16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if cx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let mut cx = self.cx();
        // `tickn_fastrep` only probes the PC-system countdown; it does not
        // consume it. Keep one budget for this entire REP instruction, even
        // when its destination spans multiple host pages.
        let mut fastrep_iterations = 0usize;
        let mut event_words_remaining = self.ticks_left_next_event() as usize;


        if self.direct_rep_bulk_allowed(true) && !self.get_df() && self.async_event == 0 {
            while cx != 0 && event_words_remaining != 0 {
                let di = self.di();
                let di_u32 = u32::from(di);
                let laddr = self.get_laddr32(BxSegregs::Es as usize, di_u32) as u64;
                let Some((host_ptr, host_remaining, paddr)) =
                    self.get_host_write_ptr_for_bulk(laddr)?
                else {
                    break;
                };
                let page_words = (0x1000usize - (laddr as usize & 0x0fff))
                    .min(host_remaining)
                    / 2;
                let segment_words = (0x1_0000usize - usize::from(di)) / 2;
                let chunk_words = usize::from(cx)
                    .min(page_words)
                    .min(segment_words)
                    .min(event_words_remaining);
                let Some(bulk_bytes) = chunk_words.checked_mul(2) else {
                    break;
                };
                if bulk_bytes == 0
                    || !self.write_virtual_checks(BxSegregs::Es as usize, di_u32, bulk_bytes as u32)
                    || self.smc_range_has_cached_code(paddr, bulk_bytes as u32)
                {
                    break;
                }

                let bulk_slice = unsafe { core::slice::from_raw_parts_mut(host_ptr, bulk_bytes) };
                let bytes_read = self.bulk_port_in(port, 2, bulk_slice);
                debug_assert!(bytes_read <= bulk_bytes && bytes_read % 2 == 0);
                let completed_bytes = bytes_read;
                if completed_bytes == 0 {
                    break;
                }
                let transferred = completed_bytes / 2;
                self.smc_write_check(paddr, completed_bytes as u32);
                let transferred_u16 = transferred as u16;
                self.set_di(di.wrapping_add(transferred_u16.wrapping_mul(2)));
                cx = cx.wrapping_sub(transferred_u16);
                self.set_cx(cx);
                self.icount += transferred.saturating_sub(1) as u64;
                fastrep_iterations += transferred;
                event_words_remaining -= transferred;
                self.tickn_fastrep(fastrep_iterations);
                if cx == 0 {
                    return Ok(());
                }
                if self.async_event != 0 {
                    self.assert_rf();
                    self.set_rip(self.prev_rip);
                    self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                    return Ok(());
                }
                self.icount += 1;
            }
        }

        loop {
            if cx != 0 {
                self.insw16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if cx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.insd16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if cx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- REP INS: 32-bit address mode ----

    fn rep_insb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        if ecx == 0 {
            return Ok(());
        }
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.insb32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
                self.set_rcx(self.ecx() as u64);
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let mut ecx = self.ecx();
        // `tickn_fastrep` only probes the PC-system countdown; it does not
        // consume it. Keep one budget for this entire REP instruction, even
        // when its destination spans multiple host pages.
        let mut fastrep_iterations = 0usize;
        let mut event_words_remaining = self.ticks_left_next_event() as usize;
        if ecx == 0 {
            return Ok(());
        }

        if self.direct_rep_bulk_allowed(true) && !self.get_df() && self.async_event == 0 {
            while ecx != 0 && event_words_remaining != 0 {
                let edi = self.edi();
                let laddr = self.get_laddr32(BxSegregs::Es as usize, edi) as u64;
                let Some((host_ptr, host_remaining, paddr)) =
                    self.get_host_write_ptr_for_bulk(laddr)?
                else {
                    break;
                };
                let page_words = (0x1000usize - (laddr as usize & 0x0fff))
                    .min(host_remaining)
                    / 2;
                let chunk_words = (ecx as usize)
                    .min(page_words)
                    .min(event_words_remaining);
                let Some(bulk_bytes) = chunk_words.checked_mul(2) else {
                    break;
                };
                if bulk_bytes == 0
                    || !self.write_virtual_checks(BxSegregs::Es as usize, edi, bulk_bytes as u32)
                    || self.smc_range_has_cached_code(paddr, bulk_bytes as u32)
                {
                    break;
                }

                let bulk_slice = unsafe { core::slice::from_raw_parts_mut(host_ptr, bulk_bytes) };
                let bytes_read = self.bulk_port_in(port, 2, bulk_slice);
                debug_assert!(bytes_read <= bulk_bytes && bytes_read % 2 == 0);
                let completed_bytes = bytes_read;
                if completed_bytes == 0 {
                    break;
                }
                let transferred = completed_bytes / 2;
                self.smc_write_check(paddr, completed_bytes as u32);
                let transferred_u32 = transferred as u32;
                self.set_rdi(edi.wrapping_add(transferred_u32.wrapping_mul(2)) as u64);
                ecx = ecx.wrapping_sub(transferred_u32);
                self.set_ecx(ecx);
                self.icount += transferred.saturating_sub(1) as u64;
                fastrep_iterations += transferred;
                event_words_remaining -= transferred;
                self.tickn_fastrep(fastrep_iterations);
                if ecx == 0 {
                    self.set_rcx(ecx as u64);
                    return Ok(());
                }
                if self.async_event != 0 {
                    self.assert_rf();
                    self.set_rip(self.prev_rip);
                    self.set_rcx(ecx as u64);
                    self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                    return Ok(());
                }
                self.icount += 1;
            }
        }

        loop {
            self.insw32(instr)?;
            self.on_repeat_iteration(instr);
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx == 0 {
                self.set_rcx(ecx as u64);
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.set_rcx(ecx as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        if ecx == 0 {
            return Ok(());
        }
        loop {
            self.insd32(instr)?;
            self.on_repeat_iteration(instr);
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx == 0 {
                self.set_rcx(ecx as u64);
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.set_rcx(ecx as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- REP OUTS: 16-bit address mode ----

    fn rep_outsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.outsb16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if cx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.outsw16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if cx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.outsd16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if cx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- REP OUTS: 32-bit address mode ----

    fn rep_outsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        if ecx == 0 {
            return Ok(());
        }
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.outsb32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
                self.set_rcx(self.ecx() as u64);
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        if ecx == 0 {
            return Ok(());
        }
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.outsw32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
                self.set_rcx(self.ecx() as u64);
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        if ecx == 0 {
            return Ok(());
        }
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.outsd32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
                self.set_rcx(self.ecx() as u64);
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- INS: 64-bit address mode (RDI/RCX, ES segment) ----
    // Bochs io.cc INSB64_YbDX / INSW64_YwDX / INSD64_YdDX

    /// INSB - Input byte from port DX to ES:RDI (64-bit address mode)
    fn insb64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let rdi = self.rdi();
        let laddr = self.prepare_rmw_virtual_byte_64(BxSegregs::Es, rdi)?;
        self.check_rmw_write_permissions(laddr, 1)?;
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        let old_value = self.read_prepared_rmw_byte();
        #[cfg(feature = "instrumentation")]
        self.report_ins_rmw_access(laddr, &[old_value]);
        let value = self.port_in(port, 1) as u8;
        self.write_rmw_linear_byte(value);
        if self.get_df() {
            self.set_rdi(rdi.wrapping_sub(1));
        } else {
            self.set_rdi(rdi.wrapping_add(1));
        }
        Ok(())
    }

    /// INSW - Input word from port DX to ES:RDI (64-bit address mode)
    /// INSW - Input word from port DX to ES:RDI (64-bit address mode)
    /// Bochs io.cc INSW64_YwDX (lines 378-391): RMW pattern with linear address.
    fn insw64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let rdi = self.rdi();
        // Translate before the physical RMW read so instrumentation faults do
        // not consume destructive MMIO state.
        let laddr = self.prepare_rmw_virtual_word_64(BxSegregs::Es, rdi)?;
        self.check_rmw_word_write_permissions(laddr)?;
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        let old_value = self.read_prepared_rmw_word();
        #[cfg(feature = "instrumentation")]
        self.report_ins_rmw_access(laddr, &old_value.to_le_bytes());
        let value = self.port_in(port, 2) as u16;
        self.commit_insw_rmw(value);
        if self.get_df() {
            self.set_rdi(rdi.wrapping_sub(2));
        } else {
            self.set_rdi(rdi.wrapping_add(2));
        }
        Ok(())
    }

    /// INSD - Input dword from port DX to ES:RDI (64-bit address mode)
    /// Bochs io.cc INSD64_YdDX (lines 454-467): RMW pattern with linear address.
    fn insd64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let rdi = self.rdi();
        let laddr = self.prepare_rmw_virtual_dword_64(BxSegregs::Es, rdi)?;
        self.check_rmw_write_permissions(laddr, 4)?;
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        let old_value = self.read_prepared_rmw_dword();
        #[cfg(feature = "instrumentation")]
        self.report_ins_rmw_access(laddr, &old_value.to_le_bytes());
        let value = self.port_in(port, 4);
        self.write_rmw_linear_dword(value);
        if self.get_df() {
            self.set_rdi(rdi.wrapping_sub(4));
        } else {
            self.set_rdi(rdi.wrapping_add(4));
        }
        Ok(())
    }

    // ---- OUTS: 64-bit address mode (RSI/RCX, segment-overridable) ----
    // Bochs io.cc OUTSB64_DXXb / OUTSW64_DXXw / OUTSD64_DXXd

    /// OUTSB - Output byte from seg:RSI to port DX (64-bit address mode)
    fn outsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let seg = BxSegregs::from(instr.seg());
        let rsi = self.rsi();
        let laddr = self.get_laddr64(seg as usize, rsi);
        let value = self.read_virtual_byte_at_laddr(laddr)?;
        self.port_out(port, value as u32, 1);
        if self.get_df() {
            self.set_rsi(rsi.wrapping_sub(1));
        } else {
            self.set_rsi(rsi.wrapping_add(1));
        }
        Ok(())
    }

    /// OUTSW - Output word from seg:RSI to port DX (64-bit address mode)
    fn outsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let seg = BxSegregs::from(instr.seg());
        let rsi = self.rsi();
        let value = self.read_virtual_word_64(seg, rsi)?;
        self.port_out(port, value as u32, 2);
        if self.get_df() {
            self.set_rsi(rsi.wrapping_sub(2));
        } else {
            self.set_rsi(rsi.wrapping_add(2));
        }
        Ok(())
    }

    /// OUTSD - Output dword from seg:RSI to port DX (64-bit address mode)
    fn outsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let seg = BxSegregs::from(instr.seg());
        let rsi = self.rsi();
        let value = self.read_virtual_dword_64(seg, rsi)?;
        self.port_out(port, value, 4);
        if self.get_df() {
            self.set_rsi(rsi.wrapping_sub(4));
        } else {
            self.set_rsi(rsi.wrapping_add(4));
        }
        Ok(())
    }

    // ---- REP INS: 64-bit address mode ----

    fn rep_insb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.insb64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if rcx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        let mut rcx = self.rcx();
        // `tickn_fastrep` only probes the PC-system countdown; it does not
        // consume it. Keep one budget for this entire REP instruction, even
        // when its destination spans multiple host pages.
        let mut fastrep_iterations = 0usize;
        let mut event_words_remaining = self.ticks_left_next_event() as usize;

        if self.direct_rep_bulk_allowed(true) && !self.get_df() && self.async_event == 0 {
            while rcx != 0 && event_words_remaining != 0 {
                let rdi = self.rdi();
                let laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);
                let Some((host_ptr, host_remaining, paddr)) =
                    self.get_host_write_ptr_for_bulk(laddr)?
                else {
                    break;
                };
                let page_words = (0x1000usize - (laddr as usize & 0x0fff))
                    .min(host_remaining)
                    / 2;
                let chunk_words = (rcx.min(usize::MAX as u64) as usize)
                    .min(page_words)
                    .min(event_words_remaining);
                let Some(bulk_bytes) = chunk_words.checked_mul(2) else {
                    break;
                };
                let Ok(bulk_bytes_u32) = u32::try_from(bulk_bytes) else {
                    break;
                };
                let Some(last_offset) = u64::from(bulk_bytes_u32).checked_sub(1) else {
                    break;
                };
                let Some(last_laddr) = laddr.checked_add(last_offset) else {
                    break;
                };
                let user = self.user_pl();
                if !self.is_canonical_access(laddr, MemoryAccessType::Write, user)
                    || !self.is_canonical_access(last_laddr, MemoryAccessType::Write, user)
                    || self.smc_range_has_cached_code(paddr, bulk_bytes_u32)
                {
                    break;
                }

                let bulk_slice = unsafe { core::slice::from_raw_parts_mut(host_ptr, bulk_bytes) };
                let bytes_read = self.bulk_port_in(port, 2, bulk_slice);
                debug_assert!(bytes_read <= bulk_bytes && bytes_read % 2 == 0);
                let completed_bytes = bytes_read;
                if completed_bytes == 0 {
                    break;
                }
                let transferred = completed_bytes / 2;
                self.smc_write_check(paddr, completed_bytes as u32);
                let transferred_u64 = transferred as u64;
                self.set_rdi(rdi.wrapping_add(transferred_u64.wrapping_mul(2)));
                rcx = rcx.wrapping_sub(transferred_u64);
                self.set_rcx(rcx);
                self.icount += transferred.saturating_sub(1) as u64;
                fastrep_iterations += transferred;
                event_words_remaining -= transferred;
                self.tickn_fastrep(fastrep_iterations);
                if rcx == 0 {
                    return Ok(());
                }
                if self.async_event != 0 {
                    self.assert_rf();
                    self.set_rip(self.prev_rip);
                    self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                    return Ok(());
                }
                self.icount += 1;
            }
        }

        loop {
            if rcx != 0 {
                self.insw64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if rcx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        if rcx == 0 {
            return Ok(());
        }
        loop {
            self.insd64(instr)?;
            self.on_repeat_iteration(instr);
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- REP OUTS: 64-bit address mode ----

    fn rep_outsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.outsb64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if rcx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.outsw64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if rcx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.outsd64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if rcx == 0 {
                return Ok(());
            }
            if self.async_event != 0 {
                break;
            }
            self.icount += 1;
        }
        self.assert_rf();
        self.set_rip(self.prev_rip);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ========================================================================
    // Unified INS/OUTS dispatch methods
    // ========================================================================

    /// SVM + VMX intercept for INS/OUTS (Bochs vmexit.cc VMexit_IO with the
    /// `BX_IA_REP_INSx` / `BX_IA_REP_OUTSx` cases). For INS the source linear
    /// address is `ES:rDI`; for OUTS it is the prefix-segment with `rSI`. The
    /// effective offset is masked by the address size.
    fn intercept_string_io(
        &mut self,
        instr: &Instruction,
        size: u32,
        direction_in: bool,
    ) -> super::Result<bool> {
        let port = self.dx();
        let as64 = instr.as64_l() != 0;
        let as32 = instr.as32_l() != 0;
        let rep = instr.lock_rep_used_value() != 0;
        let asize_bits: u8 = if as64 {
            64
        } else if as32 {
            32
        } else {
            16
        };
        self.svm_intercept_io(port, size, direction_in, true, rep, asize_bits)?;
        if !self.in_vmx_guest {
            return Ok(false);
        }
        let asize_mask: u64 = if as64 {
            u64::MAX
        } else if as32 {
            u64::from(u32::MAX)
        } else {
            u64::from(u16::MAX)
        };
        let (seg, offset) = if direction_in {
            (BxSegregs::Es as u8, self.rdi() & asize_mask)
        } else {
            (instr.seg(), self.rsi() & asize_mask)
        };
        let laddr = self.get_laddr64(usize::from(seg), offset);
        self.vmexit_check_io_string(port, size, direction_in, rep, laddr, seg, as64, as32)
    }

    #[inline]
    fn require_string_io_permission(&mut self, size: u32) -> super::Result<()> {
        if self.allow_io(self.dx(), size)? {
            Ok(())
        } else {
            self.exception(super::cpu::Exception::Gp, 0)
        }
    }

    /// INSB dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    /// Bochs io.cc REP_INSB_YbDX: checks as64L, as32L, then 16-bit
    pub fn insb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if self.intercept_string_io(instr, 1, true)? {
            return Ok(());
        }
        self.require_string_io_permission(1)?;
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep {
                self.rep_insb64(instr)?;
            } else {
                self.insb64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_insb32(instr)?;
            } else {
                self.insb32(instr)?;
            }
        } else if rep {
            self.rep_insb16(instr)?;
        } else {
            self.insb16(instr)?;
        }
        Ok(())
    }

    /// INSW dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn insw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if self.intercept_string_io(instr, 2, true)? {
            return Ok(());
        }
        self.require_string_io_permission(2)?;
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep {
                self.rep_insw64(instr)?;
            } else {
                self.insw64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_insw32(instr)?;
            } else {
                self.insw32(instr)?;
            }
        } else if rep {
            self.rep_insw16(instr)?;
        } else {
            self.insw16(instr)?;
        }
        Ok(())
    }

    /// INSD dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn insd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if self.intercept_string_io(instr, 4, true)? {
            return Ok(());
        }
        self.require_string_io_permission(4)?;
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep {
                self.rep_insd64(instr)?;
            } else {
                self.insd64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_insd32(instr)?;
            } else {
                self.insd32(instr)?;
            }
        } else if rep {
            self.rep_insd16(instr)?;
        } else {
            self.insd16(instr)?;
        }
        Ok(())
    }

    /// OUTSB dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn outsb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if self.intercept_string_io(instr, 1, false)? {
            return Ok(());
        }
        self.require_string_io_permission(1)?;
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep {
                self.rep_outsb64(instr)?;
            } else {
                self.outsb64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_outsb32(instr)?;
            } else {
                self.outsb32(instr)?;
            }
        } else if rep {
            self.rep_outsb16(instr)?;
        } else {
            self.outsb16(instr)?;
        }
        Ok(())
    }

    /// OUTSW dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn outsw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if self.intercept_string_io(instr, 2, false)? {
            return Ok(());
        }
        self.require_string_io_permission(2)?;
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep {
                self.rep_outsw64(instr)?;
            } else {
                self.outsw64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_outsw32(instr)?;
            } else {
                self.outsw32(instr)?;
            }
        } else if rep {
            self.rep_outsw16(instr)?;
        } else {
            self.outsw16(instr)?;
        }
        Ok(())
    }

    /// OUTSD dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn outsd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if self.intercept_string_io(instr, 4, false)? {
            return Ok(());
        }
        self.require_string_io_permission(4)?;
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep {
                self.rep_outsd64(instr)?;
            } else {
                self.outsd64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_outsd32(instr)?;
            } else {
                self.outsd32(instr)?;
            }
        } else if rep {
            self.rep_outsd16(instr)?;
        } else {
            self.outsd16(instr)?;
        }
        Ok(())
    }

    // ========================================================================
    // Port I/O helpers
    // ========================================================================

    /// Bulk-read whole `io_len`-byte port elements into `buf`.
    /// Returns the number of bytes actually read. If the port doesn't support
    /// bulk reads (or no IO bus is wired), returns 0.
    fn bulk_port_in(&mut self, port: u16, io_len: u8, buf: &mut [u8]) -> usize {
        #[cfg(not(feature = "alloc"))]
        let _ = (port, io_len, buf);
        #[cfg(feature = "alloc")]
        let current_ticks = self.system_ticks();
        #[cfg(feature = "alloc")]
        if let Some(io) = self.io_bus_mut() {
            let bytes_read = io.inp_bulk(port, io_len, buf, current_ticks);
            self.sync_io_events();
            return bytes_read;
        }
        0
    }

    /// Read from I/O port.
    ///
    /// When the emulator wires an I/O bus, this dispatches to `BxDevicesC::inp`.
    /// Otherwise it falls back to conservative defaults (useful for unit tests
    /// that don't wire devices and never execute real firmware).
    fn port_in(&mut self, port: u16, len: u8) -> u32 {
        let _ = &port; // used by alloc/instrumentation paths
                       // BOCHS BX_INSTR_INP(addr, len) — fires before the port read.
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_io() {
            self.instrumentation.fire_inp(port, len);
        }

        #[cfg(feature = "alloc")]
        let current_ticks = self.system_ticks();
        #[cfg(feature = "alloc")]
        let value = if let Some(io) = self.io_bus_mut() {
            let value = io.inp(port, len, current_ticks);
            self.sync_io_events();
            value
        } else {
            match len {
                1 => 0xFF,
                2 => 0xFFFF,
                4 => 0xFFFFFFFF,
                _ => 0xFF,
            }
        };
        #[cfg(not(feature = "alloc"))]
        let value = match len {
            1 => 0xFF,
            2 => 0xFFFF,
            4 => 0xFFFFFFFF,
            _ => 0xFF,
        };

        // BOCHS BX_INSTR_INP2(addr, len, val) — fires after the read with the value.
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_io() {
            let ev = super::instrumentation::IoHookEvent {
                port,
                size: len,
                value,
                access: super::instrumentation::MemAccessRW::Read,
            };
            self.instrumentation.fire_inp2(&ev);
        }

        value
    }

    /// Write to I/O port.
    ///
    /// When the emulator wires an I/O bus, this dispatches to `BxDevicesC::outp`.
    /// Otherwise it is ignored (useful for unit tests without devices).
    fn port_out(&mut self, port: u16, value: u32, len: u8) {
        // BOCHS BX_INSTR_OUTP(addr, len, val) — fires at the port write.
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_io() {
            let ev = super::instrumentation::IoHookEvent {
                port,
                size: len,
                value,
                access: super::instrumentation::MemAccessRW::Write,
            };
            self.instrumentation.fire_outp(&ev);
        }

        // Log BIOS diagnostic ports at debug level so RUST_LOG=debug catches them
        // even if something goes wrong before the device handler is reached.
        // Include RIP so we can trace which BIOS function is writing.
        if matches!(port, 0x80 | 0x84 | 0xE9 | 0x402 | 0x403 | 0x500) {
            tracing::trace!(
                "port_out: port={:#06x} value={:#04x} len={} RIP={:#010x}",
                port,
                value as u8,
                len,
                self.rip()
            );
        }
        #[cfg(feature = "alloc")]
        let current_ticks = self.system_ticks();
        #[cfg(feature = "alloc")]
        let dispatched = if let Some(io) = self.io_bus_mut() {
            io.outp(port, value, len, current_ticks);
            true
        } else {
            false
        };
        #[cfg(feature = "alloc")]
        if dispatched {
            // fw_cfg and other port handlers may have written guest RAM while
            // the I/O bus was borrowed. Flush the issuing CPU after that
            // borrow ends so a cached trace cannot execute a stale tail.
            self.sync_io_events();
            self.smc_sync_after_phys_write();
        }
    }
}
