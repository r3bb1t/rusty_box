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
    pub fn lidt_ms(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        use super::decoder::BxSegregs;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr)?;
        let base = self.read_virtual_dword(seg, eaddr.wrapping_add(2))? as u64;
        self.idtr.base = base;
        self.idtr.limit = limit;
        tracing::trace!("LIDT: base={:#010x}, limit={:#06x}", base, limit);
        Ok(())
    }

    /// LGDT - Load Global Descriptor Table Register
    /// Loads GDTR from a 6-byte memory operand (2-byte limit, 4-byte base)
    pub fn lgdt_ms(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        use super::decoder::BxSegregs;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr)?;
        let base = self.read_virtual_dword(seg, eaddr.wrapping_add(2))? as u64;
        self.gdtr.base = base;
        self.gdtr.limit = limit;
        tracing::trace!("LGDT: base={:#010x}, limit={:#06x}", base, limit);
        Ok(())
    }

    // =========================================================================
    // MOV Rd, CRn - Control Register Read Operations
    // =========================================================================

    /// MOV r32, CR0 - Read CR0 into register
    /// Note: decoder puts nnn (CR#) in meta_data[0]=dst, rm (GPR) in meta_data[1]=src
    pub fn mov_rd_cr0(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let val_32 = self.cr0.get32();
        let gpr = instr.src() as usize; // rm field = GPR
        self.set_gpr32(gpr, val_32);
        tracing::trace!("MOV r32, CR0: {:#010x} -> reg{}", val_32, gpr);
        Ok(())
    }

    /// MOV r32, CR2 - Read CR2 into register (page fault linear address)
    pub fn mov_rd_cr2(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let val_32 = self.cr2 as u32;
        let gpr = instr.src() as usize; // rm field = GPR
        self.set_gpr32(gpr, val_32);
        tracing::trace!("MOV r32, CR2: {:#010x} -> reg{}", val_32, gpr);
        Ok(())
    }

    /// MOV r32, CR3 - Read CR3 into register (page directory base)
    pub fn mov_rd_cr3(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let val_32 = self.cr3 as u32;
        let gpr = instr.src() as usize; // rm field = GPR
        self.set_gpr32(gpr, val_32);
        tracing::trace!("MOV r32, CR3: {:#010x} -> reg{}", val_32, gpr);
        Ok(())
    }

    /// MOV r32, CR4 - Read CR4 into register
    pub fn mov_rd_cr4(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let val_32 = self.cr4.get32();
        let gpr = instr.src() as usize; // rm field = GPR
        self.set_gpr32(gpr, val_32);
        tracing::trace!("MOV r32, CR4: {:#010x} -> reg{}", val_32, gpr);
        Ok(())
    }

    // =========================================================================
    // MOV CRn, Rd - Control Register Write Operations
    // =========================================================================

    /// MOV CR0, r32 - Write to CR0
    /// Matching Bochs crregs.cc SetCR0(): flushes TLB when PG/PE/WP change (mask 0x80010001)
    pub fn mov_cr0_rd(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        let old_cr0 = self.cr0.get32();

        // Set CR0
        self.cr0.set32(val_32);

        // Update CPU mode based on CR0.PE
        if self.cr0.pe() {
            self.cpu_mode = super::cpu::CpuMode::Ia32Protected;
        } else {
            self.cpu_mode = super::cpu::CpuMode::Ia32Real;
        }

        // Bochs crregs.cc:1158-1163: Modification of PG, PE, or WP flushes TLB
        if (old_cr0 & 0x80010001) != (val_32 & 0x80010001) {
            self.tlb_flush();
        } else {
            // Even without PG/PE/WP change, invalidate prefetch queue
            self.invalidate_prefetch_q();
        }

        tracing::trace!("MOV CR0, r32: {:#010x} -> {:#010x} (PE={}, PG={})",
            old_cr0, val_32, self.cr0.pe(), (val_32 >> 31) & 1);
        Ok(())
    }

    /// MOV CR2, r32 - Write to CR2 (page fault linear address)
    pub fn mov_cr2_rd(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        // TODO: Add CPL check (CPL must be 0)
        // For now, BIOS is always in ring 0 so this is safe

        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        self.cr2 = val_32 as u64;

        tracing::trace!("MOV CR2, r32: {:#010x}", val_32);
        Ok(())
    }

    /// MOV CR3, r32 - Write to CR3 (page directory base)
    /// Matching Bochs crregs.cc SetCR3(): always flushes TLB (non-global if PGE set)
    pub fn mov_cr3_rd(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        self.cr3 = val_32 as u64;

        // Bochs crregs.cc:1423-1445: flush TLB even if value does not change
        if self.cr4.pge() {
            self.tlb_flush_non_global();
        } else {
            self.tlb_flush();
        }

        tracing::trace!("MOV CR3, r32: {:#010x}", val_32);
        Ok(())
    }

    /// MOV CR4, r32 - Write to CR4
    /// Matching Bochs crregs.cc SetCR4(): flushes TLB when paging-related bits change
    pub fn mov_cr4_rd(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        let old_cr4 = self.cr4.get32();
        self.cr4.set32(val_32);

        // Bochs crregs.cc: SetCR4 flushes TLB when PAE, PGE, SMEP, SMAP, PKE, CET change
        // Simplified: flush TLB on any CR4 change that affects paging
        if old_cr4 != val_32 {
            self.tlb_flush();
        } else {
            self.invalidate_prefetch_q();
        }

        tracing::trace!("MOV CR4, r32: {:#010x}", val_32);
        Ok(())
    }

    /// LMSW - Load Machine Status Word
    /// Original: Bochs cpu/crregs.cc:870-914 LMSW_Ew
    /// Sets low 4 bits of CR0 (PE, MP, EM, TS). Cannot clear PE once set.
    pub fn lmsw_ew(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let msw = if instr.mod_c0() {
            // Register form: r/m field (meta_data[0]) has the source register
            self.get_gpr16(instr.meta_data[0] as usize)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = super::decoder::BxSegregs::from(instr.seg());
            self.read_virtual_word(seg, eaddr)?
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

    // =========================================================================
    // Flag manipulation instructions
    // Bochs: flag_ctrl.cc — CLC, STC, CMC, CLI, STI, CLD, STD
    // =========================================================================

    /// CLC — Clear Carry Flag (opcode 0xF8)
    /// Bochs: flag_ctrl.cc clear_CF()
    pub(super) fn clc(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags &= !(1 << 0);
        Ok(())
    }

    /// STC — Set Carry Flag (opcode 0xF9)
    /// Bochs: flag_ctrl.cc assert_CF()
    pub(super) fn stc(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags |= 1 << 0;
        Ok(())
    }

    /// CMC — Complement Carry Flag (opcode 0xF5)
    /// Bochs: flag_ctrl.cc set_CF(!get_CF())
    pub(super) fn cmc(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags ^= 1 << 0;
        Ok(())
    }

    /// CLI — Clear Interrupt Flag (opcode 0xFA)
    /// Bochs: flag_ctrl.cc clear_IF()
    /// Note: Full Bochs checks IOPL/CPL in protected/v8086 mode — simplified here
    pub(super) fn cli(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags &= !(1 << 9);
        tracing::debug!("CLI: Interrupts disabled");
        Ok(())
    }

    /// STI — Set Interrupt Flag (opcode 0xFB)
    /// Bochs: flag_ctrl.cc assert_IF() + inhibit_interrupts(BX_INHIBIT_INTERRUPTS)
    /// Note: Full Bochs checks IOPL/CPL in protected/v8086 mode — simplified here
    pub(super) fn sti(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags |= 1 << 9;
        tracing::debug!("STI: Interrupts enabled");
        Ok(())
    }

    /// CLD — Clear Direction Flag (opcode 0xFC)
    /// Bochs: flag_ctrl.cc clear_DF()
    pub(super) fn cld(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags &= !(1 << 10);
        tracing::debug!("CLD: Direction flag cleared");
        Ok(())
    }

    /// STD — Set Direction Flag (opcode 0xFD)
    /// Bochs: flag_ctrl.cc assert_DF()
    pub(super) fn std_(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags |= 1 << 10;
        tracing::debug!("STD: Direction flag set");
        Ok(())
    }

    // =========================================================================
    // System control instructions
    // Bochs: proc_ctrl.cc (WBINVD), paging.cc (INVLPG), crregs.cc (CLTS)
    // =========================================================================

    /// WBINVD — Write-back and Invalidate Cache (opcode 0F 09)
    /// Bochs: proc_ctrl.cc — no-op aside from VMX/SVM intercepts
    pub(super) fn wbinvd(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        tracing::trace!("WBINVD: no-op (no cache)");
        Ok(())
    }

    /// INVLPG — Invalidate TLB Entry (opcode 0F 01 /7)
    /// Bochs: paging.cc — resolves linear address, flushes from TLB
    pub(super) fn invlpg(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let laddr = self.get_laddr32(seg as usize, eaddr);
        self.dtlb.invlpg(laddr.into());
        self.itlb.invlpg(laddr.into());
        self.invalidate_prefetch_q();
        tracing::trace!("INVLPG: laddr={:#x}", laddr);
        Ok(())
    }

    /// CLTS — Clear Task-Switched flag in CR0 (opcode 0F 06)
    /// Bochs: crregs.cc — clears CR0.TS (bit 3)
    pub(super) fn clts(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let cr0_val = self.cr0.get32();
        self.cr0.set32(cr0_val & !(1u32 << 3));
        tracing::trace!("CLTS: CR0.TS cleared, CR0={:#010x}", cr0_val & !(1u32 << 3));
        Ok(())
    }

    // =========================================================================
    // MSR instructions
    // Bochs: msr.cc — RDMSR / WRMSR
    // =========================================================================

    /// RDMSR — Read Model-Specific Register (opcode 0F 32)
    /// Bochs: msr.cc — reads ECX as MSR index, returns value in EDX:EAX
    /// Stubbed: only APIC_BASE (0x1B) and MTRRCAP (0xFE) return non-zero
    pub(super) fn rdmsr(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let msr = self.ecx();
        let val: u64 = match msr {
            0x1B => 0xFEE00900,   // IA32_APIC_BASE: APIC enabled, base 0xFEE00000
            0xFE => 0x0508,       // IA32_MTRRCAP: 8 variable MTRRs, WC type supported
            _ => 0,
        };
        tracing::debug!("RDMSR: MSR={:#010x} -> {:#018x}", msr, val);
        self.set_rax((val & 0xFFFF_FFFF) as u64);
        self.set_rdx((val >> 32) as u64);
        Ok(())
    }

    /// WRMSR — Write Model-Specific Register (opcode 0F 30)
    /// Bochs: msr.cc — reads ECX as MSR index, value from EDX:EAX
    /// Stubbed: logs and ignores all writes
    pub(super) fn wrmsr(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let msr = self.ecx();
        let val = ((self.edx() as u64) << 32) | (self.eax() as u64);
        tracing::debug!("WRMSR: MSR={:#010x} = {:#018x} (ignored)", msr, val);
        Ok(())
    }

    // =========================================================================
    // MOV Rd, DRn / MOV DRn, Rd — Debug Register Operations (0F 21 / 0F 23)
    // =========================================================================

    /// MOV r32, DRn (0F 21 /r) — Read debug register into GPR
    /// Bochs: crregs.cc MOV_RdDd
    pub(super) fn mov_rd_dd(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let dr_idx = instr.src1() as usize; // nnn = DR index
        let dst_gpr = instr.dst() as usize; // rm = destination GPR
        let val: u32 = match dr_idx {
            0..=3 => self.dr[dr_idx] as u32,
            4 | 6 => self.dr6.val32,
            5 | 7 => self.dr7.val32,
            _ => 0,
        };
        self.set_gpr32(dst_gpr, val);
        tracing::trace!("MOV r32, DR{}: DR{}={:#010x} -> reg{}", dr_idx, dr_idx, val, dst_gpr);
        Ok(())
    }

    /// MOV DRn, r32 (0F 23 /r) — Write GPR into debug register
    /// Bochs: crregs.cc MOV_DdRd
    pub(super) fn mov_dd_rd(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let dr_idx = instr.dst() as usize; // nnn = DR index (destination)
        let src_gpr = instr.src1() as usize; // rm = source GPR
        let val = self.get_gpr32(src_gpr);
        match dr_idx {
            0..=3 => { self.dr[dr_idx] = val as u64; }
            4 | 6 => { self.dr6.val32 = val; }
            5 | 7 => { self.dr7.val32 = val; }
            _ => {}
        }
        tracing::trace!("MOV DR{}, r32: reg{}={:#010x} -> DR{}", dr_idx, src_gpr, val, dr_idx);
        Ok(())
    }
}
