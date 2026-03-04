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
    // MONITOR — Setup monitor address for MWAIT (opcode 0F 01 C8)
    // Bochs: mwait.cc:74-128 MONITOR instruction
    // =========================================================================

    pub(super) fn monitor(
        &mut self,
        instr: &super::decoder::Instruction,
    ) -> crate::cpu::Result<()> {
        tracing::debug!("MONITOR: RAX={:#x}", self.rax());

        // Bochs mwait.cc:79-84: MONITOR requires CPL==0 (CPL always 0 in real mode)
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("MONITOR: CPL={} != 0, #UD", cpl);
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        // Bochs mwait.cc:95-98: RCX must be 0 (no optional extensions supported)
        if self.rcx() != 0 {
            tracing::error!(
                "MONITOR: no optional extensions supported, RCX={:#x}",
                self.rcx()
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bochs mwait.cc:100: effective address = RAX & asize_mask
        let asize_mask: u64 = if instr.as32_l() != 0 {
            0xFFFF_FFFF
        } else {
            0xFFFF
        };
        let eaddr = self.rax() & asize_mask;

        // Bochs mwait.cc:102-103: MONITOR performs same segmentation and paging
        // checks as a 1-byte read (tickle_read_virtual)
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let _ = self.read_virtual_byte(seg, eaddr as u32)?;

        // Bochs mwait.cc:105: get physical address from address translation
        let paddr = self.address_xlation.paddress1;

        // Bochs mwait.cc:121: invalidate page in monitoring system
        // (In Bochs this calls bx_pc_system.invlpg(paddr) to clear any
        // cached page state. We don't need this since we check is_monitor
        // on every memory write.)

        // Bochs mwait.cc:123: arm the monitor with the physical address
        #[cfg(feature = "bx_support_monitor_mwait")]
        {
            self.monitor
                .arm(paddr, super::cpu::BX_MONITOR_ARMED_BY_MONITOR);
            tracing::debug!(
                "MONITOR: armed for phys_addr={:#x}",
                self.monitor.monitor_addr
            );
        }

        Ok(())
    }

    // =========================================================================
    // MWAIT — Monitor Wait (opcode 0F 01 C9)
    // Bochs: mwait.cc:137-242 MWAIT instruction
    // =========================================================================

    pub(super) fn mwait(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        tracing::debug!("MWAIT: ECX={:#x}", self.ecx());

        // Bochs mwait.cc:142-147: MWAIT requires CPL==0 (CPL always 0 in real mode)
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("MWAIT: CPL={} != 0, #UD", cpl);
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        // Bochs mwait.cc:158-172: Check ECX extensions
        // ECX[0] = interrupt MWAIT even if EFLAGS.IF = 0
        // ECX[1] = timed MWAITX (MWAITX only, not applicable here)
        // ECX[2] = monitorless MWAIT
        // All other bits reserved
        let supported_bits: u64 = 0x1; // Only bit 0 supported for MWAIT
        if self.rcx() & !supported_bits != 0 {
            tracing::error!(
                "MWAIT: incorrect optional extensions in RCX={:#x}",
                self.rcx()
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bochs mwait.cc:183-198: If monitor not armed, just return
        #[cfg(feature = "bx_support_monitor_mwait")]
        {
            if !self.monitor.armed_by_monitor() {
                tracing::debug!("MWAIT: monitor not armed or already triggered, returning");
                return Ok(());
            }
        }

        // Bochs mwait.cc:216-228: Determine sleep state
        // ECX[0] = 1: wake on interrupt even if IF=0
        let mwait_if = self.ecx() & 0x1 != 0;

        // Bochs mwait.cc:238: enter_sleep_state(new_state)
        // Matches the pattern in hlt() — set activity state and async event
        if mwait_if {
            self.activity_state = super::cpu::CpuActivityState::MwaitIf;
            tracing::debug!("MWAIT: entering sleep state MwaitIf (wake on interrupt even if IF=0)");
        } else {
            self.activity_state = super::cpu::CpuActivityState::Mwait;
            tracing::debug!("MWAIT: entering sleep state Mwait");
        }
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE | Self::BX_ASYNC_EVENT_SLEEP;

        Ok(())
    }

    // =========================================================================
    // CLAC — Clear AC Flag (SMAP, opcode 0F 01 CA)
    // =========================================================================

    pub(super) fn clac(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.clear_ac();
        Ok(())
    }

    // =========================================================================
    // STAC — Set AC Flag (SMAP, opcode 0F 01 CB)
    // =========================================================================

    pub(super) fn stac(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.assert_ac();
        Ok(())
    }

    // =========================================================================
    // CLFLUSH — Cache Line Flush (opcode 0F AE /7)
    // =========================================================================

    pub(super) fn clflush(
        &mut self,
        _instr: &super::decoder::Instruction,
    ) -> crate::cpu::Result<()> {
        // NOP — no cache to flush
        Ok(())
    }

    // =========================================================================
    // RDTSCP — Read Time Stamp Counter and Processor ID (opcode 0F 01 F9)
    // Bochs: proc_ctrl.cc RDTSCP
    // =========================================================================

    pub(super) fn rdtscp(
        &mut self,
        _instr: &super::decoder::Instruction,
    ) -> crate::cpu::Result<()> {
        // Check CR4.TSD — if set, RDTSCP is only allowed at CPL=0
        if self.cr4.tsd() {
            let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
                .selector
                .rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        let ticks = self.get_tsc(self.icount);
        self.set_rax((ticks & 0xFFFF_FFFF) as u64);
        self.set_rdx((ticks >> 32) as u64);
        // ECX = IA32_TSC_AUX MSR (processor ID) — return 0
        self.set_rcx(0);

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
            4 | 6 => self.dr6.get32(), // DR4 aliases DR6 when CR4.DE=0
            5 | 7 => self.dr7.get32(), // DR5 aliases DR7 when CR4.DE=0
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
                self.dr6
                    .set32((self.dr6.get32() & 0xFFFF0FF0) | (val & 0x0000E00F));
            }
            5 | 7 => {
                // DR7: mask off reserved bits and set bit 10 (always 1)
                // Bochs crregs.cc: (val & 0xFFFF2FFF) | 0x00000400
                self.dr7.set32((val & 0xFFFF2FFF) | 0x00000400);
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

    pub(super) fn fxsave(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
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

    pub(super) fn fxrstor(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
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

    pub(super) fn ldmxcsr(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
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

    pub(super) fn stmxcsr(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
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

    // ========================================================================
    // SYSENTER — Fast System Call Entry (opcode 0F 34)
    // Bochs: proc_ctrl.cc:861-963
    // ========================================================================

    pub(super) fn sysenter(&mut self, _instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;
        use super::descriptor::{
            SEG_ACCESS_ROK, SEG_ACCESS_ROK4_G, SEG_ACCESS_WOK, SEG_ACCESS_WOK4_G, SEG_VALID_CACHE,
        };

        if self.real_mode() {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        if (self.msr.sysenter_cs_msr & 0xFFFC) == 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        self.invalidate_prefetch_q();

        // Clear VM, IF, RF (Bochs proc_ctrl.cc:877-879)
        self.clear_vm();
        self.clear_if();
        self.clear_rf();

        // Long mode: canonical checks (Bochs proc_ctrl.cc:882-891)
        if self.long_mode() {
            if !self.is_canonical(self.msr.sysenter_eip_msr) {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            if !self.is_canonical(self.msr.sysenter_esp_msr) {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        // Load CS: flat code segment, DPL=0 (Bochs proc_ctrl.cc:901-916)
        let cs_idx = BxSegregs::Cs as usize;
        super::segment_ctrl_pro::parse_selector(
            (self.msr.sysenter_cs_msr & 0xFFFC) as u16,
            &mut self.sregs[cs_idx].selector,
        );
        let seg_valid =
            SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK | SEG_ACCESS_ROK4_G | SEG_ACCESS_WOK4_G;
        self.sregs[cs_idx].cache.valid = seg_valid;
        self.sregs[cs_idx].cache.p = true;
        self.sregs[cs_idx].cache.dpl = 0;
        self.sregs[cs_idx].cache.segment = true;
        self.sregs[cs_idx].cache.r#type = 0xb; // CODE_EXEC_READ_ACCESSED
        unsafe {
            self.sregs[cs_idx].cache.u.segment.base = 0;
            self.sregs[cs_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
            self.sregs[cs_idx].cache.u.segment.g = true;
            self.sregs[cs_idx].cache.u.segment.avl = false;
            self.sregs[cs_idx].cache.u.segment.d_b = !self.long_mode();
            self.sregs[cs_idx].cache.u.segment.l = self.long_mode();
        }

        self.handle_cpu_mode_change();
        self.alignment_check_mask = 0;
        self.user_pl = false;

        // Load SS: flat data segment, DPL=0 (Bochs proc_ctrl.cc:928-943)
        let ss_idx = BxSegregs::Ss as usize;
        super::segment_ctrl_pro::parse_selector(
            ((self.msr.sysenter_cs_msr + 8) & 0xFFFC) as u16,
            &mut self.sregs[ss_idx].selector,
        );
        self.sregs[ss_idx].cache.valid = seg_valid;
        self.sregs[ss_idx].cache.p = true;
        self.sregs[ss_idx].cache.dpl = 0;
        self.sregs[ss_idx].cache.segment = true;
        self.sregs[ss_idx].cache.r#type = 0x3; // DATA_READ_WRITE_ACCESSED
        unsafe {
            self.sregs[ss_idx].cache.u.segment.base = 0;
            self.sregs[ss_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
            self.sregs[ss_idx].cache.u.segment.g = true;
            self.sregs[ss_idx].cache.u.segment.d_b = true;
            self.sregs[ss_idx].cache.u.segment.avl = false;
            self.sregs[ss_idx].cache.u.segment.l = false;
        }

        // Load RSP/RIP from MSRs (Bochs proc_ctrl.cc:946-955)
        if self.long_mode() {
            self.set_rsp(self.msr.sysenter_esp_msr);
            self.set_rip(self.msr.sysenter_eip_msr);
        } else {
            self.set_esp(self.msr.sysenter_esp_msr as u32);
            self.set_eip(self.msr.sysenter_eip_msr as u32);
        }

        Ok(())
    }

    // ========================================================================
    // SYSEXIT — Fast System Call Exit (opcode 0F 35)
    // Bochs: proc_ctrl.cc:965-1074
    // ========================================================================

    pub(super) fn sysexit(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;
        use super::descriptor::{
            SEG_ACCESS_ROK, SEG_ACCESS_ROK4_G, SEG_ACCESS_WOK, SEG_ACCESS_WOK4_G, SEG_VALID_CACHE,
        };

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if self.real_mode() || cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        if (self.msr.sysenter_cs_msr & 0xFFFC) == 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        self.invalidate_prefetch_q();
        self.monitor.reset_umonitor();

        let seg_valid =
            SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK | SEG_ACCESS_ROK4_G | SEG_ACCESS_WOK4_G;
        let cs_idx = BxSegregs::Cs as usize;
        let ss_idx = BxSegregs::Ss as usize;

        // 64-bit SYSEXIT (Bochs proc_ctrl.cc:986-1012)
        if instr.os64_l() != 0 {
            if !self.is_canonical(self.rdx()) {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            if !self.is_canonical(self.rcx()) {
                return self.exception(super::cpu::Exception::Gp, 0);
            }

            // CS = (sysenter_cs_msr + 32) | 3, 64-bit code DPL=3
            super::segment_ctrl_pro::parse_selector(
                (((self.msr.sysenter_cs_msr + 32) & 0xFFFC) | 3) as u16,
                &mut self.sregs[cs_idx].selector,
            );
            self.sregs[cs_idx].cache.valid = seg_valid;
            self.sregs[cs_idx].cache.p = true;
            self.sregs[cs_idx].cache.dpl = 3;
            self.sregs[cs_idx].cache.segment = true;
            self.sregs[cs_idx].cache.r#type = 0xb;
            unsafe {
                self.sregs[cs_idx].cache.u.segment.base = 0;
                self.sregs[cs_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                self.sregs[cs_idx].cache.u.segment.g = true;
                self.sregs[cs_idx].cache.u.segment.avl = false;
                self.sregs[cs_idx].cache.u.segment.d_b = false;
                self.sregs[cs_idx].cache.u.segment.l = true; // 64-bit
            }

            self.set_rsp(self.rcx());
            self.set_rip(self.rdx());
        } else {
            // 32-bit SYSEXIT: CS = (sysenter_cs_msr + 16) | 3 (Bochs proc_ctrl.cc:1016-1036)
            super::segment_ctrl_pro::parse_selector(
                (((self.msr.sysenter_cs_msr + 16) & 0xFFFC) | 3) as u16,
                &mut self.sregs[cs_idx].selector,
            );
            self.sregs[cs_idx].cache.valid = seg_valid;
            self.sregs[cs_idx].cache.p = true;
            self.sregs[cs_idx].cache.dpl = 3;
            self.sregs[cs_idx].cache.segment = true;
            self.sregs[cs_idx].cache.r#type = 0xb;
            unsafe {
                self.sregs[cs_idx].cache.u.segment.base = 0;
                self.sregs[cs_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                self.sregs[cs_idx].cache.u.segment.g = true;
                self.sregs[cs_idx].cache.u.segment.avl = false;
                self.sregs[cs_idx].cache.u.segment.d_b = true;
                self.sregs[cs_idx].cache.u.segment.l = false;
            }

            self.set_esp(self.ecx());
            self.set_eip(self.edx());
        }

        self.handle_cpu_mode_change();
        self.handle_alignment_check();
        self.user_pl = true;

        // SS = (sysenter_cs_msr + (os64 ? 40 : 24)) | 3 (Bochs proc_ctrl.cc:1046-1061)
        let ss_offset: u32 = if instr.os64_l() != 0 { 40 } else { 24 };
        super::segment_ctrl_pro::parse_selector(
            (((self.msr.sysenter_cs_msr + ss_offset) & 0xFFFC) | 3) as u16,
            &mut self.sregs[ss_idx].selector,
        );
        self.sregs[ss_idx].cache.valid = seg_valid;
        self.sregs[ss_idx].cache.p = true;
        self.sregs[ss_idx].cache.dpl = 3;
        self.sregs[ss_idx].cache.segment = true;
        self.sregs[ss_idx].cache.r#type = 0x3;
        unsafe {
            self.sregs[ss_idx].cache.u.segment.base = 0;
            self.sregs[ss_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
            self.sregs[ss_idx].cache.u.segment.g = true;
            self.sregs[ss_idx].cache.u.segment.d_b = true;
            self.sregs[ss_idx].cache.u.segment.avl = false;
            self.sregs[ss_idx].cache.u.segment.l = false;
        }

        Ok(())
    }

    // ========================================================================
    // SYSCALL — Fast System Call (opcode 0F 05)
    // Bochs: proc_ctrl.cc:1076-1218
    // ========================================================================

    pub(super) fn syscall(&mut self, _instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;
        use super::descriptor::{
            SEG_ACCESS_ROK, SEG_ACCESS_ROK4_G, SEG_ACCESS_WOK, SEG_ACCESS_WOK4_G, SEG_VALID_CACHE,
        };
        use super::eflags::EFlags;

        if !self.efer.sce() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        self.invalidate_prefetch_q();

        let seg_valid =
            SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK | SEG_ACCESS_ROK4_G | SEG_ACCESS_WOK4_G;
        let cs_idx = BxSegregs::Cs as usize;
        let ss_idx = BxSegregs::Ss as usize;

        if self.long_mode() {
            // Long mode SYSCALL (Bochs proc_ctrl.cc:1096-1148)
            let saved_rip = self.rip();
            self.set_rcx(saved_rip);
            let saved_rflags = self.eflags.bits() & !EFlags::RF.bits();
            self.set_r11(saved_rflags as u64);

            let temp_rip = if self.cpu_mode == super::cpu::CpuMode::Long64 {
                self.msr.lstar
            } else {
                self.msr.cstar
            };

            // CS: flat 64-bit code, DPL=0 (Bochs proc_ctrl.cc:1109-1122)
            super::segment_ctrl_pro::parse_selector(
                ((self.msr.star >> 32) & 0xFFFC) as u16,
                &mut self.sregs[cs_idx].selector,
            );
            self.sregs[cs_idx].cache.valid = seg_valid;
            self.sregs[cs_idx].cache.p = true;
            self.sregs[cs_idx].cache.dpl = 0;
            self.sregs[cs_idx].cache.segment = true;
            self.sregs[cs_idx].cache.r#type = 0xb;
            unsafe {
                self.sregs[cs_idx].cache.u.segment.base = 0;
                self.sregs[cs_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                self.sregs[cs_idx].cache.u.segment.g = true;
                self.sregs[cs_idx].cache.u.segment.d_b = false;
                self.sregs[cs_idx].cache.u.segment.l = true; // 64-bit code
                self.sregs[cs_idx].cache.u.segment.avl = false;
            }

            self.handle_cpu_mode_change();
            self.alignment_check_mask = 0;
            self.user_pl = false;

            // SS: flat data, DPL=0 (Bochs proc_ctrl.cc:1131-1144)
            super::segment_ctrl_pro::parse_selector(
                (((self.msr.star >> 32) + 8) & 0xFFFC) as u16,
                &mut self.sregs[ss_idx].selector,
            );
            self.sregs[ss_idx].cache.valid = seg_valid;
            self.sregs[ss_idx].cache.p = true;
            self.sregs[ss_idx].cache.dpl = 0;
            self.sregs[ss_idx].cache.segment = true;
            self.sregs[ss_idx].cache.r#type = 0x3;
            unsafe {
                self.sregs[ss_idx].cache.u.segment.base = 0;
                self.sregs[ss_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                self.sregs[ss_idx].cache.u.segment.g = true;
                self.sregs[ss_idx].cache.u.segment.d_b = true;
                self.sregs[ss_idx].cache.u.segment.l = false;
                self.sregs[ss_idx].cache.u.segment.avl = false;
            }

            // Mask RFLAGS with FMASK, clear RF (Bochs proc_ctrl.cc:1146)
            let new_flags = saved_rflags & !(self.msr.fmask as u32) & !EFlags::RF.bits();
            self.write_eflags(new_flags, EFlags::VALID_MASK.bits());
            self.set_rip(temp_rip);
        } else {
            // Legacy mode SYSCALL (Bochs proc_ctrl.cc:1151-1203)
            let saved_eip = self.eip();
            self.set_ecx(saved_eip);
            let temp_rip = self.msr.star as u32;

            // CS: flat 32-bit code, DPL=0 (Bochs proc_ctrl.cc:1158-1173)
            super::segment_ctrl_pro::parse_selector(
                ((self.msr.star >> 32) & 0xFFFC) as u16,
                &mut self.sregs[cs_idx].selector,
            );
            self.sregs[cs_idx].cache.valid = seg_valid;
            self.sregs[cs_idx].cache.p = true;
            self.sregs[cs_idx].cache.dpl = 0;
            self.sregs[cs_idx].cache.segment = true;
            self.sregs[cs_idx].cache.r#type = 0xb;
            unsafe {
                self.sregs[cs_idx].cache.u.segment.base = 0;
                self.sregs[cs_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                self.sregs[cs_idx].cache.u.segment.g = true;
                self.sregs[cs_idx].cache.u.segment.d_b = true;
                self.sregs[cs_idx].cache.u.segment.l = false;
                self.sregs[cs_idx].cache.u.segment.avl = false;
            }

            self.handle_cpu_mode_change();
            self.alignment_check_mask = 0;
            self.user_pl = false;

            // SS: flat data, DPL=0 (Bochs proc_ctrl.cc:1182-1197)
            super::segment_ctrl_pro::parse_selector(
                (((self.msr.star >> 32) + 8) & 0xFFFC) as u16,
                &mut self.sregs[ss_idx].selector,
            );
            self.sregs[ss_idx].cache.valid = seg_valid;
            self.sregs[ss_idx].cache.p = true;
            self.sregs[ss_idx].cache.dpl = 0;
            self.sregs[ss_idx].cache.segment = true;
            self.sregs[ss_idx].cache.r#type = 0x3;
            unsafe {
                self.sregs[ss_idx].cache.u.segment.base = 0;
                self.sregs[ss_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                self.sregs[ss_idx].cache.u.segment.g = true;
                self.sregs[ss_idx].cache.u.segment.d_b = true;
                self.sregs[ss_idx].cache.u.segment.l = false;
                self.sregs[ss_idx].cache.u.segment.avl = false;
            }

            self.clear_vm();
            self.clear_if();
            self.clear_rf();
            self.set_rip(temp_rip as u64);
        }

        Ok(())
    }

    // ========================================================================
    // SYSRET — Fast System Call Return (opcode 0F 07)
    // Bochs: proc_ctrl.cc:1220-1358
    // ========================================================================

    pub(super) fn sysret(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;
        use super::descriptor::{
            SEG_ACCESS_ROK, SEG_ACCESS_ROK4_G, SEG_ACCESS_WOK, SEG_ACCESS_WOK4_G, SEG_VALID_CACHE,
        };
        use super::eflags::EFlags;

        if !self.efer.sce() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if !self.protected_mode() || cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        self.invalidate_prefetch_q();
        self.monitor.reset_umonitor();

        let seg_valid =
            SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK | SEG_ACCESS_ROK4_G | SEG_ACCESS_WOK4_G;
        let cs_idx = BxSegregs::Cs as usize;
        let ss_idx = BxSegregs::Ss as usize;

        if self.cpu_mode == super::cpu::CpuMode::Long64 {
            // 64-bit mode SYSRET (Bochs proc_ctrl.cc:1244-1306)
            if instr.os64_l() != 0 {
                // Return to 64-bit mode (Bochs proc_ctrl.cc:1247-1269)
                if !self.is_canonical(self.rcx()) {
                    return self.exception(super::cpu::Exception::Gp, 0);
                }

                // CS = ((star >> 48) + 16) | 3, 64-bit code DPL=3
                super::segment_ctrl_pro::parse_selector(
                    ((((self.msr.star >> 48) + 16) & 0xFFFC) | 3) as u16,
                    &mut self.sregs[cs_idx].selector,
                );
                self.sregs[cs_idx].cache.valid = seg_valid;
                self.sregs[cs_idx].cache.p = true;
                self.sregs[cs_idx].cache.dpl = 3;
                self.sregs[cs_idx].cache.segment = true;
                self.sregs[cs_idx].cache.r#type = 0xb;
                unsafe {
                    self.sregs[cs_idx].cache.u.segment.base = 0;
                    self.sregs[cs_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                    self.sregs[cs_idx].cache.u.segment.g = true;
                    self.sregs[cs_idx].cache.u.segment.d_b = false;
                    self.sregs[cs_idx].cache.u.segment.l = true; // 64-bit
                    self.sregs[cs_idx].cache.u.segment.avl = false;
                }

                self.set_rip(self.rcx());
            } else {
                // Return to 32-bit compat mode (Bochs proc_ctrl.cc:1271-1289)
                super::segment_ctrl_pro::parse_selector(
                    (((self.msr.star >> 48) & 0xFFFC) | 3) as u16,
                    &mut self.sregs[cs_idx].selector,
                );
                self.sregs[cs_idx].cache.valid = seg_valid;
                self.sregs[cs_idx].cache.p = true;
                self.sregs[cs_idx].cache.dpl = 3;
                self.sregs[cs_idx].cache.segment = true;
                self.sregs[cs_idx].cache.r#type = 0xb;
                unsafe {
                    self.sregs[cs_idx].cache.u.segment.base = 0;
                    self.sregs[cs_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                    self.sregs[cs_idx].cache.u.segment.g = true;
                    self.sregs[cs_idx].cache.u.segment.d_b = true;
                    self.sregs[cs_idx].cache.u.segment.l = false;
                    self.sregs[cs_idx].cache.u.segment.avl = false;
                }

                self.set_rip(self.ecx() as u64);
            }

            self.handle_cpu_mode_change();
            self.handle_alignment_check();
            self.user_pl = true;

            // SS: (star >> 48) + 8) | 3 (Bochs proc_ctrl.cc:1296-1304)
            super::segment_ctrl_pro::parse_selector(
                ((((self.msr.star >> 48) + 8) & 0xFFFC) | 3) as u16,
                &mut self.sregs[ss_idx].selector,
            );
            self.sregs[ss_idx].cache.valid = seg_valid;
            self.sregs[ss_idx].cache.p = true;
            self.sregs[ss_idx].cache.dpl = 3;
            self.sregs[ss_idx].cache.segment = true;
            self.sregs[ss_idx].cache.r#type = 0x3;

            // Restore RFLAGS from R11 (Bochs proc_ctrl.cc:1305)
            self.write_eflags(self.r11() as u32, EFlags::VALID_MASK.bits());
        } else {
            // Legacy/compat mode SYSRET (Bochs proc_ctrl.cc:1309-1344)
            super::segment_ctrl_pro::parse_selector(
                (((self.msr.star >> 48) & 0xFFFC) | 3) as u16,
                &mut self.sregs[cs_idx].selector,
            );
            self.sregs[cs_idx].cache.valid = seg_valid;
            self.sregs[cs_idx].cache.p = true;
            self.sregs[cs_idx].cache.dpl = 3;
            self.sregs[cs_idx].cache.segment = true;
            self.sregs[cs_idx].cache.r#type = 0xb;
            unsafe {
                self.sregs[cs_idx].cache.u.segment.base = 0;
                self.sregs[cs_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                self.sregs[cs_idx].cache.u.segment.g = true;
                self.sregs[cs_idx].cache.u.segment.d_b = true;
                self.sregs[cs_idx].cache.u.segment.l = false;
                self.sregs[cs_idx].cache.u.segment.avl = false;
            }

            self.handle_cpu_mode_change();
            self.handle_alignment_check();
            self.user_pl = true;

            // SS: (star >> 48) + 8) | 3 (Bochs proc_ctrl.cc:1333-1340)
            super::segment_ctrl_pro::parse_selector(
                ((((self.msr.star >> 48) + 8) & 0xFFFC) | 3) as u16,
                &mut self.sregs[ss_idx].selector,
            );
            self.sregs[ss_idx].cache.valid = seg_valid;
            self.sregs[ss_idx].cache.p = true;
            self.sregs[ss_idx].cache.dpl = 3;
            self.sregs[ss_idx].cache.segment = true;
            self.sregs[ss_idx].cache.r#type = 0x3;

            // Restore IF, set RIP from ECX (Bochs proc_ctrl.cc:1342-1343)
            self.eflags.insert(super::eflags::EFlags::IF_);
            self.set_rip(self.ecx() as u64);
        }

        self.handle_cpu_mode_change();

        Ok(())
    }

    // ========================================================================
    // XGETBV — Get Extended Control Register (opcode 0F 01 D0)
    // Bochs: proc_ctrl.cc:1195-1226
    // ========================================================================

    pub(super) fn xgetbv(&mut self, _instr: &super::decoder::Instruction) -> super::Result<()> {
        // CR4.OSXSAVE must be set
        if !self.cr4.osxsave() {
            tracing::debug!("XGETBV: CR4.OSXSAVE not set, #UD");
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let ecx = self.ecx();
        if ecx != 0 {
            tracing::debug!("XGETBV: invalid XCR{}, #GP(0)", ecx);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // XCR0 → EDX:EAX
        let xcr0_val = self.xcr0.get32() as u64;
        self.set_rax(xcr0_val & 0xFFFF_FFFF);
        self.set_rdx(xcr0_val >> 32);

        tracing::trace!("XGETBV: XCR0={:#010x}", xcr0_val);
        Ok(())
    }

    // ========================================================================
    // XSETBV — Set Extended Control Register (opcode 0F 01 D1)
    // Bochs: proc_ctrl.cc:1229-1302
    // ========================================================================

    pub(super) fn xsetbv(&mut self, _instr: &super::decoder::Instruction) -> super::Result<()> {
        // CR4.OSXSAVE must be set
        if !self.cr4.osxsave() {
            tracing::debug!("XSETBV: CR4.OSXSAVE not set, #UD");
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        // Must be CPL=0
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("XSETBV: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let ecx = self.ecx();
        if ecx != 0 {
            tracing::debug!("XSETBV: invalid XCR{}, #GP(0)", ecx);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let eax = self.eax();
        let edx = self.edx();

        // EDX must be 0 for XCR0 (only 32-bit features supported)
        // EAX must not set unsupported bits, and FPU bit (bit 0) must be set
        if edx != 0 || (eax & !self.xcr0_suppmask) != 0 || (eax & 0x1) == 0 {
            tracing::debug!(
                "XSETBV: invalid value EDX:EAX={:#010x}:{:#010x} suppmask={:#010x}, #GP(0)",
                edx,
                eax,
                self.xcr0_suppmask
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // AVX requires SSE: if YMM bit set, SSE must also be set
        if (eax & 0x4) != 0 && (eax & 0x2) == 0 {
            tracing::debug!("XSETBV: attempt to enable AVX without SSE, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        self.xcr0.set32(eax);
        tracing::debug!("XSETBV: XCR0={:#010x}", eax);

        Ok(())
    }

    // ========================================================================
    // XSAVE — Save Processor Extended State (opcode 0F AE /4)
    // Bochs: xsave.cc:50-132
    // Saves x87 + SSE state + XSAVE header based on XCR0 & EDX:EAX mask
    // ========================================================================

    pub(super) fn xsave(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;

        // Check CR4.OSXSAVE and CR0.TS
        if !self.cr4.osxsave() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if self.cr0.ts() {
            return self.exception(super::cpu::Exception::Nm, 0);
        }

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        // Must be 64-byte aligned
        let laddr = self.get_laddr32(seg as usize, eaddr);
        if (laddr & 0x3F) != 0 {
            tracing::debug!("XSAVE: not 64-byte aligned, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let requested = self.xcr0.get32() & self.eax();

        // Read existing xstate_bv from header
        let mut xstate_bv = self.read_virtual_qword(seg, eaddr.wrapping_add(512))?;

        // Save x87 FPU state if requested (bit 0)
        if (requested & 0x1) != 0 {
            self.xsave_x87_state(seg, eaddr)?;
            xstate_bv |= 0x1;
        }

        // Save MXCSR if SSE or YMM requested (Bochs xsave.cc:87-92)
        if (requested & 0x6) != 0 {
            self.write_virtual_dword(seg, eaddr.wrapping_add(24), self.mxcsr.mxcsr)?;
            self.write_virtual_dword(seg, eaddr.wrapping_add(28), self.mxcsr_mask)?;
        }

        // Save SSE state if requested (bit 1)
        if (requested & 0x2) != 0 {
            self.xsave_sse_state(seg, eaddr.wrapping_add(160))?;
            xstate_bv |= 0x2;
        }

        // Write XSAVE header: xstate_bv at offset 512
        self.write_virtual_qword(seg, eaddr.wrapping_add(512), xstate_bv)?;
        // Clear xcomp_bv and reserved header fields (offsets 520-575)
        self.write_virtual_qword(seg, eaddr.wrapping_add(520), 0)?;
        self.write_virtual_qword(seg, eaddr.wrapping_add(528), 0)?;

        Ok(())
    }

    // ========================================================================
    // XRSTOR — Restore Processor Extended State (opcode 0F AE /5)
    // Bochs: xsave.cc:242-449
    // ========================================================================

    pub(super) fn xrstor(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;

        // Check CR4.OSXSAVE and CR0.TS
        if !self.cr4.osxsave() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if self.cr0.ts() {
            return self.exception(super::cpu::Exception::Nm, 0);
        }

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        // Must be 64-byte aligned
        let laddr = self.get_laddr32(seg as usize, eaddr);
        if (laddr & 0x3F) != 0 {
            tracing::debug!("XRSTOR: not 64-byte aligned, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Read XSAVE header
        let xstate_bv = self.read_virtual_qword(seg, eaddr.wrapping_add(512))?;
        let xcomp_bv = self.read_virtual_qword(seg, eaddr.wrapping_add(520))?;
        let header3 = self.read_virtual_qword(seg, eaddr.wrapping_add(528))?;

        // Reserved header fields must be zero (standard XRSTOR)
        if header3 != 0 || xcomp_bv != 0 {
            tracing::debug!("XRSTOR: reserved header fields not zero, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let xcr0 = self.xcr0.get32() as u64;
        // xstate_bv must not set bits outside XCR0
        if ((!xcr0) & xstate_bv) != 0 {
            tracing::debug!("XRSTOR: xstate_bv has bits not in XCR0, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let requested = (xcr0 & self.eax() as u64) as u32;

        // Restore x87 FPU state
        if (requested & 0x1) != 0 {
            if (xstate_bv & 0x1) != 0 {
                self.xrstor_x87_state(seg, eaddr)?;
            } else {
                self.xrstor_init_x87_state();
            }
        }

        // Restore MXCSR if SSE requested
        if (requested & 0x2) != 0 {
            // Legacy XRSTOR loads MXCSR when SSE or YMM in RFBM
            let new_mxcsr = self.read_virtual_dword(seg, eaddr.wrapping_add(24))?;
            if (new_mxcsr & !self.mxcsr_mask) != 0 {
                tracing::debug!("XRSTOR: invalid MXCSR={:#010x}, #GP(0)", new_mxcsr);
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            self.mxcsr.mxcsr = new_mxcsr;
        }

        // Restore SSE state
        if (requested & 0x2) != 0 {
            if (xstate_bv & 0x2) != 0 {
                self.xrstor_sse_state(seg, eaddr.wrapping_add(160))?;
            } else {
                self.xrstor_init_sse_state();
            }
        }

        Ok(())
    }

    // ========================================================================
    // XSAVE/XRSTOR helper methods
    // ========================================================================

    /// Save x87 FPU state to XSAVE area (offset 0-159)
    /// Same layout as FXSAVE bytes 0-159
    fn xsave_x87_state(&mut self, seg: super::decoder::BxSegregs, eaddr: u32) -> super::Result<()> {
        // FCW
        self.write_virtual_word(seg, eaddr, self.the_i387.cwd)?;
        // FSW
        self.write_virtual_word(seg, eaddr.wrapping_add(2), self.the_i387.swd)?;
        // Abridged FTW
        let aftw = self.abridged_ftw();
        self.write_virtual_byte(seg, eaddr.wrapping_add(4), aftw)?;
        // Reserved + FOP
        self.write_virtual_byte(seg, eaddr.wrapping_add(5), 0)?;
        self.write_virtual_word(seg, eaddr.wrapping_add(6), 0)?;
        // FIP, FCS
        self.write_virtual_dword(seg, eaddr.wrapping_add(8), 0)?;
        self.write_virtual_word(seg, eaddr.wrapping_add(12), 0)?;
        self.write_virtual_word(seg, eaddr.wrapping_add(14), 0)?;
        // FDP, FDS
        self.write_virtual_dword(seg, eaddr.wrapping_add(16), 0)?;
        self.write_virtual_word(seg, eaddr.wrapping_add(20), 0)?;
        self.write_virtual_word(seg, eaddr.wrapping_add(22), 0)?;

        // ST0-ST7 (bytes 32-159, 16 bytes each)
        for i in 0..8u32 {
            let offset = eaddr.wrapping_add(32 + i * 16);
            let signif = self.the_i387.st_space[i as usize].signif;
            let sign_exp = self.the_i387.st_space[i as usize].sign_exp;
            self.write_virtual_qword(seg, offset, signif)?;
            self.write_virtual_word(seg, offset.wrapping_add(8), sign_exp)?;
            self.write_virtual_word(seg, offset.wrapping_add(10), 0)?;
            self.write_virtual_dword(seg, offset.wrapping_add(12), 0)?;
        }

        Ok(())
    }

    /// Save SSE state to XSAVE area (at given offset, 256 bytes: XMM0-XMM7)
    fn xsave_sse_state(&mut self, seg: super::decoder::BxSegregs, base: u32) -> super::Result<()> {
        for i in 0..8u32 {
            let offset = base.wrapping_add(i * 16);
            let lo = unsafe { self.vmm[i as usize].zmm64u[0] };
            let hi = unsafe { self.vmm[i as usize].zmm64u[1] };
            self.write_virtual_qword(seg, offset, lo)?;
            self.write_virtual_qword(seg, offset.wrapping_add(8), hi)?;
        }
        Ok(())
    }

    /// Restore x87 FPU state from XSAVE area (offset 0-159)
    fn xrstor_x87_state(
        &mut self,
        seg: super::decoder::BxSegregs,
        eaddr: u32,
    ) -> super::Result<()> {
        let fcw = self.read_virtual_word(seg, eaddr)?;
        let fsw = self.read_virtual_word(seg, eaddr.wrapping_add(2))?;
        let aftw = self.read_virtual_byte(seg, eaddr.wrapping_add(4))?;

        self.the_i387.cwd = fcw;
        self.the_i387.swd = fsw;
        self.the_i387.tos = ((fsw >> 11) & 7) as u8;
        self.restore_ftw_from_abridged(aftw);

        for i in 0..8u32 {
            let offset = eaddr.wrapping_add(32 + i * 16);
            let signif = self.read_virtual_qword(seg, offset)?;
            let sign_exp = self.read_virtual_word(seg, offset.wrapping_add(8))?;
            self.the_i387.st_space[i as usize].signif = signif;
            self.the_i387.st_space[i as usize].sign_exp = sign_exp;
        }

        Ok(())
    }

    /// Initialize x87 FPU state to reset values
    fn xrstor_init_x87_state(&mut self) {
        self.the_i387.cwd = 0x037F;
        self.the_i387.swd = 0;
        self.the_i387.tos = 0;
        self.the_i387.twd = 0xFFFF; // All empty
        for i in 0..8 {
            self.the_i387.st_space[i].signif = 0;
            self.the_i387.st_space[i].sign_exp = 0;
        }
    }

    /// Restore SSE state from XSAVE area
    fn xrstor_sse_state(&mut self, seg: super::decoder::BxSegregs, base: u32) -> super::Result<()> {
        for i in 0..8u32 {
            let offset = base.wrapping_add(i * 16);
            let lo = self.read_virtual_qword(seg, offset)?;
            let hi = self.read_virtual_qword(seg, offset.wrapping_add(8))?;
            unsafe {
                self.vmm[i as usize].zmm64u[0] = lo;
                self.vmm[i as usize].zmm64u[1] = hi;
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

    /// Initialize SSE state to reset values
    fn xrstor_init_sse_state(&mut self) {
        use super::xmm::MXCSR_RESET;
        self.mxcsr.mxcsr = MXCSR_RESET;
        for i in 0..8 {
            unsafe {
                self.vmm[i].zmm64u[0] = 0;
                self.vmm[i].zmm64u[1] = 0;
            }
        }
    }
}
