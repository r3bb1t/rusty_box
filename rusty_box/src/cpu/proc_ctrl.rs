#![allow(unused_variables)]

#![allow(unused_unsafe)]

use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn handle_cpu_context_change(&mut self) {
        self.tlb_flush();

        self.invalidate_prefetch_q();
        self.invalidate_stack_cache();

        self.handle_interrupt_mask_change();

        self.handle_alignment_check();

        self.handle_cpu_mode_change();

        self.handle_fpu_mmx_mode_change();
        self.handle_sse_mode_change();
        self.handle_avx_mode_change();

        // Bochs calls updateFetchModeMask() after every CS reload and mode change.
        // This updates the icache hash discriminator so 16-bit and 32-bit decoded
        // traces at the same physical address don't collide.
        self.update_fetch_mode_mask();
    }

    /// Update cpu_mode based on CR0.PE, EFLAGS.VM, EFER.LMA, CS.L
    /// Based on Bochs proc_ctrl.cc handleCpuModeChange()
    pub(super) fn handle_cpu_mode_change(&mut self) {
        use super::cpu::CpuMode;
        use super::eflags::EFlags;

        // Bochs proc_ctrl.cc — check EFER.LMA first (long mode active)
        if self.efer.lma() {
            if !self.cr0.pe() {
                // EFER.LMA set when CR0.PE=0 should not happen
                tracing::error!("handle_cpu_mode_change: EFER.LMA is set when CR0.PE=0!");
            }
            // Bochs proc_ctrl.cc — check CS.L bit for 64-bit vs compat mode
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        let cs_l = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .cache
            .u
            .segment_l();
            if cs_l {
                self.cpu_mode = CpuMode::Long64;
            } else {
                self.cpu_mode = CpuMode::LongCompat;
                // Bochs proc_ctrl.cc — clear upper 32 bits of RIP/RSP
                // when leaving 64-bit mode to compatibility mode
                let rip = self.rip() & 0xFFFF_FFFF;
                self.set_rip(rip);
                let rsp = self.rsp() & 0xFFFF_FFFF;
                self.set_rsp(rsp);
            }
            // Bochs proc_ctrl.cc — invalidate stack cache on mode switch
            self.invalidate_stack_cache();
        } else {
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
                // Bochs proc_ctrl.cc — CS segment in real mode allows full access
                // SAFETY: descriptor cache fields set atomically; union write matches descriptor type
                unsafe {
                    let seg = &mut self.sregs[super::decoder::BxSegregs::Cs as usize];
                    seg.cache.p = true; // present (Bochs line 394)
                    seg.cache.segment = true; // data/code segment (Bochs line 395)
                    seg.cache.r#type = 3; // DATA_READ_WRITE_ACCESSED (Bochs line 396)
                    // Note: Bochs does NOT set d_b here — the CS descriptor cache
                    // retains its previous d_b setting. This is important for
                    // "big real mode" / "unreal mode" where d_b=1 allows >64K access.
                    seg.selector.rpl = 0; // CPL = 0 (Bochs line 398)
                }
            }
        }

        // Bochs proc_ctrl.cc — updateFetchModeMask() after every mode change
        self.update_fetch_mode_mask();

        // Bochs proc_ctrl.cc — handleAvxModeChange() after mode change
        self.handle_avx_mode_change();
    }

    // Bochs proc_ctrl.cc — update FPU/MMX permission based on CR0.EM, CR0.TS
    pub(super) fn handle_fpu_mmx_mode_change(&mut self) {
        use super::opcodes_table::FetchModeMask;
        if self.cr0.em() || self.cr0.ts() {
            self.fetch_mode_mask.remove(FetchModeMask::FPU_MMX_OK);
        } else {
            self.fetch_mode_mask.insert(FetchModeMask::FPU_MMX_OK);
        }
    }

    // Bochs proc_ctrl.cc — update SSE permission based on CR0.TS, CR0.EM, CR4.OSFXSR
    pub(super) fn handle_sse_mode_change(&mut self) {
        use super::opcodes_table::FetchModeMask;
        if self.cr0.ts() || self.cr0.em() || !self.cr4.osfxsr() {
            self.fetch_mode_mask.remove(FetchModeMask::SSE_OK);
        } else {
            self.fetch_mode_mask.insert(FetchModeMask::SSE_OK);
        }
    }

    // Bochs proc_ctrl.cc — update AVX permission
    pub(super) fn handle_avx_mode_change(&mut self) {
        use super::opcodes_table::FetchModeMask;
        if self.cr0.ts() || !self.protected_mode() || !self.cr4.osxsave() {
            self.fetch_mode_mask.remove(FetchModeMask::AVX_OK);
        } else {
            self.fetch_mode_mask.insert(FetchModeMask::AVX_OK);
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

    /// Update fetchModeMask — must be called every time CS.L / CS.D_B /
    /// CR0.PE / CR0.TS / CR0.EM / CR4.OSFXSR / CR4.OSXSAVE changes.
    /// Bochs cpu.h updateFetchModeMask()
    #[inline]
    pub(super) fn update_fetch_mode_mask(&mut self) {
        use super::cpu::CpuMode;
        use super::opcodes_table::FetchModeMask;

        // Bochs: fetchModeMask = cpu_state_use_ok | (long64<<1) | d_b
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        let d_b = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .cache
            .u
            .segment_d_b();
        let long64 = self.cpu_mode == CpuMode::Long64;

        // Keep FPU/SSE/AVX readiness (bits 2-7), update D_B and LONG64 (bits 0-1)
        self.fetch_mode_mask.set(FetchModeMask::D_B, d_b);
        self.fetch_mode_mask.set(FetchModeMask::LONG64, long64);

        // Bochs cpu.h — also update user_pl
        self.user_pl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl
            == 3;
    }

    /// Get the Time Stamp Counter value.
    ///
    /// Bochs: TSC = bx_pc_system.time_ticks() + tsc_adjust (no scaling).
    /// time_ticks() advances via pc_system tickn(), NOT via icount.
    /// This matches Bochs where BX_TICKN(10) in HLT advances time_ticks()
    /// but NOT icount, so TSC advances during HLT without inflating icount.
    const TSC_SCALE: u64 = 1;

    pub fn get_tsc(&self, system_ticks: u64) -> u64 {
        (system_ticks.wrapping_mul(Self::TSC_SCALE)).wrapping_add(self.tsc_adjust as u64)
    }

    /// Set the Time Stamp Counter to a specific value
    pub fn set_tsc(&mut self, newval: u64, system_ticks: u64) {
        self.tsc_adjust = newval.wrapping_sub(system_ticks.wrapping_mul(Self::TSC_SCALE)) as i64
    }

    /// Get current system ticks from pc_system (Bochs: bx_pc_system.time_ticks()).
    /// Falls back to icount when pc_system is not wired (unit tests).
    #[inline]
    fn system_ticks(&self) -> u64 {
        if let Some(ps) = self.pc_system_ptr {
            // SAFETY: PcSystem pointer valid for emulator lifetime; single-threaded access
            unsafe { ps.as_ref().time_ticks() }
        } else {
            self.icount
        }
    }

    // =========================================================================
    // System control instructions
    // =========================================================================

    /// WBINVD — Write Back and Invalidate Cache
    /// Based on Bochs proc_ctrl.cc
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
    /// Based on Bochs proc_ctrl.cc
    pub(super) fn invd(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("INVD: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        // Bochs proc_ctrl.cc: flushICaches() — invalidate instruction cache
        self.invalidate_prefetch_q();
        self.i_cache.flush_all();
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
        let eaddr = self.resolve_addr(instr);
        let laddr: u64 = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.get_laddr32(seg as usize, eaddr as u32) as u64
        };
        // Bochs paging.cc TLB_invlpg: invalidate prefetch, stack cache, TLB entries, icache links
        self.invalidate_prefetch_q();
        self.invalidate_stack_cache();
        self.dtlb.invlpg(laddr);
        self.itlb.invlpg(laddr);
        // Bochs paging.cc — iCache.breakLinks()
        self.i_cache.break_links();

        Ok(())
    }

    /// CLTS — Clear Task-Switched Flag in CR0
    /// Based on Bochs crregs.cc
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
    // Bochs: mwait.cc MONITOR instruction
    // =========================================================================

    pub(super) fn monitor(
        &mut self,
        instr: &super::decoder::Instruction,
    ) -> crate::cpu::Result<()> {
        tracing::debug!("MONITOR: RAX={:#x}", self.rax());

        // Bochs mwait.cc: MONITOR requires CPL==0 (CPL always 0 in real mode)
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("MONITOR: CPL={} != 0, #UD", cpl);
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        // Bochs mwait.cc: RCX must be 0 (no optional extensions supported)
        if self.rcx() != 0 {
            tracing::error!(
                "MONITOR: no optional extensions supported, RCX={:#x}",
                self.rcx()
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bochs mwait.cc: bx_address eaddr = RAX & i->asize_mask();
        let seg = super::decoder::BxSegregs::from(instr.seg());

        // Match Bochs asize_mask() lookup table: asize = metaInfo1 & 0x3
        // [0]=16-bit, [1]=32-bit, [2]=64-bit, [3]=64-bit
        const ASIZE_MASK: [u64; 4] = [
            0xFFFF,
            0xFFFF_FFFF,
            0xFFFF_FFFF_FFFF_FFFF,
            0xFFFF_FFFF_FFFF_FFFF,
        ];
        let asize = (instr.as32_l() != 0) as usize | (((instr.as64_l() != 0) as usize) << 1);
        let eaddr = self.rax() & ASIZE_MASK[asize];

        // Bochs mwait.cc: tickle_read_virtual (1-byte read check)
        // Compute the linear address, then translate directly to physical.
        // (read_virtual_byte / v_read_byte don't populate paddress1 —
        //  they call translate_data_read which returns paddr directly.)
        let laddr = if self.long64_mode() {
            // In 64-bit mode the effective address IS the linear address
            eaddr
        } else {
            let seg_base = self.get_segment_base(seg);
            seg_base.wrapping_add(eaddr)
        };
        let paddr = self.translate_data_read(laddr)?;

        // Bochs mwait.cc: validate monitored address has valid host mapping.
        // MMIO addresses (host_page_addr=0) cannot be monitored — MWAIT may
        // never wake. MONITOR still succeeds (acceptable — just warn).
        if self.get_host_write_ptr(laddr).is_none() {
            tracing::warn!(
                "MONITOR: laddr={:#x} paddr={:#x} has no host mapping (MMIO?), MWAIT may never trigger",
                laddr, paddr
            );
        }

        // Bochs mwait.cc: invalidate page in monitoring system
        // (In Bochs this calls bx_pc_system.invlpg(paddr) to clear any
        // cached page state. We don't need this since we check is_monitor
        // on every memory write.)

        // Bochs mwait.cc: arm the monitor with the physical address
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
    // Bochs: mwait.cc MWAIT instruction
    // =========================================================================

    pub(super) fn mwait(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        tracing::debug!("MWAIT: ECX={:#x}", self.ecx());

        // Bochs mwait.cc: MWAIT requires CPL==0 (CPL always 0 in real mode)
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("MWAIT: CPL={} != 0, #UD", cpl);
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        // Bochs mwait.cc: Check ECX extensions
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

        // Bochs mwait.cc: If monitor not armed, just return
        #[cfg(feature = "bx_support_monitor_mwait")]
        {
            if !self.monitor.armed_by_monitor() {
                tracing::debug!("MWAIT: monitor not armed or already triggered, returning");
                return Ok(());
            }
        }

        // Bochs mwait.cc: Determine sleep state
        // ECX[0] = 1: wake on interrupt even if IF=0
        let mwait_if = self.ecx() & 0x1 != 0;

        // Bochs mwait.cc: enter_sleep_state(new_state)
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
        // Bochs flag_ctrl.cc: CPL must be 0, else #UD
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        self.clear_ac();
        Ok(())
    }

    // =========================================================================
    // STAC — Set AC Flag (SMAP, opcode 0F 01 CB)
    // =========================================================================

    pub(super) fn stac(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        // Bochs flag_ctrl.cc: CPL must be 0, else #UD
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
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

        let ticks = self.get_tsc(self.system_ticks());
        self.set_rax(ticks & 0xFFFF_FFFF  );
        self.set_rdx(ticks >> 32  );
        // ECX = IA32_TSC_AUX MSR (processor ID) — Bochs proc_ctrl.cc
        self.set_rcx(self.msr.tsc_aux as u64);

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

        // Use system_ticks (pc_system.time_ticks) as time source.
        // time_ticks advances during HLT via tickn(), matching Bochs behavior.
        let ticks = self.get_tsc(self.system_ticks());

        self.set_rax(ticks & 0xFFFF_FFFF  );
        self.set_rdx(ticks >> 32  );


        Ok(())
    }

    // =========================================================================
    // MSR instructions
    // =========================================================================

    /// RDMSR — Read Model Specific Register
    /// Based on Bochs msr.cc
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
            BX_MSR_TSC => self.get_tsc(self.system_ticks()),
            #[cfg(feature = "bx_support_apic")]
            BX_MSR_APICBASE => self.msr.apicbase,
            #[cfg(not(feature = "bx_support_apic"))]
            BX_MSR_APICBASE => BX_MSR_APICBASE_DEFAULT,
            BX_MSR_BIOS_SIGN_ID => 0x02000065, // Skylake-X microcode revision
            BX_MSR_MTRRCAP => BX_MSR_MTRRCAP_DEFAULT,
            BX_MSR_PMC0..=BX_MSR_PMC7 => 0, // Performance counters — return 0
            BX_MSR_PERFEVTSEL0..=BX_MSR_PERFEVTSEL7 => 0, // Perf event selects — return 0
            BX_MSR_SYSENTER_CS => self.msr.sysenter_cs_msr as u64,
            BX_MSR_SYSENTER_ESP => self.msr.sysenter_esp_msr,
            BX_MSR_SYSENTER_EIP => self.msr.sysenter_eip_msr,
            BX_MSR_PAT => self.msr.pat.U64(),
            BX_MSR_MTRR_DEFTYPE => self.msr.mtrr_deftype as u64,
            n @ BX_MSR_MTRRPHYSBASE0..=BX_MSR_MTRRPHYSMASK7 => {
                self.msr.mtrrphys[(n - BX_MSR_MTRRPHYSBASE0) as usize]
            }
            // Fixed MTRR registers (Bochs msr.cc)
            BX_MSR_MTRRFIX64K_00000 => self.msr.mtrrfix64k.U64(),
            BX_MSR_MTRRFIX16K_80000..=BX_MSR_MTRRFIX16K_A0000 => {
                let idx = (msr - BX_MSR_MTRRFIX16K_80000) as usize;
                self.msr.mtrrfix16k[idx].U64()
            }
            BX_MSR_MTRRFIX4K_C0000..=BX_MSR_MTRRFIX4K_F8000 => {
                let idx = (msr - BX_MSR_MTRRFIX4K_C0000) as usize;
                self.msr.mtrrfix4k[idx].U64()
            }
            // Long-mode MSRs (Bochs msr.cc)
            BX_MSR_EFER => self.efer.get32() as u64,
            BX_MSR_STAR => self.msr.star,
            BX_MSR_LSTAR => self.msr.lstar,
            BX_MSR_CSTAR => self.msr.cstar,
            BX_MSR_FMASK => self.msr.fmask as u64,
            BX_MSR_FSBASE => {
                self.get_segment_base(super::decoder::BxSegregs::Fs)
            }
            BX_MSR_GSBASE => {
                self.get_segment_base(super::decoder::BxSegregs::Gs)
            }
            BX_MSR_KERNELGSBASE => self.msr.kernelgsbase,
            BX_MSR_TSC_AUX => self.msr.tsc_aux as u64,
            // VMX capability MSRs (Bochs msr.cc)
            // Return Bochs-compatible default values so kernel VMX probing doesn't #GP
            0x480 => {
                // IA32_VMX_BASIC: VMCS revision=1, VMCS size=4096, memory type=WB(6)
                // Bits 48=1 (true controls supported), bit 55=1 (INS/OUTS exit info)
                0x0001_0006_0000_0001u64
            }
            0x481 => 0x0000_003F_0000_003Fu64, // IA32_VMX_PINBASED_CTLS
            0x482 => 0x0401_E172_0401_E172u64, // IA32_VMX_PROCBASED_CTLS
            0x483 => 0x0003_6FFF_0000_0000u64, // IA32_VMX_EXIT_CTLS
            0x484 => 0x0000_FFFF_0000_0011u64, // IA32_VMX_ENTRY_CTLS
            0x485 => 0x0000_0000_0000_0000u64, // IA32_VMX_MISC
            0x486 => 0x0000_0000_8000_0000u64, // IA32_VMX_CR0_FIXED0
            0x487 => 0x0000_0000_FFFF_FFFFu64, // IA32_VMX_CR0_FIXED1
            0x488 => 0x0000_0000_0000_2000u64, // IA32_VMX_CR4_FIXED0
            0x489 => 0x0000_0000_003F_27FFu64, // IA32_VMX_CR4_FIXED1
            0x48A => 0x0000_002C_0000_0000u64, // IA32_VMX_VMCS_ENUM
            0x48B => 0x0000_0000_0000_0000u64, // IA32_VMX_PROCBASED_CTLS2
            0x48C => 0x0000_003F_0000_003Fu64, // IA32_VMX_TRUE_PINBASED_CTLS
            0x48D => 0x0401_E172_0401_E172u64, // IA32_VMX_TRUE_PROCBASED_CTLS
            0x48E => 0x0003_6FFF_0000_0000u64, // IA32_VMX_TRUE_EXIT_CTLS
            0x48F => 0x0000_FFFF_0000_0011u64, // IA32_VMX_TRUE_ENTRY_CTLS
            0x490 => 0x0000_0000_0000_0000u64, // IA32_VMX_VMFUNC
            0x491 => 0x0000_0000_0000_0000u64, // IA32_VMX_PROCBASED_CTLS3
            _ => {
                // Bochs: unknown MSRs raise #GP(0)
                if !self.ignore_bad_msrs {
                    tracing::debug!("RDMSR: unknown MSR={:#010x}, #GP(0)", msr);
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                0
            }
        };
        tracing::debug!("RDMSR: MSR={:#010x} -> {:#018x}", msr, val);
        self.set_rax((val & 0xFFFF_FFFF) as u64);
        self.set_rdx((val >> 32) as u64);
        Ok(())
    }

    /// WRMSR — Write Model Specific Register
    /// Based on Bochs msr.cc
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
            BX_MSR_TSC => self.set_tsc(val, self.system_ticks()),
            #[cfg(feature = "bx_support_apic")]
            BX_MSR_APICBASE => self.msr.apicbase = val as _,
            BX_MSR_SYSENTER_CS => self.msr.sysenter_cs_msr = val as u32,
            BX_MSR_SYSENTER_ESP => self.msr.sysenter_esp_msr = val,
            BX_MSR_SYSENTER_EIP => self.msr.sysenter_eip_msr = val,
            BX_MSR_PAT => {
                self.msr.pat.set_U64(val);
            }
            BX_MSR_MTRR_DEFTYPE => self.msr.mtrr_deftype = val as u32,
            n @ BX_MSR_MTRRPHYSBASE0..=BX_MSR_MTRRPHYSMASK7 => {
                self.msr.mtrrphys[(n - BX_MSR_MTRRPHYSBASE0) as usize] = val;
            }
            // Fixed MTRR registers (Bochs msr.cc)
            // SAFETY: descriptor cache fields set atomically; union write matches descriptor type
            BX_MSR_MTRRFIX64K_00000 => unsafe {
                self.msr.mtrrfix64k.set_U64(val);
            },
            BX_MSR_MTRRFIX16K_80000..=BX_MSR_MTRRFIX16K_A0000 => {
                let idx = (msr - BX_MSR_MTRRFIX16K_80000) as usize;
                // SAFETY: descriptor cache fields set atomically; union write matches descriptor type
                unsafe {
                    self.msr.mtrrfix16k[idx].set_U64(val);
                }
            }
            BX_MSR_MTRRFIX4K_C0000..=BX_MSR_MTRRFIX4K_F8000 => {
                let idx = (msr - BX_MSR_MTRRFIX4K_C0000) as usize;
                // SAFETY: descriptor cache fields set atomically; union write matches descriptor type
                unsafe {
                    self.msr.mtrrfix4k[idx].set_U64(val);
                }
            }
            BX_MSR_MTRRCAP => {
                // MTRRCAP is read-only (Bochs msr.cc)
                tracing::debug!("WRMSR: MTRRCAP is read-only, #GP(0)");
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            // Long-mode MSRs (Bochs msr.cc)
            BX_MSR_EFER => {
                // Bochs crregs.cc SetEFER()
                let val32 = val as u32;
                // Check reserved bits against efer_suppmask
                if (val32 & !self.efer_suppmask) != 0 {
                    tracing::debug!(
                        "WRMSR EFER: attempt to set reserved bits {:#010x} (mask={:#010x}), #GP(0)",
                        val32 & !self.efer_suppmask,
                        self.efer_suppmask
                    );
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                // Cannot change LME when CR0.PG=1 (Bochs crregs.cc)
                if self.efer.lme() != ((val32 >> 8) & 1 != 0) && self.cr0.pg() {
                    tracing::debug!("WRMSR EFER: attempt to change LME when CR0.PG=1, #GP(0)");
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                // Keep LMA untouched — it's controlled by CR0.PG + EFER.LME
                // Bochs crregs.cc
                use super::crregs::BxEfer;
                let new_efer = BxEfer::from_bits_truncate(
                    (val32 & self.efer_suppmask & !BxEfer::LMA.bits())
                        | (self.efer.get32() & BxEfer::LMA.bits()),
                );
                self.efer = new_efer;
            }
            BX_MSR_STAR => self.msr.star = val,
            BX_MSR_LSTAR => {
                if !self.is_canonical(val) {
                    tracing::debug!("WRMSR: non-canonical value for MSR_LSTAR, #GP(0)");
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                self.msr.lstar = val;
            }
            BX_MSR_CSTAR => {
                if !self.is_canonical(val) {
                    tracing::debug!("WRMSR: non-canonical value for MSR_CSTAR, #GP(0)");
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                self.msr.cstar = val;
            }
            BX_MSR_FMASK => self.msr.fmask = val as u32,
            BX_MSR_FSBASE => {
                if !self.is_canonical(val) {
                    tracing::debug!("WRMSR: non-canonical value for MSR_FSBASE, #GP(0)");
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                self.set_segment_base(super::decoder::BxSegregs::Fs, val);
            }
            BX_MSR_GSBASE => {
                if !self.is_canonical(val) {
                    tracing::debug!("WRMSR: non-canonical value for MSR_GSBASE, #GP(0)");
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                self.set_segment_base(super::decoder::BxSegregs::Gs, val);
            }
            BX_MSR_KERNELGSBASE => {
                if !self.is_canonical(val) {
                    tracing::debug!("WRMSR: non-canonical value for MSR_KERNELGSBASE, #GP(0)");
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                self.msr.kernelgsbase = val;
            }
            BX_MSR_TSC_AUX => self.msr.tsc_aux = val as u32,
            _ => {
                // Bochs: unknown MSRs raise #GP(0)
                if !self.ignore_bad_msrs {
                    tracing::debug!("WRMSR: unknown MSR={:#010x}, #GP(0)", msr);
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
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

        Ok(())
    }

    // ========================================================================
    // FXSAVE — Save x87 FPU, MMX, SSE state (512 bytes)
    // Bochs: FXSAVE in proc_ctrl.cc
    // ========================================================================

    pub(super) fn fxsave(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;

        // Bochs sse_move.cc: check CR0.EM or CR0.TS → #NM
        if self.cr0.em() || self.cr0.ts() {
            return self.exception(super::cpu::Exception::Nm, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());

        // Must be 16-byte aligned
        if (eaddr & 0xF) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bytes 0-1: FCW (FPU control word)
        self.v_write_word(seg, eaddr, self.the_i387.cwd)?;
        // Bytes 2-3: FSW (FPU status word)
        self.v_write_word(seg, eaddr.wrapping_add(2), self.the_i387.swd)?;
        // Byte 4: FTW (abridged tag word — compact form)
        let abridged_ftw = self.abridged_ftw();
        self.v_write_byte(seg, eaddr.wrapping_add(4), abridged_ftw)?;
        // Byte 5: reserved
        self.v_write_byte(seg, eaddr.wrapping_add(5), 0)?;
        // Bytes 6-7: FOP (last FPU opcode) — not tracked, write 0
        self.v_write_word(seg, eaddr.wrapping_add(6), 0)?;
        // Bytes 8-11: FIP (FPU instruction pointer) — not tracked
        self.v_write_dword(seg, eaddr.wrapping_add(8), 0)?;
        // Bytes 12-13: FCS — not tracked
        self.v_write_word(seg, eaddr.wrapping_add(12), 0)?;
        // Bytes 14-15: reserved
        self.v_write_word(seg, eaddr.wrapping_add(14), 0)?;
        // Bytes 16-19: FDP (FPU data pointer) — not tracked
        self.v_write_dword(seg, eaddr.wrapping_add(16), 0)?;
        // Bytes 20-21: FDS — not tracked
        self.v_write_word(seg, eaddr.wrapping_add(20), 0)?;
        // Bytes 22-23: reserved
        self.v_write_word(seg, eaddr.wrapping_add(22), 0)?;
        // Bytes 24-27: MXCSR
        self.v_write_dword(seg, eaddr.wrapping_add(24), self.mxcsr.mxcsr)?;
        // Bytes 28-31: MXCSR_MASK
        self.v_write_dword(seg, eaddr.wrapping_add(28), self.mxcsr_mask)?;

        // Bytes 32-159: FPU/MMX registers ST0-ST7 (16 bytes each = 80-bit + 6 padding)
        for i in 0..8u64 {
            let offset = eaddr.wrapping_add(32 + i * 16);
            let signif = self.the_i387.st_space[i as usize].signif;
            let sign_exp = self.the_i387.st_space[i as usize].sign_exp;
            self.v_write_qword(seg, offset, signif)?;
            self.v_write_word(seg, offset.wrapping_add(8), sign_exp)?;
            // Bytes 10-15 of each entry are padding (write zeros)
            self.v_write_word(seg, offset.wrapping_add(10), 0)?;
            self.v_write_dword(seg, offset.wrapping_add(12), 0)?;
        }

        // Bytes 160-415: XMM registers XMM0-XMM7 (16 bytes each, 32-bit mode)
        for i in 0..8u64 {
            let offset = eaddr.wrapping_add(160 + i * 16);
            let lo = self.vmm[i as usize].zmm64u(0);
            let hi = self.vmm[i as usize].zmm64u(1);
            self.v_write_qword(seg, offset, lo)?;
            self.v_write_qword(seg, offset.wrapping_add(8), hi)?;
        }

        // Bytes 416-511: reserved (zeros)
        for i in (416u64..512).step_by(8) {
            self.v_write_qword(seg, eaddr.wrapping_add(i), 0)?;
        }

        Ok(())
    }

    // ========================================================================
    // FXRSTOR — Restore x87 FPU, MMX, SSE state (512 bytes)
    // Bochs: FXRSTOR in proc_ctrl.cc
    // ========================================================================

    pub(super) fn fxrstor(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;

        // Bochs sse_move.cc: check CR0.EM or CR0.TS → #NM
        if self.cr0.em() || self.cr0.ts() {
            return self.exception(super::cpu::Exception::Nm, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());

        // Must be 16-byte aligned
        if (eaddr & 0xF) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bytes 0-1: FCW
        let fcw = self.v_read_word(seg, eaddr)?;
        // Bytes 2-3: FSW
        let fsw = self.v_read_word(seg, eaddr.wrapping_add(2))?;
        // Byte 4: abridged FTW
        let abridged_ftw = self.v_read_byte(seg, eaddr.wrapping_add(4))?;
        // Bytes 24-27: MXCSR
        let new_mxcsr = self.v_read_dword(seg, eaddr.wrapping_add(24))?;

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
        for i in 0..8u64 {
            let offset = eaddr.wrapping_add(32 + i * 16);
            let signif = self.v_read_qword(seg, offset)?;
            let sign_exp = self.v_read_word(seg, offset.wrapping_add(8))?;
            self.the_i387.st_space[i as usize].signif = signif;
            self.the_i387.st_space[i as usize].sign_exp = sign_exp;
        }

        // Restore XMM registers only if CR4.OSFXSR is set (Bochs sse_move.cc)
        if self.cr4.osfxsr() {
            for i in 0..8u64 {
                let offset = eaddr.wrapping_add(160 + i * 16);
                let lo = self.v_read_qword(seg, offset)?;
                let hi = self.v_read_qword(seg, offset.wrapping_add(8))?;
                // SAFETY: zmm union access; index within register file bounds
                unsafe {
                    self.vmm[i as usize].set_zmm64u(0, lo);
                    self.vmm[i as usize].set_zmm64u(1, hi);
                    // Clear upper bits
                    self.vmm[i as usize].set_zmm64u(2, 0);
                    self.vmm[i as usize].set_zmm64u(3, 0);
                    self.vmm[i as usize].set_zmm64u(4, 0);
                    self.vmm[i as usize].set_zmm64u(5, 0);
                    self.vmm[i as usize].set_zmm64u(6, 0);
                    self.vmm[i as usize].set_zmm64u(7, 0);
                }
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

        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let new_mxcsr = self.v_read_dword(seg, eaddr)?;

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

        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        self.v_write_dword(seg, eaddr, self.mxcsr.mxcsr & self.mxcsr_mask)?;
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
    // Bochs: proc_ctrl.cc
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

        // Clear VM, IF, RF (Bochs proc_ctrl.cc)
        self.clear_vm();
        self.clear_if();
        self.clear_rf();

        // Long mode: canonical checks (Bochs proc_ctrl.cc)
        if self.long_mode() {
            if !self.is_canonical(self.msr.sysenter_eip_msr) {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            if !self.is_canonical(self.msr.sysenter_esp_msr) {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        // Load CS: flat code segment, DPL=0 (Bochs proc_ctrl.cc)
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
        self.sregs[cs_idx].cache.u.set_segment_base(0);
        self.sregs[cs_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
        self.sregs[cs_idx].cache.u.set_segment_g(true);
        self.sregs[cs_idx].cache.u.set_segment_avl(false);
        self.sregs[cs_idx].cache.u.set_segment_d_b(!self.long_mode());
        self.sregs[cs_idx].cache.u.set_segment_l(self.long_mode());

        self.handle_cpu_mode_change();
        self.alignment_check_mask = 0;
        // Bochs proc_ctrl.cc — updateFetchModeMask() after CS reload
        self.update_fetch_mode_mask();

        // Load SS: flat data segment, DPL=0 (Bochs proc_ctrl.cc)
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
        self.sregs[ss_idx].cache.u.set_segment_base(0);
        self.sregs[ss_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
        self.sregs[ss_idx].cache.u.set_segment_g(true);
        self.sregs[ss_idx].cache.u.set_segment_d_b(true);
        self.sregs[ss_idx].cache.u.set_segment_avl(false);
        self.sregs[ss_idx].cache.u.set_segment_l(false);

        // Load RSP/RIP from MSRs (Bochs proc_ctrl.cc)
        if self.long_mode() {
            self.set_rsp(self.msr.sysenter_esp_msr);
            self.set_rip(self.msr.sysenter_eip_msr);
        } else {
            self.set_esp(self.msr.sysenter_esp_msr as u32);
            self.set_eip(self.msr.sysenter_eip_msr as u32);
        }

        // Bochs: BX_NEXT_TRACE(i) — force trace break after RIP change
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ========================================================================
    // SYSEXIT — Fast System Call Exit (opcode 0F 35)
    // Bochs: proc_ctrl.cc
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

        // 64-bit SYSEXIT (Bochs proc_ctrl.cc)
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
                self.sregs[cs_idx].cache.u.set_segment_base(0);
                self.sregs[cs_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
                self.sregs[cs_idx].cache.u.set_segment_g(true);
                self.sregs[cs_idx].cache.u.set_segment_avl(false);
                self.sregs[cs_idx].cache.u.set_segment_d_b(false);
                self.sregs[cs_idx].cache.u.set_segment_l(true); // 64-bit

            self.set_rsp(self.rcx());
            self.set_rip(self.rdx());
        } else {
            // 32-bit SYSEXIT: CS = (sysenter_cs_msr + 16) | 3 (Bochs proc_ctrl.cc)
            super::segment_ctrl_pro::parse_selector(
                (((self.msr.sysenter_cs_msr + 16) & 0xFFFC) | 3) as u16,
                &mut self.sregs[cs_idx].selector,
            );
            self.sregs[cs_idx].cache.valid = seg_valid;
            self.sregs[cs_idx].cache.p = true;
            self.sregs[cs_idx].cache.dpl = 3;
            self.sregs[cs_idx].cache.segment = true;
            self.sregs[cs_idx].cache.r#type = 0xb;
                self.sregs[cs_idx].cache.u.set_segment_base(0);
                self.sregs[cs_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
                self.sregs[cs_idx].cache.u.set_segment_g(true);
                self.sregs[cs_idx].cache.u.set_segment_avl(false);
                self.sregs[cs_idx].cache.u.set_segment_d_b(true);
                self.sregs[cs_idx].cache.u.set_segment_l(false);

            self.set_esp(self.ecx());
            self.set_eip(self.edx());
        }

        self.handle_cpu_mode_change();
        self.handle_alignment_check();
        // Bochs proc_ctrl.cc — updateFetchModeMask() after CS reload
        self.update_fetch_mode_mask();

        // SS = (sysenter_cs_msr + (os64 ? 40 : 24)) | 3 (Bochs proc_ctrl.cc)
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
        self.sregs[ss_idx].cache.u.set_segment_base(0);
        self.sregs[ss_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
        self.sregs[ss_idx].cache.u.set_segment_g(true);
        self.sregs[ss_idx].cache.u.set_segment_d_b(true);
        self.sregs[ss_idx].cache.u.set_segment_avl(false);
        self.sregs[ss_idx].cache.u.set_segment_l(false);

        // Bochs: BX_NEXT_TRACE(i) — force trace break after RIP change
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ========================================================================
    // SWAPGS — Swap GS base with KernelGSbase MSR (opcode 0F 01 F8)
    // Bochs: proc_ctrl.cc
    // ========================================================================

    pub(super) fn swapgs(&mut self, _instr: &super::decoder::Instruction) -> super::Result<()> {
        // SWAPGS is only valid in 64-bit mode at CPL=0
        if !self.long64_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Swap GS.base with MSR_KERNELGSBASE
        let gs_base = self.get_segment_base(super::decoder::BxSegregs::Gs);
        let kernel_gs = self.msr.kernelgsbase;
        self.set_segment_base(super::decoder::BxSegregs::Gs, kernel_gs);
        self.msr.kernelgsbase = gs_base;
        Ok(())
    }

    // ========================================================================
    // RDFSBASE/RDGSBASE/WRFSBASE/WRGSBASE — Read/Write FS/GS Base (FSGSBASE)
    // Bochs: proc_ctrl.cc RDFSBASE/RDGSBASE/WRFSBASE/WRGSBASE
    // Requires: 64-bit mode + CR4.FSGSBASE
    // ========================================================================

    pub(super) fn rdfsbase_ed(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        if !self.long64_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if !self.cr4.fsgsbase() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let fsbase = self.get_segment_base(super::decoder::BxSegregs::Fs);
        self.set_gpr32(instr.dst().into(), fsbase as u32);
        Ok(())
    }

    pub(super) fn rdfsbase_eq(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        if !self.long64_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if !self.cr4.fsgsbase() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let fsbase = self.get_segment_base(super::decoder::BxSegregs::Fs);
        self.set_gpr64(instr.dst() as usize, fsbase);
        Ok(())
    }

    pub(super) fn rdgsbase_ed(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        if !self.long64_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if !self.cr4.fsgsbase() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let gsbase = self.get_segment_base(super::decoder::BxSegregs::Gs);
        self.set_gpr32(instr.dst().into(), gsbase as u32);
        Ok(())
    }

    pub(super) fn rdgsbase_eq(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        if !self.long64_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if !self.cr4.fsgsbase() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        let gsbase = self.get_segment_base(super::decoder::BxSegregs::Gs);
        self.set_gpr64(instr.dst() as usize, gsbase);
        Ok(())
    }

    pub(super) fn wrfsbase_ed(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        if !self.long64_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if !self.cr4.fsgsbase() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        // Group 15 opcode: dst() = rm = register operand (not src1() which is nnn = opcode extension)
        let val = self.get_gpr32(instr.dst().into()) as u64;
        self.set_segment_base(super::decoder::BxSegregs::Fs, val);
        Ok(())
    }

    pub(super) fn wrfsbase_eq(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        if !self.long64_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if !self.cr4.fsgsbase() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        // Group 15 opcode: dst() = rm = register operand
        let val = self.get_gpr64(instr.dst() as usize);
        if !self.is_canonical(val) {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        self.set_segment_base(super::decoder::BxSegregs::Fs, val);
        Ok(())
    }

    pub(super) fn wrgsbase_ed(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        if !self.long64_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if !self.cr4.fsgsbase() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        // Group 15 opcode: dst() = rm = register operand
        let val = self.get_gpr32(instr.dst().into()) as u64;
        self.set_segment_base(super::decoder::BxSegregs::Gs, val);
        Ok(())
    }

    pub(super) fn wrgsbase_eq(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        if !self.long64_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if !self.cr4.fsgsbase() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        // Group 15 opcode: dst() = rm = register operand
        let val = self.get_gpr64(instr.dst() as usize);
        if !self.is_canonical(val) {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        self.set_segment_base(super::decoder::BxSegregs::Gs, val);
        Ok(())
    }

    // ========================================================================
    // SYSCALL — Fast System Call (opcode 0F 05)
    // Bochs: proc_ctrl.cc
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

        // Record syscall in diagnostic ring buffer
        #[cfg(debug_assertions)] {
            let nr = self.rax();
            let arg0 = self.rdi();
            let arg1 = self.rsi();
            let ic = self.icount;
            let idx = self.diag_syscall_ring_idx % 32;
            self.diag_syscall_ring[idx] = (nr, arg0, ic);
            self.diag_syscall_ring_idx += 1;
            self.diag_syscall_count += 1;
        }
        self.invalidate_prefetch_q();

        let seg_valid =
            SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK | SEG_ACCESS_ROK4_G | SEG_ACCESS_WOK4_G;
        let cs_idx = BxSegregs::Cs as usize;
        let ss_idx = BxSegregs::Ss as usize;

        if self.long_mode() {
            // Long mode SYSCALL (Bochs proc_ctrl.cc)
            let saved_rip = self.rip();
            self.set_rcx(saved_rip);
            let saved_rflags = self.eflags.bits() & !EFlags::RF.bits();
            self.set_r11(saved_rflags as u64);

            let temp_rip = if self.cpu_mode == super::cpu::CpuMode::Long64 {
                self.msr.lstar
            } else {
                self.msr.cstar
            };

            // CS: flat 64-bit code, DPL=0 (Bochs proc_ctrl.cc)
            super::segment_ctrl_pro::parse_selector(
                ((self.msr.star >> 32) & 0xFFFC) as u16,
                &mut self.sregs[cs_idx].selector,
            );
            self.sregs[cs_idx].cache.valid = seg_valid;
            self.sregs[cs_idx].cache.p = true;
            self.sregs[cs_idx].cache.dpl = 0;
            self.sregs[cs_idx].cache.segment = true;
            self.sregs[cs_idx].cache.r#type = 0xb;
            self.sregs[cs_idx].cache.u.set_segment_base(0);
            self.sregs[cs_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
            self.sregs[cs_idx].cache.u.set_segment_g(true);
            self.sregs[cs_idx].cache.u.set_segment_d_b(false);
            self.sregs[cs_idx].cache.u.set_segment_l(true); // 64-bit code
            self.sregs[cs_idx].cache.u.set_segment_avl(false);

            self.handle_cpu_mode_change();
            self.alignment_check_mask = 0;
            // Bochs proc_ctrl.cc — updateFetchModeMask() after CS reload
            self.update_fetch_mode_mask();

            // SS: flat data, DPL=0 (Bochs proc_ctrl.cc)
            super::segment_ctrl_pro::parse_selector(
                (((self.msr.star >> 32) + 8) & 0xFFFC) as u16,
                &mut self.sregs[ss_idx].selector,
            );
            self.sregs[ss_idx].cache.valid = seg_valid;
            self.sregs[ss_idx].cache.p = true;
            self.sregs[ss_idx].cache.dpl = 0;
            self.sregs[ss_idx].cache.segment = true;
            self.sregs[ss_idx].cache.r#type = 0x3;
            self.sregs[ss_idx].cache.u.set_segment_base(0);
            self.sregs[ss_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
            self.sregs[ss_idx].cache.u.set_segment_g(true);
            self.sregs[ss_idx].cache.u.set_segment_d_b(true);
            self.sregs[ss_idx].cache.u.set_segment_l(false);
            self.sregs[ss_idx].cache.u.set_segment_avl(false);

            // Mask RFLAGS with FMASK, clear RF (Bochs proc_ctrl.cc)
            let new_flags = saved_rflags & !self.msr.fmask & !EFlags::RF.bits();
            self.write_eflags(new_flags, EFlags::VALID_MASK.bits());
            self.set_rip(temp_rip);
        } else {
            // Legacy mode SYSCALL (Bochs proc_ctrl.cc)
            let saved_eip = self.eip();
            self.set_ecx(saved_eip);
            let temp_rip = self.msr.star as u32;

            // CS: flat 32-bit code, DPL=0 (Bochs proc_ctrl.cc)
            super::segment_ctrl_pro::parse_selector(
                ((self.msr.star >> 32) & 0xFFFC) as u16,
                &mut self.sregs[cs_idx].selector,
            );
            self.sregs[cs_idx].cache.valid = seg_valid;
            self.sregs[cs_idx].cache.p = true;
            self.sregs[cs_idx].cache.dpl = 0;
            self.sregs[cs_idx].cache.segment = true;
            self.sregs[cs_idx].cache.r#type = 0xb;
            self.sregs[cs_idx].cache.u.set_segment_base(0);
            self.sregs[cs_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
            self.sregs[cs_idx].cache.u.set_segment_g(true);
            self.sregs[cs_idx].cache.u.set_segment_d_b(true);
            self.sregs[cs_idx].cache.u.set_segment_l(false);
            self.sregs[cs_idx].cache.u.set_segment_avl(false);

            self.handle_cpu_mode_change();
            self.alignment_check_mask = 0;
            // Bochs proc_ctrl.cc — updateFetchModeMask() after CS reload
            self.update_fetch_mode_mask();

            // SS: flat data, DPL=0 (Bochs proc_ctrl.cc)
            super::segment_ctrl_pro::parse_selector(
                (((self.msr.star >> 32) + 8) & 0xFFFC) as u16,
                &mut self.sregs[ss_idx].selector,
            );
            self.sregs[ss_idx].cache.valid = seg_valid;
            self.sregs[ss_idx].cache.p = true;
            self.sregs[ss_idx].cache.dpl = 0;
            self.sregs[ss_idx].cache.segment = true;
            self.sregs[ss_idx].cache.r#type = 0x3;
            self.sregs[ss_idx].cache.u.set_segment_base(0);
            self.sregs[ss_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
            self.sregs[ss_idx].cache.u.set_segment_g(true);
            self.sregs[ss_idx].cache.u.set_segment_d_b(true);
            self.sregs[ss_idx].cache.u.set_segment_l(false);
            self.sregs[ss_idx].cache.u.set_segment_avl(false);

            self.clear_vm();
            self.clear_if();
            self.clear_rf();
            self.set_rip(temp_rip as u64);
        }

        // Bochs: BX_NEXT_TRACE(i) — force trace break after RIP change
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ========================================================================
    // SYSRET — Fast System Call Return (opcode 0F 07)
    // Bochs: proc_ctrl.cc
    // ========================================================================

    pub(super) fn sysret(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;
        use super::descriptor::{
            SEG_ACCESS_ROK, SEG_ACCESS_ROK4_G, SEG_ACCESS_WOK, SEG_ACCESS_WOK4_G, SEG_VALID_CACHE,
        };
        use super::eflags::EFlags;

        // Track SYSRET for diagnostics
        #[cfg(debug_assertions)] { self.diag_sysret_count += 1; }

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

        // Bochs proc_ctrl.cc — temp_RIP stores the return address;
        // RIP is set AFTER all mode changes (line 1348).
        let temp_rip: u64;

        if self.cpu_mode == super::cpu::CpuMode::Long64 {
            // 64-bit mode SYSRET (Bochs proc_ctrl.cc)
            if instr.os64_l() != 0 {
                // Return to 64-bit mode (Bochs proc_ctrl.cc)
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
                    self.sregs[cs_idx].cache.u.set_segment_base(0);
                    self.sregs[cs_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
                    self.sregs[cs_idx].cache.u.set_segment_g(true);
                    self.sregs[cs_idx].cache.u.set_segment_d_b(false);
                    self.sregs[cs_idx].cache.u.set_segment_l(true); // 64-bit
                    self.sregs[cs_idx].cache.u.set_segment_avl(false);

                // Bochs proc_ctrl.cc — save RCX for later RIP assignment
                temp_rip = self.rcx();
            } else {
                // Return to 32-bit compat mode (Bochs proc_ctrl.cc)
                super::segment_ctrl_pro::parse_selector(
                    (((self.msr.star >> 48) & 0xFFFC) | 3) as u16,
                    &mut self.sregs[cs_idx].selector,
                );
                self.sregs[cs_idx].cache.valid = seg_valid;
                self.sregs[cs_idx].cache.p = true;
                self.sregs[cs_idx].cache.dpl = 3;
                self.sregs[cs_idx].cache.segment = true;
                self.sregs[cs_idx].cache.r#type = 0xb;
                    self.sregs[cs_idx].cache.u.set_segment_base(0);
                    self.sregs[cs_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
                    self.sregs[cs_idx].cache.u.set_segment_g(true);
                    self.sregs[cs_idx].cache.u.set_segment_d_b(true);
                    self.sregs[cs_idx].cache.u.set_segment_l(false);
                    self.sregs[cs_idx].cache.u.set_segment_avl(false);

                // Bochs proc_ctrl.cc — save ECX for later RIP assignment
                temp_rip = self.ecx() as u64;
            }

            // Bochs proc_ctrl.cc — handleCpuModeChange (mode change from CS.L)
            self.handle_cpu_mode_change();
            // Bochs proc_ctrl.cc — handleAlignmentCheck (CPL change)
            self.handle_alignment_check();

            // SS: (star >> 48) + 8) | 3 (Bochs proc_ctrl.cc)
            super::segment_ctrl_pro::parse_selector(
                ((((self.msr.star >> 48) + 8) & 0xFFFC) | 3) as u16,
                &mut self.sregs[ss_idx].selector,
            );
            self.sregs[ss_idx].cache.valid = seg_valid;
            self.sregs[ss_idx].cache.p = true;
            self.sregs[ss_idx].cache.dpl = 3;
            self.sregs[ss_idx].cache.segment = true;
            self.sregs[ss_idx].cache.r#type = 0x3;

            // Bochs proc_ctrl.cc — restore RFLAGS from R11
            self.write_eflags(self.r11() as u32, EFlags::VALID_MASK.bits());
        } else {
            // Legacy/compat mode SYSRET (Bochs proc_ctrl.cc)
            super::segment_ctrl_pro::parse_selector(
                (((self.msr.star >> 48) & 0xFFFC) | 3) as u16,
                &mut self.sregs[cs_idx].selector,
            );
            self.sregs[cs_idx].cache.valid = seg_valid;
            self.sregs[cs_idx].cache.p = true;
            self.sregs[cs_idx].cache.dpl = 3;
            self.sregs[cs_idx].cache.segment = true;
            self.sregs[cs_idx].cache.r#type = 0xb;
            self.sregs[cs_idx].cache.u.set_segment_base(0);
            self.sregs[cs_idx].cache.u.set_segment_limit_scaled(0xFFFF_FFFF);
            self.sregs[cs_idx].cache.u.set_segment_g(true);
            self.sregs[cs_idx].cache.u.set_segment_d_b(true);
            self.sregs[cs_idx].cache.u.set_segment_l(false);
            self.sregs[cs_idx].cache.u.set_segment_avl(false);

            // Bochs proc_ctrl.cc — updateFetchModeMask after CS reload
            // (NOT handleCpuModeChange — that's only called at line 1346 outside)
            self.update_fetch_mode_mask();
            // Bochs proc_ctrl.cc — handleAlignmentCheck (CPL change)
            self.handle_alignment_check();

            // SS: (star >> 48) + 8) | 3 (Bochs proc_ctrl.cc)
            super::segment_ctrl_pro::parse_selector(
                ((((self.msr.star >> 48) + 8) & 0xFFFC) | 3) as u16,
                &mut self.sregs[ss_idx].selector,
            );
            self.sregs[ss_idx].cache.valid = seg_valid;
            self.sregs[ss_idx].cache.p = true;
            self.sregs[ss_idx].cache.dpl = 3;
            self.sregs[ss_idx].cache.segment = true;
            self.sregs[ss_idx].cache.r#type = 0x3;

            // Bochs proc_ctrl.cc — assert_IF()
            self.eflags.insert(super::eflags::EFlags::IF_);
            self.handle_interrupt_mask_change();
            // Bochs proc_ctrl.cc — temp_RIP = ECX
            temp_rip = self.ecx() as u64;
        }

        // Bochs proc_ctrl.cc — handleCpuModeChange (final, outside if/else)
        self.handle_cpu_mode_change();

        // Bochs proc_ctrl.cc — RIP = temp_RIP (set AFTER all mode changes)
        self.set_rip(temp_rip);

        // Bochs: BX_NEXT_TRACE(i) — force trace break after RIP change
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
        Ok(())
    }

    // ========================================================================
    // XGETBV — Get Extended Control Register (opcode 0F 01 D0)
    // Bochs: proc_ctrl.cc
    // ========================================================================

    pub(super) fn xgetbv(&mut self, _instr: &super::decoder::Instruction) -> super::Result<()> {
        // CR4.OSXSAVE must be set
        if !self.cr4.osxsave() {
            tracing::debug!("XGETBV: CR4.OSXSAVE not set, #UD");
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let ecx = self.ecx();
        if ecx == 0 {
            // XCR0 → EDX:EAX
            let xcr0_val = self.xcr0.get32() as u64;
            self.set_rax(xcr0_val & 0xFFFF_FFFF);
            self.set_rdx(xcr0_val >> 32);
            return Ok(());
        }

        if ecx == 1 {
            // XGETBV ECX=1 returns XINUSE vector (requires XSAVEC support)
            let xinuse = self.get_xinuse_vector(self.xcr0.get32() as u64);
            self.set_rdx(0);
            self.set_rax(xinuse);
            return Ok(());
        }

        tracing::debug!("XGETBV: invalid XCR{}, #GP(0)", ecx);
        self.exception(super::cpu::Exception::Gp, 0)
    }

    // ========================================================================
    // XSETBV — Set Extended Control Register (opcode 0F 01 D1)
    // Bochs: proc_ctrl.cc
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

        // AVX-512: if any of OPMASK/ZMM_HI256/HI_ZMM set, all of FPU+SSE+YMM+OPMASK+ZMM_HI256+HI_ZMM must be set
        if (eax & 0xE0) != 0 {
            // bits 5,6,7 = OPMASK, ZMM_HI256, HI_ZMM
            let avx512_mask = 0x01 | 0x02 | 0x04 | 0x20 | 0x40 | 0x80; // FPU+SSE+YMM+OPMASK+ZMM_HI256+HI_ZMM
            if (eax & avx512_mask) != avx512_mask {
                tracing::debug!("XSETBV: AVX-512 partial enable without all dependencies, #GP(0)");
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        self.xcr0.set32(eax);
        self.handle_avx_mode_change();
        tracing::debug!("XSETBV: XCR0={:#010x}", eax);

        Ok(())
    }

    // ========================================================================
    // XSAVE — Save Processor Extended State (opcode 0F AE /4)
    // Bochs: xsave.cc
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

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());

        // Must be 64-byte aligned
        let laddr: u64 = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.get_laddr32(seg as usize, eaddr as u32) as u64
        };
        if (laddr & 0x3F) != 0 {
            tracing::debug!("XSAVE: not 64-byte aligned, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let xcr0 = self.xcr0.get32() as u64;
        let requested = xcr0 & self.eax() as u64;

        // Read existing xstate_bv from header
        let mut xstate_bv = self.v_read_qword(seg, eaddr.wrapping_add(512))?;

        // Compute xinuse vector (Bochs xsave.cc)
        let xinuse = self.get_xinuse_vector(requested);

        // Save x87 FPU state if requested (bit 0)
        // Bochs: always saves if requested (not XSAVEOPT), updates xstate_bv per xinuse
        if (requested & 1) != 0 {
            self.xsave_x87_state(seg, eaddr, instr.os64_l() != 0)?;
            if (xinuse & 1) != 0 {
                xstate_bv |= 1;
            } else {
                xstate_bv &= !1;
            }
        }

        // Save MXCSR if SSE or YMM requested (Bochs xsave.cc)
        if (requested & 0x6) != 0 {
            self.v_write_dword(seg, eaddr.wrapping_add(24), self.mxcsr.mxcsr)?;
            self.v_write_dword(seg, eaddr.wrapping_add(28), self.mxcsr_mask)?;
        }

        // Save SSE state if requested (bit 1)
        if (requested & 2) != 0 {
            self.xsave_sse_state(seg, eaddr.wrapping_add(160))?;
            if (xinuse & 2) != 0 {
                xstate_bv |= 2;
            } else {
                xstate_bv &= !2;
            }
        }

        // Save extended features at standard (fixed) offsets
        for feature in 2..=7u32 {
            let mask = 1u64 << feature;
            if (requested & mask) != 0 {
                let offset = Self::xsave_component_offset(feature);
                self.xsave_extended_component(seg, eaddr.wrapping_add(offset), feature)?;
                if (xinuse & mask) != 0 {
                    xstate_bv |= mask;
                } else {
                    xstate_bv &= !mask;
                }
            }
        }

        // Write XSAVE header: xstate_bv at offset 512
        // XSAVE must NOT modify bytes 519:63 (xcomp_bv and reserved fields)
        self.v_write_qword(seg, eaddr.wrapping_add(512), xstate_bv)?;

        Ok(())
    }

    // ========================================================================
    // XRSTOR — Restore Processor Extended State (opcode 0F AE /5)
    // Bochs: xsave.cc — delegates to xrstor_unified
    // ========================================================================

    pub(super) fn xrstor(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        self.xrstor_unified(instr, false)
    }

    // ========================================================================
    // XSAVE/XRSTOR helper methods
    // ========================================================================

    /// Save x87 FPU state to XSAVE area (offset 0-159)
    /// Same layout as FXSAVE bytes 0-159
    fn xsave_x87_state(&mut self, seg: super::decoder::BxSegregs, eaddr: u64, os64: bool) -> super::Result<()> {
        // FCW
        self.v_write_word(seg, eaddr, self.the_i387.cwd)?;
        // FSW
        self.v_write_word(seg, eaddr.wrapping_add(2), self.the_i387.swd)?;
        // Abridged FTW
        let aftw = self.abridged_ftw();
        self.v_write_byte(seg, eaddr.wrapping_add(4), aftw)?;
        // Reserved byte at offset 5
        self.v_write_byte(seg, eaddr.wrapping_add(5), 0)?;
        // FOP (opcode, 11 bits)
        self.v_write_word(seg, eaddr.wrapping_add(6), self.the_i387.foo & 0x7FF)?;
        // FIP, FCS / FDP, FDS — format depends on operand size, not CPU mode
        if os64 {
            // 64-bit mode: FIP as u64 at offset 8, FDP as u64 at offset 16
            self.v_write_qword(seg, eaddr.wrapping_add(8), self.the_i387.fip)?;
            self.v_write_qword(seg, eaddr.wrapping_add(16), self.the_i387.fdp)?;
        } else {
            // 32-bit mode: FIP as u32, FCS as u16, FDP as u32, FDS as u16
            self.v_write_dword(seg, eaddr.wrapping_add(8), self.the_i387.fip as u32)?;
            self.v_write_word(seg, eaddr.wrapping_add(12), self.the_i387.fcs)?;
            self.v_write_word(seg, eaddr.wrapping_add(14), 0)?;
            self.v_write_dword(seg, eaddr.wrapping_add(16), self.the_i387.fdp as u32)?;
            self.v_write_word(seg, eaddr.wrapping_add(20), self.the_i387.fds)?;
            self.v_write_word(seg, eaddr.wrapping_add(22), 0)?;
        }

        // ST0-ST7 (bytes 32-159, 16 bytes each)
        for i in 0..8u64 {
            let offset = eaddr.wrapping_add(32 + i * 16);
            let signif = self.the_i387.st_space[i as usize].signif;
            let sign_exp = self.the_i387.st_space[i as usize].sign_exp;
            self.v_write_qword(seg, offset, signif)?;
            self.v_write_word(seg, offset.wrapping_add(8), sign_exp)?;
            self.v_write_word(seg, offset.wrapping_add(10), 0)?;
            self.v_write_dword(seg, offset.wrapping_add(12), 0)?;
        }

        Ok(())
    }

    /// Save SSE state to XSAVE area (at given offset, up to 256 bytes: XMM0-XMM15)
    /// Bochs xsave.cc: 8 regs in 32-bit mode, 16 in 64-bit mode
    fn xsave_sse_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        let num = if self.long64_mode() { 16u64 } else { 8u64 };
        for i in 0..num {
            let offset = base.wrapping_add(i * 16);
            let lo = self.vmm[i as usize].zmm64u(0);
            let hi = self.vmm[i as usize].zmm64u(1);
            self.v_write_qword(seg, offset, lo)?;
            self.v_write_qword(seg, offset.wrapping_add(8), hi)?;
        }
        Ok(())
    }

    /// Restore x87 FPU state from XSAVE area (offset 0-159)
    fn xrstor_x87_state(
        &mut self,
        seg: super::decoder::BxSegregs,
        eaddr: u64,
        os64: bool,
    ) -> super::Result<()> {
        let fcw = self.v_read_word(seg, eaddr)?;
        let fsw = self.v_read_word(seg, eaddr.wrapping_add(2))?;
        let aftw = self.v_read_byte(seg, eaddr.wrapping_add(4))?;

        // Bochs forces CW bit 6 always set, clear reserved bits (6,7,13,14,15)
        let cwd = (fcw & !0xe0c0u16) | 0x0040;
        self.the_i387.cwd = cwd;
        self.the_i387.swd = fsw;
        self.the_i387.tos = ((fsw >> 11) & 7) as u8;
        self.restore_ftw_from_abridged(aftw);

        // Restore FOP (opcode, 11 bits)
        let fop_raw = self.v_read_word(seg, eaddr.wrapping_add(6))?;
        self.the_i387.foo = fop_raw & 0x7FF;

        // Restore FIP/FCS/FDP/FDS — format depends on operand size, not CPU mode
        if os64 {
            // 64-bit mode: FIP as u64 at offset 8, FDP as u64 at offset 16
            self.the_i387.fip = self.v_read_qword(seg, eaddr.wrapping_add(8))?;
            self.the_i387.fdp = self.v_read_qword(seg, eaddr.wrapping_add(16))?;
            self.the_i387.fcs = 0;
            self.the_i387.fds = 0;
        } else {
            // 32-bit mode: FIP as u32, FCS as u16, FDP as u32, FDS as u16
            self.the_i387.fip = self.v_read_dword(seg, eaddr.wrapping_add(8))? as u64;
            self.the_i387.fcs = self.v_read_word(seg, eaddr.wrapping_add(12))?;
            self.the_i387.fdp = self.v_read_dword(seg, eaddr.wrapping_add(16))? as u64;
            self.the_i387.fds = self.v_read_word(seg, eaddr.wrapping_add(20))?;
        }

        // Restore ST0-ST7
        for i in 0..8u64 {
            let offset = eaddr.wrapping_add(32 + i * 16);
            let signif = self.v_read_qword(seg, offset)?;
            let sign_exp = self.v_read_word(seg, offset.wrapping_add(8))?;
            self.the_i387.st_space[i as usize].signif = signif;
            self.the_i387.st_space[i as usize].sign_exp = sign_exp;
        }

        // Update B and ES bits based on unmasked exceptions
        // Bochs: if unmasked exceptions exist, set Summary + Backward bits
        let mut swd = self.the_i387.swd;
        if (swd & !cwd) & 0x3F != 0 {
            swd |= 0xC000; // FPU_SW_Summary | FPU_SW_Backward
        } else {
            swd &= !0xC000u16;
        }
        self.the_i387.swd = swd;

        Ok(())
    }

    /// Initialize x87 FPU state to reset values
    fn xrstor_init_x87_state(&mut self) {
        self.the_i387.cwd = 0x037F;
        self.the_i387.swd = 0;
        self.the_i387.tos = 0;
        self.the_i387.twd = 0xFFFF; // All empty
        self.the_i387.foo = 0;
        self.the_i387.fip = 0;
        self.the_i387.fcs = 0;
        self.the_i387.fdp = 0;
        self.the_i387.fds = 0;
        for i in 0..8 {
            self.the_i387.st_space[i].signif = 0;
            self.the_i387.st_space[i].sign_exp = 0;
        }
    }

    /// Restore SSE state from XSAVE area
    /// Bochs xsave.cc: 8 regs in 32-bit mode, 16 in 64-bit mode
    /// Only modifies lower 128 bits (XMM); upper YMM/ZMM bits are separate components
    fn xrstor_sse_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        let num = if self.long64_mode() { 16u64 } else { 8u64 };
        for i in 0..num {
            let offset = base.wrapping_add(i * 16);
            let lo = self.v_read_qword(seg, offset)?;
            let hi = self.v_read_qword(seg, offset.wrapping_add(8))?;
            // SAFETY: zmm union access; index within register file bounds
            unsafe {
                self.vmm[i as usize].set_zmm64u(0, lo);
                self.vmm[i as usize].set_zmm64u(1, hi);
            }
        }
        Ok(())
    }

    /// INVPCID — Invalidate Process-Context Identifier
    /// Bochs vmx.cc
    /// Opcode: 66 0F 38 82 (memory-only, #UD if mod==11)
    pub(super) fn invpcid(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        // v8086 mode: #GP(0)
        if self.v8086_mode() {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // CPL != 0: #GP(0)
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Read type from register operand (Bochs: i->dst())
        let inv_type = if instr.os64_l() != 0 {
            self.get_gpr64(instr.dst() as usize)
        } else {
            self.get_gpr32(instr.dst() as usize) as u64
        };

        // Read 128-bit descriptor from memory (Bochs: read_virtual_xmmword)
        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let desc = self.v_read_xmmword(seg, eaddr)?;

        // Descriptor bits [63:12] must be zero (reserved)
        let desc_lo = desc.xmm64u(0);
        let desc_hi = desc.xmm64u(1);
        if desc_lo > 0xFFF {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // PCID from descriptor (bits [11:0])
        let _pcid = (desc_lo & 0xFFF) as u16;

        match inv_type {
            // Type 0: Individual address, non-global invalidation
            0 => {
                // Canonical check on linear address (descriptor[127:64])
                if !self.is_canonical(desc_hi) {
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                // PCID check: if CR4.PCIDE=0, PCID must be 0
                if !self.cr4.pcide() && _pcid != 0 {
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                self.tlb_flush_non_global();
            }
            // Type 1: Single-context, non-global invalidation
            1 => {
                if !self.cr4.pcide() && _pcid != 0 {
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                self.tlb_flush_non_global();
            }
            // Type 2: All-context invalidation (including globals)
            2 => {
                self.tlb_flush();
            }
            // Type 3: All-context, non-global invalidation
            3 => {
                self.tlb_flush_non_global();
            }
            _ => {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        Ok(())
    }

    /// Initialize SSE state to reset values
    fn xrstor_init_sse_state(&mut self) {
        let num = if self.long64_mode() { 16 } else { 8 };
        for i in 0..num {
            // SAFETY: zmm union access; index within register file bounds
            unsafe {
                self.vmm[i].set_zmm64u(0, 0);
                self.vmm[i].set_zmm64u(1, 0);
            }
        }
    }

    // ========================================================================
    // Extended XSAVE component helpers (YMM, OPMASK, ZMM_HI256, HI_ZMM)
    // Bochs xsave.cc per-component save/restore/init methods
    // ========================================================================

    /// YMM state: upper 128 bits of YMM0-YMM15 (256 bytes max)
    fn xsave_ymm_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        let num = if self.long64_mode() { 16u64 } else { 8u64 };
        for i in 0..num {
            let offset = base.wrapping_add(i * 16);
            // SAFETY: zmm union access; index within register file bounds
            unsafe {
                self.v_write_qword(seg, offset, self.vmm[i as usize].zmm64u(2))?;
                self.v_write_qword(seg, offset.wrapping_add(8), self.vmm[i as usize].zmm64u(3))?;
            }
        }
        Ok(())
    }

    fn xrstor_ymm_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        let num = if self.long64_mode() { 16u64 } else { 8u64 };
        for i in 0..num {
            let offset = base.wrapping_add(i * 16);
            // SAFETY: zmm union access; index within register file bounds
            unsafe {
                let __tmp = self.v_read_qword(seg, offset)?;

                self.vmm[i as usize].set_zmm64u(2, __tmp);
                let __tmp = self.v_read_qword(seg, offset.wrapping_add(8))?;

                self.vmm[i as usize].set_zmm64u(3, __tmp);
            }
        }
        Ok(())
    }

    fn xrstor_init_ymm_state(&mut self) {
        let num = if self.long64_mode() { 16 } else { 8 };
        for i in 0..num {
            // SAFETY: zmm union access; index within register file bounds
            unsafe {
                self.vmm[i].set_zmm64u(2, 0);
                self.vmm[i].set_zmm64u(3, 0);
            }
        }
    }

    /// OPMASK state: k0-k7 (64 bytes)
    fn xsave_opmask_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        for i in 0..8u64 {
            let val = self.opmask[i as usize].rrx();
            self.v_write_qword(seg, base.wrapping_add(i * 8), val)?;
        }
        Ok(())
    }

    fn xrstor_opmask_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        for i in 0..8u64 {
            let val = self.v_read_qword(seg, base.wrapping_add(i * 8))?;
            self.bx_write_opmask(i as usize, val);
        }
        Ok(())
    }

    fn xrstor_init_opmask_state(&mut self) {
        for i in 0..8 {
            self.bx_write_opmask(i, 0);
        }
    }

    /// ZMM_HI256 state: upper 256 bits of ZMM0-ZMM15 (512 bytes max)
    fn xsave_zmm_hi256_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        let num = if self.long64_mode() { 16u64 } else { 8u64 };
        for i in 0..num {
            let offset = base.wrapping_add(i * 32);
            // SAFETY: zmm union access; index within register file bounds
            unsafe {
                self.v_write_qword(seg, offset, self.vmm[i as usize].zmm64u(4))?;
                self.v_write_qword(seg, offset.wrapping_add(8), self.vmm[i as usize].zmm64u(5))?;
                self.v_write_qword(seg, offset.wrapping_add(16), self.vmm[i as usize].zmm64u(6))?;
                self.v_write_qword(seg, offset.wrapping_add(24), self.vmm[i as usize].zmm64u(7))?;
            }
        }
        Ok(())
    }

    fn xrstor_zmm_hi256_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        let num = if self.long64_mode() { 16u64 } else { 8u64 };
        for i in 0..num {
            let offset = base.wrapping_add(i * 32);
            // SAFETY: zmm union access; index within register file bounds
            unsafe {
                let __tmp = self.v_read_qword(seg, offset)?;

                self.vmm[i as usize].set_zmm64u(4, __tmp);
                let __tmp = self.v_read_qword(seg, offset.wrapping_add(8))?;

                self.vmm[i as usize].set_zmm64u(5, __tmp);
                let __tmp = self.v_read_qword(seg, offset.wrapping_add(16))?;

                self.vmm[i as usize].set_zmm64u(6, __tmp);
                let __tmp = self.v_read_qword(seg, offset.wrapping_add(24))?;

                self.vmm[i as usize].set_zmm64u(7, __tmp);
            }
        }
        Ok(())
    }

    fn xrstor_init_zmm_hi256_state(&mut self) {
        let num = if self.long64_mode() { 16 } else { 8 };
        for i in 0..num {
            // SAFETY: zmm union access; index within register file bounds
            unsafe {
                self.vmm[i].set_zmm64u(4, 0);
                self.vmm[i].set_zmm64u(5, 0);
                self.vmm[i].set_zmm64u(6, 0);
                self.vmm[i].set_zmm64u(7, 0);
            }
        }
    }

    /// HI_ZMM state: full ZMM16-ZMM31 (1024 bytes, 64-bit mode only)
    fn xsave_hi_zmm_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        if self.long64_mode() {
            for idx in 16..32u64 {
                let offset = base.wrapping_add((idx - 16) * 64);
                // SAFETY: zmm union access; index within register file bounds
                unsafe {
                    for j in 0..8u64 {
                        self.v_write_qword(seg, offset.wrapping_add(j * 8),
                            self.vmm[idx as usize].zmm64u(j as usize))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn xrstor_hi_zmm_state(&mut self, seg: super::decoder::BxSegregs, base: u64) -> super::Result<()> {
        if self.long64_mode() {
            for idx in 16..32u64 {
                let offset = base.wrapping_add((idx - 16) * 64);
                // SAFETY: zmm union access; index within register file bounds
                unsafe {
                    for j in 0..8u64 {
                        let __tmp = self.v_read_qword(seg, offset.wrapping_add(j * 8))?;

                        self.vmm[idx as usize].set_zmm64u(j as usize, __tmp);
                    }
                }
            }
        }
        Ok(())
    }

    fn xrstor_init_hi_zmm_state(&mut self) {
        if self.long64_mode() {
            for idx in 16..32 {
                // SAFETY: zmm union access; index within register file bounds
                unsafe {
                    for j in 0..8 {
                        self.vmm[idx].set_zmm64u(j, 0);
                    }
                }
            }
        }
    }

    /// Save an extended component at the given offset
    /// Used by both standard XSAVE and compacted XSAVEC
    fn xsave_extended_component(&mut self, seg: super::decoder::BxSegregs, base: u64, feature: u32) -> super::Result<()> {
        match feature {
            2 => self.xsave_ymm_state(seg, base),
            5 => self.xsave_opmask_state(seg, base),
            6 => self.xsave_zmm_hi256_state(seg, base),
            7 => self.xsave_hi_zmm_state(seg, base),
            _ => Ok(()),
        }
    }

    /// Restore an extended component from the given offset
    fn xrstor_extended_component(&mut self, seg: super::decoder::BxSegregs, base: u64, feature: u32) -> super::Result<()> {
        match feature {
            2 => self.xrstor_ymm_state(seg, base),
            5 => self.xrstor_opmask_state(seg, base),
            6 => self.xrstor_zmm_hi256_state(seg, base),
            7 => self.xrstor_hi_zmm_state(seg, base),
            _ => Ok(()),
        }
    }

    /// Init an extended component to reset values
    fn xrstor_init_extended_component(&mut self, feature: u32) {
        match feature {
            2 => self.xrstor_init_ymm_state(),
            5 => self.xrstor_init_opmask_state(),
            6 => self.xrstor_init_zmm_hi256_state(),
            7 => self.xrstor_init_hi_zmm_state(),
            _ => {}
        }
    }

    /// Get the size of an extended XSAVE component
    /// Bochs xsave_restore[] table sizes
    fn xsave_component_len(feature: u32) -> u64 {
        match feature {
            0 => 160,   // FPU
            1 => 256,   // SSE
            2 => 256,   // YMM
            5 => 64,    // OPMASK
            6 => 512,   // ZMM_HI256
            7 => 1024,  // HI_ZMM
            _ => 0,
        }
    }

    /// Get the standard (non-compacted) offset for an extended component
    /// From CPUID leaf 0xD sub-leaves
    fn xsave_component_offset(feature: u32) -> u64 {
        match feature {
            2 => 576,   // YMM
            5 => 1088,  // OPMASK
            6 => 1152,  // ZMM_HI256
            7 => 1664,  // HI_ZMM
            _ => 0,
        }
    }

    /// Check which XSAVE components have non-init state
    /// Bochs xsave.cc get_xinuse_vector()
    fn get_xinuse_vector(&self, rfbm: u64) -> u64 {
        use super::xmm::MXCSR_RESET;
        let mut xinuse: u64 = 0;

        // FPU (bit 0) — Bochs xsave.cc
        if (rfbm & 1) != 0 {
            if self.the_i387.cwd != 0x037F || self.the_i387.swd != 0
                || self.the_i387.twd != 0xFFFF
                || self.the_i387.foo != 0
                || self.the_i387.fip != 0
                || self.the_i387.fcs != 0
                || self.the_i387.fdp != 0
                || self.the_i387.fds != 0
            {
                xinuse |= 1;
            } else {
                for i in 0..8 {
                    if self.the_i387.st_space[i].signif != 0 || self.the_i387.st_space[i].sign_exp != 0 {
                        xinuse |= 1;
                        break;
                    }
                }
            }
        }

        // SSE (bit 1) — also set if MXCSR != reset (Bochs xsave.cc)
        if (rfbm & 2) != 0 {
            if self.mxcsr.mxcsr != MXCSR_RESET {
                xinuse |= 2;
            } else {
                let num = if self.long64_mode() { 16 } else { 8 };
                for i in 0..num {
                    // SAFETY: zmm union access; index within register file bounds
                    unsafe {
                        if self.vmm[i].zmm64u(0) != 0 || self.vmm[i].zmm64u(1) != 0 {
                            xinuse |= 2;
                            break;
                        }
                    }
                }
            }
        }

        // YMM (bit 2) — upper 128 bits
        if (rfbm & 4) != 0 {
            let num = if self.long64_mode() { 16 } else { 8 };
            for i in 0..num {
                // SAFETY: zmm union access; index within register file bounds
                unsafe {
                    if self.vmm[i].zmm64u(2) != 0 || self.vmm[i].zmm64u(3) != 0 {
                        xinuse |= 4;
                        break;
                    }
                }
            }
        }

        // OPMASK (bit 5)
        if (rfbm & (1 << 5)) != 0 {
            for i in 0..8 {
                if self.opmask[i].rrx() != 0 {
                    xinuse |= 1 << 5;
                    break;
                }
            }
        }

        // ZMM_HI256 (bit 6) — upper 256 bits of ZMM0-15
        if (rfbm & (1 << 6)) != 0 {
            let num = if self.long64_mode() { 16 } else { 8 };
            for i in 0..num {
                // SAFETY: zmm union access; index within register file bounds
                unsafe {
                    if self.vmm[i].zmm64u(4) != 0 || self.vmm[i].zmm64u(5) != 0
                        || self.vmm[i].zmm64u(6) != 0 || self.vmm[i].zmm64u(7) != 0
                    {
                        xinuse |= 1 << 6;
                        break;
                    }
                }
            }
        }

        // HI_ZMM (bit 7) — ZMM16-31 (64-bit mode only)
        if (rfbm & (1 << 7)) != 0 && self.long64_mode() {
            for i in 16..32 {
                // SAFETY: zmm union access; index within register file bounds
                unsafe {
                    if self.vmm[i].zmm64u(0) != 0 || self.vmm[i].zmm64u(1) != 0
                        || self.vmm[i].zmm64u(2) != 0 || self.vmm[i].zmm64u(3) != 0
                        || self.vmm[i].zmm64u(4) != 0 || self.vmm[i].zmm64u(5) != 0
                        || self.vmm[i].zmm64u(6) != 0 || self.vmm[i].zmm64u(7) != 0
                    {
                        xinuse |= 1 << 7;
                        break;
                    }
                }
            }
        }

        xinuse
    }

    // ========================================================================
    // XSAVEOPT — Optimized Save (opcode 0F AE /6)
    // Bochs xsave.cc (shared with XSAVE, xsaveopt flag)
    // Same as XSAVE but only saves components that are in-use
    // ========================================================================

    pub(super) fn xsaveopt(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;

        if !self.cr4.osxsave() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if self.cr0.ts() {
            return self.exception(super::cpu::Exception::Nm, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());

        let laddr: u64 = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.get_laddr32(seg as usize, eaddr as u32) as u64
        };
        if (laddr & 0x3F) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let xcr0 = self.xcr0.get32() as u64;
        let requested = xcr0 & self.eax() as u64;
        let xinuse = self.get_xinuse_vector(requested);

        // Read existing xstate_bv
        let mut xstate_bv = self.v_read_qword(seg, eaddr.wrapping_add(512))?;

        // FPU (bit 0): only save if in-use (XSAVEOPT optimization)
        if (requested & 1) != 0 {
            if (xinuse & 1) != 0 {
                self.xsave_x87_state(seg, eaddr, instr.os64_l() != 0)?;
                xstate_bv |= 1;
            } else {
                xstate_bv &= !1;
            }
        }

        // MXCSR: always written when SSE or YMM requested (Bochs xsave.cc)
        // NOT gated on xinuse — matches standard XSAVE behavior
        if (requested & 0x6) != 0 {
            self.v_write_dword(seg, eaddr.wrapping_add(24), self.mxcsr.mxcsr)?;
            self.v_write_dword(seg, eaddr.wrapping_add(28), self.mxcsr_mask)?;
        }

        // SSE (bit 1)
        if (requested & 2) != 0 {
            if (xinuse & 2) != 0 {
                self.xsave_sse_state(seg, eaddr.wrapping_add(160))?;
                xstate_bv |= 2;
            } else {
                xstate_bv &= !2;
            }
        }

        // Extended features at standard offsets
        for feature in 2..=7u32 {
            let mask = 1u64 << feature;
            if (requested & mask) != 0 {
                if (xinuse & mask) != 0 {
                    let offset = Self::xsave_component_offset(feature);
                    self.xsave_extended_component(seg, eaddr.wrapping_add(offset), feature)?;
                    xstate_bv |= mask;
                } else {
                    xstate_bv &= !mask;
                }
            }
        }

        // Write XSAVE header: xstate_bv at offset 512
        // XSAVEOPT must NOT modify bytes 519:63 (xcomp_bv and reserved fields)
        self.v_write_qword(seg, eaddr.wrapping_add(512), xstate_bv)?;

        Ok(())
    }

    // ========================================================================
    // XSAVEC — Compacted Save (opcode 0F C7 /4)
    // XSAVES — Compacted Save with Supervisor state (opcode 0F C7 /5)
    // Bochs xsave.cc
    // ========================================================================

    pub(super) fn xsavec(&mut self, instr: &super::decoder::Instruction, is_xsaves: bool) -> super::Result<()> {
        use super::decoder::BxSegregs;

        if !self.cr4.osxsave() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if self.cr0.ts() {
            return self.exception(super::cpu::Exception::Nm, 0);
        }

        // XSAVES requires CPL=0
        if is_xsaves {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());

        let laddr: u64 = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.get_laddr32(seg as usize, eaddr as u32) as u64
        };
        if (laddr & 0x3F) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Feature mask: XCR0 for XSAVEC, XCR0|XSS for XSAVES
        let mut xcr0 = self.xcr0.get32() as u64;
        if is_xsaves {
            xcr0 |= self.msr.ia32_xss;
        }

        let requested = xcr0 & self.eax() as u64;
        let xinuse = self.get_xinuse_vector(requested);
        let xstate_bv = requested & xinuse;
        let xcomp_bv = requested | (1u64 << 63); // XSAVEC_COMPACTION_ENABLED

        // FPU (bit 0) at standard offset
        if (requested & 1) != 0 && (xinuse & 1) != 0 {
            self.xsave_x87_state(seg, eaddr, instr.os64_l() != 0)?;
        }

        // MXCSR if SSE or YMM in xstate_bv
        if (xstate_bv & 0x6) != 0 {
            self.v_write_dword(seg, eaddr.wrapping_add(24), self.mxcsr.mxcsr)?;
            self.v_write_dword(seg, eaddr.wrapping_add(28), self.mxcsr_mask)?;
        }

        // SSE (bit 1) at standard offset 160
        if (requested & 2) != 0 && (xinuse & 2) != 0 {
            self.xsave_sse_state(seg, eaddr.wrapping_add(160))?;
        }

        // Extended features in compacted format starting at offset 576
        // Bochs xsave.cc — offset advances for every requested feature
        let mut offset: u64 = 576; // XSAVE_YMM_STATE_OFFSET
        for feature in 2..=7u32 {
            let mask = 1u64 << feature;
            if (requested & mask) != 0 {
                if (xinuse & mask) != 0 {
                    self.xsave_extended_component(seg, eaddr.wrapping_add(offset), feature)?;
                }
                offset += Self::xsave_component_len(feature);
            }
        }

        // Write XSAVE header: xstate_bv + xcomp_bv
        self.v_write_qword(seg, eaddr.wrapping_add(512), xstate_bv)?;
        self.v_write_qword(seg, eaddr.wrapping_add(520), xcomp_bv)?;
        // Clear reserved header fields (offsets 528-575)
        for i in (528u64..576).step_by(8) {
            self.v_write_qword(seg, eaddr.wrapping_add(i), 0)?;
        }

        Ok(())
    }

    // ========================================================================
    // XRSTOR unified (standard + compacted) / XRSTORS
    // Bochs xsave.cc
    // Replaces the basic xrstor() — handles both standard and compacted format
    // XRSTORS is the same but requires CPL=0 and compaction, adds XSS
    // ========================================================================

    pub(super) fn xrstor_unified(&mut self, instr: &super::decoder::Instruction, is_xrstors: bool) -> super::Result<()> {
        use super::decoder::BxSegregs;

        if !self.cr4.osxsave() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if self.cr0.ts() {
            return self.exception(super::cpu::Exception::Nm, 0);
        }

        // XRSTORS requires CPL=0
        if is_xrstors {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());

        let laddr: u64 = if self.long64_mode() {
            self.get_laddr64(seg as usize, eaddr)
        } else {
            self.get_laddr32(seg as usize, eaddr as u32) as u64
        };
        if (laddr & 0x3F) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Read XSAVE header
        let xstate_bv = self.v_read_qword(seg, eaddr.wrapping_add(512))?;
        let xcomp_bv = self.v_read_qword(seg, eaddr.wrapping_add(520))?;
        let header3 = self.v_read_qword(seg, eaddr.wrapping_add(528))?;

        // Reserved header field must be zero
        if header3 != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let compaction = (xcomp_bv >> 63) & 1 != 0;

        // Feature mask: XCR0 for XRSTOR, XCR0|XSS for XRSTORS
        let mut xcr0 = self.xcr0.get32() as u64;
        if is_xrstors {
            xcr0 |= self.msr.ia32_xss;
        }

        if compaction {
            // Compacted format validation
            let xcomp_features = xcomp_bv & !(1u64 << 63);
            // xcomp_bv features must be subset of xcr0
            if (xcomp_features & !xcr0) != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            // xstate_bv must be subset of xcomp_bv
            if (xstate_bv & !xcomp_features) != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            // Header words 4-8 (offsets 536-575) must be zero
            for i in (536u64..576).step_by(8) {
                let val = self.v_read_qword(seg, eaddr.wrapping_add(i))?;
                if val != 0 {
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
            }
        } else {
            // Standard format: xcomp_bv must be 0
            if xcomp_bv != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            // XRSTORS requires compaction
            if is_xrstors {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            // xstate_bv must be subset of xcr0
            if (xstate_bv & !xcr0) != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        let requested = xcr0 & self.eax() as u64;

        // For compacted format, 'format' = features present in compacted area
        // For standard, 'format' = all possible features (fixed offsets)
        let format = if compaction {
            xcomp_bv & !(1u64 << 63)
        } else {
            !(1u64 << 63) // Bochs: ~XSAVEC_COMPACTION_ENABLED
        };
        let restore_mask = xstate_bv & format;

        // --- FPU (bit 0) ---
        if (requested & 1) != 0 {
            if (restore_mask & 1) != 0 {
                self.xrstor_x87_state(seg, eaddr, instr.os64_l() != 0)?;
            } else {
                self.xrstor_init_x87_state();
            }
        }

        // --- MXCSR ---
        if compaction {
            // Compacted: load MXCSR only if SSE is requested AND in restore_mask
            if (requested & 2) != 0 && (restore_mask & 2) != 0 {
                let new_mxcsr = self.v_read_dword(seg, eaddr.wrapping_add(24))?;
                if (new_mxcsr & !self.mxcsr_mask) != 0 {
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                self.mxcsr.mxcsr = new_mxcsr;
            }
        } else {
            // Standard: load MXCSR when SSE or YMM is requested
            if (requested & 0x6) != 0 {
                let new_mxcsr = self.v_read_dword(seg, eaddr.wrapping_add(24))?;
                if (new_mxcsr & !self.mxcsr_mask) != 0 {
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                self.mxcsr.mxcsr = new_mxcsr;
            }
        }

        // --- SSE (bit 1) at standard offset 160 ---
        if (requested & 2) != 0 {
            if (restore_mask & 2) != 0 {
                self.xrstor_sse_state(seg, eaddr.wrapping_add(160))?;
            } else {
                self.xrstor_init_sse_state();
            }
        }

        // --- Extended features (YMM and beyond) ---
        if compaction {
            // Compacted format: offset starts at 576, advances per component in xcomp_bv
            let mut offset: u64 = 576;
            for feature in 2..=7u32 {
                let mask = 1u64 << feature;
                if (requested & mask) != 0 {
                    if (restore_mask & mask) != 0 {
                        self.xrstor_extended_component(seg, eaddr.wrapping_add(offset), feature)?;
                    } else {
                        self.xrstor_init_extended_component(feature);
                    }

                    // Offset advances inside the requested block (Bochs xsave.cc)
                    if (format & mask) != 0 {
                        offset += Self::xsave_component_len(feature);
                    }
                }
            }
        } else {
            // Standard format: each feature at its fixed offset
            for feature in 2..=7u32 {
                let mask = 1u64 << feature;
                if (requested & mask) != 0 {
                    if (xstate_bv & mask) != 0 {
                        let comp_offset = Self::xsave_component_offset(feature);
                        self.xrstor_extended_component(seg, eaddr.wrapping_add(comp_offset), feature)?;
                    } else {
                        self.xrstor_init_extended_component(feature);
                    }
                }
            }
        }

        Ok(())
    }
}
