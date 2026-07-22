//! String operations for x86 CPU emulation
//!
//! Based on Bochs string.cc
//!
//! Implements MOVS, STOS, LODS, CMPS, SCAS instructions
//!
//! Both 16-bit and 32-bit address variants use virtual memory access with
//! segment limit checks and paging translation (required for protected mode
//! with paging).

use super::{
    access::{
        forward_byte_copy, host_fill_bytes, host_offset, host_offset_mut, read_host_byte,
        read_unaligned_u16, read_unaligned_u32, write_host_byte, write_unaligned_u16,
        write_unaligned_u32,
    },
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

use crate::{
    config::BxPhyAddress,
    cpu::rusty_box::MemoryAccessType,
    memory::memory_rusty_box::bx_guest_ram_span,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // Helper: Get direction flag (DF)
    // =========================================================================

    /// Returns true if direction flag is set (decrement mode)
    #[inline]
    pub(super) fn get_df(&self) -> bool {
        self.eflags.contains(EFlags::DF)
    }

    /// Return an all-or-nothing direct bulk span measured in string elements.
    #[inline]
    fn fast_rep_elements(
        &self,
        guest_count: usize,
        source_remaining: Option<usize>,
        destination_remaining: usize,
        element_size: usize,
    ) -> Option<(usize, u32)> {
        let source_elements = source_remaining
            .unwrap_or(usize::MAX)
            .checked_div(element_size)?;
        let destination_elements = destination_remaining.checked_div(element_size)?;
        let event_elements = usize::try_from(self.ticks_left_next_event()).ok()?;
        let elements = guest_count
            .min(source_elements)
            .min(destination_elements)
            .min(event_elements);
        let bytes = elements.checked_mul(element_size)?;
        let bytes = u32::try_from(bytes).ok()?;
        (elements != 0).then_some((elements, bytes))
    }

    /// Check the complete canonical/LASS span before a direct 64-bit bulk access.
    #[inline]
    fn fast_rep_span64(
        &self,
        laddr: u64,
        bytes: u32,
        access: MemoryAccessType,
    ) -> bool {
        let Some(last_offset) = u64::from(bytes).checked_sub(1) else {
            return false;
        };
        let Some(last) = laddr.checked_add(last_offset) else {
            return false;
        };
        let user = self.user_pl();
        self.is_canonical_access(laddr, access, user)
            && self.is_canonical_access(last, access, user)
    }

    /// Prove a complete 32-bit-address MOVS direct-transfer chunk before mutation.
    #[inline]
    fn fast_rep_movs32_chunk(
        &mut self,
        source_seg: BxSegregs,
        source_offset: u32,
        destination_offset: u32,
        guest_count: u32,
        element_size: usize,
    ) -> super::Result<Option<(*const u8, *mut u8, BxPhyAddress, usize, u32)>> {
        if !self.direct_rep_bulk_allowed(false) || self.get_df() || self.async_event != 0 {
            return Ok(None);
        }
        let source_laddr = u64::from(self.get_laddr32(source_seg as usize, source_offset));
        let destination_laddr =
            u64::from(self.get_laddr32(BxSegregs::Es as usize, destination_offset));
        let Some((source_ptr, source_remaining)) = self.get_host_read_ptr(source_laddr)? else {
            return Ok(None);
        };
        let Some((destination_ptr, destination_remaining, destination_paddr)) =
            self.get_host_write_ptr_for_bulk(destination_laddr)?
        else {
            return Ok(None);
        };
        let Some(guest_count) = usize::try_from(guest_count).ok() else {
            return Ok(None);
        };
        let Some((elements, bytes)) = self.fast_rep_elements(
            guest_count,
            Some(source_remaining),
            destination_remaining,
            element_size,
        ) else {
            return Ok(None);
        };
        if !self.read_virtual_checks(source_seg as usize, source_offset, bytes)
            || !self.write_virtual_checks(BxSegregs::Es as usize, destination_offset, bytes)
            || self.smc_range_has_cached_code(destination_paddr, bytes)
        {
            return Ok(None);
        }
        Ok(Some((
            source_ptr,
            destination_ptr,
            destination_paddr,
            elements,
            bytes,
        )))
    }

    /// Prove a complete 32-bit-address STOS direct-transfer chunk before mutation.
    #[inline]
    fn fast_rep_stos32_chunk(
        &mut self,
        destination_offset: u32,
        guest_count: u32,
        element_size: usize,
    ) -> super::Result<Option<(*mut u8, BxPhyAddress, usize, u32)>> {
        if !self.direct_rep_bulk_allowed(false) || self.get_df() || self.async_event != 0 {
            return Ok(None);
        }
        let destination_laddr =
            u64::from(self.get_laddr32(BxSegregs::Es as usize, destination_offset));
        let Some((destination_ptr, destination_remaining, destination_paddr)) =
            self.get_host_write_ptr_for_bulk(destination_laddr)?
        else {
            return Ok(None);
        };
        let Some(guest_count) = usize::try_from(guest_count).ok() else {
            return Ok(None);
        };
        let Some((elements, bytes)) =
            self.fast_rep_elements(guest_count, None, destination_remaining, element_size)
        else {
            return Ok(None);
        };
        if !self.write_virtual_checks(BxSegregs::Es as usize, destination_offset, bytes)
            || self.smc_range_has_cached_code(destination_paddr, bytes)
        {
            return Ok(None);
        }
        Ok(Some((destination_ptr, destination_paddr, elements, bytes)))
    }

    /// Prove a complete 64-bit-address MOVS direct-transfer chunk before mutation.
    #[inline]
    fn fast_rep_movs64_chunk(
        &mut self,
        source_seg: BxSegregs,
        source_offset: u64,
        destination_offset: u64,
        guest_count: u64,
        element_size: usize,
    ) -> super::Result<Option<(*const u8, *mut u8, BxPhyAddress, usize, u32)>> {
        if !self.direct_rep_bulk_allowed(false) || self.get_df() || self.async_event != 0 {
            return Ok(None);
        }
        let source_laddr = self.get_laddr64(source_seg as usize, source_offset);
        let destination_laddr = self.get_laddr64(BxSegregs::Es as usize, destination_offset);
        let Some((source_ptr, source_remaining)) = self.get_host_read_ptr(source_laddr)? else {
            return Ok(None);
        };
        let Some((destination_ptr, destination_remaining, destination_paddr)) =
            self.get_host_write_ptr_for_bulk(destination_laddr)?
        else {
            return Ok(None);
        };
        let guest_count = usize::try_from(guest_count).unwrap_or(usize::MAX);
        let Some((elements, bytes)) = self.fast_rep_elements(
            guest_count,
            Some(source_remaining),
            destination_remaining,
            element_size,
        ) else {
            return Ok(None);
        };
        if !self.fast_rep_span64(source_laddr, bytes, MemoryAccessType::Read)
            || !self.fast_rep_span64(destination_laddr, bytes, MemoryAccessType::Write)
            || self.smc_range_has_cached_code(destination_paddr, bytes)
        {
            return Ok(None);
        }
        Ok(Some((
            source_ptr,
            destination_ptr,
            destination_paddr,
            elements,
            bytes,
        )))
    }

    /// Prove a complete 64-bit-address STOS direct-transfer chunk before mutation.
    #[inline]
    fn fast_rep_stos64_chunk(
        &mut self,
        destination_offset: u64,
        guest_count: u64,
        element_size: usize,
    ) -> super::Result<Option<(*mut u8, BxPhyAddress, usize, u32)>> {
        if !self.direct_rep_bulk_allowed(false) || self.get_df() || self.async_event != 0 {
            return Ok(None);
        }
        let destination_laddr = self.get_laddr64(BxSegregs::Es as usize, destination_offset);
        let Some((destination_ptr, destination_remaining, destination_paddr)) =
            self.get_host_write_ptr_for_bulk(destination_laddr)?
        else {
            return Ok(None);
        };
        let guest_count = usize::try_from(guest_count).unwrap_or(usize::MAX);
        let Some((elements, bytes)) =
            self.fast_rep_elements(guest_count, None, destination_remaining, element_size)
        else {
            return Ok(None);
        };
        if !self.fast_rep_span64(destination_laddr, bytes, MemoryAccessType::Write)
            || self.smc_range_has_cached_code(destination_paddr, bytes)
        {
            return Ok(None);
        }
        Ok(Some((destination_ptr, destination_paddr, elements, bytes)))
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
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.movsb16(instr)?;
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

    /// REP MOVSW CX times (16-bit)
    pub fn rep_movsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.movsw16(instr)?;
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

    /// REP MOVSD CX times (16-bit)
    pub fn rep_movsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.movsd16(instr)?;
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

    /// REP STOSB CX times (16-bit)
    pub fn rep_stosb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.stosb16(instr)?;
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

    /// REP STOSW CX times (16-bit)
    pub fn rep_stosw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.stosw16(instr)?;
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

    /// REP STOSD CX times (16-bit)
    pub fn rep_stosd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.stosd16(instr)?;
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

    /// REP LODSB CX times (16-bit)
    pub fn rep_lodsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.lodsb16(instr)?;
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

    /// REP LODSW CX times (16-bit)
    pub fn rep_lodsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.lodsw16(instr)?;
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

    /// REP LODSD CX times (16-bit)
    pub fn rep_lodsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if cx != 0 {
                self.lodsd16(instr)?;
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

    /// REPE CMPSB CX (16-bit)
    pub fn repe_cmpsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if cx != 0 {
                self.cmpsb16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if !self.get_zf() || cx == 0 {
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

    /// REPNE CMPSB CX (16-bit)
    pub fn repne_cmpsb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if cx != 0 {
                self.cmpsb16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if self.get_zf() || cx == 0 {
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

    /// REPE CMPSW CX (16-bit)
    pub fn repe_cmpsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if cx != 0 {
                self.cmpsw16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if !self.get_zf() || cx == 0 {
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

    /// REPNE CMPSW CX (16-bit)
    pub fn repne_cmpsw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if cx != 0 {
                self.cmpsw16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if self.get_zf() || cx == 0 {
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

    /// REPE CMPSD CX (16-bit)
    pub fn repe_cmpsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if cx != 0 {
                self.cmpsd16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if !self.get_zf() || cx == 0 {
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

    /// REPNE CMPSD CX (16-bit)
    pub fn repne_cmpsd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if cx != 0 {
                self.cmpsd16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if self.get_zf() || cx == 0 {
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

    /// REPE SCASB CX (16-bit)
    pub fn repe_scasb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if cx != 0 {
                self.scasb16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if !self.get_zf() || cx == 0 {
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

    /// REPNE SCASB CX (16-bit)
    pub fn repne_scasb16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if cx != 0 {
                self.scasb16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if self.get_zf() || cx == 0 {
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

    /// REPE SCASW CX (16-bit)
    pub fn repe_scasw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if cx != 0 {
                self.scasw16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if !self.get_zf() || cx == 0 {
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

    /// REPNE SCASW CX (16-bit)
    pub fn repne_scasw16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if cx != 0 {
                self.scasw16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if self.get_zf() || cx == 0 {
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

    /// REPE SCASD CX (16-bit)
    pub fn repe_scasd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if cx != 0 {
                self.scasd16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if !self.get_zf() || cx == 0 {
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

    /// REPNE SCASD CX (16-bit)
    pub fn repne_scasd16(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut cx = self.cx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if cx != 0 {
                self.scasd16(instr)?;
                self.on_repeat_iteration(instr);
                cx = cx.wrapping_sub(1);
                self.set_cx(cx);
            }
            if self.get_zf() || cx == 0 {
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

    // =========================================================================
    // REP prefix handling — 32-bit address mode (with paging translation)
    // =========================================================================

    /// REP MOVSB ECX times (32-bit)
    pub fn rep_movsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep direct chunks are proved in full before copying.
        while ecx != 0 && !df {
            let esi = self.esi();
            let edi = self.edi();
            let Some((src_ptr, dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_movs32_chunk(seg, esi, edi, ecx, 1)?
            else {
                break;
            };
            forward_byte_copy(src_ptr, dst_ptr, elements);
            let elements = u32::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rsi(esi.wrapping_add(bytes) as u64);
            self.set_rdi(edi.wrapping_add(bytes) as u64);
            ecx = ecx.wrapping_sub(elements);
            self.set_ecx(ecx);
            self.icount += u64::from(elements) - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("u32 fits usize"));
            if ecx == 0 {
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.movsb32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
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

    /// REP MOVSW ECX times (32-bit)
    pub fn rep_movsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep direct chunks are proved in full before copying.
        while ecx != 0 && !df {
            let esi = self.esi();
            let edi = self.edi();
            let Some((src_ptr, dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_movs32_chunk(seg, esi, edi, ecx, 2)?
            else {
                break;
            };
            forward_byte_copy(
                src_ptr,
                dst_ptr,
                usize::try_from(bytes).expect("u32 fits usize"),
            );
            let elements = u32::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rsi(esi.wrapping_add(bytes) as u64);
            self.set_rdi(edi.wrapping_add(bytes) as u64);
            ecx = ecx.wrapping_sub(elements);
            self.set_ecx(ecx);
            self.icount += u64::from(elements) - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("u32 fits usize"));
            if ecx == 0 {
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.movsw32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
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

    /// REP MOVSD ECX times (32-bit)
    pub fn rep_movsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let seg = BxSegregs::from(instr.seg());

        // FastRep direct chunks are proved in full before copying.
        while ecx != 0 && !df {
            let esi = self.esi();
            let edi = self.edi();
            let Some((src_ptr, dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_movs32_chunk(seg, esi, edi, ecx, 4)?
            else {
                break;
            };
            forward_byte_copy(
                src_ptr,
                dst_ptr,
                usize::try_from(bytes).expect("u32 fits usize"),
            );
            let elements = u32::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rsi(esi.wrapping_add(bytes) as u64);
            self.set_rdi(edi.wrapping_add(bytes) as u64);
            ecx = ecx.wrapping_sub(elements);
            self.set_ecx(ecx);
            self.icount += u64::from(elements) - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("u32 fits usize"));
            if ecx == 0 {
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.movsd32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
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

    /// REP STOSB ECX times (32-bit)
    pub fn rep_stosb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let al = self.al();

        // FastRep direct chunks are proved in full before filling.
        while ecx != 0 && !df {
            let edi = self.edi();
            let Some((dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_stos32_chunk(edi, ecx, 1)?
            else {
                break;
            };
            host_fill_bytes(dst_ptr, al, usize::try_from(bytes).expect("u32 fits usize"));
            let elements = u32::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rdi(edi.wrapping_add(bytes) as u64);
            ecx = ecx.wrapping_sub(elements);
            self.set_ecx(ecx);
            self.icount += u64::from(elements) - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("u32 fits usize"));
            if ecx == 0 {
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.stosb32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
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

    /// REP STOSW ECX times (32-bit)
    pub fn rep_stosw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let ax = self.ax();

        // FastRep direct chunks are proved in full before filling.
        while ecx != 0 && !df {
            let edi = self.edi();
            let Some((dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_stos32_chunk(edi, ecx, 2)?
            else {
                break;
            };
            let dst_slice = unsafe {
                super::access::host_slice_mut_u16(
                    dst_ptr,
                    usize::try_from(elements).expect("u32 fits usize"),
                )
            };
            for word in dst_slice.iter_mut() {
                *word = ax;
            }
            let elements = u32::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rdi(edi.wrapping_add(bytes) as u64);
            ecx = ecx.wrapping_sub(elements);
            self.set_ecx(ecx);
            self.icount += u64::from(elements) - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("u32 fits usize"));
            if ecx == 0 {
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.stosw32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
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

    /// REP STOSD ECX times (32-bit)
    pub fn rep_stosd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        let df = self.get_df();
        let eax = self.eax();

        // FastRep direct chunks are proved in full before filling.
        while ecx != 0 && !df {
            let edi = self.edi();
            let Some((dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_stos32_chunk(edi, ecx, 4)?
            else {
                break;
            };
            let dst_slice = unsafe {
                super::access::host_slice_mut_u32(
                    dst_ptr,
                    usize::try_from(elements).expect("u32 fits usize"),
                )
            };
            for dword in dst_slice.iter_mut() {
                *dword = eax;
            }
            let elements = u32::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rdi(edi.wrapping_add(bytes) as u64);
            ecx = ecx.wrapping_sub(elements);
            self.set_ecx(ecx);
            self.icount += u64::from(elements) - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("u32 fits usize"));
            if ecx == 0 {
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.stosd32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
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

    /// REP LODSB ECX times (32-bit)
    pub fn rep_lodsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.lodsb32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
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

    /// REP LODSW ECX times (32-bit)
    pub fn rep_lodsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.lodsw32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
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

    /// REP LODSD ECX times (32-bit)
    pub fn rep_lodsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if ecx != 0 {
                self.lodsd32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if ecx == 0 {
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

    /// REPE CMPSB ECX (32-bit)
    pub fn repe_cmpsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if ecx != 0 {
                self.cmpsb32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if !self.get_zf() || ecx == 0 {
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

    /// REPNE CMPSB ECX (32-bit)
    pub fn repne_cmpsb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if ecx != 0 {
                self.cmpsb32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if self.get_zf() || ecx == 0 {
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

    /// REPE CMPSW ECX (32-bit)
    pub fn repe_cmpsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if ecx != 0 {
                self.cmpsw32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if !self.get_zf() || ecx == 0 {
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

    /// REPNE CMPSW ECX (32-bit)
    pub fn repne_cmpsw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if ecx != 0 {
                self.cmpsw32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if self.get_zf() || ecx == 0 {
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

    /// REPE CMPSD ECX (32-bit)
    pub fn repe_cmpsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if ecx != 0 {
                self.cmpsd32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if !self.get_zf() || ecx == 0 {
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

    /// REPNE CMPSD ECX (32-bit)
    pub fn repne_cmpsd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if ecx != 0 {
                self.cmpsd32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if self.get_zf() || ecx == 0 {
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

    /// REPE SCASB ECX (32-bit)
    pub fn repe_scasb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if ecx != 0 {
                self.scasb32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if !self.get_zf() || ecx == 0 {
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

    /// REPNE SCASB ECX (32-bit)
    pub fn repne_scasb32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if ecx != 0 {
                self.scasb32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if self.get_zf() || ecx == 0 {
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

    /// REPE SCASW ECX (32-bit)
    pub fn repe_scasw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if ecx != 0 {
                self.scasw32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if !self.get_zf() || ecx == 0 {
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

    /// REPNE SCASW ECX (32-bit)
    pub fn repne_scasw32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if ecx != 0 {
                self.scasw32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if self.get_zf() || ecx == 0 {
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

    /// REPE SCASD ECX (32-bit)
    pub fn repe_scasd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if ecx != 0 {
                self.scasd32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if !self.get_zf() || ecx == 0 {
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

    /// REPNE SCASD ECX (32-bit)
    pub fn repne_scasd32(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut ecx = self.ecx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if ecx != 0 {
                self.scasd32(instr)?;
                self.on_repeat_iteration(instr);
                ecx = ecx.wrapping_sub(1);
                self.set_ecx(ecx);
            }
            if self.get_zf() || ecx == 0 {
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

    // =========================================================================
    // Memory access helpers using the stored memory pointer
    // =========================================================================


    #[inline(always)]
    fn direct_ram_offset(&self, addr: u64, len: usize) -> Option<(BxPhyAddress, usize)> {
        let a20 = addr & self.a20_mask;
        let end = a20.checked_add(u64::try_from(len).ok()?)?;
        let plain = (a20 < 0xA0000 && end <= 0xA0000) || a20 >= 0x100000;
        if !plain || self.mem_host_base.is_null() {
            return None;
        }
        bx_guest_ram_span(a20, len, self.mem_host_len).map(|span| (a20, span.start))
    }
    #[inline(always)]
    pub(super) fn mem_read_byte(&self, addr: u64) -> u8 {
        // Fast path: direct host pointer for plain RAM.
        // This matches what Bochs does via hostPageAddr in TLB entries — the vast
        // majority of physical accesses hit RAM and can be served with a single
        // pointer dereference.  We apply A20 masking and check the address is in
        // the plain-RAM range (below VGA at 0xA0000, or above BIOS shadow at 0x100000).
        if let Some((_a20_addr, linear)) = self.direct_ram_offset(addr, 1) {
            return read_host_byte(self.mem_host_base, linear);
        }

        self.mem_read_byte_slow(addr)
    }

    /// Slow path for mem_read_byte: MMIO/VGA/ROM through memory system handlers.
    /// Separated to keep the inlined fast path small for better icache utilization.
    #[cold]
    #[inline(never)]
    fn mem_read_byte_slow(&self, addr: u64) -> u8 {
        // LAPIC MMIO intercept at byte level (fallback for non-dword accesses)
        {
            let a20_addr = (addr & self.a20_mask) as BxPhyAddress;
            if self.lapic.is_selected(a20_addr) {
                // Read aligned dword, extract requested byte
                let aligned = a20_addr & !0x3;
                let dword = self.lapic.read(aligned, 4, self.icount);
                let byte_offset = (a20_addr & 0x3) as u32;
                return (dword >> (byte_offset * 8)) as u8;
            }
        }
        let paddr: BxPhyAddress = addr as BxPhyAddress;
        if let Some((policy, mem)) = unsafe { self.mem_bus_with_policy(paddr) } {
            if let Ok(Some(slice)) = mem.get_host_mem_addr_pinned(
                paddr,
                MemoryAccessType::Read,
                self.active_tlb_pins(),
                policy,
            ) {
                let val = slice.first().copied().unwrap_or(0);
                return val;
            }

            let mut data = [0u8; 1];
            if mem
                .read_physical_page(self.active_tlb_pins(), policy, paddr, 1, &mut data)
                .is_ok()
            {
                return data[0];
            }

            return 0;
        }


        0
    }

    #[inline(always)]
    pub(super) fn mem_write_byte(&mut self, addr: u64, value: u8) {
        // Fast path: direct host pointer for plain RAM.
        if let Some((a20_addr, linear)) = self.direct_ram_offset(addr, 1) {
            write_host_byte(self.mem_host_base, linear, value);
            self.smc_write_check(a20_addr, 1);
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
        {
            let a20_addr = (addr & self.a20_mask) as BxPhyAddress;
            if self.lapic.is_selected(a20_addr) {
                // Byte-level write to LAPIC: read-modify-write the aligned dword.
                // In practice, LAPIC is always accessed as dword — this is a safety net.
                let aligned = a20_addr & !0x3;
                // The two halves of this RMW take DIFFERENT time domains:
                // - LAPIC reads convert through `live_ticks(icount)` (apic.cc
                //   get_current_timer_count path), which subtracts the LAPIC's
                //   `icount_at_sync` — raw icount, like the sibling read paths.
                // - LAPIC writes store the argument directly into tick-domain
                //   state (apic.cc set_initial_timer_count: `ticksInitial =
                //   bx_pc_system.time_ticks()`; activation deadlines feed
                //   pc_system ticks) — `system_ticks()`, like the sibling
                //   word/dword write paths.
                let old = self.lapic.read(aligned, 4, self.icount);
                let byte_offset = (a20_addr & 0x3) as u32;
                let mask = !(0xFFu32 << (byte_offset * 8));
                let new_val = (old & mask) | ((value as u32) << (byte_offset * 8));
                let current_ticks = self.system_ticks();
                self.lapic.write(aligned, new_val, 4, current_ticks);
                self.sync_lapic_events();
                return;
            }
        }
        let paddr: BxPhyAddress = addr as BxPhyAddress;
        if let Some((policy, mem)) = unsafe { self.mem_bus_with_policy(paddr) } {
            if let Ok(Some(slice)) = mem.get_host_mem_addr_pinned(
                paddr,
                MemoryAccessType::Write,
                self.active_tlb_pins(),
                policy,
            ) {
                if let Some(b) = slice.get_mut(0) {
                    *b = value;
                }
                self.smc_write_check(paddr, 1);
                return;
            }

            // Vetoed: go through handler-aware physical write.
            let mut data = [value];
            if let Err(e) =
                mem.write_physical_page(self.active_tlb_pins(), policy, paddr, 1, &mut data)
            {
                tracing::warn!("physical write failed at paddr={:#x}: {e}", paddr);
            }
            self.smc_write_check(paddr, 1);
            return;
        }

    }

    #[inline(always)]
    pub(super) fn mem_read_word(&self, addr: u64) -> u16 {
        let a20_addr = addr & self.a20_mask;
        let crosses_physical_page = (a20_addr & 0x0fff) == 0x0fff;
        if !crosses_physical_page {
            // Fast path: direct host pointer for plain RAM.
            if let Some((_a20_addr, linear)) = self.direct_ram_offset(addr, 2) {
                return read_unaligned_u16(host_offset(self.mem_host_base, linear));
            }
            if self.lapic.is_selected(a20_addr as BxPhyAddress) {
                return self.lapic.read(a20_addr as BxPhyAddress, 2, self.icount) as u16;
            }
            let paddr = addr as BxPhyAddress;
            if let Some((policy, mem)) = unsafe { self.mem_bus_with_policy(paddr) } {
                let mut data = [0u8; 2];
                if mem
                    .read_physical_page(self.active_tlb_pins(), policy, paddr, 2, &mut data)
                    .is_ok()
                {
                    return u16::from_le_bytes(data);
                }
            }
        }

        // A physical page split has no single width-two transaction. Preserve
        // byte fallback behavior only for that case or after handler failure.
        let lo = self.mem_read_byte(addr) as u16;
        let hi = self.mem_read_byte(addr.wrapping_add(1)) as u16;
        lo | (hi << 8)
    }

    #[inline(always)]
    pub(super) fn mem_write_word(&mut self, addr: u64, value: u16) {
        let a20_addr = addr & self.a20_mask;
        let crosses_physical_page = (a20_addr & 0x0fff) == 0x0fff;
        if !crosses_physical_page {
            // Fast path: direct host pointer for plain RAM.
            if let Some((a20_addr, linear)) = self.direct_ram_offset(addr, 2) {
                write_unaligned_u16(host_offset_mut(self.mem_host_base, linear), value);
                self.smc_write_check(a20_addr, 2);
                return;
            }
            if self.lapic.is_selected(a20_addr as BxPhyAddress) {
                let current_ticks = self.system_ticks();
                self.lapic
                    .write(a20_addr as BxPhyAddress, u32::from(value), 2, current_ticks);
                self.sync_lapic_events();
                return;
            }
            let paddr = addr as BxPhyAddress;
            if let Some((policy, mem)) = unsafe { self.mem_bus_with_policy(paddr) } {
                let mut data = value.to_le_bytes();
                if mem
                    .write_physical_page(self.active_tlb_pins(), policy, paddr, 2, &mut data)
                    .is_ok()
                {
                    self.smc_write_check(paddr, 2);
                    return;
                }
            }
        }

        // A physical page split has no single width-two transaction. Preserve
        // byte fallback behavior only for that case or after handler failure.
        self.mem_write_byte(addr, value as u8);
        self.mem_write_byte(addr.wrapping_add(1), (value >> 8) as u8);
    }

    #[inline(always)]
    pub(super) fn mem_read_dword(&self, addr: u64) -> u32 {
        let a20_addr = addr & self.a20_mask;
        // Fast path: direct host pointer for plain RAM
        if let Some((_a20_addr, linear)) = self.direct_ram_offset(addr, 4) {
            return read_unaligned_u32(host_offset(self.mem_host_base, linear));
        }
        // LAPIC MMIO intercept: 32-bit aligned register access
        // Bochs apic.cc read() — LAPIC registers are always dword-accessed.
        if self.lapic.is_selected(a20_addr as BxPhyAddress) {
            return self.lapic.read(a20_addr as BxPhyAddress, 4, self.icount);
        }
        // Slow path: route through read_physical_page to hit registered MMIO handlers
        // (IOAPIC, VGA, etc.) with proper dword access width.
        let paddr: BxPhyAddress = addr as BxPhyAddress;
        if let Some((policy, mem)) = unsafe { self.mem_bus_with_policy(paddr) } {
            let mut data = [0u8; 4];
            if mem
                .read_physical_page(self.active_tlb_pins(), policy, paddr, 4, &mut data)
                .is_ok()
            {
                return u32::from_le_bytes(data);
            }
        }
        // Fallback: per-word reads
        let lo = self.mem_read_word(addr) as u32;
        let hi = self.mem_read_word(addr + 2) as u32;
        lo | (hi << 16)
    }

    pub(super) fn mem_write_dword(&mut self, addr: u64, value: u32) {
        let a20_addr = addr & self.a20_mask;
        // Fast path: direct host pointer for plain RAM
        if let Some((a20_addr, linear)) = self.direct_ram_offset(addr, 4) {
            write_unaligned_u32(host_offset_mut(self.mem_host_base, linear), value);
            self.smc_write_check(a20_addr, 4);
            return;
        }
        // LAPIC MMIO intercept: 32-bit aligned register access
        // Bochs apic.cc write() — LAPIC registers are always dword-accessed.
        if self.lapic.is_selected(a20_addr as BxPhyAddress) {
            let current_ticks = self.system_ticks();
            self.lapic
                .write(a20_addr as BxPhyAddress, value, 4, current_ticks);
            self.sync_lapic_events();
            return;
        }
        // Slow path: route through write_physical_page to hit registered MMIO handlers
        // (IOAPIC, VGA, etc.) with proper dword access width.
        let paddr: BxPhyAddress = addr as BxPhyAddress;
        if let Some((policy, mem)) = unsafe { self.mem_bus_with_policy(paddr) } {
            let mut data = value.to_le_bytes();
            if mem
                .write_physical_page(self.active_tlb_pins(), policy, paddr, 4, &mut data)
                .is_ok()
            {
                self.smc_write_check(paddr, 4);
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
        if rep {
            self.clear_rf();
        } // Bochs cpu.cc repeat(): clear_RF() when REP prefix is used
        if instr.as64_l() != 0 {
            if rep {
                self.rep_movsb64(instr)?;
            } else {
                self.movsb64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_movsb32(instr)?;
            } else {
                self.movsb32(instr)?;
            }
        } else {
            if rep {
                self.rep_movsb16(instr)?;
            } else {
                self.movsb16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch MOVSW: 16/32/64-bit address, with or without REP prefix.
    pub fn movsw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if rep {
            self.clear_rf();
        } // Bochs cpu.cc repeat(): clear_RF() when REP prefix is used
        if instr.as64_l() != 0 {
            if rep {
                self.rep_movsw64(instr)?;
            } else {
                self.movsw64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_movsw32(instr)?;
            } else {
                self.movsw32(instr)?;
            }
        } else {
            if rep {
                self.rep_movsw16(instr)?;
            } else {
                self.movsw16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch MOVSD: 16/32/64-bit address, with or without REP prefix.
    pub fn movsd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if rep {
            self.clear_rf();
        } // Bochs cpu.cc repeat(): clear_RF() when REP prefix is used
        if instr.as64_l() != 0 {
            if rep {
                self.rep_movsd64(instr)?;
            } else {
                self.movsd64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_movsd32(instr)?;
            } else {
                self.movsd32(instr)?;
            }
        } else {
            if rep {
                self.rep_movsd16(instr)?;
            } else {
                self.movsd16(instr)?;
            }
        }
        Ok(())
    }

    // ---- STOS ----

    /// Dispatch STOSB: 16/32/64-bit address, with or without REP prefix.
    pub fn stosb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if rep {
            self.clear_rf();
        } // Bochs cpu.cc repeat(): clear_RF() when REP prefix is used
        if instr.as64_l() != 0 {
            if rep {
                self.rep_stosb64(instr)?;
            } else {
                self.stosb64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_stosb32(instr)?;
            } else {
                self.stosb32(instr)?;
            }
        } else {
            if rep {
                self.rep_stosb16(instr)?;
            } else {
                self.stosb16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch STOSW: 16/32/64-bit address, with or without REP prefix.
    pub fn stosw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if rep {
            self.clear_rf();
        } // Bochs cpu.cc repeat(): clear_RF() when REP prefix is used
        if instr.as64_l() != 0 {
            if rep {
                self.rep_stosw64(instr)?;
            } else {
                self.stosw64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_stosw32(instr)?;
            } else {
                self.stosw32(instr)?;
            }
        } else {
            if rep {
                self.rep_stosw16(instr)?;
            } else {
                self.stosw16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch STOSD: 16/32/64-bit address, with or without REP prefix.
    pub fn stosd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if rep {
            self.clear_rf();
        } // Bochs cpu.cc repeat(): clear_RF() when REP prefix is used
        if instr.as64_l() != 0 {
            if rep {
                self.rep_stosd64(instr)?;
            } else {
                self.stosd64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_stosd32(instr)?;
            } else {
                self.stosd32(instr)?;
            }
        } else {
            if rep {
                self.rep_stosd16(instr)?;
            } else {
                self.stosd16(instr)?;
            }
        }
        Ok(())
    }

    // ---- LODS ----

    /// Dispatch LODSB: 16/32/64-bit address, with or without REP prefix.
    pub fn lodsb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if rep {
            self.clear_rf();
        } // Bochs cpu.cc repeat(): clear_RF() when REP prefix is used
        if instr.as64_l() != 0 {
            if rep {
                self.rep_lodsb64(instr)?;
            } else {
                self.lodsb64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_lodsb32(instr)?;
            } else {
                self.lodsb32(instr)?;
            }
        } else {
            if rep {
                self.rep_lodsb16(instr)?;
            } else {
                self.lodsb16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch LODSW: 16/32/64-bit address, with or without REP prefix.
    pub fn lodsw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if rep {
            self.clear_rf();
        } // Bochs cpu.cc repeat(): clear_RF() when REP prefix is used
        if instr.as64_l() != 0 {
            if rep {
                self.rep_lodsw64(instr)?;
            } else {
                self.lodsw64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_lodsw32(instr)?;
            } else {
                self.lodsw32(instr)?;
            }
        } else {
            if rep {
                self.rep_lodsw16(instr)?;
            } else {
                self.lodsw16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch LODSD: 16/32/64-bit address, with or without REP prefix.
    pub fn lodsd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value() != 0;
        if rep {
            self.clear_rf();
        } // Bochs cpu.cc repeat(): clear_RF() when REP prefix is used
        if instr.as64_l() != 0 {
            if rep {
                self.rep_lodsd64(instr)?;
            } else {
                self.lodsd64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep {
                self.rep_lodsd32(instr)?;
            } else {
                self.lodsd32(instr)?;
            }
        } else {
            if rep {
                self.rep_lodsd16(instr)?;
            } else {
                self.lodsd16(instr)?;
            }
        }
        Ok(())
    }

    // ---- SCAS (6-way: REPE=3, REPNE=2, none) ----

    /// Dispatch SCASB: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn scasb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep != 0 {
            self.clear_rf();
        } // Bochs cpu.cc repeat_ZF(): clear_RF() when REP/REPE/REPNE prefix used
        if instr.as64_l() != 0 {
            if rep == 3 {
                self.repe_scasb64(instr)?;
            } else if rep == 2 {
                self.repne_scasb64(instr)?;
            } else {
                self.scasb64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep == 3 {
                self.repe_scasb32(instr)?;
            } else if rep == 2 {
                self.repne_scasb32(instr)?;
            } else {
                self.scasb32(instr)?;
            }
        } else {
            if rep == 3 {
                self.repe_scasb16(instr)?;
            } else if rep == 2 {
                self.repne_scasb16(instr)?;
            } else {
                self.scasb16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch SCASW: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn scasw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep != 0 {
            self.clear_rf();
        } // Bochs cpu.cc repeat_ZF(): clear_RF() when REP/REPE/REPNE prefix used
        if instr.as64_l() != 0 {
            if rep == 3 {
                self.repe_scasw64(instr)?;
            } else if rep == 2 {
                self.repne_scasw64(instr)?;
            } else {
                self.scasw64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep == 3 {
                self.repe_scasw32(instr)?;
            } else if rep == 2 {
                self.repne_scasw32(instr)?;
            } else {
                self.scasw32(instr)?;
            }
        } else {
            if rep == 3 {
                self.repe_scasw16(instr)?;
            } else if rep == 2 {
                self.repne_scasw16(instr)?;
            } else {
                self.scasw16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch SCASD: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn scasd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep != 0 {
            self.clear_rf();
        } // Bochs cpu.cc repeat_ZF(): clear_RF() when REP/REPE/REPNE prefix used
        if instr.as64_l() != 0 {
            if rep == 3 {
                self.repe_scasd64(instr)?;
            } else if rep == 2 {
                self.repne_scasd64(instr)?;
            } else {
                self.scasd64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep == 3 {
                self.repe_scasd32(instr)?;
            } else if rep == 2 {
                self.repne_scasd32(instr)?;
            } else {
                self.scasd32(instr)?;
            }
        } else {
            if rep == 3 {
                self.repe_scasd16(instr)?;
            } else if rep == 2 {
                self.repne_scasd16(instr)?;
            } else {
                self.scasd16(instr)?;
            }
        }
        Ok(())
    }

    // ---- CMPS (6-way: REPE=3, REPNE=2, none) ----

    /// Dispatch CMPSB: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn cmpsb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep != 0 {
            self.clear_rf();
        } // Bochs cpu.cc repeat_ZF(): clear_RF() when REP/REPE/REPNE prefix used
        if instr.as64_l() != 0 {
            if rep == 3 {
                self.repe_cmpsb64(instr)?;
            } else if rep == 2 {
                self.repne_cmpsb64(instr)?;
            } else {
                self.cmpsb64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep == 3 {
                self.repe_cmpsb32(instr)?;
            } else if rep == 2 {
                self.repne_cmpsb32(instr)?;
            } else {
                self.cmpsb32(instr)?;
            }
        } else {
            if rep == 3 {
                self.repe_cmpsb16(instr)?;
            } else if rep == 2 {
                self.repne_cmpsb16(instr)?;
            } else {
                self.cmpsb16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch CMPSW: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn cmpsw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep != 0 {
            self.clear_rf();
        } // Bochs cpu.cc repeat_ZF(): clear_RF() when REP/REPE/REPNE prefix used
        if instr.as64_l() != 0 {
            if rep == 3 {
                self.repe_cmpsw64(instr)?;
            } else if rep == 2 {
                self.repne_cmpsw64(instr)?;
            } else {
                self.cmpsw64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep == 3 {
                self.repe_cmpsw32(instr)?;
            } else if rep == 2 {
                self.repne_cmpsw32(instr)?;
            } else {
                self.cmpsw32(instr)?;
            }
        } else {
            if rep == 3 {
                self.repe_cmpsw16(instr)?;
            } else if rep == 2 {
                self.repne_cmpsw16(instr)?;
            } else {
                self.cmpsw16(instr)?;
            }
        }
        Ok(())
    }

    /// Dispatch CMPSD: 16/32/64-bit address, with REPE/REPNE/no-REP prefix.
    pub fn cmpsd_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep != 0 {
            self.clear_rf();
        } // Bochs cpu.cc repeat_ZF(): clear_RF() when REP/REPE/REPNE prefix used
        if instr.as64_l() != 0 {
            if rep == 3 {
                self.repe_cmpsd64(instr)?;
            } else if rep == 2 {
                self.repne_cmpsd64(instr)?;
            } else {
                self.cmpsd64(instr)?;
            }
        } else if instr.as32_l() != 0 {
            if rep == 3 {
                self.repe_cmpsd32(instr)?;
            } else if rep == 2 {
                self.repne_cmpsd32(instr)?;
            } else {
                self.cmpsd32(instr)?;
            }
        } else {
            if rep == 3 {
                self.repe_cmpsd16(instr)?;
            } else if rep == 2 {
                self.repne_cmpsd16(instr)?;
            } else {
                self.cmpsd16(instr)?;
            }
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

        // FastRep direct chunks are proved in full before copying.
        while rcx != 0 && !df {
            let rsi = self.rsi();
            let rdi = self.rdi();
            let Some((src_ptr, dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_movs64_chunk(seg, rsi, rdi, rcx, 1)?
            else {
                break;
            };
            forward_byte_copy(src_ptr, dst_ptr, elements);
            let elements = u64::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rsi(rsi.wrapping_add(u64::from(bytes)));
            self.set_rdi(rdi.wrapping_add(u64::from(bytes)));
            rcx = rcx.wrapping_sub(elements);
            self.set_rcx(rcx);
            self.icount += elements - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("page-bounded element count"));
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.movsb64(instr)?;
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

        // FastRep direct chunks are proved in full before copying.
        while rcx != 0 && !df {
            let rsi = self.rsi();
            let rdi = self.rdi();
            let Some((src_ptr, dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_movs64_chunk(seg, rsi, rdi, rcx, 2)?
            else {
                break;
            };
            forward_byte_copy(
                src_ptr,
                dst_ptr,
                usize::try_from(bytes).expect("u32 fits usize"),
            );
            let elements = u64::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rsi(rsi.wrapping_add(u64::from(bytes)));
            self.set_rdi(rdi.wrapping_add(u64::from(bytes)));
            rcx = rcx.wrapping_sub(elements);
            self.set_rcx(rcx);
            self.icount += elements - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("page-bounded element count"));
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.movsw64(instr)?;
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

        // FastRep direct chunks are proved in full before copying.
        while rcx != 0 && !df {
            let rsi = self.rsi();
            let rdi = self.rdi();
            let Some((src_ptr, dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_movs64_chunk(seg, rsi, rdi, rcx, 4)?
            else {
                break;
            };
            forward_byte_copy(
                src_ptr,
                dst_ptr,
                usize::try_from(bytes).expect("u32 fits usize"),
            );
            let elements = u64::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rsi(rsi.wrapping_add(u64::from(bytes)));
            self.set_rdi(rdi.wrapping_add(u64::from(bytes)));
            rcx = rcx.wrapping_sub(elements);
            self.set_rcx(rcx);
            self.icount += elements - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("page-bounded element count"));
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.movsd64(instr)?;
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

    pub fn rep_stosb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let al = self.al();

        // FastRep direct chunks are proved in full before filling.
        while rcx != 0 && !df {
            let rdi = self.rdi();
            let Some((dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_stos64_chunk(rdi, rcx, 1)?
            else {
                break;
            };
            host_fill_bytes(dst_ptr, al, usize::try_from(bytes).expect("u32 fits usize"));
            let elements = u64::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rdi(rdi.wrapping_add(u64::from(bytes)));
            rcx = rcx.wrapping_sub(elements);
            self.set_rcx(rcx);
            self.icount += elements - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("page-bounded element count"));
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.stosb64(instr)?;
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

    /// STOSW with 64-bit addressing
    pub fn stosw64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let ax = self.ax();
        self.write_virtual_word_64(BxSegregs::Es, rdi, ax)?;
        let delta: u64 = if self.get_df() { (-2i64) as u64 } else { 2 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_stosw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let ax = self.ax();

        // FastRep direct chunks are proved in full before filling.
        while rcx != 0 && !df {
            let rdi = self.rdi();
            let Some((dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_stos64_chunk(rdi, rcx, 2)?
            else {
                break;
            };
            let dst_slice = unsafe {
                super::access::host_slice_mut_u16(
                    dst_ptr,
                    elements,
                )
            };
            for word in dst_slice.iter_mut() {
                *word = ax;
            }
            let elements = u64::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rdi(rdi.wrapping_add(u64::from(bytes)));
            rcx = rcx.wrapping_sub(elements);
            self.set_rcx(rcx);
            self.icount += elements - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("page-bounded element count"));
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.stosw64(instr)?;
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

    /// STOSD with 64-bit addressing
    pub fn stosd64(&mut self, _instr: &Instruction) -> super::Result<()> {
        let rdi = self.rdi();
        let eax = self.eax();
        self.write_virtual_dword_64(BxSegregs::Es, rdi, eax)?;
        let delta: u64 = if self.get_df() { (-4i64) as u64 } else { 4 };
        self.set_rdi(rdi.wrapping_add(delta));
        Ok(())
    }

    pub fn rep_stosd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let eax = self.eax();

        // FastRep direct chunks are proved in full before filling.
        while rcx != 0 && !df {
            let rdi = self.rdi();
            let Some((dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_stos64_chunk(rdi, rcx, 4)?
            else {
                break;
            };
            let dst_slice = unsafe {
                super::access::host_slice_mut_u32(
                    dst_ptr,
                    elements,
                )
            };
            for dword in dst_slice.iter_mut() {
                *dword = eax;
            }
            let elements = u64::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rdi(rdi.wrapping_add(u64::from(bytes)));
            rcx = rcx.wrapping_sub(elements);
            self.set_rcx(rcx);
            self.icount += elements - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("page-bounded element count"));
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.stosd64(instr)?;
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
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.lodsb64(instr)?;
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
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.lodsw64(instr)?;
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
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.lodsd64(instr)?;
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
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if rcx != 0 {
                self.cmpsb64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if !self.get_zf() || rcx == 0 {
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

    pub fn repne_cmpsb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if rcx != 0 {
                self.cmpsb64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if self.get_zf() || rcx == 0 {
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
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if rcx != 0 {
                self.cmpsw64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if !self.get_zf() || rcx == 0 {
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

    pub fn repne_cmpsw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if rcx != 0 {
                self.cmpsw64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if self.get_zf() || rcx == 0 {
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
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if rcx != 0 {
                self.cmpsd64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if !self.get_zf() || rcx == 0 {
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

    pub fn repne_cmpsd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if rcx != 0 {
                self.cmpsd64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if self.get_zf() || rcx == 0 {
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
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if rcx != 0 {
                self.scasb64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if !self.get_zf() || rcx == 0 {
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

    pub fn repne_scasb64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if rcx != 0 {
                self.scasb64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if self.get_zf() || rcx == 0 {
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
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if rcx != 0 {
                self.scasw64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if !self.get_zf() || rcx == 0 {
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

    pub fn repne_scasw64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if rcx != 0 {
                self.scasw64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if self.get_zf() || rcx == 0 {
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
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if rcx != 0 {
                self.scasd64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if !self.get_zf() || rcx == 0 {
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

    pub fn repne_scasd64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if rcx != 0 {
                self.scasd64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if self.get_zf() || rcx == 0 {
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

        // FastRep direct chunks are proved in full before copying.
        while rcx != 0 && !df {
            let rsi = self.rsi();
            let rdi = self.rdi();
            let Some((src_ptr, dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_movs64_chunk(seg, rsi, rdi, rcx, 8)?
            else {
                break;
            };
            forward_byte_copy(
                src_ptr,
                dst_ptr,
                usize::try_from(bytes).expect("u32 fits usize"),
            );
            let elements = u64::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rsi(rsi.wrapping_add(u64::from(bytes)));
            self.set_rdi(rdi.wrapping_add(u64::from(bytes)));
            rcx = rcx.wrapping_sub(elements);
            self.set_rcx(rcx);
            self.icount += elements - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("page-bounded element count"));
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.movsq64(instr)?;
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
    pub fn rep_stosq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        let df = self.get_df();
        let rax = self.rax();

        // FastRep direct chunks are proved in full before filling.
        while rcx != 0 && !df {
            let rdi = self.rdi();
            let Some((dst_ptr, paddr, elements, bytes)) =
                self.fast_rep_stos64_chunk(rdi, rcx, 8)?
            else {
                break;
            };
            let dst_slice = unsafe {
                super::access::host_slice_mut_u64(
                    dst_ptr,
                    elements,
                )
            };
            for qword in dst_slice.iter_mut() {
                *qword = rax;
            }
            let elements = u64::try_from(elements).expect("page-bounded element count");
            self.smc_write_check(paddr, bytes);
            self.set_rdi(rdi.wrapping_add(u64::from(bytes)));
            rcx = rcx.wrapping_sub(elements);
            self.set_rcx(rcx);
            self.icount += elements - 1;
            self.tickn_fastrep(usize::try_from(elements).expect("page-bounded element count"));
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

        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.stosq64(instr)?;
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
        // Bochs cpu.cc:395-467 repeat(): natural exit returns; async break
        // falls through to assert_RF + RIP=prev_rip + STOP_TRACE tail.
        loop {
            if rcx != 0 {
                self.lodsq64(instr)?;
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
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if rcx != 0 {
                self.cmpsq64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if !self.get_zf() || rcx == 0 {
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

    /// REPNE CMPSQ -- Compare RCX qwords, stop if equal
    pub fn repne_cmpsq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if rcx != 0 {
                self.cmpsq64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if self.get_zf() || rcx == 0 {
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
        // Bochs cpu.cc:470-602 repeat_ZF() rep==3 (F3/REPE): natural exit on !ZF||count==0.
        loop {
            if rcx != 0 {
                self.scasq64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if !self.get_zf() || rcx == 0 {
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

    /// REPNE SCASQ -- Scan RCX qwords, stop if equal
    pub fn repne_scasq64(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut rcx = self.rcx();
        // Bochs cpu.cc:470-602 repeat_ZF() rep==2 (F2/REPNE): natural exit on ZF||count==0.
        loop {
            if rcx != 0 {
                self.scasq64(instr)?;
                self.on_repeat_iteration(instr);
                rcx = rcx.wrapping_sub(1);
                self.set_rcx(rcx);
            }
            if self.get_zf() || rcx == 0 {
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

    // =========================================================================
    // 64-bit string dispatch functions (qword data)
    // =========================================================================

    /// Dispatch MOVSQ: 64-bit only, with or without REP prefix.
    pub fn movsq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.lock_rep_used_value() != 0 {
            self.clear_rf(); // Bochs cpu.cc repeat(): clear_RF() when REP prefix used
            self.rep_movsq64(instr)
        } else {
            self.movsq64(instr)
        }
    }

    /// Dispatch STOSQ: 64-bit only, with or without REP prefix.
    pub fn stosq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.lock_rep_used_value() != 0 {
            self.clear_rf(); // Bochs cpu.cc repeat(): clear_RF() when REP prefix used
            self.rep_stosq64(instr)
        } else {
            self.stosq64(instr)
        }
    }

    /// Dispatch LODSQ: 64-bit only, with or without REP prefix.
    pub fn lodsq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.lock_rep_used_value() != 0 {
            self.clear_rf(); // Bochs cpu.cc repeat(): clear_RF() when REP prefix used
            self.rep_lodsq64(instr)
        } else {
            self.lodsq64(instr)
        }
    }

    /// Dispatch CMPSQ: 64-bit only, with REPE/REPNE/no-REP prefix.
    pub fn cmpsq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep != 0 {
            self.clear_rf();
        } // Bochs cpu.cc repeat_ZF(): clear_RF() when REP/REPE/REPNE prefix used
        if rep == 3 {
            self.repe_cmpsq64(instr)
        } else if rep == 2 {
            self.repne_cmpsq64(instr)
        } else {
            self.cmpsq64(instr)
        }
    }

    /// Dispatch SCASQ: 64-bit only, with REPE/REPNE/no-REP prefix.
    pub fn scasq_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        let rep = instr.lock_rep_used_value();
        if rep != 0 {
            self.clear_rf();
        } // Bochs cpu.cc repeat_ZF(): clear_RF() when REP/REPE/REPNE prefix used
        if rep == 3 {
            self.repe_scasq64(instr)
        } else if rep == 2 {
            self.repne_scasq64(instr)
        } else {
            self.scasq64(instr)
        }
    }
}
