use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn handle_cpu_context_change(&mut self) {
        self.tlb_flush();

        self.invalidate_prefetch_q();
        self.invalidate_stack_cache();

        self.handle_interrupt_mask_change();

        self.handle_alignment_check();

        self.handle_cpu_mode_change();

        // FPU/SSE/AVX mode changes not needed for 32-bit Linux 1.3.89
        // handleFpuMmxModeChange();
        // handleSseModeChange();
        // handleAvxModeChange();
    }

    /// Update cpu_mode based on CR0.PE and EFLAGS.VM
    /// Based on Bochs proc_ctrl.cc handleCpuModeChange()
    pub(super) fn handle_cpu_mode_change(&mut self) {
        use super::cpu::CpuMode;
        use super::eflags::EFlags;

        if self.cr0.pe() {
            if self.eflags.contains(EFlags::VM) {
                self.cpu_mode = CpuMode::Ia32V8086;
                // Bochs: CPL = 3 in V8086 mode
                self.sregs[super::decoder::BxSegregs::Cs as usize]
                    .selector
                    .rpl = 3;
            } else {
                self.cpu_mode = CpuMode::Ia32Protected;
            }
        } else {
            self.cpu_mode = CpuMode::Ia32Real;
            // Bochs proc_ctrl.cc: When entering real mode, set CS cache
            // to a writable data segment with CPL=0 (required for far jumps
            // in real mode after leaving protected mode)
            unsafe {
                let seg = &mut self.sregs[super::decoder::BxSegregs::Cs as usize];
                seg.cache.p = true; // present
                seg.cache.u.segment.d_b = false; // 16-bit default
                seg.cache.r#type = 3; // DATA_READ_WRITE_ACCESSED
                seg.selector.rpl = 0; // CPL = 0
            }
        }
    }

    pub(super) fn handle_alignment_check(&mut self) {
        if self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl
            == 3
            && self.cr0.am()
            && self.get_ac() != 0
        {
            self.alignment_check_mask = 0xf;
        } else {
            self.alignment_check_mask = 0;
        }
    }

    /// Get the Time Stamp Counter value
    pub fn get_tsc(&self, system_ticks: u64) -> u64 {
        system_ticks.wrapping_add(self.tsc_adjust as u64)
    }

    /// Set the Time Stamp Counter to a specific value
    pub fn set_tsc(&mut self, newval: u64, system_ticks: u64) {
        self.tsc_adjust = newval.wrapping_sub(system_ticks) as i64
    }

    // =========================================================================
    // System control instructions
    // =========================================================================

    /// WBINVD — Write Back and Invalidate Cache
    /// Based on Bochs proc_ctrl.cc:269-298
    pub(super) fn wbinvd(
        &mut self,
        _instr: &super::decoder::Instruction,
    ) -> crate::cpu::Result<()> {
        // CPL is always 0 in real mode
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("WBINVD: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        // No-op functionally (no cache to write back)
        Ok(())
    }

    /// INVD — Invalidate Cache
    /// Based on Bochs proc_ctrl.cc:238-266
    pub(super) fn invd(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("INVD: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        self.invalidate_prefetch_q();
        Ok(())
    }

    pub(super) fn invlpg(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        // INVLPG is a privileged instruction (CPL=0 only)
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let laddr = self.get_laddr32(seg as usize, eaddr);
        self.dtlb.invlpg(laddr.into());
        self.itlb.invlpg(laddr.into());
        self.invalidate_prefetch_q();
        tracing::trace!("INVLPG: laddr={:#x}", laddr);
        Ok(())
    }

    /// CLTS — Clear Task-Switched Flag in CR0
    /// Based on Bochs crregs.cc:1566-1599
    pub(super) fn clts(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("CLTS: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let cr0_val = self.cr0.get32();
        self.cr0.set32(cr0_val & !(1u32 << 3));
        Ok(())
    }

    // =========================================================================
    // RDTSC — Read Time Stamp Counter (opcode 0F 31)
    // Matches Bochs proc_ctrl.cc BX_CPU_C::RDTSC
    // =========================================================================

    pub(super) fn rdtsc(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        // Check CR4.TSD — if set, RDTSC is only allowed at CPL=0
        if self.cr4.tsd() {
            let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
                .selector
                .rpl;
            if cpl != 0 {
                tracing::debug!("RDTSC: CR4.TSD=1 and CPL={}, #GP(0)", cpl);
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        // Use icount as time source (matches Bochs bx_pc_system.time_ticks() model)
        let ticks = self.get_tsc(self.icount);

        self.set_rax((ticks & 0xFFFF_FFFF) as u64);
        self.set_rdx((ticks >> 32) as u64);

        tracing::trace!(
            "RDTSC: ticks={:#018x} -> EDX:EAX={:#010x}:{:#010x}",
            ticks,
            self.edx(),
            self.eax()
        );
        Ok(())
    }

    // =========================================================================
    // MSR instructions
    // =========================================================================

    /// RDMSR — Read Model Specific Register
    /// Based on Bochs msr.cc:656-698
    pub(super) fn rdmsr(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        use super::msr::*;

        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("RDMSR: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let msr = self.ecx();
        let val: u64 = match msr {
            BX_MSR_TSC => self.get_tsc(self.icount),
            #[cfg(feature = "bx_support_apic")]
            BX_MSR_APICBASE => self.msr.apicbase as u64,
            #[cfg(not(feature = "bx_support_apic"))]
            BX_MSR_APICBASE => BX_MSR_APICBASE_DEFAULT,
            BX_MSR_MTRRCAP => BX_MSR_MTRRCAP_DEFAULT,
            BX_MSR_SYSENTER_CS => self.msr.sysenter_cs_msr as u64,
            BX_MSR_SYSENTER_ESP => self.msr.sysenter_esp_msr,
            BX_MSR_SYSENTER_EIP => self.msr.sysenter_eip_msr,
            BX_MSR_PAT => unsafe { self.msr.pat.U64 },
            BX_MSR_MTRR_DEFTYPE => self.msr.mtrr_deftype as u64,
            n @ BX_MSR_MTRRPHYSBASE0..=BX_MSR_MTRRPHYSMASK7 => {
                self.msr.mtrrphys[(n - BX_MSR_MTRRPHYSBASE0) as usize]
            }
            // Fixed MTRR registers (Bochs msr.cc)
            BX_MSR_MTRRFIX64K_00000 => unsafe { self.msr.mtrrfix64k.U64 },
            BX_MSR_MTRRFIX16K_80000..=BX_MSR_MTRRFIX16K_A0000 => {
                let idx = (msr - BX_MSR_MTRRFIX16K_80000) as usize;
                unsafe { self.msr.mtrrfix16k[idx].U64 }
            }
            BX_MSR_MTRRFIX4K_C0000..=BX_MSR_MTRRFIX4K_F8000 => {
                let idx = (msr - BX_MSR_MTRRFIX4K_C0000) as usize;
                unsafe { self.msr.mtrrfix4k[idx].U64 }
            }
            _ => {
                tracing::trace!("RDMSR: unhandled MSR={:#010x}, returning 0", msr);
                0
            }
        };
        tracing::debug!("RDMSR: MSR={:#010x} -> {:#018x}", msr, val);
        self.set_rax((val & 0xFFFF_FFFF) as u64);
        self.set_rdx((val >> 32) as u64);
        Ok(())
    }

    /// WRMSR — Write Model Specific Register
    /// Based on Bochs msr.cc:1372-1414
    pub(super) fn wrmsr(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        use super::msr::*;

        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("WRMSR: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        self.invalidate_prefetch_q();

        let msr = self.ecx();
        let val = ((self.edx() as u64) << 32) | (self.eax() as u64);
        match msr {
            BX_MSR_TSC => self.set_tsc(val, self.icount),
            #[cfg(feature = "bx_support_apic")]
            BX_MSR_APICBASE => self.msr.apicbase = val as _,
            BX_MSR_SYSENTER_CS => self.msr.sysenter_cs_msr = val as u32,
            BX_MSR_SYSENTER_ESP => self.msr.sysenter_esp_msr = val,
            BX_MSR_SYSENTER_EIP => self.msr.sysenter_eip_msr = val,
            BX_MSR_PAT => {
                self.msr.pat.U64 = val;
            }
            BX_MSR_MTRR_DEFTYPE => self.msr.mtrr_deftype = val as u32,
            n @ BX_MSR_MTRRPHYSBASE0..=BX_MSR_MTRRPHYSMASK7 => {
                self.msr.mtrrphys[(n - BX_MSR_MTRRPHYSBASE0) as usize] = val;
            }
            // Fixed MTRR registers (Bochs msr.cc)
            BX_MSR_MTRRFIX64K_00000 => unsafe {
                self.msr.mtrrfix64k.U64 = val;
            },
            BX_MSR_MTRRFIX16K_80000..=BX_MSR_MTRRFIX16K_A0000 => {
                let idx = (msr - BX_MSR_MTRRFIX16K_80000) as usize;
                unsafe {
                    self.msr.mtrrfix16k[idx].U64 = val;
                }
            }
            BX_MSR_MTRRFIX4K_C0000..=BX_MSR_MTRRFIX4K_F8000 => {
                let idx = (msr - BX_MSR_MTRRFIX4K_C0000) as usize;
                unsafe {
                    self.msr.mtrrfix4k[idx].U64 = val;
                }
            }
            BX_MSR_MTRRCAP => {
                // MTRRCAP is read-only (Bochs msr.cc)
                tracing::debug!("WRMSR: MTRRCAP is read-only, #GP(0)");
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            _ => {
                tracing::trace!("WRMSR: unhandled MSR={:#010x} = {:#018x}", msr, val);
            }
        }
        tracing::debug!("WRMSR: MSR={:#010x} = {:#018x}", msr, val);
        Ok(())
    }

    // =========================================================================
    // MOV Rd, DRn / MOV DRn, Rd -- Debug Register Operations
    // =========================================================================

    pub(super) fn mov_rd_dd(
        &mut self,
        instr: &super::decoder::Instruction,
    ) -> crate::cpu::Result<()> {
        // Bochs crregs.cc: CPL must be 0 for MOV DRn
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        // Decoder: for 0F 21 (MOV Rd, DRn): b1=0x121, (b1 & 0x0F)==0x01 → Ed,Gd branch:
        // DST=rm=GPR destination, SRC1=nnn=DR number
        // Bochs crregs.cc: switch(i->src())=DR, BX_WRITE_32BIT_REGZ(i->dst())=GPR
        // Our decoder maps: dst()=rm=GPR, src1()=nnn=DR
        let dr_idx = instr.src1() as usize; // nnn = DR register number
        let dst_gpr = instr.dst() as usize; // rm = GPR destination register

        // Bochs crregs.cc: CR4.DE check — DR4/DR5 access raises #UD when DE=1
        if (dr_idx == 4 || dr_idx == 5) && self.cr4.de() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let val: u32 = match dr_idx {
            0..=3 => self.dr[dr_idx] as u32,
            4 | 6 => self.dr6.val32, // DR4 aliases DR6 when CR4.DE=0
            5 | 7 => self.dr7.val32, // DR5 aliases DR7 when CR4.DE=0
            _ => 0,
        };
        self.set_gpr32(dst_gpr, val);
        tracing::trace!(
            "MOV r32, DR{}: DR{}={:#010x} -> reg{}",
            dr_idx,
            dr_idx,
            val,
            dst_gpr
        );
        Ok(())
    }

    pub(super) fn mov_dd_rd(
        &mut self,
        instr: &super::decoder::Instruction,
    ) -> crate::cpu::Result<()> {
        // Bochs crregs.cc: CPL must be 0 for MOV DRn
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        self.invalidate_prefetch_q();

        let dr_idx = instr.dst() as usize;
        let src_gpr = instr.src1() as usize;

        // Bochs crregs.cc: CR4.DE check — DR4/DR5 access raises #UD when DE=1
        if (dr_idx == 4 || dr_idx == 5) && self.cr4.de() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let val = self.get_gpr32(src_gpr);
        match dr_idx {
            0..=3 => {
                self.dr[dr_idx] = val as u64;
                // Bochs: TLB_invlpg at breakpoint address
                self.dtlb.invlpg(val as u64);
                self.itlb.invlpg(val as u64);
            }
            4 | 6 => {
                // DR6: preserve reserved bits, only allow bits 0-3 (B0-B3) and bits 13-15 (BD,BS,BT)
                // Bochs crregs.cc: (dr6.val32 & 0xFFFF0FF0) | (val & 0x0000E00F)
                self.dr6.val32 = (self.dr6.val32 & 0xFFFF0FF0) | (val & 0x0000E00F);
            }
            5 | 7 => {
                // DR7: mask off reserved bits and set bit 10 (always 1)
                // Bochs crregs.cc: (val & 0xFFFF2FFF) | 0x00000400
                self.dr7.val32 = (val & 0xFFFF2FFF) | 0x00000400;
                // Bochs: TLB_flush after DR7 write
                self.tlb_flush();
            }
            _ => {}
        }
        tracing::trace!(
            "MOV DR{}, r32: reg{}={:#010x} -> DR{}",
            dr_idx,
            src_gpr,
            val,
            dr_idx
        );
        Ok(())
    }

    // ========================================================================
    // FXSAVE — Save x87 FPU, MMX, SSE state (512 bytes)
    // Bochs: FXSAVE in proc_ctrl.cc
    // ========================================================================

    pub(super) fn fxsave(
        &mut self,
        instr: &super::decoder::Instruction,
    ) -> super::Result<()> {
        use super::decoder::BxSegregs;
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        // Must be 16-byte aligned
        if (eaddr & 0xF) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bytes 0-1: FCW (FPU control word)
        self.write_virtual_word(seg, eaddr, self.the_i387.cwd)?;
        // Bytes 2-3: FSW (FPU status word)
        self.write_virtual_word(seg, eaddr.wrapping_add(2), self.the_i387.swd)?;
        // Byte 4: FTW (abridged tag word — compact form)
        let abridged_ftw = self.abridged_ftw();
        self.write_virtual_byte(seg, eaddr.wrapping_add(4), abridged_ftw)?;
        // Byte 5: reserved
        self.write_virtual_byte(seg, eaddr.wrapping_add(5), 0)?;
        // Bytes 6-7: FOP (last FPU opcode) — not tracked, write 0
        self.write_virtual_word(seg, eaddr.wrapping_add(6), 0)?;
        // Bytes 8-11: FIP (FPU instruction pointer) — not tracked
        self.write_virtual_dword(seg, eaddr.wrapping_add(8), 0)?;
        // Bytes 12-13: FCS — not tracked
        self.write_virtual_word(seg, eaddr.wrapping_add(12), 0)?;
        // Bytes 14-15: reserved
        self.write_virtual_word(seg, eaddr.wrapping_add(14), 0)?;
        // Bytes 16-19: FDP (FPU data pointer) — not tracked
        self.write_virtual_dword(seg, eaddr.wrapping_add(16), 0)?;
        // Bytes 20-21: FDS — not tracked
        self.write_virtual_word(seg, eaddr.wrapping_add(20), 0)?;
        // Bytes 22-23: reserved
        self.write_virtual_word(seg, eaddr.wrapping_add(22), 0)?;
        // Bytes 24-27: MXCSR
        self.write_virtual_dword(seg, eaddr.wrapping_add(24), self.mxcsr.mxcsr)?;
        // Bytes 28-31: MXCSR_MASK
        self.write_virtual_dword(seg, eaddr.wrapping_add(28), self.mxcsr_mask)?;

        // Bytes 32-159: FPU/MMX registers ST0-ST7 (16 bytes each = 80-bit + 6 padding)
        for i in 0..8 {
            let offset = eaddr.wrapping_add(32 + i * 16);
            let signif = self.the_i387.st_space[i as usize].signif;
            let sign_exp = self.the_i387.st_space[i as usize].sign_exp;
            self.write_virtual_qword(seg, offset, signif)?;
            self.write_virtual_word(seg, offset.wrapping_add(8), sign_exp)?;
            // Bytes 10-15 of each entry are padding (write zeros)
            self.write_virtual_word(seg, offset.wrapping_add(10), 0)?;
            self.write_virtual_dword(seg, offset.wrapping_add(12), 0)?;
        }

        // Bytes 160-415: XMM registers XMM0-XMM7 (16 bytes each, 32-bit mode)
        for i in 0..8u32 {
            let offset = eaddr.wrapping_add(160 + i * 16);
            let lo = unsafe { self.vmm[i as usize].zmm64u[0] };
            let hi = unsafe { self.vmm[i as usize].zmm64u[1] };
            self.write_virtual_qword(seg, offset, lo)?;
            self.write_virtual_qword(seg, offset.wrapping_add(8), hi)?;
        }

        // Bytes 416-511: reserved (zeros)
        for i in (416u32..512).step_by(8) {
            self.write_virtual_qword(seg, eaddr.wrapping_add(i), 0)?;
        }

        Ok(())
    }

    // ========================================================================
    // FXRSTOR — Restore x87 FPU, MMX, SSE state (512 bytes)
    // Bochs: FXRSTOR in proc_ctrl.cc
    // ========================================================================

    pub(super) fn fxrstor(
        &mut self,
        instr: &super::decoder::Instruction,
    ) -> super::Result<()> {
        use super::decoder::BxSegregs;
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        // Must be 16-byte aligned
        if (eaddr & 0xF) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bytes 0-1: FCW
        let fcw = self.read_virtual_word(seg, eaddr)?;
        // Bytes 2-3: FSW
        let fsw = self.read_virtual_word(seg, eaddr.wrapping_add(2))?;
        // Byte 4: abridged FTW
        let abridged_ftw = self.read_virtual_byte(seg, eaddr.wrapping_add(4))?;
        // Bytes 24-27: MXCSR
        let new_mxcsr = self.read_virtual_dword(seg, eaddr.wrapping_add(24))?;

        // Validate MXCSR — reserved bits must be zero
        if (new_mxcsr & !self.mxcsr_mask) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Now commit all state (no faults past this point)
        self.the_i387.cwd = fcw;
        self.the_i387.swd = fsw;
        self.the_i387.tos = ((fsw >> 11) & 7) as u8;
        self.restore_ftw_from_abridged(abridged_ftw);
        self.mxcsr.mxcsr = new_mxcsr;

        // Restore FPU/MMX registers
        for i in 0..8 {
            let offset = eaddr.wrapping_add(32 + i * 16);
            let signif = self.read_virtual_qword(seg, offset)?;
            let sign_exp = self.read_virtual_word(seg, offset.wrapping_add(8))?;
            self.the_i387.st_space[i as usize].signif = signif;
            self.the_i387.st_space[i as usize].sign_exp = sign_exp;
        }

        // Restore XMM registers (XMM0-XMM7 in 32-bit mode)
        for i in 0..8u32 {
            let offset = eaddr.wrapping_add(160 + i * 16);
            let lo = self.read_virtual_qword(seg, offset)?;
            let hi = self.read_virtual_qword(seg, offset.wrapping_add(8))?;
            unsafe {
                self.vmm[i as usize].zmm64u[0] = lo;
                self.vmm[i as usize].zmm64u[1] = hi;
                // Clear upper bits
                self.vmm[i as usize].zmm64u[2] = 0;
                self.vmm[i as usize].zmm64u[3] = 0;
                self.vmm[i as usize].zmm64u[4] = 0;
                self.vmm[i as usize].zmm64u[5] = 0;
                self.vmm[i as usize].zmm64u[6] = 0;
                self.vmm[i as usize].zmm64u[7] = 0;
            }
        }

        Ok(())
    }

    // ========================================================================
    // LDMXCSR — Load MXCSR from memory
    // Bochs: LDMXCSR in proc_ctrl.cc
    // ========================================================================

    pub(super) fn ldmxcsr(
        &mut self,
        instr: &super::decoder::Instruction,
    ) -> super::Result<()> {
        self.prepare_sse()?;

        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let new_mxcsr = self.read_virtual_dword(seg, eaddr)?;

        // Validate: reserved bits must be zero per mxcsr_mask
        if (new_mxcsr & !self.mxcsr_mask) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        self.mxcsr.mxcsr = new_mxcsr;
        Ok(())
    }

    // ========================================================================
    // STMXCSR — Store MXCSR to memory
    // Bochs: STMXCSR in proc_ctrl.cc
    // ========================================================================

    pub(super) fn stmxcsr(
        &mut self,
        instr: &super::decoder::Instruction,
    ) -> super::Result<()> {
        self.prepare_sse()?;

        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        self.write_virtual_dword(seg, eaddr, self.mxcsr.mxcsr)?;
        Ok(())
    }

    /// Compute abridged FPU tag word for FXSAVE
    /// Converts 16-bit tag word to 8-bit abridged form
    fn abridged_ftw(&self) -> u8 {
        let mut abridged: u8 = 0;
        for i in 0..8 {
            let tag = (self.the_i387.twd >> (i * 2)) & 3;
            if tag != 3 {
                // Not empty
                abridged |= 1 << i;
            }
        }
        abridged
    }

    /// Restore full FPU tag word from abridged FXRSTOR form
    fn restore_ftw_from_abridged(&mut self, abridged: u8) {
        let mut twd: u16 = 0;
        for i in 0..8 {
            if (abridged & (1 << i)) != 0 {
                // Tag is "valid" — set to 00 (valid)
                // A more accurate implementation would examine the actual register
                // value, but 00 (valid) is sufficient for most uses.
                twd |= 0 << (i * 2);
            } else {
                // Tag is "empty" — set to 11
                twd |= 3 << (i * 2);
            }
        }
        self.the_i387.twd = twd;
    }
}
