use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn handle_cpu_context_change(&mut self) {
        self.tlb_flush();

        self.invalidate_prefetch_q();
        self.invalidate_stack_cache();

        self.handle_interrupt_mask_change();

        self.handle_alignment_check();

        // FIXME: implement these
        // handleCpuModeChange();
        //
        // handleFpuMmxModeChange();
        // handleSseModeChange();
        // handleAvxModeChange();
    }

    fn handle_alignment_check(&mut self) {
        if self.cl() == 3 && self.cr0.am() && self.get_ac() != 0 {
            self.alignment_check_mask = 0xf;
        } else {
            self.alignment_check_mask = 0;
        }
    }

    /// Get the Time Stamp Counter value
    /// 
    /// # Arguments
    /// * `system_ticks` - Current system ticks from BxPcSystemC
    pub fn get_tsc(&self, system_ticks: u64) -> u64 {
        system_ticks.wrapping_add(self.tsc_adjust as u64)
    }

    /// Set the Time Stamp Counter to a specific value
    /// 
    /// # Arguments
    /// * `newval` - The value to set TSC to
    /// * `system_ticks` - Current system ticks from BxPcSystemC
    pub fn set_tsc(&mut self, newval: u64, system_ticks: u64) {
        // compute the correct setting of tsc_adjust so that a get_tsc()
        // will return newval
        self.tsc_adjust = newval.wrapping_sub(system_ticks) as i64
    }

    /// LIDT - Load Interrupt Descriptor Table Register
    /// Loads IDTR from a 6-byte memory operand (2-byte limit, 4-byte base)
    pub fn lidt_ms(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        use super::decoder::BxSegregs;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr);
        let base = self.read_virtual_dword(seg, eaddr.wrapping_add(2)) as u64;
        self.idtr.base = base;
        self.idtr.limit = limit;
        tracing::trace!("LIDT: base={:#010x}, limit={:#06x}", base, limit);
        Ok(())
    }

    /// LGDT - Load Global Descriptor Table Register
    /// Loads GDTR from a 6-byte memory operand (2-byte limit, 4-byte base)
    pub fn lgdt_ms(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        use super::decoder::BxSegregs;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr);
        let base = self.read_virtual_dword(seg, eaddr.wrapping_add(2)) as u64;
        self.gdtr.base = base;
        self.gdtr.limit = limit;
        tracing::trace!("LGDT: base={:#010x}, limit={:#06x}", base, limit);
        Ok(())
    }

    // =========================================================================
    // MOV Rd, CRn - Control Register Read Operations
    // =========================================================================

    /// MOV r32, CR0 - Read CR0 into register
    pub fn mov_rd_cr0(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        // TODO: Add CPL check (CPL must be 0)
        // For now, BIOS is always in ring 0 so this is safe

        // Read CR0 value
        let val_32 = self.cr0.get32();

        // Write to destination register
        let dst = instr.dst() as usize;
        self.set_gpr32(dst, val_32);

        tracing::trace!("MOV r32, CR0: {:#010x}", val_32);
        Ok(())
    }

    /// MOV r32, CR2 - Read CR2 into register (page fault linear address)
    pub fn mov_rd_cr2(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        // TODO: Add CPL check (CPL must be 0)
        // For now, BIOS is always in ring 0 so this is safe

        // Read CR2 value
        let val_32 = self.cr2 as u32;

        // Write to destination register
        let dst = instr.dst() as usize;
        self.set_gpr32(dst, val_32);

        tracing::trace!("MOV r32, CR2: {:#010x}", val_32);
        Ok(())
    }

    /// MOV r32, CR3 - Read CR3 into register (page directory base)
    pub fn mov_rd_cr3(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        // TODO: Add CPL check (CPL must be 0)
        // For now, BIOS is always in ring 0 so this is safe

        // Read CR3 value
        let val_32 = self.cr3 as u32;

        // Write to destination register
        let dst = instr.dst() as usize;
        self.set_gpr32(dst, val_32);

        tracing::trace!("MOV r32, CR3: {:#010x}", val_32);
        Ok(())
    }

    /// MOV r32, CR4 - Read CR4 into register
    pub fn mov_rd_cr4(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        // TODO: Add CPL check (CPL must be 0)
        // For now, BIOS is always in ring 0 so this is safe

        // Read CR4 value
        let val_32 = self.cr4.get32();

        // Write to destination register
        let dst = instr.dst() as usize;
        self.set_gpr32(dst, val_32);

        tracing::trace!("MOV r32, CR4: {:#010x}", val_32);
        Ok(())
    }

    // =========================================================================
    // MOV CRn, Rd - Control Register Write Operations
    // =========================================================================

    /// MOV CR0, r32 - Write to CR0
    pub fn mov_cr0_rd(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        // TODO: Add CPL check (CPL must be 0)
        // For now, BIOS is always in ring 0 so this is safe

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);

        // Set CR0 (bit 4 is hardwired to 1)
        self.cr0.set32(val_32);

        // Update CPU mode based on CR0.PE
        if self.cr0.pe() {
            self.cpu_mode = super::cpu::CpuMode::Ia32Protected;
        } else {
            self.cpu_mode = super::cpu::CpuMode::Ia32Real;
        }

        tracing::trace!("MOV CR0, r32: {:#010x} (PE={})", val_32, self.cr0.pe());
        Ok(())
    }

    /// MOV CR2, r32 - Write to CR2 (page fault linear address)
    pub fn mov_cr2_rd(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        // TODO: Add CPL check (CPL must be 0)
        // For now, BIOS is always in ring 0 so this is safe

        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        self.cr2 = val_32 as u64;

        tracing::trace!("MOV CR2, r32: {:#010x}", val_32);
        Ok(())
    }

    /// MOV CR3, r32 - Write to CR3 (page directory base)
    pub fn mov_cr3_rd(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        // TODO: Add CPL check (CPL must be 0)
        // For now, BIOS is always in ring 0 so this is safe

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        self.cr3 = val_32 as u64;

        // Invalidate TLB
        // TODO: Implement TLB invalidation

        tracing::trace!("MOV CR3, r32: {:#010x}", val_32);
        Ok(())
    }

    /// MOV CR4, r32 - Write to CR4
    pub fn mov_cr4_rd(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        // TODO: Add CPL check (CPL must be 0)
        // For now, BIOS is always in ring 0 so this is safe

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        self.cr4.set32(val_32);

        tracing::trace!("MOV CR4, r32: {:#010x}", val_32);
        Ok(())
    }

    /// LMSW - Load Machine Status Word
    /// Original: Bochs cpu/crregs.cc:870-914 LMSW_Ew
    /// Sets low 4 bits of CR0 (PE, MP, EM, TS). Cannot clear PE once set.
    pub fn lmsw_ew(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        let msw = if instr.mod_c0() {
            // Register form: r/m field (meta_data[0]) has the source register
            self.get_gpr16(instr.meta_data[0] as usize)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = super::decoder::BxSegregs::from(instr.seg());
            self.read_virtual_word(seg, eaddr)
        };

        // LMSW cannot clear PE
        let mut msw = msw;
        if self.cr0.pe() {
            msw |= 1; // keep PE set
        }

        // LMSW only affects low 4 bits (PE, MP, EM, TS)
        let msw = msw & 0xF;
        let cr0_val = (self.cr0.get32() & 0xFFFFFFF0) | msw as u32;

        self.cr0.set32(cr0_val);

        // Update CPU mode based on CR0.PE
        if self.cr0.pe() {
            self.cpu_mode = super::cpu::CpuMode::Ia32Protected;
        } else {
            self.cpu_mode = super::cpu::CpuMode::Ia32Real;
        }

        // Invalidate prefetch
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        tracing::debug!("LMSW: msw={:#06x}, CR0={:#010x} (PE={})", msw, cr0_val, self.cr0.pe());
        Ok(())
    }
}
