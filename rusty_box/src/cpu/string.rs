//! String operations for x86 CPU emulation
//!
//! Based on Bochs string.cc
//! Copyright (C) 2001-2019 The Bochs Project
//!
//! Implements MOVS, STOS, LODS, CMPS, SCAS instructions
//!
//! Both 16-bit and 32-bit address variants use virtual memory access with
//! segment limit checks and paging translation (required for protected mode
//! with paging).

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

use crate::{
    config::BxPhyAddress,
    cpu::{icache::BxPageWriteStampTable, rusty_box::MemoryAccessType},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Helper: Get direction flag (DF)
    // =========================================================================

    /// Returns true if direction flag is set (decrement mode)
    #[inline]
    pub(super) fn get_df(&self) -> bool {
        self.eflags.contains(EFlags::DF)
    }

    // =========================================================================
    // MOVSB - Move String Byte
    // =========================================================================

    /// MOVSB - Move byte from DS:SI to ES:DI (16-bit address mode)
    pub fn movsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let si = self.si() as u32;
        let di = self.di() as u32;

        let byte = self.read_virtual_byte(BxSegregs::from(instr.seg()), si)?;
        self.write_virtual_byte(BxSegregs::Es, di, byte)?;

        if self.get_df() {
            self.set_si(self.si().wrapping_sub(1));
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_si(self.si().wrapping_add(1));
            self.set_di(self.di().wrapping_add(1));
        }

        Ok(())
    }

    /// MOVSB - Move byte from DS:ESI to ES:EDI (32-bit address mode)
    /// Uses virtual memory access for proper segment limits + paging translation.
    pub fn movsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let esi = self.esi();
        let edi = self.edi();

        let byte = self.read_virtual_byte(BxSegregs::from(instr.seg()), esi)?;
        self.write_virtual_byte(BxSegregs::Es, edi, byte)?;

        let increment: u32 = if self.get_df() { 0xFFFFFFFF } else { 1 };
        self.set_rsi(esi.wrapping_add(increment) as u64);
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// MOVSW - Move word from DS:SI to ES:DI (16-bit address mode)
    pub fn movsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let si = self.si() as u32;
        let di = self.di() as u32;

        let word = self.read_virtual_word(BxSegregs::from(instr.seg()), si)?;
        self.write_virtual_word(BxSegregs::Es, di, word)?;

        if self.get_df() {
            self.set_si(self.si().wrapping_sub(2));
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_si(self.si().wrapping_add(2));
            self.set_di(self.di().wrapping_add(2));
        }

        Ok(())
    }

    /// MOVSW - Move word from DS:ESI to ES:EDI (32-bit address mode)
    pub fn movsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let esi = self.esi();
        let edi = self.edi();

        let word = self.read_virtual_word(BxSegregs::from(instr.seg()), esi)?;
        self.write_virtual_word(BxSegregs::Es, edi, word)?;

        let increment: u32 = if self.get_df() { 0xFFFFFFFE } else { 2 };
        self.set_rsi(esi.wrapping_add(increment) as u64);
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// MOVSD - Move dword from DS:SI to ES:DI (16-bit address mode)
    pub fn movsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let si = self.si() as u32;
        let di = self.di() as u32;

        let dword = self.read_virtual_dword(BxSegregs::from(instr.seg()), si)?;
        self.write_virtual_dword(BxSegregs::Es, di, dword)?;

        if self.get_df() {
            self.set_si(self.si().wrapping_sub(4));
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_si(self.si().wrapping_add(4));
            self.set_di(self.di().wrapping_add(4));
        }

        Ok(())
    }

    /// MOVSD - Move dword from DS:ESI to ES:EDI (32-bit address mode)
    pub fn movsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let esi = self.esi();
        let edi = self.edi();

        let dword = self.read_virtual_dword(BxSegregs::from(instr.seg()), esi)?;
        self.write_virtual_dword(BxSegregs::Es, edi, dword)?;

        let increment: u32 = if self.get_df() { 0xFFFFFFFC } else { 4 };
        self.set_rsi(esi.wrapping_add(increment) as u64);
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    // =========================================================================
    // STOSB - Store String Byte
    // =========================================================================

    /// STOSB - Store AL at ES:DI (16-bit address mode)
    pub fn stosb16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let di = self.di() as u32;
        let al = self.al();

        self.write_virtual_byte(BxSegregs::Es, di, al)?;

        if self.get_df() {
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_di(self.di().wrapping_add(1));
        }

        Ok(())
    }

    /// STOSB - Store AL at ES:EDI (32-bit address mode)
    pub fn stosb32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let edi = self.edi();
        let al = self.al();

        self.write_virtual_byte(BxSegregs::Es, edi, al)?;

        let increment: u32 = if self.get_df() { 0xFFFFFFFF } else { 1 };
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// STOSW - Store AX at ES:DI (16-bit address mode)
    pub fn stosw16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let di = self.di() as u32;
        let ax = self.ax();

        self.write_virtual_word(BxSegregs::Es, di, ax)?;

        if self.get_df() {
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_di(self.di().wrapping_add(2));
        }

        Ok(())
    }

    /// STOSW - Store AX at ES:EDI (32-bit address mode)
    pub fn stosw32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let edi = self.edi();
        let ax = self.ax();

        self.write_virtual_word(BxSegregs::Es, edi, ax)?;

        let increment: u32 = if self.get_df() { 0xFFFFFFFE } else { 2 };
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// STOSD - Store EAX at ES:DI (16-bit address mode)
    pub fn stosd16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let di = self.di() as u32;
        let eax = self.eax();

        self.write_virtual_dword(BxSegregs::Es, di, eax)?;

        if self.get_df() {
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_di(self.di().wrapping_add(4));
        }

        Ok(())
    }

    /// STOSD - Store EAX at ES:EDI (32-bit address mode)
    pub fn stosd32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let edi = self.edi();
        let eax = self.eax();

        self.write_virtual_dword(BxSegregs::Es, edi, eax)?;

        let increment: u32 = if self.get_df() { 0xFFFFFFFC } else { 4 };
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    // =========================================================================
    // LODSB - Load String Byte
    // =========================================================================

    /// LODSB - Load byte from DS:SI into AL (16-bit address mode)
    pub fn lodsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let si = self.si() as u32;

        let byte = self.read_virtual_byte(BxSegregs::from(instr.seg()), si)?;

        self.set_al(byte);

        if self.get_df() {
            self.set_si(self.si().wrapping_sub(1));
        } else {
            self.set_si(self.si().wrapping_add(1));
        }

        Ok(())
    }

    /// LODSB - Load byte from DS:ESI into AL (32-bit address mode)
    pub fn lodsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let esi = self.esi();

        let byte = self.read_virtual_byte(BxSegregs::from(instr.seg()), esi)?;
        self.set_al(byte);

        let increment: u32 = if self.get_df() { 0xFFFFFFFF } else { 1 };
        self.set_rsi(esi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// LODSW - Load word from DS:SI into AX (16-bit address mode)
    pub fn lodsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let si = self.si() as u32;

        let word = self.read_virtual_word(BxSegregs::from(instr.seg()), si)?;

        self.set_ax(word);

        if self.get_df() {
            self.set_si(self.si().wrapping_sub(2));
        } else {
            self.set_si(self.si().wrapping_add(2));
        }

        Ok(())
    }

    /// LODSW - Load word from DS:ESI into AX (32-bit address mode)
    pub fn lodsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let esi = self.esi();

        let word = self.read_virtual_word(BxSegregs::from(instr.seg()), esi)?;
        self.set_ax(word);

        let increment: u32 = if self.get_df() { 0xFFFFFFFE } else { 2 };
        self.set_rsi(esi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// LODSD - Load dword from DS:SI into EAX (16-bit address mode)
    pub fn lodsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let si = self.si() as u32;

        let dword = self.read_virtual_dword(BxSegregs::from(instr.seg()), si)?;

        self.set_eax(dword);

        if self.get_df() {
            self.set_si(self.si().wrapping_sub(4));
        } else {
            self.set_si(self.si().wrapping_add(4));
        }

        Ok(())
    }

    /// LODSD - Load dword from DS:ESI into EAX (32-bit address mode)
    pub fn lodsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let esi = self.esi();

        let dword = self.read_virtual_dword(BxSegregs::from(instr.seg()), esi)?;
        self.set_eax(dword);

        let increment: u32 = if self.get_df() { 0xFFFFFFFC } else { 4 };
        self.set_rsi(esi.wrapping_add(increment) as u64);

        Ok(())
    }

    // =========================================================================
    // CMPSB - Compare String Byte
    // =========================================================================

    /// CMPSB - Compare bytes at DS:SI and ES:DI (16-bit address mode)
    pub fn cmpsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let si = self.si() as u32;
        let di = self.di() as u32;

        let op1 = self.read_virtual_byte(BxSegregs::from(instr.seg()), si)?;
        let op2 = self.read_virtual_byte(BxSegregs::Es, di)?;

        let result = op1.wrapping_sub(op2);
        self.update_flags_sub8(op1, op2, result);

        if self.get_df() {
            self.set_si(self.si().wrapping_sub(1));
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_si(self.si().wrapping_add(1));
            self.set_di(self.di().wrapping_add(1));
        }

        Ok(())
    }

    /// CMPSB - Compare bytes at DS:ESI and ES:EDI (32-bit address mode)
    pub fn cmpsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let esi = self.esi();
        let edi = self.edi();

        let op1 = self.read_virtual_byte(BxSegregs::from(instr.seg()), esi)?;
        let op2 = self.read_virtual_byte(BxSegregs::Es, edi)?;

        let result = op1.wrapping_sub(op2);
        self.update_flags_sub8(op1, op2, result);

        let increment: u32 = if self.get_df() { 0xFFFFFFFF } else { 1 };
        self.set_rsi(esi.wrapping_add(increment) as u64);
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// CMPSW - Compare words at DS:SI and ES:DI (16-bit address mode)
    pub fn cmpsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let si = self.si() as u32;
        let di = self.di() as u32;

        let op1 = self.read_virtual_word(BxSegregs::from(instr.seg()), si)?;
        let op2 = self.read_virtual_word(BxSegregs::Es, di)?;

        let result = op1.wrapping_sub(op2);
        self.update_flags_sub16(op1, op2, result);

        if self.get_df() {
            self.set_si(self.si().wrapping_sub(2));
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_si(self.si().wrapping_add(2));
            self.set_di(self.di().wrapping_add(2));
        }

        Ok(())
    }

    /// CMPSW - Compare words at DS:ESI and ES:EDI (32-bit address mode)
    pub fn cmpsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let esi = self.esi();
        let edi = self.edi();

        let op1 = self.read_virtual_word(BxSegregs::from(instr.seg()), esi)?;
        let op2 = self.read_virtual_word(BxSegregs::Es, edi)?;

        let result = op1.wrapping_sub(op2);
        self.update_flags_sub16(op1, op2, result);

        let increment: u32 = if self.get_df() { 0xFFFFFFFE } else { 2 };
        self.set_rsi(esi.wrapping_add(increment) as u64);
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// CMPSD - Compare dwords at DS:SI and ES:DI (16-bit address mode)
    pub fn cmpsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let si = self.si() as u32;
        let di = self.di() as u32;

        let op1 = self.read_virtual_dword(BxSegregs::from(instr.seg()), si)?;
        let op2 = self.read_virtual_dword(BxSegregs::Es, di)?;

        let result = op1.wrapping_sub(op2);
        self.update_flags_sub32(op1, op2, result);

        if self.get_df() {
            self.set_si(self.si().wrapping_sub(4));
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_si(self.si().wrapping_add(4));
            self.set_di(self.di().wrapping_add(4));
        }

        Ok(())
    }

    /// CMPSD - Compare dwords at DS:ESI and ES:EDI (32-bit address mode)
    pub fn cmpsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let esi = self.esi();
        let edi = self.edi();

        let op1 = self.read_virtual_dword(BxSegregs::from(instr.seg()), esi)?;
        let op2 = self.read_virtual_dword(BxSegregs::Es, edi)?;

        let result = op1.wrapping_sub(op2);
        self.update_flags_sub32(op1, op2, result);

        let increment: u32 = if self.get_df() { 0xFFFFFFFC } else { 4 };
        self.set_rsi(esi.wrapping_add(increment) as u64);
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    // =========================================================================
    // SCASB - Scan String Byte
    // =========================================================================

    /// SCASB - Compare AL with byte at ES:DI (16-bit address mode)
    pub fn scasb16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let di = self.di() as u32;
        let al = self.al();

        let op2 = self.read_virtual_byte(BxSegregs::Es, di)?;

        let result = al.wrapping_sub(op2);
        self.update_flags_sub8(al, op2, result);

        if self.get_df() {
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_di(self.di().wrapping_add(1));
        }

        Ok(())
    }

    /// SCASB - Compare AL with byte at ES:EDI (32-bit address mode)
    pub fn scasb32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let edi = self.edi();
        let al = self.al();

        let op2 = self.read_virtual_byte(BxSegregs::Es, edi)?;

        let result = al.wrapping_sub(op2);
        self.update_flags_sub8(al, op2, result);

        let increment: u32 = if self.get_df() { 0xFFFFFFFF } else { 1 };
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// SCASW - Compare AX with word at ES:DI (16-bit address mode)
    pub fn scasw16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let di = self.di() as u32;
        let ax = self.ax();

        let op2 = self.read_virtual_word(BxSegregs::Es, di)?;

        let result = ax.wrapping_sub(op2);
        self.update_flags_sub16(ax, op2, result);

        if self.get_df() {
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_di(self.di().wrapping_add(2));
        }

        Ok(())
    }

    /// SCASW - Compare AX with word at ES:EDI (32-bit address mode)
    pub fn scasw32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let edi = self.edi();
        let ax = self.ax();

        let op2 = self.read_virtual_word(BxSegregs::Es, edi)?;

        let result = ax.wrapping_sub(op2);
        self.update_flags_sub16(ax, op2, result);

        let increment: u32 = if self.get_df() { 0xFFFFFFFE } else { 2 };
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    /// SCASD - Compare EAX with dword at ES:DI (16-bit address mode)
    pub fn scasd16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let di = self.di() as u32;
        let eax = self.eax();

        let op2 = self.read_virtual_dword(BxSegregs::Es, di)?;

        let result = eax.wrapping_sub(op2);
        self.update_flags_sub32(eax, op2, result);

        if self.get_df() {
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_di(self.di().wrapping_add(4));
        }

        Ok(())
    }

    /// SCASD - Compare EAX with dword at ES:EDI (32-bit address mode)
    pub fn scasd32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let edi = self.edi();
        let eax = self.eax();

        let op2 = self.read_virtual_dword(BxSegregs::Es, edi)?;

        let result = eax.wrapping_sub(op2);
        self.update_flags_sub32(eax, op2, result);

        let increment: u32 = if self.get_df() { 0xFFFFFFFC } else { 4 };
        self.set_rdi(edi.wrapping_add(increment) as u64);

        Ok(())
    }

    // =========================================================================
    // REP prefix handling — 16-bit address mode
    // =========================================================================

    /// REP MOVSB CX times (16-bit)
    pub fn rep_movsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.movsb16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if cx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP MOVSW CX times (16-bit)
    pub fn rep_movsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.movsw16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if cx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP MOVSD CX times (16-bit)
    pub fn rep_movsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.movsd16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if cx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP STOSB CX times (16-bit)
    pub fn rep_stosb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.stosb16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if cx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP STOSW CX times (16-bit)
    pub fn rep_stosw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.stosw16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if cx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP STOSD CX times (16-bit)
    pub fn rep_stosd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.stosd16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if cx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP LODSB CX times (16-bit)
    pub fn rep_lodsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.lodsb16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if cx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP LODSW CX times (16-bit)
    pub fn rep_lodsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.lodsw16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if cx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP LODSD CX times (16-bit)
    pub fn rep_lodsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.lodsd16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if cx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE CMPSB CX (16-bit)
    pub fn repe_cmpsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsb16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE CMPSB CX (16-bit)
    pub fn repne_cmpsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsb16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE CMPSW CX (16-bit)
    pub fn repe_cmpsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsw16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE CMPSW CX (16-bit)
    pub fn repne_cmpsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsw16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE CMPSD CX (16-bit)
    pub fn repe_cmpsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsd16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE CMPSD CX (16-bit)
    pub fn repne_cmpsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsd16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE SCASB CX (16-bit)
    pub fn repe_scasb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasb16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE SCASB CX (16-bit)
    pub fn repne_scasb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasb16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE SCASW CX (16-bit)
    pub fn repe_scasw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasw16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE SCASW CX (16-bit)
    pub fn repne_scasw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasw16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE SCASD CX (16-bit)
    pub fn repe_scasd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasd16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE SCASD CX (16-bit)
    pub fn repne_scasd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasd16(instr)?;
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() || cx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // REP prefix handling — 32-bit address mode (with paging translation)
    // =========================================================================

    /// REP MOVSB ECX times (32-bit)
    pub fn rep_movsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep: try bulk memcpy when DF=0 (Bochs faststring.cc:91-92)
        while ecx != 0 && !df && self.async_event == 0 {
            let esi = self.esi();
            let edi = self.edi();
            let src_laddr = self.get_laddr32(seg as usize, esi) as u64;
            let dst_laddr = self.get_laddr32(BxSegregs::Es as usize, edi) as u64;

            if let (Some((src_ptr, src_rem)), Some((dst_ptr, dst_rem))) =
                (self.get_host_read_ptr(src_laddr), self.get_host_write_ptr(dst_laddr))
            {
                let count = (ecx as usize).min(src_rem).min(dst_rem)
                    .min(self.ticks_left_next_event() as usize);
                if count > 0 {
                    // Bochs faststring.cc:99-103 — forward byte-by-byte loop.
                    // Must NOT use memcpy: overlapping regions (LZ decompression)
                    // rely on reading already-written bytes during forward copy.
                    unsafe {
                        for j in 0..count {
                            *dst_ptr.add(j) = *src_ptr.add(j);
                        }
                    }
                    self.set_rsi(esi.wrapping_add(count as u32) as u64);
                    self.set_rdi(edi.wrapping_add(count as u32) as u64);
                    self.icount += count as u64 - 1;
                    self.tickn_fastrep(count);
                    ecx -= count as u32;
                    self.set_ecx(ecx);
                    if ecx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.set_rcx(self.ecx() as u64);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while ecx != 0 {
            self.movsb32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP MOVSW ECX times (32-bit)
    pub fn rep_movsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep: try bulk memcpy when DF=0 (Bochs FastRepMOVSB with granularity=2)
        while ecx != 0 && !df && self.async_event == 0 {
            let esi = self.esi();
            let edi = self.edi();
            let src_laddr = self.get_laddr32(seg as usize, esi) as u64;
            let dst_laddr = self.get_laddr32(BxSegregs::Es as usize, edi) as u64;

            if let (Some((src_ptr, src_rem)), Some((dst_ptr, dst_rem))) =
                (self.get_host_read_ptr(src_laddr), self.get_host_write_ptr(dst_laddr))
            {
                let max_bytes = (ecx as usize) * 2;
                let count_bytes = max_bytes.min(src_rem).min(dst_rem)
                    .min(self.ticks_left_next_event() as usize) & !1;
                let count_words = count_bytes / 2;
                if count_words > 0 {
                    unsafe {
                        // Forward byte-by-byte (Bochs faststring.cc:99-103)
                        for j in 0..count_bytes {
                            *dst_ptr.add(j) = *src_ptr.add(j);
                        }
                    }
                    self.set_rsi(esi.wrapping_add(count_bytes as u32) as u64);
                    self.set_rdi(edi.wrapping_add(count_bytes as u32) as u64);
                    self.icount += count_words as u64 - 1;
                    self.tickn_fastrep(count_words);
                    ecx -= count_words as u32;
                    self.set_ecx(ecx);
                    if ecx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.set_rcx(self.ecx() as u64);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while ecx != 0 {
            self.movsw32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP MOVSD ECX times (32-bit)
    pub fn rep_movsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep: try bulk memcpy when DF=0 (Bochs faststring.cc:91-92 granularity=4)
        while ecx != 0 && !df && self.async_event == 0 {
            let esi = self.esi();
            let edi = self.edi();
            let src_laddr = self.get_laddr32(seg as usize, esi) as u64;
            let dst_laddr = self.get_laddr32(BxSegregs::Es as usize, edi) as u64;

            if let (Some((src_ptr, src_rem)), Some((dst_ptr, dst_rem))) =
                (self.get_host_read_ptr(src_laddr), self.get_host_write_ptr(dst_laddr))
            {
                let max_bytes = (ecx as usize) * 4;
                let count_bytes = max_bytes.min(src_rem).min(dst_rem)
                    .min(self.ticks_left_next_event() as usize) & !3;
                let count_dwords = count_bytes / 4;
                if count_dwords > 0 {
                    unsafe {
                        // Forward byte-by-byte (Bochs faststring.cc:99-103)
                        for j in 0..count_bytes {
                            *dst_ptr.add(j) = *src_ptr.add(j);
                        }
                    }
                    self.set_rsi(esi.wrapping_add(count_bytes as u32) as u64);
                    self.set_rdi(edi.wrapping_add(count_bytes as u32) as u64);
                    self.icount += count_dwords as u64 - 1;
                    self.tickn_fastrep(count_dwords);
                    ecx -= count_dwords as u32;
                    self.set_ecx(ecx);
                    if ecx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.set_rcx(self.ecx() as u64);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while ecx != 0 {
            self.movsd32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP STOSB ECX times (32-bit)
    pub fn rep_stosb32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let al = self.al();

        // FastRep: try bulk memset when DF=0 (Bochs faststring.cc:146-147)
        while ecx != 0 && !df && self.async_event == 0 {
            let edi = self.edi();
            let dst_laddr = self.get_laddr32(BxSegregs::Es as usize, edi) as u64;

            if let Some((dst_ptr, dst_rem)) = self.get_host_write_ptr(dst_laddr) {
                let count = (ecx as usize).min(dst_rem)
                    .min(self.ticks_left_next_event() as usize);
                if count > 0 {
                    unsafe {
                        core::ptr::write_bytes(dst_ptr, al, count);
                    }
                    self.set_rdi(edi.wrapping_add(count as u32) as u64);
                    self.icount += count as u64 - 1;
                    self.tickn_fastrep(count);
                    ecx -= count as u32;
                    self.set_ecx(ecx);
                    if ecx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.set_rcx(self.ecx() as u64);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while ecx != 0 {
            self.stosb32(_instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP STOSW ECX times (32-bit)
    pub fn rep_stosw32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let ax = self.ax();

        // FastRep: try bulk word fill when DF=0 (Bochs faststring.cc:198-199)
        while ecx != 0 && !df && self.async_event == 0 {
            let edi = self.edi();
            let dst_laddr = self.get_laddr32(BxSegregs::Es as usize, edi) as u64;

            if let Some((dst_ptr, dst_rem)) = self.get_host_write_ptr(dst_laddr) {
                let max_bytes = (ecx as usize) * 2;
                let count_bytes = max_bytes.min(dst_rem) & !1;
                let count_words = (count_bytes / 2)
                    .min(self.ticks_left_next_event() as usize);
                if count_words > 0 {
                    let dst_slice = unsafe {
                        core::slice::from_raw_parts_mut(dst_ptr as *mut u16, count_words)
                    };
                    for w in dst_slice.iter_mut() {
                        *w = ax;
                    }
                    self.set_rdi(edi.wrapping_add((count_words * 2) as u32) as u64);
                    self.icount += count_words as u64 - 1;
                    self.tickn_fastrep(count_words);
                    ecx -= count_words as u32;
                    self.set_ecx(ecx);
                    if ecx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.set_rcx(self.ecx() as u64);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while ecx != 0 {
            self.stosw32(_instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP STOSD ECX times (32-bit)
    pub fn rep_stosd32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let eax = self.eax();

        // FastRep: try bulk dword fill when DF=0 (Bochs faststring.cc:250-251)
        while ecx != 0 && !df && self.async_event == 0 {
            let edi = self.edi();
            let dst_laddr = self.get_laddr32(BxSegregs::Es as usize, edi) as u64;

            if let Some((dst_ptr, dst_rem)) = self.get_host_write_ptr(dst_laddr) {
                let max_bytes = (ecx as usize) * 4;
                let count_bytes = max_bytes.min(dst_rem) & !3;
                let count_dwords = (count_bytes / 4)
                    .min(self.ticks_left_next_event() as usize);
                if count_dwords > 0 {
                    let dst_slice = unsafe {
                        core::slice::from_raw_parts_mut(dst_ptr as *mut u32, count_dwords)
                    };
                    for d in dst_slice.iter_mut() {
                        *d = eax;
                    }
                    self.set_rdi(edi.wrapping_add((count_dwords * 4) as u32) as u64);
                    self.icount += count_dwords as u64 - 1;
                    self.tickn_fastrep(count_dwords);
                    ecx -= count_dwords as u32;
                    self.set_ecx(ecx);
                    if ecx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.set_rcx(self.ecx() as u64);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while ecx != 0 {
            self.stosd32(_instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP LODSB ECX times (32-bit)
    pub fn rep_lodsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.lodsb32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP LODSW ECX times (32-bit)
    pub fn rep_lodsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.lodsw32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REP LODSD ECX times (32-bit)
    pub fn rep_lodsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.lodsd32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if ecx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE CMPSB ECX (32-bit)
    pub fn repe_cmpsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.cmpsb32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if !self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE CMPSB ECX (32-bit)
    pub fn repne_cmpsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.cmpsb32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE CMPSW ECX (32-bit)
    pub fn repe_cmpsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.cmpsw32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if !self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE CMPSW ECX (32-bit)
    pub fn repne_cmpsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.cmpsw32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE CMPSD ECX (32-bit)
    pub fn repe_cmpsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.cmpsd32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if !self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE CMPSD ECX (32-bit)
    pub fn repne_cmpsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.cmpsd32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE SCASB ECX (32-bit)
    pub fn repe_scasb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.scasb32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if !self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE SCASB ECX (32-bit)
    pub fn repne_scasb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.scasb32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE SCASW ECX (32-bit)
    pub fn repe_scasw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.scasw32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if !self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE SCASW ECX (32-bit)
    pub fn repne_scasw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.scasw32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPE SCASD ECX (32-bit)
    pub fn repe_scasd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.scasd32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if !self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE SCASD ECX (32-bit)
    pub fn repne_scasd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.scasd32(instr)?;
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
            if self.get_zf() || ecx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.set_rcx(self.ecx() as u64);
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // Memory access helpers using the stored memory pointer
    // =========================================================================

    #[inline(always)]
    pub(super) fn mem_read_byte(&self, addr: u64) -> u8 {
        // Fast path: direct host pointer for plain RAM (bypass get_host_mem_addr).
        // This matches what Bochs does via hostPageAddr in TLB entries — the vast
        // majority of physical accesses hit RAM and can be served with a single
        // pointer dereference.  We apply A20 masking and check the address is in
        // the plain-RAM range (below VGA at 0xA0000, or above BIOS shadow at 0x100000).
        let a20_addr = (addr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null()
            && (a20_addr < 0xA0000 || (a20_addr >= 0x100000 && a20_addr < self.mem_host_len))
        {
            return unsafe { *host_base.add(a20_addr) };
        }

        self.mem_read_byte_slow(addr)
    }

    /// Slow path for mem_read_byte: MMIO/VGA/ROM through memory system handlers.
    /// Separated to keep the inlined fast path small for better icache utilization.
    #[cold]
    #[inline(never)]
    fn mem_read_byte_slow(&self, addr: u64) -> u8 {
        // LAPIC MMIO intercept at byte level (fallback for non-dword accesses)
        #[cfg(feature = "bx_support_apic")]
        {
            let a20_addr = (addr & self.a20_mask) as BxPhyAddress;
            if self.lapic.is_selected(a20_addr) {
                // Read aligned dword, extract requested byte
                let aligned = a20_addr & !0x3;
                let dword = self.lapic.read(aligned, 4);
                let byte_offset = (a20_addr & 0x3) as u32;
                return (dword >> (byte_offset * 8)) as u8;
            }
        }
        if let Some(mem_bus) = self.mem_bus {
            let mem = unsafe { &mut *mem_bus.as_ptr() };
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let paddr: BxPhyAddress = addr as BxPhyAddress;

            if let Ok(Some(slice)) =
                mem.get_host_mem_addr(paddr, MemoryAccessType::Read, &[cpu_ref])
            {
                let val = slice.get(0).copied().unwrap_or(0);
                return val;
            }

            let mut data = [0u8; 1];
            if mem
                .read_physical_page(&[cpu_ref], paddr, 1, &mut data)
                .is_ok()
            {
                return data[0];
            }

            return 0;
        }

        // Fallback: raw pointer access (best-effort) when not running inside cpu_loop.
        if let Some(ptr) = self.mem_ptr {
            let addr = addr as usize;
            if addr < self.mem_len {
                return unsafe { *ptr.add(addr) };
            }
        }

        0
    }

    #[inline(always)]
    pub(super) fn mem_write_byte(&mut self, addr: u64, value: u8) {
        // Fast path: direct host pointer for plain RAM (bypass get_host_mem_addr).
        let a20_addr = (addr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null()
            && (a20_addr < 0xA0000 || (a20_addr >= 0x100000 && a20_addr < self.mem_host_len))
        {
            unsafe { *host_base.add(a20_addr) = value };
            self.i_cache.smc_write_check(a20_addr as BxPhyAddress, 1);
            return;
        }

        self.mem_write_byte_slow(addr, value);
    }

    /// Slow path for mem_write_byte: MMIO/VGA/ROM through memory system handlers.
    /// Separated to keep the inlined fast path small for better icache utilization.
    #[cold]
    #[inline(never)]
    fn mem_write_byte_slow(&mut self, addr: u64, value: u8) {
        // LAPIC MMIO intercept at byte level (fallback for non-dword accesses)
        #[cfg(feature = "bx_support_apic")]
        {
            let a20_addr = (addr & self.a20_mask) as BxPhyAddress;
            if self.lapic.is_selected(a20_addr) {
                // Byte-level write to LAPIC: read-modify-write the aligned dword.
                // In practice, LAPIC is always accessed as dword — this is a safety net.
                let aligned = a20_addr & !0x3;
                let old = self.lapic.read(aligned, 4);
                let byte_offset = (a20_addr & 0x3) as u32;
                let mask = !(0xFFu32 << (byte_offset * 8));
                let new_val = (old & mask) | ((value as u32) << (byte_offset * 8));
                self.lapic.write(aligned, new_val, 4);
                return;
            }
        }
        if let Some(mem_bus) = self.mem_bus {
            let mem = unsafe { &mut *mem_bus.as_ptr() };
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let paddr: BxPhyAddress = addr as BxPhyAddress;

            if let Ok(Some(slice)) =
                mem.get_host_mem_addr(paddr, MemoryAccessType::Write, &[cpu_ref])
            {
                if let Some(b) = slice.get_mut(0) {
                    *b = value;
                }
                self.i_cache.smc_write_check(paddr, 1);
                return;
            }

            // Vetoed: go through handler-aware physical write.
            let mut dummy_mapping: [u32; 0] = [];
            let mut stamp = BxPageWriteStampTable::new(&mut dummy_mapping);
            let mut data = [value];
            mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 1, &mut data)
                .ok();
            self.i_cache.smc_write_check(paddr, 1);
            return;
        }

        // Fallback: raw pointer access (best-effort) when not running inside cpu_loop.
        if let Some(ptr) = self.mem_ptr {
            let addr_usize = addr as usize;
            if addr_usize < self.mem_len {
                unsafe {
                    *ptr.add(addr_usize) = value;
                }
                self.i_cache.smc_write_check(addr as BxPhyAddress, 1);
            }
        }
    }

    #[inline(always)]
    pub(super) fn mem_read_word(&self, addr: u64) -> u16 {
        // Fast path: direct host pointer for plain RAM
        let a20_addr = (addr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null()
            && (a20_addr < 0xA0000 || (a20_addr >= 0x100000 && a20_addr + 1 < self.mem_host_len))
        {
            return unsafe { (host_base.add(a20_addr) as *const u16).read_unaligned() };
        }
        // Slow path: per-byte reads (handles MMIO/VGA/ROM)
        let lo = self.mem_read_byte(addr) as u16;
        let hi = self.mem_read_byte(addr + 1) as u16;
        lo | (hi << 8)
    }

    #[inline(always)]
    pub(super) fn mem_write_word(&mut self, addr: u64, value: u16) {
        // Fast path: direct host pointer for plain RAM
        let a20_addr = (addr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null()
            && (a20_addr < 0xA0000 || (a20_addr >= 0x100000 && a20_addr + 1 < self.mem_host_len))
        {
            unsafe { (host_base.add(a20_addr) as *mut u16).write_unaligned(value) };
            self.i_cache.smc_write_check(a20_addr as BxPhyAddress, 2);
            return;
        }
        // Slow path: per-byte writes (handles MMIO/VGA/ROM)
        self.mem_write_byte(addr, value as u8);
        self.mem_write_byte(addr + 1, (value >> 8) as u8);
    }

    #[inline(always)]
    pub(super) fn mem_read_dword(&self, addr: u64) -> u32 {
        // Fast path: direct host pointer for plain RAM
        let a20_addr = (addr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null()
            && (a20_addr < 0xA0000 || (a20_addr >= 0x100000 && a20_addr + 3 < self.mem_host_len))
        {
            return unsafe { (host_base.add(a20_addr) as *const u32).read_unaligned() };
        }
        // LAPIC MMIO intercept: 32-bit aligned register access
        // Bochs apic.cc read() — LAPIC registers are always dword-accessed.
        #[cfg(feature = "bx_support_apic")]
        if self.lapic.is_selected(a20_addr as BxPhyAddress) {
            return self.lapic.read(a20_addr as BxPhyAddress, 4);
        }
        // Slow path: route through read_physical_page to hit registered MMIO handlers
        // (IOAPIC, VGA, etc.) with proper dword access width.
        if let Some(mem_bus) = self.mem_bus {
            let mem = unsafe { &mut *mem_bus.as_ptr() };
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let paddr: BxPhyAddress = addr as BxPhyAddress;
            let mut data = [0u8; 4];
            if mem.read_physical_page(&[cpu_ref], paddr, 4, &mut data).is_ok() {
                return u32::from_le_bytes(data);
            }
        }
        // Fallback: per-word reads
        let lo = self.mem_read_word(addr) as u32;
        let hi = self.mem_read_word(addr + 2) as u32;
        lo | (hi << 16)
    }

    pub(super) fn mem_write_dword(&mut self, addr: u64, value: u32) {
        // Fast path: direct host pointer for plain RAM
        let a20_addr = (addr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null()
            && (a20_addr < 0xA0000 || (a20_addr >= 0x100000 && a20_addr + 3 < self.mem_host_len))
        {
            unsafe { (host_base.add(a20_addr) as *mut u32).write_unaligned(value) };
            self.i_cache.smc_write_check(a20_addr as BxPhyAddress, 4);
            return;
        }
        // LAPIC MMIO intercept: 32-bit aligned register access
        // Bochs apic.cc write() — LAPIC registers are always dword-accessed.
        #[cfg(feature = "bx_support_apic")]
        if self.lapic.is_selected(a20_addr as BxPhyAddress) {
            self.lapic.write(a20_addr as BxPhyAddress, value, 4);
            return;
        }
        // Slow path: route through write_physical_page to hit registered MMIO handlers
        // (IOAPIC, VGA, etc.) with proper dword access width.
        if let Some(mem_bus) = self.mem_bus {
            let mem = unsafe { &mut *mem_bus.as_ptr() };
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let paddr: BxPhyAddress = addr as BxPhyAddress;
            let mut dummy_mapping: [u32; 0] = [];
            let mut stamp = BxPageWriteStampTable::new(&mut dummy_mapping);
            let mut data = value.to_le_bytes();
            if mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 4, &mut data).is_ok() {
                self.i_cache.smc_write_check(paddr, 4);
                return;
            }
        }
        // Fallback: per-word writes
        self.mem_write_word(addr, value as u16);
        self.mem_write_word(addr + 2, (value >> 16) as u16);
    }

    // =========================================================================
    // Unified dispatch methods — called from dispatcher.rs
    //
    // Each method handles the 4-way (or 6-way for SCAS/CMPS) branching on
    // address size (as32_l) and REP prefix (lock_rep_used_value) so the
    // dispatcher can be a single method call per opcode.
    // =========================================================================

    // ---- MOVS ----

    /// Dispatch MOVSB: 16/32/64-bit address, with or without REP prefix.
    pub fn movsb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_movsb64(instr)?; } else { self.movsb64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_movsb32(instr)?; } else { self.movsb32(instr)?; }
        } else {
            if rep { self.rep_movsb16(instr)?; } else { self.movsb16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch MOVSW: 16/32/64-bit address, with or without REP prefix.
    pub fn movsw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_movsw64(instr)?; } else { self.movsw64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_movsw32(instr)?; } else { self.movsw32(instr)?; }
        } else {
            if rep { self.rep_movsw16(instr)?; } else { self.movsw16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch MOVSD: 16/32/64-bit address, with or without REP prefix.
    pub fn movsd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_movsd64(instr)?; } else { self.movsd64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_movsd32(instr)?; } else { self.movsd32(instr)?; }
        } else {
            if rep { self.rep_movsd16(instr)?; } else { self.movsd16(instr)?; }
        }
        Ok(())
    }

    // ---- STOS ----

    /// Dispatch STOSB: 16/32/64-bit address, with or without REP prefix.
    pub fn stosb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_stosb64(instr)?; } else { self.stosb64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_stosb32(instr)?; } else { self.stosb32(instr)?; }
        } else {
            if rep { self.rep_stosb16(instr)?; } else { self.stosb16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch STOSW: 16/32/64-bit address, with or without REP prefix.
    pub fn stosw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_stosw64(instr)?; } else { self.stosw64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_stosw32(instr)?; } else { self.stosw32(instr)?; }
        } else {
            if rep { self.rep_stosw16(instr)?; } else { self.stosw16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch STOSD: 16/32/64-bit address, with or without REP prefix.
    pub fn stosd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_stosd64(instr)?; } else { self.stosd64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_stosd32(instr)?; } else { self.stosd32(instr)?; }
        } else {
            if rep { self.rep_stosd16(instr)?; } else { self.stosd16(instr)?; }
        }
        Ok(())
    }

    // ---- LODS ----

    /// Dispatch LODSB: 16/32/64-bit address, with or without REP prefix.
    pub fn lodsb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_lodsb64(instr)?; } else { self.lodsb64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_lodsb32(instr)?; } else { self.lodsb32(instr)?; }
        } else {
            if rep { self.rep_lodsb16(instr)?; } else { self.lodsb16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch LODSW: 16/32/64-bit address, with or without REP prefix.
    pub fn lodsw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_lodsw64(instr)?; } else { self.lodsw64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_lodsw32(instr)?; } else { self.lodsw32(instr)?; }
        } else {
            if rep { self.rep_lodsw16(instr)?; } else { self.lodsw16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch LODSD: 16/32/64-bit address, with or without REP prefix.
    pub fn lodsd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if instr.as64_l() != 0 {
            if rep { self.rep_lodsd64(instr)?; } else { self.lodsd64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep { self.rep_lodsd32(instr)?; } else { self.lodsd32(instr)?; }
        } else {
            if rep { self.rep_lodsd16(instr)?; } else { self.lodsd16(instr)?; }
        }
        Ok(())
    }

    // ---- SCAS (6-way: REPE=3, REPNE=2, none) ----

    /// Dispatch SCASB: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn scasb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if instr.as64_l() != 0 {
            if rep == 3 { self.repe_scasb64(instr)?; }
            else if rep == 2 { self.repne_scasb64(instr)?; }
            else { self.scasb64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep == 3 { self.repe_scasb32(instr)?; }
            else if rep == 2 { self.repne_scasb32(instr)?; }
            else { self.scasb32(instr)?; }
        } else {
            if rep == 3 { self.repe_scasb16(instr)?; }
            else if rep == 2 { self.repne_scasb16(instr)?; }
            else { self.scasb16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch SCASW: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn scasw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if instr.as64_l() != 0 {
            if rep == 3 { self.repe_scasw64(instr)?; }
            else if rep == 2 { self.repne_scasw64(instr)?; }
            else { self.scasw64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep == 3 { self.repe_scasw32(instr)?; }
            else if rep == 2 { self.repne_scasw32(instr)?; }
            else { self.scasw32(instr)?; }
        } else {
            if rep == 3 { self.repe_scasw16(instr)?; }
            else if rep == 2 { self.repne_scasw16(instr)?; }
            else { self.scasw16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch SCASD: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn scasd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if instr.as64_l() != 0 {
            if rep == 3 { self.repe_scasd64(instr)?; }
            else if rep == 2 { self.repne_scasd64(instr)?; }
            else { self.scasd64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep == 3 { self.repe_scasd32(instr)?; }
            else if rep == 2 { self.repne_scasd32(instr)?; }
            else { self.scasd32(instr)?; }
        } else {
            if rep == 3 { self.repe_scasd16(instr)?; }
            else if rep == 2 { self.repne_scasd16(instr)?; }
            else { self.scasd16(instr)?; }
        }
        Ok(())
    }

    // ---- CMPS (6-way: REPE=3, REPNE=2, none) ----

    /// Dispatch CMPSB: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn cmpsb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if instr.as64_l() != 0 {
            if rep == 3 { self.repe_cmpsb64(instr)?; }
            else if rep == 2 { self.repne_cmpsb64(instr)?; }
            else { self.cmpsb64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep == 3 { self.repe_cmpsb32(instr)?; }
            else if rep == 2 { self.repne_cmpsb32(instr)?; }
            else { self.cmpsb32(instr)?; }
        } else {
            if rep == 3 { self.repe_cmpsb16(instr)?; }
            else if rep == 2 { self.repne_cmpsb16(instr)?; }
            else { self.cmpsb16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch CMPSW: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn cmpsw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if instr.as64_l() != 0 {
            if rep == 3 { self.repe_cmpsw64(instr)?; }
            else if rep == 2 { self.repne_cmpsw64(instr)?; }
            else { self.cmpsw64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep == 3 { self.repe_cmpsw32(instr)?; }
            else if rep == 2 { self.repne_cmpsw32(instr)?; }
            else { self.cmpsw32(instr)?; }
        } else {
            if rep == 3 { self.repe_cmpsw16(instr)?; }
            else if rep == 2 { self.repne_cmpsw16(instr)?; }
            else { self.cmpsw16(instr)?; }
        }
        Ok(())
    }

    /// Dispatch CMPSD: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn cmpsd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if instr.as64_l() != 0 {
            if rep == 3 { self.repe_cmpsd64(instr)?; }
            else if rep == 2 { self.repne_cmpsd64(instr)?; }
            else { self.cmpsd64(instr)?; }
        } else if instr.as32_l() != 0 {
            if rep == 3 { self.repe_cmpsd32(instr)?; }
            else if rep == 2 { self.repne_cmpsd32(instr)?; }
            else { self.cmpsd32(instr)?; }
        } else {
            if rep == 3 { self.repe_cmpsd16(instr)?; }
            else if rep == 2 { self.repne_cmpsd16(instr)?; }
            else { self.cmpsd16(instr)?; }
        }
        Ok(())
    }

    // =========================================================================
    // 64-bit address mode string operations (byte/word/dword data)
    // Matching Bochs string.cc MOVSB64/MOVSW64/MOVSD64 etc.
    // All use paging-aware read_virtual_*_64 / write_virtual_*_64.
    // =========================================================================

    // ---- MOVSB/W/D 64-bit ----

    /// MOVSB with 64-bit addressing -- move byte from [RSI] to [RDI]
    pub fn movsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let rdi = self.rdi();
        let byte = self.read_virtual_byte_64(BxSegregs::from(instr.seg()), rsi)?;
        self.write_virtual_byte_64(BxSegregs::Es, rdi, byte)?;
        let delta: u64 = if self.get_df() { u64::MAX } else { 1 };
        self.set_rsi(rsi.wrapping_add(delta));
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_movsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep: try bulk memcpy when DF=0 (Bochs faststring.cc:91-92)
        while rcx != 0 && !df && self.async_event == 0 {
            let rsi = self.rsi();
            let rdi = self.rdi();
            let src_laddr = self.get_laddr64(seg as usize, rsi);
            let dst_laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);

            if let (Some((src_ptr, src_rem)), Some((dst_ptr, dst_rem))) =
                (self.get_host_read_ptr(src_laddr), self.get_host_write_ptr(dst_laddr))
            {
                let count = (rcx as usize).min(src_rem).min(dst_rem)
                    .min(self.ticks_left_next_event() as usize);
                if count > 0 {
                    // Bochs faststring.cc:99-103 — forward byte-by-byte loop.
                    // Must NOT use memcpy: overlapping regions (LZ decompression)
                    // rely on reading already-written bytes during forward copy.
                    unsafe {
                        for j in 0..count {
                            *dst_ptr.add(j) = *src_ptr.add(j);
                        }
                    }
                    self.set_rsi(rsi.wrapping_add(count as u64));
                    self.set_rdi(rdi.wrapping_add(count as u64));
                    self.icount += count as u64 - 1;
                    self.tickn_fastrep(count);
                    rcx -= count as u64;
                    self.set_rcx(rcx);
                    if rcx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break; // TLB miss — fall through to per-byte loop
        }

        while rcx != 0 {
            self.movsb64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// MOVSW with 64-bit addressing
    pub fn movsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let rdi = self.rdi();
        let val = self.read_virtual_word_64(BxSegregs::from(instr.seg()), rsi)?;
        self.write_virtual_word_64(BxSegregs::Es, rdi, val)?;
        let delta: u64 = if self.get_df() { (-2i64) as u64 } else { 2 };
        self.set_rsi(rsi.wrapping_add(delta));
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_movsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep: try bulk memcpy when DF=0 (Bochs faststring.cc:91-92 granularity=2)
        while rcx != 0 && !df && self.async_event == 0 {
            let rsi = self.rsi();
            let rdi = self.rdi();
            let src_laddr = self.get_laddr64(seg as usize, rsi);
            let dst_laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);

            if let (Some((src_ptr, src_rem)), Some((dst_ptr, dst_rem))) =
                (self.get_host_read_ptr(src_laddr), self.get_host_write_ptr(dst_laddr))
            {
                let max_bytes = (rcx as usize) * 2;
                let count_bytes = max_bytes.min(src_rem).min(dst_rem)
                    .min(self.ticks_left_next_event() as usize) & !1;
                let count_words = count_bytes / 2;
                if count_words > 0 {
                    unsafe {
                        // Forward byte-by-byte (Bochs faststring.cc:99-103)
                        for j in 0..count_bytes {
                            *dst_ptr.add(j) = *src_ptr.add(j);
                        }
                    }
                    self.set_rsi(rsi.wrapping_add(count_bytes as u64));
                    self.set_rdi(rdi.wrapping_add(count_bytes as u64));
                    self.icount += count_words as u64 - 1;
                    self.tickn_fastrep(count_words);
                    rcx -= count_words as u64;
                    self.set_rcx(rcx);
                    if rcx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while rcx != 0 {
            self.movsw64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// MOVSD with 64-bit addressing
    pub fn movsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let rdi = self.rdi();
        let val = self.read_virtual_dword_64(BxSegregs::from(instr.seg()), rsi)?;
        self.write_virtual_dword_64(BxSegregs::Es, rdi, val)?;
        let delta: u64 = if self.get_df() { (-4i64) as u64 } else { 4 };
        self.set_rsi(rsi.wrapping_add(delta));
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_movsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep: try bulk memcpy when DF=0 (Bochs faststring.cc:91-92 granularity=4)
        while rcx != 0 && !df && self.async_event == 0 {
            let rsi = self.rsi();
            let rdi = self.rdi();
            let src_laddr = self.get_laddr64(seg as usize, rsi);
            let dst_laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);

            if let (Some((src_ptr, src_rem)), Some((dst_ptr, dst_rem))) =
                (self.get_host_read_ptr(src_laddr), self.get_host_write_ptr(dst_laddr))
            {
                let max_bytes = (rcx as usize) * 4;
                let count_bytes = max_bytes.min(src_rem).min(dst_rem)
                    .min(self.ticks_left_next_event() as usize) & !3; // align to 4
                let count_dwords = count_bytes / 4;
                if count_dwords > 0 {
                    unsafe {
                        // Forward byte-by-byte (Bochs faststring.cc:99-103)
                        for j in 0..count_bytes {
                            *dst_ptr.add(j) = *src_ptr.add(j);
                        }
                    }
                    self.set_rsi(rsi.wrapping_add(count_bytes as u64));
                    self.set_rdi(rdi.wrapping_add(count_bytes as u64));
                    self.icount += count_dwords as u64 - 1;
                    self.tickn_fastrep(count_dwords);
                    rcx -= count_dwords as u64;
                    self.set_rcx(rcx);
                    if rcx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while rcx != 0 {
            self.movsd64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- STOSB/W/D 64-bit ----

    /// STOSB with 64-bit addressing
    pub fn stosb64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let al = self.al();
        self.write_virtual_byte_64(BxSegregs::Es, rdi, al)?;
        let delta: u64 = if self.get_df() { u64::MAX } else { 1 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_stosb64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let al = self.al();

        // FastRep: try bulk memset when DF=0 (Bochs faststring.cc:146-147)
        while rcx != 0 && !df && self.async_event == 0 {
            let rdi = self.rdi();
            let dst_laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);

            if let Some((dst_ptr, dst_rem)) = self.get_host_write_ptr(dst_laddr) {
                let count = (rcx as usize).min(dst_rem)
                    .min(self.ticks_left_next_event() as usize);
                if count > 0 {
                    unsafe {
                        core::ptr::write_bytes(dst_ptr, al, count);
                    }
                    self.set_rdi(rdi.wrapping_add(count as u64));
                    self.icount += count as u64 - 1;
                    self.tickn_fastrep(count);
                    rcx -= count as u64;
                    self.set_rcx(rcx);
                    if rcx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while rcx != 0 {
            self.stosb64(_instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// STOSW with 64-bit addressing
    pub fn stosw64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let ax = self.ax();
        self.write_virtual_word_64(BxSegregs::Es, rdi, ax)?;
        let delta: u64 = if self.get_df() { (-2i64) as u64 } else { 2 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_stosw64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let ax = self.ax();

        // FastRep: try bulk word fill when DF=0 (Bochs faststring.cc:198-199)
        while rcx != 0 && !df && self.async_event == 0 {
            let rdi = self.rdi();
            let dst_laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);

            if let Some((dst_ptr, dst_rem)) = self.get_host_write_ptr(dst_laddr) {
                let max_bytes = (rcx as usize) * 2;
                let count_bytes = max_bytes.min(dst_rem) & !1;
                let count_words = (count_bytes / 2)
                    .min(self.ticks_left_next_event() as usize);
                if count_words > 0 {
                    let dst_slice = unsafe {
                        core::slice::from_raw_parts_mut(dst_ptr as *mut u16, count_words)
                    };
                    for w in dst_slice.iter_mut() {
                        *w = ax;
                    }
                    self.set_rdi(rdi.wrapping_add((count_words * 2) as u64));
                    self.icount += count_words as u64 - 1;
                    self.tickn_fastrep(count_words);
                    rcx -= count_words as u64;
                    self.set_rcx(rcx);
                    if rcx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while rcx != 0 {
            self.stosw64(_instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// STOSD with 64-bit addressing
    pub fn stosd64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let eax = self.eax();
        self.write_virtual_dword_64(BxSegregs::Es, rdi, eax)?;
        let delta: u64 = if self.get_df() { (-4i64) as u64 } else { 4 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_stosd64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let eax = self.eax();

        // FastRep: try bulk dword fill when DF=0 (Bochs faststring.cc:250-251)
        while rcx != 0 && !df && self.async_event == 0 {
            let rdi = self.rdi();
            let dst_laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);

            if let Some((dst_ptr, dst_rem)) = self.get_host_write_ptr(dst_laddr) {
                let max_bytes = (rcx as usize) * 4;
                let count_bytes = max_bytes.min(dst_rem) & !3;
                let count_dwords = (count_bytes / 4)
                    .min(self.ticks_left_next_event() as usize);
                if count_dwords > 0 {
                    let dst_slice = unsafe {
                        core::slice::from_raw_parts_mut(dst_ptr as *mut u32, count_dwords)
                    };
                    for d in dst_slice.iter_mut() {
                        *d = eax;
                    }
                    self.set_rdi(rdi.wrapping_add((count_dwords * 4) as u64));
                    self.icount += count_dwords as u64 - 1;
                    self.tickn_fastrep(count_dwords);
                    rcx -= count_dwords as u64;
                    self.set_rcx(rcx);
                    if rcx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while rcx != 0 {
            self.stosd64(_instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- LODSB/W/D 64-bit ----

    /// LODSB with 64-bit addressing
    pub fn lodsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let byte = self.read_virtual_byte_64(BxSegregs::from(instr.seg()), rsi)?;
        self.set_al(byte);
        let delta: u64 = if self.get_df() { u64::MAX } else { 1 };
        self.set_rsi(rsi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_lodsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.lodsb64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// LODSW with 64-bit addressing
    pub fn lodsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let val = self.read_virtual_word_64(BxSegregs::from(instr.seg()), rsi)?;
        self.set_ax(val);
        let delta: u64 = if self.get_df() { (-2i64) as u64 } else { 2 };
        self.set_rsi(rsi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_lodsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.lodsw64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// LODSD with 64-bit addressing
    pub fn lodsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let val = self.read_virtual_dword_64(BxSegregs::from(instr.seg()), rsi)?;
        // Bochs: RAX = val (zero-extends 32-bit to 64-bit)
        self.set_rax(val as u64);
        let delta: u64 = if self.get_df() { (-4i64) as u64 } else { 4 };
        self.set_rsi(rsi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_lodsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.lodsd64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- CMPSB/W/D 64-bit ----

    /// CMPSB with 64-bit addressing
    pub fn cmpsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let rdi = self.rdi();
        let op1 = self.read_virtual_byte_64(BxSegregs::from(instr.seg()), rsi)?;
        let op2 = self.read_virtual_byte_64(BxSegregs::Es, rdi)?;
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub8(op1, op2, result);
        let delta: u64 = if self.get_df() { u64::MAX } else { 1 };
        self.set_rsi(rsi.wrapping_add(delta));
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn repe_cmpsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.cmpsb64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if !self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    pub fn repne_cmpsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.cmpsb64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// CMPSW with 64-bit addressing
    pub fn cmpsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let rdi = self.rdi();
        let op1 = self.read_virtual_word_64(BxSegregs::from(instr.seg()), rsi)?;
        let op2 = self.read_virtual_word_64(BxSegregs::Es, rdi)?;
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub16(op1, op2, result);
        let delta: u64 = if self.get_df() { (-2i64) as u64 } else { 2 };
        self.set_rsi(rsi.wrapping_add(delta));
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn repe_cmpsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.cmpsw64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if !self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    pub fn repne_cmpsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.cmpsw64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// CMPSD with 64-bit addressing
    pub fn cmpsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let rdi = self.rdi();
        let op1 = self.read_virtual_dword_64(BxSegregs::from(instr.seg()), rsi)?;
        let op2 = self.read_virtual_dword_64(BxSegregs::Es, rdi)?;
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub32(op1, op2, result);
        let delta: u64 = if self.get_df() { (-4i64) as u64 } else { 4 };
        self.set_rsi(rsi.wrapping_add(delta));
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn repe_cmpsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.cmpsd64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if !self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    pub fn repne_cmpsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.cmpsd64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ---- SCASB/W/D 64-bit ----

    /// SCASB with 64-bit addressing
    pub fn scasb64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let al = self.al();
        let op2 = self.read_virtual_byte_64(BxSegregs::Es, rdi)?;
        let result = al.wrapping_sub(op2);
        self.update_flags_sub8(al, op2, result);
        let delta: u64 = if self.get_df() { u64::MAX } else { 1 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn repe_scasb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.scasb64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if !self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    pub fn repne_scasb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.scasb64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// SCASW with 64-bit addressing
    pub fn scasw64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let ax = self.ax();
        let op2 = self.read_virtual_word_64(BxSegregs::Es, rdi)?;
        let result = ax.wrapping_sub(op2);
        self.update_flags_sub16(ax, op2, result);
        let delta: u64 = if self.get_df() { (-2i64) as u64 } else { 2 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn repe_scasw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.scasw64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if !self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    pub fn repne_scasw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.scasw64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// SCASD with 64-bit addressing
    pub fn scasd64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let eax = self.eax();
        let op2 = self.read_virtual_dword_64(BxSegregs::Es, rdi)?;
        let result = eax.wrapping_sub(op2);
        self.update_flags_sub32(eax, op2, result);
        let delta: u64 = if self.get_df() { (-4i64) as u64 } else { 4 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn repe_scasd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.scasd64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if !self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    pub fn repne_scasd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.scasd64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // 64-bit string operations (64-bit address mode, qword data)
    // Matching Bochs string.cc MOVSQ / STOSQ / CMPSQ / LODSQ / SCASQ
    // =========================================================================

    /// MOVSQ -- Move qword from [RSI] to [RDI] (64-bit addressing)
    pub fn movsq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let rdi = self.rdi();
        let val = self.read_virtual_qword_64(BxSegregs::from(instr.seg()), rsi)?;
        self.write_virtual_qword_64(BxSegregs::Es, rdi, val)?;
        let delta: u64 = if self.get_df() { (-8i64) as u64 } else { 8 };
        self.set_rsi(rsi.wrapping_add(delta));
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    /// REP MOVSQ -- Move RCX qwords from [RSI] to [RDI]
    pub fn rep_movsq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep: try bulk memcpy when DF=0 (Bochs faststring.cc:91-92 granularity=8)
        while rcx != 0 && !df && self.async_event == 0 {
            let rsi = self.rsi();
            let rdi = self.rdi();
            let src_laddr = self.get_laddr64(seg as usize, rsi);
            let dst_laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);

            if let (Some((src_ptr, src_rem)), Some((dst_ptr, dst_rem))) =
                (self.get_host_read_ptr(src_laddr), self.get_host_write_ptr(dst_laddr))
            {
                let max_bytes = (rcx as usize) * 8;
                let count_bytes = max_bytes.min(src_rem).min(dst_rem)
                    .min(self.ticks_left_next_event() as usize) & !7;
                let count_qwords = count_bytes / 8;
                if count_qwords > 0 {
                    unsafe {
                        // Forward byte-by-byte (Bochs faststring.cc:99-103)
                        for j in 0..count_bytes {
                            *dst_ptr.add(j) = *src_ptr.add(j);
                        }
                    }
                    self.set_rsi(rsi.wrapping_add(count_bytes as u64));
                    self.set_rdi(rdi.wrapping_add(count_bytes as u64));
                    self.icount += count_qwords as u64 - 1;
                    self.tickn_fastrep(count_qwords);
                    rcx -= count_qwords as u64;
                    self.set_rcx(rcx);
                    if rcx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while rcx != 0 {
            self.movsq64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// STOSQ -- Store RAX to [RDI] (64-bit addressing)
    pub fn stosq64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let rax = self.rax();
        self.write_virtual_qword_64(BxSegregs::Es, rdi, rax)?;
        let delta: u64 = if self.get_df() { (-8i64) as u64 } else { 8 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    /// REP STOSQ -- Store RAX to RCX qwords at [RDI]
    pub fn rep_stosq64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let rax = self.rax();

        // FastRep: try bulk qword fill when DF=0 (Bochs faststring.cc:250-251 granularity=8)
        while rcx != 0 && !df && self.async_event == 0 {
            let rdi = self.rdi();
            let dst_laddr = self.get_laddr64(BxSegregs::Es as usize, rdi);

            if let Some((dst_ptr, dst_rem)) = self.get_host_write_ptr(dst_laddr) {
                let max_bytes = (rcx as usize) * 8;
                let count_bytes = max_bytes.min(dst_rem) & !7;
                let count_qwords = (count_bytes / 8)
                    .min(self.ticks_left_next_event() as usize);
                if count_qwords > 0 {
                    let dst_slice = unsafe {
                        core::slice::from_raw_parts_mut(dst_ptr as *mut u64, count_qwords)
                    };
                    for q in dst_slice.iter_mut() {
                        *q = rax;
                    }
                    self.set_rdi(rdi.wrapping_add((count_qwords * 8) as u64));
                    self.icount += count_qwords as u64 - 1;
                    self.tickn_fastrep(count_qwords);
                    rcx -= count_qwords as u64;
                    self.set_rcx(rcx);
                    if rcx != 0 && self.async_event != 0 {
                        self.set_rip(self.prev_rip);
                        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        return Ok(());
                    }
                    continue;
                }
            }
            break;
        }

        while rcx != 0 {
            self.stosq64(_instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// LODSQ -- Load qword from [RSI] into RAX (64-bit addressing)
    pub fn lodsq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let val = self.read_virtual_qword_64(BxSegregs::from(instr.seg()), rsi)?;
        self.set_rax(val);
        let delta: u64 = if self.get_df() { (-8i64) as u64 } else { 8 };
        self.set_rsi(rsi.wrapping_add(delta));
        Ok(())
    }

    /// REP LODSQ -- Load RCX qwords from [RSI] into RAX
    pub fn rep_lodsq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.lodsq64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if rcx != 0 {
                if self.async_event != 0 {
                    self.set_rip(self.prev_rip);
                    break;
                }
                self.icount += 1;
            }
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// CMPSQ -- Compare qword [RSI] with [RDI] (64-bit addressing)
    pub fn cmpsq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let rsi = self.rsi();
        let rdi = self.rdi();
        let op1 = self.read_virtual_qword_64(BxSegregs::from(instr.seg()), rsi)?;
        let op2 = self.read_virtual_qword_64(BxSegregs::Es, rdi)?;
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub64(op1, op2, result);
        let delta: u64 = if self.get_df() { (-8i64) as u64 } else { 8 };
        self.set_rsi(rsi.wrapping_add(delta));
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    /// REPE CMPSQ -- Compare RCX qwords, stop if not equal
    pub fn repe_cmpsq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.cmpsq64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if !self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE CMPSQ -- Compare RCX qwords, stop if equal
    pub fn repne_cmpsq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.cmpsq64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// SCASQ -- Compare RAX with qword at [RDI] (64-bit addressing)
    pub fn scasq64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let rax = self.rax();
        let op2 = self.read_virtual_qword_64(BxSegregs::Es, rdi)?;
        let result = rax.wrapping_sub(op2);
        self.update_flags_sub64(rax, op2, result);
        let delta: u64 = if self.get_df() { (-8i64) as u64 } else { 8 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    /// REPE SCASQ -- Scan RCX qwords, stop if not equal
    pub fn repe_scasq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.scasq64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if !self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    /// REPNE SCASQ -- Scan RCX qwords, stop if equal
    pub fn repne_scasq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        while rcx != 0 {
            self.scasq64(instr)?;
            rcx = rcx.wrapping_sub(1);
            self.set_rcx(rcx);
            if self.get_zf() || rcx == 0 { break; }
            if self.async_event != 0 {
                self.set_rip(self.prev_rip);
                break;
            }
            self.icount += 1;
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // =========================================================================
    // 64-bit string dispatch functions (qword data)
    // =========================================================================

    /// Dispatch MOVSQ: 64-bit only, with or without REP prefix.
    pub fn movsq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.lock_rep_used_value() != 0 {
            self.rep_movsq64(instr)
        } else {
            self.movsq64(instr)
        }
    }

    /// Dispatch STOSQ: 64-bit only, with or without REP prefix.
    pub fn stosq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.lock_rep_used_value() != 0 {
            self.rep_stosq64(instr)
        } else {
            self.stosq64(instr)
        }
    }

    /// Dispatch LODSQ: 64-bit only, with or without REP prefix.
    pub fn lodsq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.lock_rep_used_value() != 0 {
            self.rep_lodsq64(instr)
        } else {
            self.lodsq64(instr)
        }
    }

    /// Dispatch CMPSQ: 64-bit only, with REPE/REPNE/no-REP prefix.
    pub fn cmpsq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep == 3 { self.repe_cmpsq64(instr) }
        else if rep == 2 { self.repne_cmpsq64(instr) }
        else { self.cmpsq64(instr) }
    }

    /// Dispatch SCASQ: 64-bit only, with REPE/REPNE/no-REP prefix.
    pub fn scasq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep == 3 { self.repe_scasq64(instr) }
        else if rep == 2 { self.repne_scasq64(instr) }
        else { self.scasq64(instr) }
    }
}
