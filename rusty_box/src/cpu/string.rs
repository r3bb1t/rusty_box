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

    /// MOVSB - Move byte from DS:ESI to ES:EDI (32-bit address mode)
    /// Based on BX_CPU_C::MOVSB32_YbXb in string.cc
    pub fn movsb32(&mut self, _instr: &BxInstructionGenerated) {
        let esi = self.esi() as u64;
        let edi = self.edi() as u64;

        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };

        let src_addr = ds_base.wrapping_add(esi);
        let dst_addr = es_base.wrapping_add(edi);
        let byte = self.mem_read_byte(src_addr);
        self.mem_write_byte(dst_addr, byte);

        let increment = if self.get_df() { -1i32 } else { 1i32 };
        let new_esi = (esi as i64).wrapping_add(increment as i64) as u32;
        let new_edi = (edi as i64).wrapping_add(increment as i64) as u32;

        // Zero extension of RSI/RDI (matching original BX_CLEAR_64BIT_HIGH behavior)
        self.set_rsi(new_esi as u64);
        self.set_rdi(new_edi as u64);

        tracing::trace!("MOVSB32: DS:{:#x} -> ES:{:#x}, byte={:#04x}", esi, edi, byte);
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

    /// MOVSD - Move dword from DS:ESI to ES:EDI (32-bit address mode)
    /// Based on BX_CPU_C::MOVSD32_YdXd in string.cc
    pub fn movsd32(&mut self, _instr: &BxInstructionGenerated) {
        let esi = self.esi() as u64;
        let edi = self.edi() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        
        let src_addr = ds_base.wrapping_add(esi);
        let dst_addr = es_base.wrapping_add(edi);
        
        let dword = self.mem_read_dword(src_addr);
        self.mem_write_dword(dst_addr, dword);
        
        let increment = if self.get_df() { -4i32 } else { 4i32 };
        let new_esi = (esi as i64).wrapping_add(increment as i64) as u32;
        let new_edi = (edi as i64).wrapping_add(increment as i64) as u32;
        
        // Zero extension of RSI/RDI (matching original behavior)
        self.set_rsi(new_esi as u64);
        self.set_rdi(new_edi as u64);
        
        tracing::trace!("MOVSD32: dword={:#010x}, ESI={:#x}, EDI={:#x}", dword, new_esi, new_edi);
    }

    /// MOVSW - Move word from DS:ESI to ES:EDI (32-bit address mode)
    /// Based on BX_CPU_C::MOVSW32_YwXw in string.cc
    pub fn movsw32(&mut self, _instr: &BxInstructionGenerated) {
        let esi = self.esi() as u64;
        let edi = self.edi() as u64;

        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };

        let src_addr = ds_base.wrapping_add(esi);
        let dst_addr = es_base.wrapping_add(edi);

        let word = self.mem_read_word(src_addr);
        self.mem_write_word(dst_addr, word);

        let increment = if self.get_df() { -2i32 } else { 2i32 };
        let new_esi = (esi as i64).wrapping_add(increment as i64) as u32;
        let new_edi = (edi as i64).wrapping_add(increment as i64) as u32;

        self.set_rsi(new_esi as u64);
        self.set_rdi(new_edi as u64);

        tracing::trace!("MOVSW32: word={:#06x}, ESI={:#x}, EDI={:#x}", word, new_esi, new_edi);
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

    /// STOSB - Store AL at ES:EDI (32-bit address mode)
    /// Based on BX_CPU_C::STOSB32_YbAL in string.cc
    pub fn stosb32(&mut self, _instr: &BxInstructionGenerated) {
        let edi = self.edi() as u64;
        let al = self.al();

        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        let dst_addr = es_base.wrapping_add(edi);
        self.mem_write_byte(dst_addr, al);

        let increment = if self.get_df() { -1i32 } else { 1i32 };
        let new_edi = (edi as i64).wrapping_add(increment as i64) as u32;

        // Zero extension of RDI (matching original BX_CLEAR_64BIT_HIGH behavior)
        self.set_rdi(new_edi as u64);

        tracing::trace!("STOSB32: AL={:#04x} -> ES:{:#x}", al, edi);
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

    /// STOSD - Store EAX at ES:EDI (32-bit address mode)
    /// Based on BX_CPU_C::STOSD32_Yd in string.cc
    pub fn stosd32(&mut self, _instr: &BxInstructionGenerated) {
        let edi = self.edi() as u64;
        let eax = self.eax();
        
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        let dst_addr = es_base.wrapping_add(edi);
        self.mem_write_dword(dst_addr, eax);
        
        let increment = if self.get_df() { -4i32 } else { 4i32 };
        let new_edi = (edi as i64).wrapping_add(increment as i64) as u32;
        
        // Zero extension of RDI (matching original behavior)
        self.set_rdi(new_edi as u64);
        
        tracing::trace!("STOSD32: EAX={:#010x} -> ES:{:#x}", eax, new_edi);
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

    /// CMPSD - Compare dwords at DS:SI and ES:DI (16-bit address mode)
    pub fn cmpsd16(&mut self, _instr: &BxInstructionGenerated) {
        let si = self.si() as u64;
        let di = self.di() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        
        let src_addr = ds_base.wrapping_add(si);
        let dst_addr = es_base.wrapping_add(di);
        
        let op1 = self.mem_read_dword(src_addr);
        let op2 = self.mem_read_dword(dst_addr);
        
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub32(op1, op2, result);
        
        if self.get_df() {
            self.set_si(self.si().wrapping_sub(4));
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_si(self.si().wrapping_add(4));
            self.set_di(self.di().wrapping_add(4));
        }
        
        tracing::trace!("CMPSD16: [{:#010x}] vs [{:#010x}]", op1, op2);
    }

    /// CMPSD - Compare dwords at DS:ESI and ES:EDI (32-bit address mode)
    /// Based on BX_CPU_C::CMPSD32_XdYd in string.cc
    pub fn cmpsd32(&mut self, _instr: &BxInstructionGenerated) {
        let esi = self.esi() as u64;
        let edi = self.edi() as u64;
        
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        
        let src_addr = ds_base.wrapping_add(esi);
        let dst_addr = es_base.wrapping_add(edi);
        
        let op1 = self.mem_read_dword(src_addr);
        let op2 = self.mem_read_dword(dst_addr);
        
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub32(op1, op2, result);
        
        let increment = if self.get_df() { -4i32 } else { 4i32 };
        let new_esi = (esi as i64).wrapping_add(increment as i64) as u32;
        let new_edi = (edi as i64).wrapping_add(increment as i64) as u32;
        
        // Zero extension of RSI/RDI (matching original behavior)
        self.set_rsi(new_esi as u64);
        self.set_rdi(new_edi as u64);
        
        tracing::trace!("CMPSD32: [{:#010x}] vs [{:#010x}]", op1, op2);
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

    /// SCASD - Compare EAX with dword at ES:DI (16-bit address mode)
    pub fn scasd16(&mut self, _instr: &BxInstructionGenerated) {
        let di = self.di() as u64;
        let eax = self.eax();
        
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        let dst_addr = es_base.wrapping_add(di);
        let op2 = self.mem_read_dword(dst_addr);
        
        let result = eax.wrapping_sub(op2);
        self.update_flags_sub32(eax, op2, result);
        
        if self.get_df() {
            self.set_di(self.di().wrapping_sub(4));
        } else {
            self.set_di(self.di().wrapping_add(4));
        }
        
        tracing::trace!("SCASD16: EAX={:#010x} vs [{:#010x}]", eax, op2);
    }

    /// SCASD - Compare EAX with dword at ES:EDI (32-bit address mode)
    /// Based on BX_CPU_C::SCASD32_EAXYd in string.cc
    pub fn scasd32(&mut self, _instr: &BxInstructionGenerated) {
        let edi = self.edi() as u64;
        let eax = self.eax();
        
        let es_base = unsafe { self.sregs[BxSegregs::Es as usize].cache.u.segment.base };
        let dst_addr = es_base.wrapping_add(edi);
        let op2 = self.mem_read_dword(dst_addr);
        
        let result = eax.wrapping_sub(op2);
        self.update_flags_sub32(eax, op2, result);
        
        let increment = if self.get_df() { -4i32 } else { 4i32 };
        let new_edi = (edi as i64).wrapping_add(increment as i64) as u32;
        
        // Zero extension of RDI (matching original behavior)
        self.set_rdi(new_edi as u64);
        
        tracing::trace!("SCASD32: EAX={:#010x} vs [{:#010x}]", eax, op2);
    }

    // =========================================================================
    // REP prefix handling
    // =========================================================================
    
    /// REP MOVSB - Repeat move string byte CX times (16-bit address mode)
    pub fn rep_movsb16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.movsb16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REP MOVSB - Repeat move string byte ECX times (32-bit address mode)
    /// Based on BX_CPU_C::REP_MOVSB_YbXb with as32L() branch in string.cc
    pub fn rep_movsb32(&mut self, instr: &BxInstructionGenerated) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.movsb32(instr);
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
        }
        // Clear upper 32 bits of RCX/RSI/RDI (BX_CLEAR_64BIT_HIGH)
        self.set_rcx(self.ecx() as u64);
        // RSI/RDI are already zero-extended per-iteration inside movsb32
    }

    /// REP MOVSW - Repeat move string word CX times (16-bit address mode)
    pub fn rep_movsw16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.movsw16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REP MOVSW - Repeat move string word ECX times (32-bit address mode)
    /// Based on BX_CPU_C::REP_MOVSW_YwXw in string.cc
    pub fn rep_movsw32(&mut self, instr: &BxInstructionGenerated) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.movsw32(instr);
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
        }
        self.set_rcx(self.ecx() as u64);
    }

    /// REP STOSB - Repeat store string byte CX times (16-bit address mode)
    pub fn rep_stosb16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.stosb16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REP STOSB - Repeat store string byte ECX times (32-bit address mode)
    /// Based on BX_CPU_C::REP_STOSB_YbAL with as32L() branch in string.cc
    pub fn rep_stosb32(&mut self, instr: &BxInstructionGenerated) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.stosb32(instr);
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
        }
        // Clear upper 32 bits of RCX (BX_CLEAR_64BIT_HIGH)
        self.set_rcx(self.ecx() as u64);
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

    /// REP STOSD - Repeat store string dword CX times (16-bit address mode)
    /// Based on BX_CPU_C::REP_STOSD_YdEAX in string.cc
    pub fn rep_stosd16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.stosd16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REP STOSD - Repeat store string dword ECX times (32-bit address mode)
    /// Based on BX_CPU_C::REP_STOSD_YdEAX in string.cc
    pub fn rep_stosd32(&mut self, instr: &BxInstructionGenerated) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.stosd32(instr);
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
        }

        // Clear upper 32 bits of RCX (zero extension)
        self.set_rcx(self.ecx() as u64);
    }

    /// REP MOVSD - Repeat move string dword ECX times (32-bit address mode)
    /// Based on BX_CPU_C::REP_MOVSD_YdXd in string.cc:71-88
    /// Original: Bochs cpu/string.cc:71-88 REP_MOVSD_YdXd with as32L() check
    pub fn rep_movsd32(&mut self, instr: &BxInstructionGenerated) {
        let mut ecx = self.ecx();
        while ecx != 0 {
            self.movsd32(instr);
            ecx = ecx.wrapping_sub(1);
            self.set_ecx(ecx);
        }

        // Clear upper 32 bits of RCX (zero extension)
        // Original: Bochs cpu/string.cc:80-81 BX_CLEAR_64BIT_HIGH(BX_64BIT_REG_RSI/RDI)
        self.set_rcx(self.ecx() as u64);
    }

    /// REP MOVSD - Repeat move string dword CX times (16-bit address mode)
    /// Based on BX_CPU_C::REP_MOVSD_YdXd in string.cc:71-88
    pub fn rep_movsd16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.movsd16(instr);
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

    /// REPE SCASW - Repeat scan string word while equal
    pub fn repe_scasw16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasw16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() { break; }
        }
    }

    /// REPNE SCASW - Repeat scan string word while not equal
    pub fn repne_scasw16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasw16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() { break; }
        }
    }

    /// REPE SCASD - Repeat scan string dword while equal
    pub fn repe_scasd16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasd16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() { break; }
        }
    }

    /// REPNE SCASD - Repeat scan string dword while not equal
    pub fn repne_scasd16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.scasd16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() { break; }
        }
    }

    /// REP LODSW - Repeat load string word CX times
    pub fn rep_lodsw16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.lodsw16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
        }
    }

    /// REPE CMPSW - Repeat compare string word while equal
    pub fn repe_cmpsw16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsw16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() { break; }
        }
    }

    /// REPNE CMPSW - Repeat compare string word while not equal
    pub fn repne_cmpsw16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsw16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() { break; }
        }
    }

    /// REPE CMPSD - Repeat compare string dword while equal
    pub fn repe_cmpsd16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsd16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if !self.get_zf() { break; }
        }
    }

    /// REPNE CMPSD - Repeat compare string dword while not equal
    pub fn repne_cmpsd16(&mut self, instr: &BxInstructionGenerated) {
        let mut cx = self.cx();
        while cx != 0 {
            self.cmpsd16(instr);
            cx = cx.wrapping_sub(1);
            self.set_cx(cx);
            if self.get_zf() { break; }
        }
    }

    // =========================================================================
    // Memory access helpers using the stored memory pointer
    // =========================================================================
    
    pub(super) fn mem_read_byte(&self, addr: u64) -> u8 {
        // Prefer Bochs-style host access through the memory system when available:
        // - If direct host access is allowed, get_host_mem_addr returns Some(&mut [u8])
        // - If access is vetoed (MMIO/VGA/ROM handler), fall back to read_physical_page
        if let Some(mem_bus) = self.mem_bus {
            // SAFETY:
            // - `mem_bus` is only set for the duration of a CPU execution call.
            // - We only run one CPU today, so we rely on execution-time exclusivity.
            // - This intentionally uses interior mutability via raw pointer to avoid borrow overhead.
            let mem = unsafe { &mut *mem_bus.as_ptr() };
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let paddr: BxPhyAddress = addr as BxPhyAddress;

            if let Ok(Some(slice)) = mem.get_host_mem_addr(paddr, MemoryAccessType::Read, &[cpu_ref]) {
                let val = slice.get(0).copied().unwrap_or(0);
                return val;
            }

            let mut data = [0u8; 1];
            if mem.read_physical_page(&[cpu_ref], paddr, 1, &mut data).is_ok() {
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

    pub(super) fn mem_write_byte(&mut self, addr: u64, value: u8) {
        // Prefer Bochs-style host access through the memory system when available.
        if let Some(mem_bus) = self.mem_bus {
            // SAFETY: see mem_read_byte.
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
                // Invalidate icache for this page (SMC detection)
                self.i_cache.invalidate_page(addr & !0xFFF);
                return;
            }

            // Vetoed: go through handler-aware physical write.
            let mut dummy_mapping: [u32; 0] = [];
            let mut stamp = BxPageWriteStampTable::new(&mut dummy_mapping);
            let mut data = [value];
            let _ = mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 1, &mut data);
            // Invalidate icache for this page (SMC detection)
            self.i_cache.invalidate_page(addr & !0xFFF);
            return;
        }

        // Fallback: raw pointer access (best-effort) when not running inside cpu_loop.
        if let Some(ptr) = self.mem_ptr {
            let addr_usize = addr as usize;
            if addr_usize < self.mem_len {
                unsafe {
                    *ptr.add(addr_usize) = value;
                }
                // Invalidate icache for this page (SMC detection)
                self.i_cache.invalidate_page(addr & !0xFFF);
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
