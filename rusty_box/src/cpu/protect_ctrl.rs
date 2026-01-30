//! Protected mode control instructions
//!
//! Based on Bochs protect_ctrl.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements LGDT, LIDT, LLDT, LTR

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::BxInstructionGenerated,
    descriptor::{BxDescriptor, SystemAndGateDescriptorEnum},
    exception::Exception,
    segment_ctrl_pro::parse_selector,
    Result,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// LGDT - Load Global Descriptor Table Register
    /// Loads the GDTR register from memory
    pub fn lgdt_ms(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("LGDT: CPL != 0 causes #GP");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Read the 6-byte pseudo-descriptor from memory
        // Format: 2 bytes limit, 4 bytes base
        let seg = instr.meta_data[0] as usize;
        let offset = instr.id() as u64;
        
        // Get segment base
        let seg_base = unsafe { self.sregs[seg].cache.u.segment.base };
        let addr = seg_base.wrapping_add(offset);
        
        let limit = self.mem_read_word(addr)?;
        let base_low = self.mem_read_dword(addr + 2)?;
        
        // In 16-bit mode, ignore upper 8 bits
        let base = if instr.osize() == 32 {
            base_low
        } else {
            base_low & 0x00ffffff
        };

        self.gdtr.limit = limit;
        self.gdtr.base = base as u64;

        tracing::trace!("LGDT: limit={:#06x}, base={:#010x}", limit, base);
        Ok(())
    }

    /// LIDT - Load Interrupt Descriptor Table Register
    /// Loads the IDTR register from memory
    pub fn lidt_ms(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("LIDT: CPL != 0 causes #GP");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Read the 6-byte pseudo-descriptor from memory
        let seg = instr.meta_data[0] as usize;
        let offset = instr.id() as u64;
        
        // Get segment base
        let seg_base = unsafe { self.sregs[seg].cache.u.segment.base };
        let addr = seg_base.wrapping_add(offset);
        
        let limit = self.mem_read_word(addr)?;
        let base_low = self.mem_read_dword(addr + 2)?;
        
        // In 16-bit mode, ignore upper 8 bits
        let base = if instr.osize() == 32 {
            base_low
        } else {
            base_low & 0x00ffffff
        };

        self.idtr.limit = limit;
        self.idtr.base = base as u64;

        tracing::trace!("LIDT: limit={:#06x}, base={:#010x}", limit, base);
        Ok(())
    }

    /// LLDT - Load Local Descriptor Table Register
    /// Loads the LDTR register from a selector
    pub fn lldt_ew(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // Must be in protected mode
        if !self.protected_mode() {
            tracing::error!("LLDT: not recognized in real or virtual-8086 mode");
            return Err(super::CpuError::Exception(Exception::Ud, 0));
        }

        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("LLDT: The current privilege level is not 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Read selector from source operand
        let raw_selector = if instr.mod_c0() != 0 {
            let src = instr.meta_data[1] as usize;
            self.get_gpr16(src)
        } else {
            let seg = instr.meta_data[0] as usize;
            let offset = instr.id() as u64;
            let seg_base = unsafe { self.sregs[seg].cache.u.segment.base };
            let addr = seg_base.wrapping_add(offset);
            self.mem_read_word(addr)?
        };

        // If selector is NULL, invalidate and done
        if (raw_selector & 0xfffc) == 0 {
            self.ldtr.selector.value = raw_selector;
            self.ldtr.cache.valid = 0;
            tracing::trace!("LLDT: NULL selector, invalidated");
            return Ok(());
        }

        // Parse selector
        let mut selector = super::descriptor::BxSelector::default();
        parse_selector(raw_selector, &mut selector);

        // Selector must point into GDT (TI must be 0)
        if selector.ti != 0 {
            tracing::error!("LLDT: selector.ti != 0");
            return Err(super::CpuError::Exception(Exception::Gp, raw_selector & 0xfffc));
        }

        // Fetch descriptor from GDT
        let (dword1, dword2) = self.fetch_raw_descriptor(&selector)?;
        
        // Parse descriptor
        let descriptor = self.parse_descriptor(dword1, dword2)?;

        // Check if it's an LDT descriptor
        if descriptor.valid == 0 || descriptor.segment || 
           descriptor.r#type != SystemAndGateDescriptorEnum::BxSysSegmentLdt as u8 {
            tracing::error!("LLDT: doesn't point to an LDT descriptor!");
            return Err(super::CpuError::Exception(Exception::Gp, raw_selector & 0xfffc));
        }

        // Check if present
        if !descriptor.p {
            tracing::error!("LLDT: LDT descriptor not present!");
            return Err(super::CpuError::Exception(Exception::Np, raw_selector & 0xfffc));
        }

        // Load LDTR
        self.ldtr.selector = selector;
        self.ldtr.cache = descriptor;
        self.ldtr.cache.valid = super::descriptor::SEG_VALID_CACHE;

        tracing::trace!("LLDT: loaded selector={:#06x}", raw_selector);
        Ok(())
    }

    /// LTR - Load Task Register
    /// Loads the TR register from a selector
    pub fn ltr_ew(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // Must be in protected mode
        if !self.protected_mode() {
            tracing::error!("LTR: not recognized in real or virtual-8086 mode");
            return Err(super::CpuError::Exception(Exception::Ud, 0));
        }

        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("LTR: The current privilege level is not 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Read selector from source operand
        let raw_selector = if instr.mod_c0() != 0 {
            let src = instr.meta_data[1] as usize;
            self.get_gpr16(src)
        } else {
            let seg = instr.meta_data[0] as usize;
            let offset = instr.id() as u64;
            let seg_base = unsafe { self.sregs[seg].cache.u.segment.base };
            let addr = seg_base.wrapping_add(offset);
            self.mem_read_word(addr)?
        };

        // NULL selector not allowed for LTR
        if (raw_selector & 0xfffc) == 0 {
            tracing::error!("LTR: loading with NULL selector!");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Parse selector
        let mut selector = super::descriptor::BxSelector::default();
        parse_selector(raw_selector, &mut selector);

        // Selector must point into GDT (TI must be 0)
        if selector.ti != 0 {
            tracing::error!("LTR: selector.ti != 0");
            return Err(super::CpuError::Exception(Exception::Gp, raw_selector & 0xfffc));
        }

        // Fetch descriptor from GDT
        let (dword1, dword2) = self.fetch_raw_descriptor(&selector)?;
        
        // Parse descriptor
        let mut descriptor = self.parse_descriptor(dword1, dword2)?;

        // Check if it's an available TSS descriptor
        let tss_type = descriptor.r#type;
        if descriptor.valid == 0 || descriptor.segment ||
           (tss_type != SystemAndGateDescriptorEnum::BxSysSegmentAvail286Tss as u8 &&
            tss_type != SystemAndGateDescriptorEnum::BxSysSegmentAvail386Tss as u8) {
            tracing::error!("LTR: doesn't point to an available TSS descriptor!");
            return Err(super::CpuError::Exception(Exception::Gp, raw_selector & 0xfffc));
        }

        // Check if present
        if !descriptor.p {
            tracing::error!("LTR: TSS descriptor not present!");
            return Err(super::CpuError::Exception(Exception::Np, raw_selector & 0xfffc));
        }

        // Load TR and mark TSS as busy
        self.tr.selector = selector;
        self.tr.cache = descriptor;
        self.tr.cache.valid = super::descriptor::SEG_VALID_CACHE;
        
        // Mark TSS as busy in the descriptor
        self.tr.cache.r#type |= 0x02; // Set busy bit

        // Also mark as busy in GDT (write back to memory)
        let gdt_base = self.gdtr.base;
        let gdt_offset = (selector.index as u64) * 8 + 4;
        let new_dword2 = dword2 | 0x0200; // Set busy bit
        self.mem_write_dword(gdt_base + gdt_offset, new_dword2)?;

        tracing::trace!("LTR: loaded selector={:#06x}, marked busy", raw_selector);
        Ok(())
    }

    // =========================================================================
    // MOV CRn - Control Register Operations
    // =========================================================================

    /// MOV CR0, r32 - Write to CR0
    pub fn mov_cr0_rd(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("MOV CR0: CPL != 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        let src = instr.meta_data[1] as usize;
        let val_32 = self.get_gpr32(src);

        // Set CR0 (bit 4 is hardwired to 1)
        self.cr0.set32(val_32);

        // Update CPU mode based on CR0.PE
        if self.cr0.pe() {
            self.cpu_mode = super::cpu::CpuMode::Ia32Protected;
        } else {
            self.cpu_mode = super::cpu::CpuMode::Ia32Real;
        }

        tracing::trace!("MOV CR0: {:#010x} (PE={})", val_32, self.cr0.pe());
        Ok(())
    }

    /// MOV CR2, r32 - Write to CR2 (page fault linear address)
    pub fn mov_cr2_rd(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("MOV CR2: CPL != 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        let src = instr.meta_data[1] as usize;
        let val_32 = self.get_gpr32(src);
        self.cr2 = val_32 as u64;

        tracing::trace!("MOV CR2: {:#010x}", val_32);
        Ok(())
    }

    /// MOV CR3, r32 - Write to CR3 (page directory base)
    pub fn mov_cr3_rd(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("MOV CR3: CPL != 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        let src = instr.meta_data[1] as usize;
        let val_32 = self.get_gpr32(src);
        self.cr3 = val_32 as u64;

        // Invalidate TLB
        // TODO: Implement TLB invalidation

        tracing::trace!("MOV CR3: {:#010x}", val_32);
        Ok(())
    }

    /// MOV CR4, r32 - Write to CR4
    pub fn mov_cr4_rd(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("MOV CR4: CPL != 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        let src = instr.meta_data[1] as usize;
        let val_32 = self.get_gpr32(src);
        self.cr4.set32(val_32);

        tracing::trace!("MOV CR4: {:#010x}", val_32);
        Ok(())
    }

    // =========================================================================
    // MOV Rd, CRn - Control Register Read Operations
    // =========================================================================

    /// MOV r32, CR0 - Read CR0 into register
    pub fn mov_rd_cr0(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("MOV r32, CR0: CPL != 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Read CR0 value
        let val_32 = self.cr0.get32();

        // Write to destination register
        let dst = instr.meta_data[0] as usize;
        self.set_gpr32(dst, val_32);

        tracing::trace!("MOV r32, CR0: {:#010x}", val_32);
        Ok(())
    }

    /// MOV r32, CR2 - Read CR2 into register (page fault linear address)
    pub fn mov_rd_cr2(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("MOV r32, CR2: CPL != 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Read CR2 value
        let val_32 = self.cr2 as u32;

        // Write to destination register
        let dst = instr.meta_data[0] as usize;
        self.set_gpr32(dst, val_32);

        tracing::trace!("MOV r32, CR2: {:#010x}", val_32);
        Ok(())
    }

    /// MOV r32, CR3 - Read CR3 into register (page directory base)
    pub fn mov_rd_cr3(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("MOV r32, CR3: CPL != 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Read CR3 value
        let val_32 = self.cr3 as u32;

        // Write to destination register
        let dst = instr.meta_data[0] as usize;
        self.set_gpr32(dst, val_32);

        tracing::trace!("MOV r32, CR3: {:#010x}", val_32);
        Ok(())
    }

    /// MOV r32, CR4 - Read CR4 into register
    pub fn mov_rd_cr4(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        // CPL must be 0
        if self.get_cpl() != 0 {
            tracing::error!("MOV r32, CR4: CPL != 0");
            return Err(super::CpuError::Exception(Exception::Gp, 0));
        }

        // Read CR4 value
        let val_32 = self.cr4.get32();

        // Write to destination register
        let dst = instr.meta_data[0] as usize;
        self.set_gpr32(dst, val_32);

        tracing::trace!("MOV r32, CR4: {:#010x}", val_32);
        Ok(())
    }
}
