//! I/O Port Instructions
//!
//! Implements IN and OUT instructions for port I/O.
//! Mirrors `io.cc` from Bochs.

use super::{
    decoder::{BxSegregs, Instruction},
    BxCpuC, BxCpuIdTrait,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // I/O Privilege Check — Bochs io.cc:866-929
    // ========================================================================

    /// Check I/O port permission based on IOPL and TSS I/O permission bitmap.
    /// Returns true if access is allowed, false if #GP(0) should be raised.
    /// Based on Bochs io.cc allow_io() lines 866-929.
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
        if vm || cpl > iopl as u8 {
            // Must consult TSS I/O permission bitmap
            // Check TR points to a valid 386 TSS
            if self.tr.cache.valid == 0
                || (self.tr.cache.r#type != 0x9 && self.tr.cache.r#type != 0xB)
            {
                // TR doesn't point to available/busy 386 TSS
                return Ok(false);
            }

            let tr_limit = unsafe { self.tr.cache.u.segment.limit_scaled };
            if tr_limit < 103 {
                return Ok(false);
            }

            let tr_base = unsafe { self.tr.cache.u.segment.base };
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
    /// Bochs io.cc:54-80
    pub fn in_al_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 1) as u8;
        self.set_al(value);
        Ok(())
    }

    /// IN AX, imm8 - Input word from immediate port to AX
    /// Bochs io.cc:82-109
    pub fn in_ax_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 2) as u16;
        self.set_ax(value);
        Ok(())
    }

    /// IN EAX, imm8 - Input dword from immediate port to EAX
    /// Bochs io.cc:111-138 — writes RAX (zero-extends to 64-bit)
    pub fn in_eax_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 4);
        self.set_rax(value as u64);
        Ok(())
    }

    /// OUT imm8, AL - Output byte from AL to immediate port
    /// Bochs io.cc:140-167
    pub fn out_ib_al(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.al();
        self.port_out(port, value as u32, 1);
        Ok(())
    }

    /// OUT imm8, AX - Output word from AX to immediate port
    /// Bochs io.cc:169-197
    pub fn out_ib_ax(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.ax();
        self.port_out(port, value as u32, 2);
        Ok(())
    }

    /// OUT imm8, EAX - Output dword from EAX to immediate port
    /// Bochs io.cc:199-227
    pub fn out_ib_eax(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = instr.ib() as u16;
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.eax();
        self.port_out(port, value, 4);
        Ok(())
    }

    /// IN AL, DX - Input byte from port DX to AL
    /// Bochs io.cc:229-257
    pub fn in_al_dx(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 1) as u8;
        self.set_al(value);
        Ok(())
    }

    /// IN AX, DX - Input word from port DX to AX
    /// Bochs io.cc:259-288
    pub fn in_ax_dx(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 2) as u16;
        self.set_ax(value);
        Ok(())
    }

    /// IN EAX, DX - Input dword from port DX to EAX
    /// Bochs io.cc:290-319 — writes RAX (zero-extends to 64-bit)
    pub fn in_eax_dx(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.port_in(port, 4);
        self.set_rax(value as u64);
        Ok(())
    }

    /// OUT DX, AL - Output byte from AL to port DX
    /// Bochs io.cc:321-349
    pub fn out_dx_al(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.al();
        self.port_out(port, value as u32, 1);
        Ok(())
    }

    /// OUT DX, AX - Output word from AX to port DX
    /// Bochs io.cc:351-380
    pub fn out_dx_ax(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let value = self.ax();
        self.port_out(port, value as u32, 2);
        Ok(())
    }

    /// OUT DX, EAX - Output dword from EAX to port DX
    /// Bochs io.cc:382-411
    pub fn out_dx_eax(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
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
    // Bochs io.cc:413-470 — INS uses ES:DI, no segment override allowed

    /// INSB - Input byte from port DX to ES:DI (16-bit address mode)
    fn insb16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let di = self.di() as u32;
        let value = self.port_in(port, 1) as u8;
        self.v_write_byte(BxSegregs::Es, di, value)?;
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_di(self.di().wrapping_add(1));
        }
        Ok(())
    }

    /// INSW - Input word from port DX to ES:DI (16-bit address mode)
    fn insw16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let di = self.di() as u32;
        let value = self.port_in(port, 2) as u16;
        self.v_write_word(BxSegregs::Es, di, value)?;
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
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let di = self.di() as u32;
        let value = self.port_in(port, 4);
        self.v_write_dword(BxSegregs::Es, di, value)?;
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
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let edi = self.edi();
        let value = self.port_in(port, 1) as u8;
        self.v_write_byte(BxSegregs::Es, edi, value)?;
        if self.get_df() {
            self.set_rdi(edi.wrapping_sub(1) as u64);
        } else {
            self.set_rdi(edi.wrapping_add(1) as u64);
        }
        Ok(())
    }

    /// INSW - Input word from port DX to ES:EDI (32-bit address mode)
    /// Bochs io.cc INSW32_YwDX: writes RDI = EDI ± 2
    fn insw32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let edi = self.edi();
        let value = self.port_in(port, 2) as u16;
        self.v_write_word(BxSegregs::Es, edi, value)?;
        if self.get_df() {
            self.set_rdi(edi.wrapping_sub(2) as u64);
        } else {
            self.set_rdi(edi.wrapping_add(2) as u64);
        }
        Ok(())
    }

    /// INSD - Input dword from port DX to ES:EDI (32-bit address mode)
    /// Bochs io.cc INSD32_YdDX: writes RDI = EDI ± 4
    fn insd32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let edi = self.edi();
        let value = self.port_in(port, 4);
        self.v_write_dword(BxSegregs::Es, edi, value)?;
        if self.get_df() {
            self.set_rdi(edi.wrapping_sub(4) as u64);
        } else {
            self.set_rdi(edi.wrapping_add(4) as u64);
        }
        Ok(())
    }

    // ---- OUTS: 16-bit address mode (SI/CX, segment-overridable) ----
    // Bochs io.cc:565-631 — OUTS uses seg:SI, segment override IS allowed

    /// OUTSB - Output byte from seg:SI to port DX (16-bit address mode)
    fn outsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
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
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
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
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
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
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
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
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
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
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
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
        while cx != 0 {
            self.insb16(instr)?;
            cx -= 1;
            self.set_cx(cx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.insw16(instr)?;
            cx -= 1;
            self.set_cx(cx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.insd16(instr)?;
            cx -= 1;
            self.set_cx(cx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- REP INS: 32-bit address mode ----

    fn rep_insb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.insb32(instr)?;
            ecx -= 1;
            self.set_ecx(ecx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.insw32(instr)?;
            ecx -= 1;
            self.set_ecx(ecx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.insd32(instr)?;
            ecx -= 1;
            self.set_ecx(ecx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- REP OUTS: 16-bit address mode ----

    fn rep_outsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.outsb16(instr)?;
            cx -= 1;
            self.set_cx(cx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.outsw16(instr)?;
            cx -= 1;
            self.set_cx(cx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.outsd16(instr)?;
            cx -= 1;
            self.set_cx(cx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- REP OUTS: 32-bit address mode ----

    fn rep_outsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.outsb32(instr)?;
            ecx -= 1;
            self.set_ecx(ecx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.outsw32(instr)?;
            ecx -= 1;
            self.set_ecx(ecx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.outsd32(instr)?;
            ecx -= 1;
            self.set_ecx(ecx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- INS: 64-bit address mode (RDI/RCX, ES segment) ----
    // Bochs io.cc INSB64_YbDX / INSW64_YwDX / INSD64_YdDX

    /// INSB - Input byte from port DX to ES:RDI (64-bit address mode)
    fn insb64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let rdi = self.rdi();
        let laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);
        let value = self.port_in(port, 1) as u8;
        self.write_virtual_byte_at_laddr(laddr, value)?;
        if self.get_df() {
            self.set_rdi(rdi.wrapping_sub(1));
        } else {
            self.set_rdi(rdi.wrapping_add(1));
        }
        Ok(())
    }

    /// INSW - Input word from port DX to ES:RDI (64-bit address mode)
    fn insw64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let rdi = self.rdi();
        let value = self.port_in(port, 2) as u16;
        self.write_virtual_word_64(BxSegregs::Es, rdi, value)?;
        if self.get_df() {
            self.set_rdi(rdi.wrapping_sub(2));
        } else {
            self.set_rdi(rdi.wrapping_add(2));
        }
        Ok(())
    }

    /// INSD - Input dword from port DX to ES:RDI (64-bit address mode)
    fn insd64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let port = self.dx();
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let rdi = self.rdi();
        let value = self.port_in(port, 4);
        self.write_virtual_dword_64(BxSegregs::Es, rdi, value)?;
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
        if !self.allow_io(port, 1)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
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
        if !self.allow_io(port, 2)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
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
        if !self.allow_io(port, 4)? {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
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
        while rcx != 0 {
            self.insb64(instr)?;
            rcx -= 1;
            self.set_rcx(rcx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.insw64(instr)?;
            rcx -= 1;
            self.set_rcx(rcx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_insd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.insd64(instr)?;
            rcx -= 1;
            self.set_rcx(rcx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- REP OUTS: 64-bit address mode ----

    fn rep_outsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.outsb64(instr)?;
            rcx -= 1;
            self.set_rcx(rcx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.outsw64(instr)?;
            rcx -= 1;
            self.set_rcx(rcx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    fn rep_outsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.outsd64(instr)?;
            rcx -= 1;
            self.set_rcx(rcx);
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ========================================================================
    // Unified INS/OUTS dispatch methods
    // ========================================================================

    /// INSB dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    /// Bochs io.cc REP_INSB_YbDX: checks as64L, as32L, then 16-bit
    pub fn insb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_insb64(instr)?; } else { self.insb64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_insb32(instr)?; } else { self.insb32(instr)?; }
        } else if rep {
            self.rep_insb16(instr)?;
        } else {
            self.insb16(instr)?;
        }
        Ok(())
    }

    /// INSW dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn insw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_insw64(instr)?; } else { self.insw64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_insw32(instr)?; } else { self.insw32(instr)?; }
        } else if rep {
            self.rep_insw16(instr)?;
        } else {
            self.insw16(instr)?;
        }
        Ok(())
    }

    /// INSD dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn insd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_insd64(instr)?; } else { self.insd64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_insd32(instr)?; } else { self.insd32(instr)?; }
        } else if rep {
            self.rep_insd16(instr)?;
        } else {
            self.insd16(instr)?;
        }
        Ok(())
    }

    /// OUTSB dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn outsb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_outsb64(instr)?; } else { self.outsb64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_outsb32(instr)?; } else { self.outsb32(instr)?; }
        } else if rep {
            self.rep_outsb16(instr)?;
        } else {
            self.outsb16(instr)?;
        }
        Ok(())
    }

    /// OUTSW dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn outsw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_outsw64(instr)?; } else { self.outsw64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_outsw32(instr)?; } else { self.outsw32(instr)?; }
        } else if rep {
            self.rep_outsw16(instr)?;
        } else {
            self.outsw16(instr)?;
        }
        Ok(())
    }

    /// OUTSD dispatch - selects 16/32/64-bit address mode and REP/non-REP form
    pub fn outsd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_outsd64(instr)?; } else { self.outsd64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_outsd32(instr)?; } else { self.outsd32(instr)?; }
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

    /// Read from I/O port.
    ///
    /// When the emulator wires an I/O bus, this dispatches to `BxDevicesC::inp`.
    /// Otherwise it falls back to conservative defaults (useful for unit tests
    /// that don't wire devices and never execute real firmware).
    fn port_in(&mut self, port: u16, len: u8) -> u32 {
        if let Some(mut io_bus) = self.io_bus {
            // SAFETY: `io_bus` is set by the emulator for the duration of execution
            // and cleared afterwards. Single-CPU execution avoids concurrent access.
            let value = unsafe { io_bus.as_mut().inp(port, len) };
            return value;
        }

        // Fallback (no bus wired)
        let value = match len {
            1 => 0xFF,
            2 => 0xFFFF,
            4 => 0xFFFFFFFF,
            _ => 0xFF,
        };
        value
    }

    /// Write to I/O port.
    ///
    /// When the emulator wires an I/O bus, this dispatches to `BxDevicesC::outp`.
    /// Otherwise it is ignored (useful for unit tests without devices).
    fn port_out(&mut self, port: u16, value: u32, len: u8) {
        // Log BIOS diagnostic ports at debug level so RUST_LOG=debug catches them
        // even if something goes wrong before the device handler is reached.
        // Include RIP so we can trace which BIOS function is writing.
        if matches!(port, 0x80 | 0x84 | 0xE9 | 0x402 | 0x403 | 0x500) {
            tracing::debug!(
                "port_out: port={:#06x} value={:#04x} len={} RIP={:#010x}",
                port,
                value as u8,
                len,
                self.rip()
            );
        }
        if let Some(mut io_bus) = self.io_bus {
            // SAFETY: `io_bus` is set by the emulator for the duration of execution
            // and cleared afterwards. Single-CPU execution avoids concurrent access.
            unsafe { io_bus.as_mut().outp(port, value, len) };
        }
    }
}
