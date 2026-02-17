//! I/O Port Instructions
//!
//! Implements IN and OUT instructions for port I/O.
//! Mirrors `io.cc` from Bochs.

use super::{BxCpuC, BxCpuIdTrait, decoder::BxInstructionGenerated};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// IN AL, imm8 - Input byte from immediate port to AL
    pub fn in_al_ib(&mut self, instr: &BxInstructionGenerated) {
        let port = instr.ib() as u16;
        let value = self.port_in(port, 1) as u8;
        self.set_al(value);
        tracing::trace!("IN AL, {:#x} -> {:#x}", port, value);
    }

    /// IN AX, imm8 - Input word from immediate port to AX
    pub fn in_ax_ib(&mut self, instr: &BxInstructionGenerated) {
        let port = instr.ib() as u16;
        let value = self.port_in(port, 2) as u16;
        self.set_ax(value);
        tracing::trace!("IN AX, {:#x} -> {:#x}", port, value);
    }

    /// IN EAX, imm8 - Input dword from immediate port to EAX
    pub fn in_eax_ib(&mut self, instr: &BxInstructionGenerated) {
        let port = instr.ib() as u16;
        let value = self.port_in(port, 4);
        self.set_eax(value);
        tracing::trace!("IN EAX, {:#x} -> {:#x}", port, value);
    }

    /// OUT imm8, AL - Output byte from AL to immediate port
    pub fn out_ib_al(&mut self, instr: &BxInstructionGenerated) {
        let port = instr.ib() as u16;
        let value = self.al();
        self.port_out(port, value as u32, 1);
        tracing::trace!("OUT {:#x}, AL ({:#x})", port, value);
    }

    /// OUT imm8, AX - Output word from AX to immediate port
    pub fn out_ib_ax(&mut self, instr: &BxInstructionGenerated) {
        let port = instr.ib() as u16;
        let value = self.ax();
        self.port_out(port, value as u32, 2);
        tracing::trace!("OUT {:#x}, AX ({:#x})", port, value);
    }

    /// OUT imm8, EAX - Output dword from EAX to immediate port
    pub fn out_ib_eax(&mut self, instr: &BxInstructionGenerated) {
        let port = instr.ib() as u16;
        let value = self.eax();
        self.port_out(port, value, 4);
        tracing::trace!("OUT {:#x}, EAX ({:#x})", port, value);
    }

    /// IN AL, DX - Input byte from port DX to AL
    pub fn in_al_dx(&mut self, _instr: &BxInstructionGenerated) {
        let port = self.dx();
        let value = self.port_in(port, 1) as u8;
        self.set_al(value);
        tracing::trace!("IN AL, DX ({:#x}) -> {:#x}", port, value);
    }

    /// IN AX, DX - Input word from port DX to AX
    pub fn in_ax_dx(&mut self, _instr: &BxInstructionGenerated) {
        let port = self.dx();
        let value = self.port_in(port, 2) as u16;
        self.set_ax(value);
        tracing::trace!("IN AX, DX ({:#x}) -> {:#x}", port, value);
    }

    /// IN EAX, DX - Input dword from port DX to EAX
    pub fn in_eax_dx(&mut self, _instr: &BxInstructionGenerated) {
        let port = self.dx();
        let value = self.port_in(port, 4);
        self.set_eax(value);
        tracing::trace!("IN EAX, DX ({:#x}) -> {:#x}", port, value);
    }

    /// OUT DX, AL - Output byte from AL to port DX
    pub fn out_dx_al(&mut self, _instr: &BxInstructionGenerated) {
        let port = self.dx();
        let value = self.al();
        self.port_out(port, value as u32, 1);
        tracing::trace!("OUT DX ({:#x}), AL ({:#x})", port, value);
    }

    /// OUT DX, AX - Output word from AX to port DX
    pub fn out_dx_ax(&mut self, _instr: &BxInstructionGenerated) {
        let port = self.dx();
        let value = self.ax();
        self.port_out(port, value as u32, 2);
        tracing::trace!("OUT DX ({:#x}), AX ({:#x})", port, value);
    }

    /// OUT DX, EAX - Output dword from EAX to port DX
    pub fn out_dx_eax(&mut self, _instr: &BxInstructionGenerated) {
        let port = self.dx();
        let value = self.eax();
        self.port_out(port, value, 4);
        tracing::trace!("OUT DX ({:#x}), EAX ({:#x})", port, value);
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
        if let Some(io_bus) = self.io_bus {
            // SAFETY: `io_bus` is set by the emulator for the duration of execution
            // and cleared afterwards. Single-CPU execution avoids concurrent access.
            let value = unsafe { io_bus.as_ref().inp(port, len) };
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
        tracing::trace!("port_in (no bus): port={:#06x} len={} -> {:#x}", port, len, value);
        value
    }

    /// Write to I/O port.
    ///
    /// When the emulator wires an I/O bus, this dispatches to `BxDevicesC::outp`.
    /// Otherwise it is ignored (useful for unit tests without devices).
    fn port_out(&mut self, port: u16, value: u32, len: u8) {
        if let Some(mut io_bus) = self.io_bus {
            // SAFETY: `io_bus` is set by the emulator for the duration of execution
            // and cleared afterwards. Single-CPU execution avoids concurrent access.
            unsafe { io_bus.as_mut().outp(port, value, len) };
        }
    }
}

