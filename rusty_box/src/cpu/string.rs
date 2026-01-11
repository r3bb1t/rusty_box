//! String operations for x86 CPU emulation
//!
//! Based on Bochs string.cc
//! Copyright (C) 2001-2019 The Bochs Project
//!
//! Implements MOVS, STOS, LODS, CMPS, SCAS instructions
//!
//! NOTE: These are simplified implementations that work for basic real-mode operations.
//! Full implementation would require proper segment handling and memory access.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Helper: Get direction flag (DF)
    // =========================================================================
    
    /// Returns true if direction flag is set (decrement mode)
    #[inline]
    pub(super) fn get_df(&self) -> bool {
        (self.eflags & (1 << 10)) != 0
    }

    // =========================================================================
    // MOVSB - Move String Byte
    // =========================================================================
    
    /// MOVSB - Move byte from DS:SI to ES:DI (16-bit address mode)
    pub fn movsb16(&mut self, _instr: &BxInstructionGenerated) {
        let si = self.si() as u64;
        let di = self.di() as u64;
        
        // Get segment bases
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        
        let src_addr = ds_base.wrapping_add(si);
        let dst_addr = es_base.wrapping_add(di);
        
        // Read and write using raw memory access
        let byte = self.mem_read_byte(src_addr);
        self.mem_write_byte(dst_addr, byte);
        
        // Update SI and DI based on DF
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(1));
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_si(self.si().wrapping_add(1));
            self.set_di(self.di().wrapping_add(1));
        }
        
        tracing::trace!("MOVSB16: DS:{:04x} -> ES:{:04x}, byte={:#04x}", si, di, byte);
    }

    /// MOVSW - Move word from DS:SI to ES:DI (16-bit address mode)
    pub fn movsw16(&mut self, _instr: &BxInstructionGenerated) {
        let si = self.si() as u64;
        let di = self.di() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        
        let src_addr = ds_base.wrapping_add(si);
        let dst_addr = es_base.wrapping_add(di);
        
        let word = self.mem_read_word(src_addr);
        self.mem_write_word(dst_addr, word);
        
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(2));
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_si(self.si().wrapping_add(2));
            self.set_di(self.di().wrapping_add(2));
        }
        
        tracing::trace!("MOVSW16: word={:#06x}", word);
    }

    /// MOVSD - Move dword from DS:SI to ES:DI (16-bit address mode)
    pub fn movsd16(&mut self, _instr: &BxInstructionGenerated) {
        let si = self.si() as u64;
        let di = self.di() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        
        let src_addr = ds_base.wrapping_add(si);
        let dst_addr = es_base.wrapping_add(di);
        
        let dword = self.mem_read_dword(src_addr);
        self.mem_write_dword(dst_addr, dword);
        
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(4));
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_si(self.si().wrapping_add(4));
            self.set_di(self.di().wrapping_add(4));
        }
        
        tracing::trace!("MOVSD16: dword={:#010x}", dword);
    }

    // =========================================================================
    // STOSB - Store String Byte
    // =========================================================================
    
    /// STOSB - Store AL at ES:DI (16-bit address mode)
    pub fn stosb16(&mut self, _instr: &BxInstructionGenerated) {
        let di = self.di() as u64;
        let al = self.al();
        
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        let dst_addr = es_base.wrapping_add(di);
        self.mem_write_byte(dst_addr, al);
        
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_di(self.di().wrapping_add(1));
        }
        
        tracing::trace!("STOSB16: AL={:#04x} -> ES:{:04x}", al, di);
    }

    /// STOSW - Store AX at ES:DI (16-bit address mode)
    pub fn stosw16(&mut self, _instr: &BxInstructionGenerated) {
        let di = self.di() as u64;
        let ax = self.ax();
        
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        let dst_addr = es_base.wrapping_add(di);
        self.mem_write_word(dst_addr, ax);
        
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_di(self.di().wrapping_add(2));
        }
        
        tracing::trace!("STOSW16: AX={:#06x} -> ES:{:04x}", ax, di);
    }

    /// STOSD - Store EAX at ES:DI (16-bit address mode)
    pub fn stosd16(&mut self, _instr: &BxInstructionGenerated) {
        let di = self.di() as u64;
        let eax = self.eax();
        
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        let dst_addr = es_base.wrapping_add(di);
        self.mem_write_dword(dst_addr, eax);
        
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_di(self.di().wrapping_add(4));
        }
        
        tracing::trace!("STOSD16: EAX={:#010x} -> ES:{:04x}", eax, di);
    }

    // =========================================================================
    // LODSB - Load String Byte
    // =========================================================================
    
    /// LODSB - Load byte from DS:SI into AL (16-bit address mode)
    pub fn lodsb16(&mut self, _instr: &BxInstructionGenerated) {
        let si = self.si() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let src_addr = ds_base.wrapping_add(si);
        let byte = self.mem_read_byte(src_addr);
        
        self.set_al(byte);
        
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(1));
        } else {
            self.set_si(self.si().wrapping_add(1));
        }
        
        tracing::trace!("LODSB16: DS:{:04x} -> AL={:#04x}", si, byte);
    }

    /// LODSW - Load word from DS:SI into AX (16-bit address mode)
    pub fn lodsw16(&mut self, _instr: &BxInstructionGenerated) {
        let si = self.si() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let src_addr = ds_base.wrapping_add(si);
        let word = self.mem_read_word(src_addr);
        
        self.set_ax(word);
        
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(2));
        } else {
            self.set_si(self.si().wrapping_add(2));
        }
        
        tracing::trace!("LODSW16: DS:{:04x} -> AX={:#06x}", si, word);
    }

    /// LODSD - Load dword from DS:SI into EAX (16-bit address mode)
    pub fn lodsd16(&mut self, _instr: &BxInstructionGenerated) {
        let si = self.si() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let src_addr = ds_base.wrapping_add(si);
        let dword = self.mem_read_dword(src_addr);
        
        self.set_eax(dword);
        
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(4));
        } else {
            self.set_si(self.si().wrapping_add(4));
        }
        
        tracing::trace!("LODSD16: DS:{:04x} -> EAX={:#010x}", si, dword);
    }

    // =========================================================================
    // CMPSB - Compare String Byte
    // =========================================================================
    
    /// CMPSB - Compare bytes at DS:SI and ES:DI (16-bit address mode)
    pub fn cmpsb16(&mut self, _instr: &BxInstructionGenerated) {
        let si = self.si() as u64;
        let di = self.di() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        
        let src_addr = ds_base.wrapping_add(si);
        let dst_addr = es_base.wrapping_add(di);
        
        let op1 = self.mem_read_byte(src_addr);
        let op2 = self.mem_read_byte(dst_addr);
        
        // Perform comparison (SUB without storing result)
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub8(op1, op2, result);
        
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(1));
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_si(self.si().wrapping_add(1));
            self.set_di(self.di().wrapping_add(1));
        }
        
        tracing::trace!("CMPSB16: [{:#04x}] vs [{:#04x}]", op1, op2);
    }

    /// CMPSW - Compare words at DS:SI and ES:DI (16-bit address mode)
    pub fn cmpsw16(&mut self, _instr: &BxInstructionGenerated) {
        let si = self.si() as u64;
        let di = self.di() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        
        let src_addr = ds_base.wrapping_add(si);
        let dst_addr = es_base.wrapping_add(di);
        
        let op1 = self.mem_read_word(src_addr);
        let op2 = self.mem_read_word(dst_addr);
        
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub16(op1, op2, result);
        
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(2));
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_si(self.si().wrapping_add(2));
            self.set_di(self.di().wrapping_add(2));
        }
        
        tracing::trace!("CMPSW16: [{:#06x}] vs [{:#06x}]", op1, op2);
    }

    // =========================================================================
    // SCASB - Scan String Byte
    // =========================================================================
    
    /// SCASB - Compare AL with byte at ES:DI (16-bit address mode)
    pub fn scasb16(&mut self, _instr: &BxInstructionGenerated) {
        let di = self.di() as u64;
        let al = self.al();
        
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        let dst_addr = es_base.wrapping_add(di);
        let op2 = self.mem_read_byte(dst_addr);
        
        let result = al.wrapping_sub(op2);
        self.update_flags_sub8(al, op2, result);
        
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(1));
        } else {
            self.set_di(self.di().wrapping_add(1));
        }
        
        tracing::trace!("SCASB16: AL={:#04x} vs [{:#04x}]", al, op2);
    }

    /// SCASW - Compare AX with word at ES:DI (16-bit address mode)
    pub fn scasw16(&mut self, _instr: &BxInstructionGenerated) {
        let di = self.di() as u64;
        let ax = self.ax();
        
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        let dst_addr = es_base.wrapping_add(di);
        let op2 = self.mem_read_word(dst_addr);
        
        let result = ax.wrapping_sub(op2);
        self.update_flags_sub16(ax, op2, result);
        
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(2));
        } else {
            self.set_di(self.di().wrapping_add(2));
        }
        
        tracing::trace!("SCASW16: AX={:#06x} vs [{:#06x}]", ax, op2);
    }

    // =========================================================================
    // REP prefix handling
    // =========================================================================
    
    /// REP MOVSB - Repeat move string byte CX times
    pub fn rep_movsb16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.movsb16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REP MOVSW - Repeat move string word CX times
    pub fn rep_movsw16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.movsw16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REP STOSB - Repeat store string byte CX times
    pub fn rep_stosb16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.stosb16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REP STOSW - Repeat store string word CX times
    pub fn rep_stosw16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.stosw16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REP LODSB - Repeat load string byte CX times
    pub fn rep_lodsb16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.lodsb16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REPE CMPSB - Repeat compare string byte while equal
    pub fn repe_cmpsb16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsb16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() {
                break; // ZF=0 means not equal, stop
            }
        }
    }

    /// REPNE CMPSB - Repeat compare string byte while not equal
    pub fn repne_cmpsb16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsb16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() {
                break; // ZF=1 means equal, stop
            }
        }
    }

    /// REPE SCASB - Repeat scan string byte while equal
    pub fn repe_scasb16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasb16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() {
                break;
            }
        }
    }

    /// REPNE SCASB - Repeat scan string byte while not equal
    pub fn repne_scasb16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasb16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() {
                break;
            }
        }
    }

    // =========================================================================
    // Memory access helpers using the stored memory pointer
    // =========================================================================
    
    pub(super) fn mem_read_byte(&self, addr: u64) -> u8 {
        if let Some(ptr) = self.mem_ptr {
            let addr = addr as usize;
            if addr < self.mem_len {
                unsafe {
                    return *ptr.add(addr);
                }
            }
        }
        0
    }

    pub(super) fn mem_write_byte(&mut self, addr: u64, value: u8) {
        if let Some(ptr) = self.mem_ptr {
            let addr = addr as usize;
            if addr < self.mem_len {
                unsafe {
                    *ptr.add(addr) = value;
                }
            }
        }
    }

    pub(super) fn mem_read_word(&self, addr: u64) -> u16 {
        let lo = self.mem_read_byte(addr) as u16;
        let hi = self.mem_read_byte(addr + 1) as u16;
        lo | (hi << 8)
    }

    pub(super) fn mem_write_word(&mut self, addr: u64, value: u16) {
        self.mem_write_byte(addr, value as u8);
        self.mem_write_byte(addr + 1, (value >> 8) as u8);
    }

    pub(super) fn mem_read_dword(&self, addr: u64) -> u32 {
        let lo = self.mem_read_word(addr) as u32;
        let hi = self.mem_read_word(addr + 2) as u32;
        lo | (hi << 16)
    }

    pub(super) fn mem_write_dword(&mut self, addr: u64, value: u32) {
        self.mem_write_word(addr, value as u16);
        self.mem_write_word(addr + 2, (value >> 16) as u16);
    }

    // Flag update helpers are in cpu.rs: update_flags_sub8, update_flags_sub16
}
