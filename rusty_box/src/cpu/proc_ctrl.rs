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
    fn handle_cpu_mode_change(&mut self) {
        use super::cpu::CpuMode;
        use super::eflags::EFlags;

        if self.cr0.pe() {
            if self.eflags.contains(EFlags::VM) {
                self.cpu_mode = CpuMode::Ia32V8086;
            } else {
                self.cpu_mode = CpuMode::Ia32Protected;
            }
        } else {
            self.cpu_mode = CpuMode::Ia32Real;
        }
    }

    fn handle_alignment_check(&mut self) {
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

    pub(super) fn wbinvd(
        &mut self,
        _instr: &super::decoder::Instruction,
    ) -> crate::cpu::Result<()> {
        tracing::trace!("WBINVD: no-op (no cache)");
        Ok(())
    }

    pub(super) fn invlpg(&mut self, instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        // INVLPG is a privileged instruction (CPL=0 only)
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize].selector.rpl;
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

    pub(super) fn clts(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let cr0_val = self.cr0.get32();
        self.cr0.set32(cr0_val & !(1u32 << 3));
        tracing::trace!("CLTS: CR0.TS cleared, CR0={:#010x}", cr0_val & !(1u32 << 3));
        Ok(())
    }

    // =========================================================================
    // MSR instructions
    // =========================================================================

    pub(super) fn rdmsr(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let msr = self.ecx();
        let val: u64 = match msr {
            #[cfg(feature = "bx_support_apic")]
            0x1B => self.msr.apicbase as u64,
            #[cfg(not(feature = "bx_support_apic"))]
            0x1B => 0xFEE00900,
            0xFE => 0x0508, // MTRR_CAP
            0x174 => self.msr.sysenter_cs_msr as u64,
            0x175 => self.msr.sysenter_esp_msr,
            0x176 => self.msr.sysenter_eip_msr,
            0x277 => unsafe { self.msr.pat.U64 },
            0x2FF => self.msr.mtrr_deftype as u64,
            n @ 0x200..=0x20F => self.msr.mtrrphys[(n - 0x200) as usize],
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

    pub(super) fn wrmsr(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let msr = self.ecx();
        let val = ((self.edx() as u64) << 32) | (self.eax() as u64);
        match msr {
            #[cfg(feature = "bx_support_apic")]
            0x1B => self.msr.apicbase = val as _,
            0x174 => self.msr.sysenter_cs_msr = val as u32,
            0x175 => self.msr.sysenter_esp_msr = val,
            0x176 => self.msr.sysenter_eip_msr = val,
            0x277 => {
                self.msr.pat.U64 = val;
            }
            0x2FF => self.msr.mtrr_deftype = val as u32,
            n @ 0x200..=0x20F => self.msr.mtrrphys[(n - 0x200) as usize] = val,
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
        let dr_idx = instr.src1() as usize;
        let dst_gpr = instr.dst() as usize;
        let val: u32 = match dr_idx {
            0..=3 => self.dr[dr_idx] as u32,
            4 | 6 => self.dr6.val32,
            5 | 7 => self.dr7.val32,
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
        let dr_idx = instr.dst() as usize;
        let src_gpr = instr.src1() as usize;
        let val = self.get_gpr32(src_gpr);
        match dr_idx {
            0..=3 => {
                self.dr[dr_idx] = val as u64;
            }
            4 | 6 => {
                self.dr6.val32 = val;
            }
            5 | 7 => {
                self.dr7.val32 = val;
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
}
