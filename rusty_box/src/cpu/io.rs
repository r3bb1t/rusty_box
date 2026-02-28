//! I/O Port Instructions
//!
//! Implements IN and OUT instructions for port I/O.
//! Mirrors `io.cc` from Bochs.

use super::{
    decoder::{BxSegregs, Instruction},
    BxCpuC, BxCpuIdTrait,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// IN AL, imm8 - Input byte from immediate port to AL
    pub fn in_al_ib(&mut self, instr: &Instruction) {
        let port = instr.ib() as u16;
        let value = self.port_in(port, 1) as u8;
        self.set_al(value);
        tracing::trace!("IN AL, {:#x} -> {:#x}", port, value);
    }

    /// IN AX, imm8 - Input word from immediate port to AX
    pub fn in_ax_ib(&mut self, instr: &Instruction) {
        let port = instr.ib() as u16;
        let value = self.port_in(port, 2) as u16;
        self.set_ax(value);
        tracing::trace!("IN AX, {:#x} -> {:#x}", port, value);
    }

    /// IN EAX, imm8 - Input dword from immediate port to EAX
    pub fn in_eax_ib(&mut self, instr: &Instruction) {
        let port = instr.ib() as u16;
        let value = self.port_in(port, 4);
        self.set_eax(value);
        tracing::trace!("IN EAX, {:#x} -> {:#x}", port, value);
    }

    /// OUT imm8, AL - Output byte from AL to immediate port
    pub fn out_ib_al(&mut self, instr: &Instruction) {
        let port = instr.ib() as u16;
        let value = self.al();
        self.port_out(port, value as u32, 1);
        tracing::trace!("OUT {:#x}, AL ({:#x})", port, value);
    }

    /// OUT imm8, AX - Output word from AX to immediate port
    pub fn out_ib_ax(&mut self, instr: &Instruction) {
        let port = instr.ib() as u16;
        let value = self.ax();
        self.port_out(port, value as u32, 2);
        tracing::trace!("OUT {:#x}, AX ({:#x})", port, value);
    }

    /// OUT imm8, EAX - Output dword from EAX to immediate port
    pub fn out_ib_eax(&mut self, instr: &Instruction) {
        let port = instr.ib() as u16;
        let value = self.eax();
        self.port_out(port, value, 4);
        tracing::trace!("OUT {:#x}, EAX ({:#x})", port, value);
    }

    /// IN AL, DX - Input byte from port DX to AL
    pub fn in_al_dx(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let value = self.port_in(port, 1) as u8;
        self.set_al(value);
        tracing::trace!("IN AL, DX ({:#x}) -> {:#x}", port, value);
    }

    /// IN AX, DX - Input word from port DX to AX
    pub fn in_ax_dx(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let value = self.port_in(port, 2) as u16;
        self.set_ax(value);
        tracing::trace!("IN AX, DX ({:#x}) -> {:#x}", port, value);
    }

    /// IN EAX, DX - Input dword from port DX to EAX
    pub fn in_eax_dx(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let value = self.port_in(port, 4);
        self.set_eax(value);
        tracing::trace!("IN EAX, DX ({:#x}) -> {:#x}", port, value);
    }

    /// OUT DX, AL - Output byte from AL to port DX
    pub fn out_dx_al(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let value = self.al();
        self.port_out(port, value as u32, 1);
        tracing::trace!("OUT DX ({:#x}), AL ({:#x})", port, value);
    }

    /// OUT DX, AX - Output word from AX to port DX
    pub fn out_dx_ax(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let value = self.ax();
        self.port_out(port, value as u32, 2);
        tracing::trace!("OUT DX ({:#x}), AX ({:#x})", port, value);
    }

    /// OUT DX, EAX - Output dword from EAX to port DX
    pub fn out_dx_eax(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let value = self.eax();
        self.port_out(port, value, 4);
        tracing::trace!("OUT DX ({:#x}), EAX ({:#x})", port, value);
    }

    // ========================================================================
    // INS/OUTS - String I/O instructions
    // ========================================================================

    // ---- INS: 16-bit address mode (DI/CX, ES segment) ----
    // Bochs INS uses write_virtual_* which translates through paging.

    /// INSB - Input byte from port DX to ES:DI (16-bit address mode)
    pub fn insb16(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let di = self.di() as u32;
        let value = self.port_in(port, 1) as u8;
        let _ = self.write_virtual_byte(BxSegregs::Es, di, value);
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_di(self.di().wrapping_add(1));
        }
    }

    /// INSW - Input word from port DX to ES:DI (16-bit address mode)
    pub fn insw16(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let di = self.di() as u32;
        let value = self.port_in(port, 2) as u16;
        let _ = self.write_virtual_word(BxSegregs::Es, di, value);
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_di(self.di().wrapping_add(2));
        }
    }

    /// INSD - Input dword from port DX to ES:DI (16-bit address mode)
    pub fn insd16(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let di = self.di() as u32;
        let value = self.port_in(port, 4);
        let _ = self.write_virtual_dword(BxSegregs::Es, di, value);
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_di(self.di().wrapping_add(4));
        }
    }

    // ---- INS: 32-bit address mode (EDI/ECX, ES segment) ----

    /// INSB - Input byte from port DX to ES:EDI (32-bit address mode)
    pub fn insb32(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let edi = self.edi();
        let value = self.port_in(port, 1) as u8;
        let _ = self.write_virtual_byte(BxSegregs::Es, edi, value);
        if self.get_df() {
            self.set_edi(self.edi().wrapping_sub(1));
        } else {
            self.set_edi(self.edi().wrapping_add(1));
        }
    }

    /// INSW - Input word from port DX to ES:EDI (32-bit address mode)
    pub fn insw32(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let edi = self.edi();
        let value = self.port_in(port, 2) as u16;
        let _ = self.write_virtual_word(BxSegregs::Es, edi, value);
        if self.get_df() {
            self.set_edi(self.edi().wrapping_sub(2));
        } else {
            self.set_edi(self.edi().wrapping_add(2));
        }
    }

    /// INSD - Input dword from port DX to ES:EDI (32-bit address mode)
    pub fn insd32(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let edi = self.edi();
        let value = self.port_in(port, 4);
        let _ = self.write_virtual_dword(BxSegregs::Es, edi, value);
        if self.get_df() {
            self.set_edi(self.edi().wrapping_sub(4));
        } else {
            self.set_edi(self.edi().wrapping_add(4));
        }
    }

    // ---- OUTS: 16-bit address mode (SI/CX, DS segment) ----
    // Bochs OUTS uses read_virtual_* which translates through paging.

    /// OUTSB - Output byte from DS:SI to port DX (16-bit address mode)
    pub fn outsb16(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let si = self.si() as u32;
        if let Ok(value) = self.read_virtual_byte(BxSegregs::Ds, si) {
            self.port_out(port, value as u32, 1);
        }
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(1));
        } else {
            self.set_si(self.si().wrapping_add(1));
        }
    }

    /// OUTSW - Output word from DS:SI to port DX (16-bit address mode)
    pub fn outsw16(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let si = self.si() as u32;
        if let Ok(value) = self.read_virtual_word(BxSegregs::Ds, si) {
            self.port_out(port, value as u32, 2);
        }
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(2));
        } else {
            self.set_si(self.si().wrapping_add(2));
        }
    }

    /// OUTSD - Output dword from DS:SI to port DX (16-bit address mode)
    pub fn outsd16(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let si = self.si() as u32;
        if let Ok(value) = self.read_virtual_dword(BxSegregs::Ds, si) {
            self.port_out(port, value, 4);
        }
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(4));
        } else {
            self.set_si(self.si().wrapping_add(4));
        }
    }

    // ---- OUTS: 32-bit address mode (ESI/ECX, DS segment) ----

    /// OUTSB - Output byte from DS:ESI to port DX (32-bit address mode)
    pub fn outsb32(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let esi = self.esi();
        if let Ok(value) = self.read_virtual_byte(BxSegregs::Ds, esi) {
            self.port_out(port, value as u32, 1);
        }
        if self.get_df() {
            self.set_esi(self.esi().wrapping_sub(1));
        } else {
            self.set_esi(self.esi().wrapping_add(1));
        }
    }

    /// OUTSW - Output word from DS:ESI to port DX (32-bit address mode)
    pub fn outsw32(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let esi = self.esi();
        if let Ok(value) = self.read_virtual_word(BxSegregs::Ds, esi) {
            self.port_out(port, value as u32, 2);
        }
        if self.get_df() {
            self.set_esi(self.esi().wrapping_sub(2));
        } else {
            self.set_esi(self.esi().wrapping_add(2));
        }
    }

    /// OUTSD - Output dword from DS:ESI to port DX (32-bit address mode)
    pub fn outsd32(&mut self, _instr: &Instruction) {
        let port = self.dx();
        let esi = self.esi();
        if let Ok(value) = self.read_virtual_dword(BxSegregs::Ds, esi) {
            self.port_out(port, value, 4);
        }
        if self.get_df() {
            self.set_esi(self.esi().wrapping_sub(4));
        } else {
            self.set_esi(self.esi().wrapping_add(4));
        }
    }

    // ---- REP INS: 16-bit address mode ----

    /// REP INSB - Repeat input byte from port DX CX times (16-bit addr)
    pub fn rep_insb16(&mut self, instr: &Instruction) {
        let mut cx = self.cx();
        while cx != 0 {
            self.insb16(instr);
            cx -= 1;
            self.set_cx(cx);
        }
    }

    /// REP INSW - Repeat input word from port DX CX times (16-bit addr)
    pub fn rep_insw16(&mut self, instr: &Instruction) {
        let mut cx = self.cx();
        while cx != 0 {
            self.insw16(instr);
            cx -= 1;
            self.set_cx(cx);
        }
    }

    /// REP INSD - Repeat input dword from port DX CX times (16-bit addr)
    pub fn rep_insd16(&mut self, instr: &Instruction) {
        let mut cx = self.cx();
        while cx != 0 {
            self.insd16(instr);
            cx -= 1;
            self.set_cx(cx);
        }
    }

    // ---- REP INS: 32-bit address mode ----

    /// REP INSB - Repeat input byte from port DX ECX times (32-bit addr)
    pub fn rep_insb32(&mut self, instr: &Instruction) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.insb32(instr);
            ecx -= 1;
            self.set_ecx(ecx);
        }
    }

    /// REP INSW - Repeat input word from port DX ECX times (32-bit addr)
    pub fn rep_insw32(&mut self, instr: &Instruction) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.insw32(instr);
            ecx -= 1;
            self.set_ecx(ecx);
        }
    }

    /// REP INSD - Repeat input dword from port DX ECX times (32-bit addr)
    pub fn rep_insd32(&mut self, instr: &Instruction) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.insd32(instr);
            ecx -= 1;
            self.set_ecx(ecx);
        }
    }

    // ---- REP OUTS: 16-bit address mode ----

    /// REP OUTSB - Repeat output byte to port DX CX times (16-bit addr)
    pub fn rep_outsb16(&mut self, instr: &Instruction) {
        let mut cx = self.cx();
        while cx != 0 {
            self.outsb16(instr);
            cx -= 1;
            self.set_cx(cx);
        }
    }

    /// REP OUTSW - Repeat output word to port DX CX times (16-bit addr)
    pub fn rep_outsw16(&mut self, instr: &Instruction) {
        let mut cx = self.cx();
        while cx != 0 {
            self.outsw16(instr);
            cx -= 1;
            self.set_cx(cx);
        }
    }

    /// REP OUTSD - Repeat output dword to port DX CX times (16-bit addr)
    pub fn rep_outsd16(&mut self, instr: &Instruction) {
        let mut cx = self.cx();
        while cx != 0 {
            self.outsd16(instr);
            cx -= 1;
            self.set_cx(cx);
        }
    }

    // ---- REP OUTS: 32-bit address mode ----

    /// REP OUTSB - Repeat output byte to port DX ECX times (32-bit addr)
    pub fn rep_outsb32(&mut self, instr: &Instruction) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.outsb32(instr);
            ecx -= 1;
            self.set_ecx(ecx);
        }
    }

    /// REP OUTSW - Repeat output word to port DX ECX times (32-bit addr)
    pub fn rep_outsw32(&mut self, instr: &Instruction) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.outsw32(instr);
            ecx -= 1;
            self.set_ecx(ecx);
        }
    }

    /// REP OUTSD - Repeat output dword to port DX ECX times (32-bit addr)
    pub fn rep_outsd32(&mut self, instr: &Instruction) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.outsd32(instr);
            ecx -= 1;
            self.set_ecx(ecx);
        }
    }

    // ========================================================================
    // Unified INS/OUTS dispatch methods
    // ========================================================================

    /// INSB dispatch - selects 16/32-bit address mode and REP/non-REP form
    pub fn insb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.as32_l() != 0 {
            if instr.lock_rep_used_value() != 0 {
                self.rep_insb32(instr);
            } else {
                self.insb32(instr);
            }
        } else {
            if instr.lock_rep_used_value() != 0 {
                self.rep_insb16(instr);
            } else {
                self.insb16(instr);
            }
        }
        Ok(())
    }

    /// INSW dispatch - selects 16/32-bit address mode and REP/non-REP form
    pub fn insw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.as32_l() != 0 {
            if instr.lock_rep_used_value() != 0 {
                self.rep_insw32(instr);
            } else {
                self.insw32(instr);
            }
        } else {
            if instr.lock_rep_used_value() != 0 {
                self.rep_insw16(instr);
            } else {
                self.insw16(instr);
            }
        }
        Ok(())
    }

    /// INSD dispatch - selects 16/32-bit address mode and REP/non-REP form
    pub fn insd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.as32_l() != 0 {
            if instr.lock_rep_used_value() != 0 {
                self.rep_insd32(instr);
            } else {
                self.insd32(instr);
            }
        } else {
            if instr.lock_rep_used_value() != 0 {
                self.rep_insd16(instr);
            } else {
                self.insd16(instr);
            }
        }
        Ok(())
    }

    /// OUTSB dispatch - selects 16/32-bit address mode and REP/non-REP form
    pub fn outsb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.as32_l() != 0 {
            if instr.lock_rep_used_value() != 0 {
                self.rep_outsb32(instr);
            } else {
                self.outsb32(instr);
            }
        } else {
            if instr.lock_rep_used_value() != 0 {
                self.rep_outsb16(instr);
            } else {
                self.outsb16(instr);
            }
        }
        Ok(())
    }

    /// OUTSW dispatch - selects 16/32-bit address mode and REP/non-REP form
    pub fn outsw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.as32_l() != 0 {
            if instr.lock_rep_used_value() != 0 {
                self.rep_outsw32(instr);
            } else {
                self.outsw32(instr);
            }
        } else {
            if instr.lock_rep_used_value() != 0 {
                self.rep_outsw16(instr);
            } else {
                self.outsw16(instr);
            }
        }
        Ok(())
    }

    /// OUTSD dispatch - selects 16/32-bit address mode and REP/non-REP form
    pub fn outsd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.as32_l() != 0 {
            if instr.lock_rep_used_value() != 0 {
                self.rep_outsd32(instr);
            } else {
                self.outsd32(instr);
            }
        } else {
            if instr.lock_rep_used_value() != 0 {
                self.rep_outsd16(instr);
            } else {
                self.outsd16(instr);
            }
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
            tracing::trace!("port_in: port={:#06x} len={} -> {:#x}", port, len, value);
            return value;
        }

        // Fallback (no bus wired)
        let value = match len {
            1 => 0xFF,
            2 => 0xFFFF,
            4 => 0xFFFFFFFF,
            _ => 0xFF,
        };
        tracing::trace!(
            "port_in (no bus): port={:#06x} len={} -> {:#x}",
            port,
            len,
            value
        );
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
