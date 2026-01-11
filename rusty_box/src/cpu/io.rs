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
    // Port I/O helpers (stub implementations - need device connection)
    // ========================================================================

    /// Read from I/O port (stub - returns 0xFF for unhandled ports)
    fn port_in(&self, port: u16, len: u8) -> u32 {
        // TODO: Connect to device manager for actual I/O
        // For now, return typical values for common ports
        match port {
            // POST diagnostic port
            0x80 => 0x00,
            // PIC master command/data
            0x20 => 0x00,
            0x21 => 0xFF, // All IRQs masked
            // PIC slave
            0xA0 => 0x00,
            0xA1 => 0xFF,
            // RTC/CMOS
            0x70 => 0x00,
            0x71 => 0x00,
            // Keyboard controller status
            0x64 => 0x00, // Not busy, no data
            // Keyboard data
            0x60 => 0x00,
            // Default
            _ => {
                tracing::trace!("port_in: unhandled port {:#x}, len={}", port, len);
                match len {
                    1 => 0xFF,
                    2 => 0xFFFF,
                    4 => 0xFFFFFFFF,
                    _ => 0xFF,
                }
            }
        }
    }

    /// Write to I/O port (stub - logs the operation)
    fn port_out(&self, port: u16, value: u32, len: u8) {
        // TODO: Connect to device manager for actual I/O
        match port {
            // POST diagnostic port - commonly written during BIOS POST
            0x80 => tracing::debug!("POST code: {:#04x}", value as u8),
            // PIC master
            0x20 | 0x21 => tracing::trace!("PIC master write: port={:#x}, value={:#x}", port, value),
            // PIC slave
            0xA0 | 0xA1 => tracing::trace!("PIC slave write: port={:#x}, value={:#x}", port, value),
            // RTC/CMOS
            0x70 | 0x71 => tracing::trace!("CMOS write: port={:#x}, value={:#x}", port, value),
            // Keyboard controller
            0x60 | 0x64 => tracing::trace!("Keyboard write: port={:#x}, value={:#x}", port, value),
            // Default
            _ => tracing::trace!("port_out: unhandled port {:#x}, value={:#x}, len={}", port, value, len),
        }
    }
}

