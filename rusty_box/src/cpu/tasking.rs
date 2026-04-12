#![allow(unused_variables)]
//! Task switching implementation
//!
//! Based on Bochs cpu/tasking.cc

use super::{
    cpu::Exception,
    cpuid::BxCpuIdTrait,
    decoder::BxSegregs,
    descriptor::{
        is_code_segment, is_code_segment_non_conforming, is_code_segment_readable, is_data_segment,
        is_data_segment_writable, BxDescriptor, BxSelector,
    },
    segment_ctrl_pro::parse_selector,
    Result,
};

// Task switch source constants (matches Bochs)
pub(super) const BX_TASK_FROM_JUMP: u32 = 0x0;
pub(super) const BX_TASK_FROM_CALL: u32 = 0x1;
pub(super) const BX_TASK_FROM_INT: u32 = 0x2;
pub(super) const BX_TASK_FROM_IRET: u32 = 0x3;

impl<I: BxCpuIdTrait> super::cpu::BxCpuC<'_, I> {
    /// Perform task switch
    /// Based on BX_CPU_C::task_switch in 
    #[allow(clippy::too_many_arguments)]
    pub(super) fn task_switch(
        &mut self,
        tss_selector: &BxSelector,
        tss_descriptor: &BxDescriptor,
        source: u32, // BX_TASK_FROM_*
        _dword1: u32,
        dword2: u32,
        push_error: bool,
        error_code: u32,
    ) -> Result<()> {
        tracing::debug!("task_switch(): ENTER, source={}", source);

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // Discard any traps and inhibits for new context; traps will
        // resume upon return. (Bochs )
        self.debug_trap &= !Self::BX_DEBUG_SINGLE_STEP_BIT;
        self.inhibit_mask = 0;

        // STEP 2: The processor performs limit-checking on the target TSS
        // Gather info about new TSS (matches lines 158-164)
        let new_tss_max = if tss_descriptor.r#type <= 3 {
            0x2B // 286 TSS
        } else {
            0x67 // 386 TSS
        };

        let nbase32 = tss_descriptor.u.segment_base() as u32;
        let new_tss_limit = tss_descriptor.u.segment_limit_scaled();

        if new_tss_limit < new_tss_max {
            tracing::error!(
                "task_switch(): new TSS limit ({}) < {}",
                new_tss_limit,
                new_tss_max
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: 0,
            });
        }

        // Gather info about old TSS (matches lines 196-210)
        let old_tss_max = if self.tr.cache.r#type <= 3 {
            0x29
        } else {
            0x5F
        };

        let obase32 = self.tr.cache.u.segment_base() as u32;
        let old_tss_limit = self.tr.cache.u.segment_limit_scaled();

        if old_tss_limit < old_tss_max {
            tracing::error!(
                "task_switch(): old TSS limit ({}) < {}",
                old_tss_limit,
                old_tss_max
            );
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: 0,
            });
        }

        // Capture old EFLAGS before modification (Bochs )
        let mut old_eflags = self.eflags.bits();

        // If moving to busy task, clear NT bit (Bochs )
        if tss_descriptor.r#type == 0x3 || tss_descriptor.r#type == 0xB {
            old_eflags &= !super::eflags::EFlags::NT.bits(); // Clear NT
        }

        // Step 3: If JMP or IRET, clear busy bit in old task TSS descriptor (matches lines 243-249)
        if source == BX_TASK_FROM_JUMP || source == BX_TASK_FROM_IRET {
            let laddr = (self.gdtr.base + (self.tr.selector.index as u64 * 8) + 4) as u32;
            let mut temp32 = self.system_read_dword(laddr as u64)?;
            temp32 &= !0x200; // Clear busy bit
            self.system_write_dword(laddr as u64, temp32)?;
        }

        // STEP 5: Save the current task state in the TSS (matches lines 269-332)
        if self.tr.cache.r#type <= 3 {
            // 286 TSS - save 16-bit registers
            self.system_write_word((obase32 + 14) as u64, self.get_ip())?;
            self.system_write_word((obase32 + 16) as u64, old_eflags as u16)?;
            self.system_write_word((obase32 + 18) as u64, self.ax())?;
            self.system_write_word((obase32 + 20) as u64, self.cx())?;
            self.system_write_word((obase32 + 22) as u64, self.dx())?;
            self.system_write_word((obase32 + 24) as u64, self.bx())?;
            self.system_write_word((obase32 + 26) as u64, self.sp())?;
            self.system_write_word((obase32 + 28) as u64, self.bp())?;
            self.system_write_word((obase32 + 30) as u64, self.si())?;
            self.system_write_word((obase32 + 32) as u64, self.di())?;
            self.system_write_word(
                (obase32 + 34) as u64,
                self.sregs[BxSegregs::Es as usize].selector.value,
            )?;
            self.system_write_word(
                (obase32 + 36) as u64,
                self.sregs[BxSegregs::Cs as usize].selector.value,
            )?;
            self.system_write_word(
                (obase32 + 38) as u64,
                self.sregs[BxSegregs::Ss as usize].selector.value,
            )?;
            self.system_write_word(
                (obase32 + 40) as u64,
                self.sregs[BxSegregs::Ds as usize].selector.value,
            )?;
        } else {
            // 386 TSS - save 32-bit registers
            self.system_write_dword((obase32 as u64) + 0x20, self.eip())?;
            self.system_write_dword((obase32 as u64) + 0x24, old_eflags)?;
            self.system_write_dword((obase32 as u64) + 0x28, self.eax())?;
            self.system_write_dword((obase32 as u64) + 0x2c, self.ecx())?;
            self.system_write_dword((obase32 as u64) + 0x30, self.edx())?;
            self.system_write_dword((obase32 as u64) + 0x34, self.ebx())?;
            self.system_write_dword((obase32 as u64) + 0x38, self.esp())?;
            self.system_write_dword((obase32 as u64) + 0x3c, self.ebp())?;
            self.system_write_dword((obase32 as u64) + 0x40, self.esi())?;
            self.system_write_dword((obase32 as u64) + 0x44, self.edi())?;
            self.system_write_word(
                (obase32 as u64) + 0x48,
                self.sregs[BxSegregs::Es as usize].selector.value,
            )?;
            self.system_write_word(
                (obase32 as u64) + 0x4c,
                self.sregs[BxSegregs::Cs as usize].selector.value,
            )?;
            self.system_write_word(
                (obase32 as u64) + 0x50,
                self.sregs[BxSegregs::Ss as usize].selector.value,
            )?;
            self.system_write_word(
                (obase32 as u64) + 0x54,
                self.sregs[BxSegregs::Ds as usize].selector.value,
            )?;
            self.system_write_word(
                (obase32 as u64) + 0x58,
                self.sregs[BxSegregs::Fs as usize].selector.value,
            )?;
            self.system_write_word(
                (obase32 as u64) + 0x5c,
                self.sregs[BxSegregs::Gs as usize].selector.value,
            )?;
        }

        // Effect on link field of new task (matches lines 334-339)
        if source == BX_TASK_FROM_CALL || source == BX_TASK_FROM_INT {
            // set to selector of old task's TSS
            self.system_write_word(nbase32 as u64, self.tr.selector.value)?;
        }

        // STEP 6: The new-task state is loaded from the TSS (matches lines 341-411)
        // Returns: (new_cr3, trap_word, new_eip, new_eflags, GPRs, segment selectors, ldt)
        let (
            new_cr3,
            trap_word,
            new_eip,
            new_eflags,
            new_eax,
            new_ecx,
            new_edx,
            new_ebx,
            new_esp,
            new_ebp,
            new_esi,
            new_edi,
            raw_es_selector,
            raw_cs_selector,
            raw_ss_selector,
            raw_ds_selector,
            raw_fs_selector,
            raw_gs_selector,
            raw_ldt_selector,
        ) = if tss_descriptor.r#type <= 3 {
            // 286 TSS — no CR3, no trap word
            let new_eip = self.system_read_word((nbase32 + 14) as u64)? as u32;
            let new_eflags = (self.system_read_word((nbase32 + 16) as u64)? as u32) & 0xFFFF;
            let new_eax = 0xFFFF0000 | (self.system_read_word((nbase32 + 18) as u64)? as u32);
            let new_ecx = 0xFFFF0000 | (self.system_read_word((nbase32 + 20) as u64)? as u32);
            let new_edx = 0xFFFF0000 | (self.system_read_word((nbase32 + 22) as u64)? as u32);
            let new_ebx = 0xFFFF0000 | (self.system_read_word((nbase32 + 24) as u64)? as u32);
            let new_esp = 0xFFFF0000 | (self.system_read_word((nbase32 + 26) as u64)? as u32);
            let new_ebp = 0xFFFF0000 | (self.system_read_word((nbase32 + 28) as u64)? as u32);
            let new_esi = 0xFFFF0000 | (self.system_read_word((nbase32 + 30) as u64)? as u32);
            let new_edi = 0xFFFF0000 | (self.system_read_word((nbase32 + 32) as u64)? as u32);
            let raw_es = self.system_read_word((nbase32 + 34) as u64)?;
            let raw_cs = self.system_read_word((nbase32 + 36) as u64)?;
            let raw_ss = self.system_read_word((nbase32 + 38) as u64)?;
            let raw_ds = self.system_read_word((nbase32 + 40) as u64)?;
            let raw_ldt = self.system_read_word((nbase32 + 42) as u64)?;
            (
                0u32, 0u16, // no CR3, no trap_word for 286 TSS
                new_eip, new_eflags, new_eax, new_ecx, new_edx, new_ebx, new_esp, new_ebp, new_esi,
                new_edi, raw_es, raw_cs, raw_ss, raw_ds, 0u16, 0u16, raw_ldt,
            )
        } else {
            // 386 TSS
            // Read CR3 now (step 6) but apply after commit point (step 9)
            let new_cr3 = if self.cr0.pg() {
                self.system_read_dword((nbase32 + 0x1c) as u64)?
            } else {
                0
            };

            let new_eip = self.system_read_dword((nbase32 + 0x20) as u64)?;
            let new_eflags = self.system_read_dword((nbase32 + 0x24) as u64)?;
            let new_eax = self.system_read_dword((nbase32 + 0x28) as u64)?;
            let new_ecx = self.system_read_dword((nbase32 + 0x2c) as u64)?;
            let new_edx = self.system_read_dword((nbase32 + 0x30) as u64)?;
            let new_ebx = self.system_read_dword((nbase32 + 0x34) as u64)?;
            let new_esp = self.system_read_dword((nbase32 + 0x38) as u64)?;
            let new_ebp = self.system_read_dword((nbase32 + 0x3c) as u64)?;
            let new_esi = self.system_read_dword((nbase32 + 0x40) as u64)?;
            let new_edi = self.system_read_dword((nbase32 + 0x44) as u64)?;
            let raw_es = self.system_read_word((nbase32 + 0x48) as u64)?;
            let raw_cs = self.system_read_word((nbase32 + 0x4c) as u64)?;
            let raw_ss = self.system_read_word((nbase32 + 0x50) as u64)?;
            let raw_ds = self.system_read_word((nbase32 + 0x54) as u64)?;
            let raw_fs = self.system_read_word((nbase32 + 0x58) as u64)?;
            let raw_gs = self.system_read_word((nbase32 + 0x5c) as u64)?;
            let raw_ldt = self.system_read_word((nbase32 + 0x60) as u64)?;
            // Read trap word from TSS offset 0x64 (Bochs )
            let trap_word = self.system_read_word((nbase32 + 0x64) as u64)?;
            (
                new_cr3, trap_word, new_eip, new_eflags, new_eax, new_ecx, new_edx, new_ebx,
                new_esp, new_ebp, new_esi, new_edi, raw_es, raw_cs, raw_ss, raw_ds, raw_fs, raw_gs,
                raw_ldt,
            )
        };

        // Step 7: If CALL, interrupt, or JMP, set busy flag in new task's TSS descriptor
        // Re-read dword2 from GDT for atomicity (Bochs )
        if source != BX_TASK_FROM_IRET {
            let laddr = (self.gdtr.base + (tss_selector.index as u64 * 8) + 4) as u32;
            let mut new_dword2 = self.system_read_dword(laddr as u64)?;
            new_dword2 |= 0x200; // Set busy bit
            self.system_write_dword(laddr as u64, new_dword2)?;
        }

        // Step 8: Load the task register with the segment selector and descriptor for the new task TSS (matches lines 464-469)
        self.tr.selector = tss_selector.clone();
        self.tr.cache = tss_descriptor.clone();
        self.tr.cache.r#type |= 2; // mark TSS in TR as busy

        // Step 9: Set TS flag in CR0 (matches line 472)
        self.cr0.set32(self.cr0.get32() | (1 << 3));

        // Task switch clears LE/L3/L2/L1/L0 in DR7 (Bochs )
        self.dr7
            .set32(self.dr7.get32() & !Self::DR7_LOCAL_ENABLE_MASK);

        // CR3 change — after commit point (Bochs )
        if tss_descriptor.r#type >= 9 && self.cr0.pg()
            && new_cr3 != 0 && (new_cr3 as u64) != self.cr3 {
                tracing::debug!("task_switch(): changing CR3 to {:#x}", new_cr3);
                self.cr3 = new_cr3 as u64;
                if self.cr4.pge() {
                    self.tlb_flush_non_global();
                } else {
                    self.tlb_flush();
                }
            }

        // Step 10: If call or interrupt, set the NT flag in the eflags (matches lines 481-484)
        let mut final_eflags = new_eflags;
        if source == BX_TASK_FROM_CALL || source == BX_TASK_FROM_INT {
            final_eflags |= super::eflags::EFlags::NT.bits(); // Set NT flag
        }

        // Step 11: Load the new task (dynamic) state from new TSS (matches lines 486-503)
        self.prev_rip = new_eip as u64; // Bochs 
        self.set_eip(new_eip);
        // Use write_eflags for proper side effects (TF, IF, VM, AC) — Bochs 
        self.write_eflags(final_eflags, super::eflags::EFlags::VALID_MASK.bits());
        // Load all GPRs from new TSS
        self.set_gpr32(0, new_eax); // EAX
        self.set_gpr32(1, new_ecx); // ECX
        self.set_gpr32(2, new_edx); // EDX
        self.set_gpr32(3, new_ebx); // EBX
        self.set_gpr32(4, new_esp); // ESP
        self.set_gpr32(5, new_ebp); // EBP
        self.set_gpr32(6, new_esi); // ESI
        self.set_gpr32(7, new_edi); // EDI
        tracing::debug!(
            "task_switch(): Loaded new EIP={:#x}, EFLAGS={:#x}, EAX={:#x}, ECX={:#x}",
            new_eip,
            final_eflags,
            new_eax,
            new_ecx
        );

        // Fill in selectors for all segment registers (matches lines 508-523)
        let mut cs_selector = BxSelector::default();
        parse_selector(raw_cs_selector, &mut cs_selector);
        self.sregs[BxSegregs::Cs as usize].selector = cs_selector.clone();

        let mut ss_selector = BxSelector::default();
        parse_selector(raw_ss_selector, &mut ss_selector);
        self.sregs[BxSegregs::Ss as usize].selector = ss_selector.clone();

        let mut ds_selector = BxSelector::default();
        parse_selector(raw_ds_selector, &mut ds_selector);
        self.sregs[BxSegregs::Ds as usize].selector = ds_selector.clone();

        let mut es_selector = BxSelector::default();
        parse_selector(raw_es_selector, &mut es_selector);
        self.sregs[BxSegregs::Es as usize].selector = es_selector.clone();

        let mut fs_selector = BxSelector::default();
        parse_selector(raw_fs_selector, &mut fs_selector);
        self.sregs[BxSegregs::Fs as usize].selector = fs_selector.clone();

        let mut gs_selector = BxSelector::default();
        parse_selector(raw_gs_selector, &mut gs_selector);
        self.sregs[BxSegregs::Gs as usize].selector = gs_selector.clone();

        let mut ldt_selector = BxSelector::default();
        parse_selector(raw_ldt_selector, &mut ldt_selector);
        self.ldtr.selector = ldt_selector.clone();

        // Start out with invalid descriptor caches (matches lines 525-533)
        self.ldtr.cache.valid = 0;
        self.sregs[BxSegregs::Cs as usize].cache.valid = 0;
        self.sregs[BxSegregs::Ss as usize].cache.valid = 0;
        self.sregs[BxSegregs::Ds as usize].cache.valid = 0;
        self.sregs[BxSegregs::Es as usize].cache.valid = 0;
        self.sregs[BxSegregs::Fs as usize].cache.valid = 0;
        self.sregs[BxSegregs::Gs as usize].cache.valid = 0;

        // ─── Segment descriptor validation (Bochs ) ───

        // Temporarily set CPL to 3 so that privilege level change and stack switch
        // happen if SS is not properly loaded (Bochs )
        let save_cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        self.sregs[BxSegregs::Cs as usize].selector.rpl = 3;

        // LDTR validation (Bochs )
        if ldt_selector.ti != 0 {
            tracing::error!("task_switch: bad LDT selector TI=1");
            return self.exception(Exception::Ts, raw_ldt_selector & 0xfffc);
        }
        if (raw_ldt_selector & 0xfffc) != 0 {
            let good = self.fetch_raw_descriptor(&ldt_selector);
            match good {
                Ok((dword1, dword2)) => {
                    let ldt_descriptor = self.parse_descriptor(dword1, dword2)?;
                    if ldt_descriptor.valid == 0
                        || ldt_descriptor.r#type != 0x2  // BX_SYS_SEGMENT_LDT
                        || ldt_descriptor.segment
                    {
                        tracing::error!("task_switch: bad LDT segment");
                        return self.exception(Exception::Ts, raw_ldt_selector & 0xfffc);
                    }
                    if !ldt_descriptor.p {
                        tracing::error!("task_switch: LDT not present");
                        return self.exception(Exception::Ts, raw_ldt_selector & 0xfffc);
                    }
                    self.ldtr.cache = ldt_descriptor;
                }
                Err(_) => {
                    tracing::error!("task_switch: bad LDT fetch");
                    return self.exception(Exception::Ts, raw_ldt_selector & 0xfffc);
                }
            }
        }
        // else: NULL LDT selector is OK, leave cache invalid

        // Check if V8086 mode (Bochs )
        if (final_eflags & super::eflags::EFlags::VM.bits()) != 0 {
            // V8086 mode — load seg regs as real-mode
            self.load_seg_reg_real_mode(BxSegregs::Ss, raw_ss_selector);
            self.load_seg_reg_real_mode(BxSegregs::Ds, raw_ds_selector);
            self.load_seg_reg_real_mode(BxSegregs::Es, raw_es_selector);
            self.load_seg_reg_real_mode(BxSegregs::Fs, raw_fs_selector);
            self.load_seg_reg_real_mode(BxSegregs::Gs, raw_gs_selector);
            self.load_seg_reg_real_mode(BxSegregs::Cs, raw_cs_selector);
        } else {
            // Protected mode segment validation

            // SS validation (Bochs )
            if (raw_ss_selector & 0xfffc) != 0 {
                match self.fetch_raw_descriptor(&ss_selector) {
                    Ok((dword1, dword2)) => {
                        let mut ss_descriptor = self.parse_descriptor(dword1, dword2)?;
                        if ss_descriptor.valid == 0
                            || !ss_descriptor.segment
                            || is_code_segment(ss_descriptor.r#type)
                            || !is_data_segment_writable(ss_descriptor.r#type)
                        {
                            tracing::error!("task_switch: SS not valid or writable segment");
                            return self.exception(Exception::Ts, raw_ss_selector & 0xfffc);
                        }
                        if !ss_descriptor.p {
                            tracing::error!("task_switch: SS not present");
                            return self.exception(Exception::Ss, raw_ss_selector & 0xfffc);
                        }
                        if ss_descriptor.dpl != cs_selector.rpl {
                            tracing::error!("task_switch: SS.dpl != CS.RPL");
                            return self.exception(Exception::Ts, raw_ss_selector & 0xfffc);
                        }
                        if ss_selector.rpl != ss_descriptor.dpl {
                            tracing::error!("task_switch: SS.rpl != SS.dpl");
                            return self.exception(Exception::Ts, raw_ss_selector & 0xfffc);
                        }
                        self.touch_segment(&ss_selector, &mut ss_descriptor)?;
                        self.sregs[BxSegregs::Ss as usize].cache = ss_descriptor;
                        self.invalidate_stack_cache();
                    }
                    Err(_) => {
                        tracing::error!("task_switch: bad SS fetch");
                        return self.exception(Exception::Ts, raw_ss_selector & 0xfffc);
                    }
                }
            } else {
                tracing::error!("task_switch: SS NULL");
                return self.exception(Exception::Ts, raw_ss_selector & 0xfffc);
            }

            // Restore CPL (Bochs )
            self.sregs[BxSegregs::Cs as usize].selector.rpl = save_cpl;

            // DS/ES/FS/GS validation via task_switch_load_selector (Bochs )
            self.task_switch_load_selector(
                BxSegregs::Ds,
                &ds_selector,
                raw_ds_selector,
                cs_selector.rpl,
            )?;
            self.task_switch_load_selector(
                BxSegregs::Es,
                &es_selector,
                raw_es_selector,
                cs_selector.rpl,
            )?;
            self.task_switch_load_selector(
                BxSegregs::Fs,
                &fs_selector,
                raw_fs_selector,
                cs_selector.rpl,
            )?;
            self.task_switch_load_selector(
                BxSegregs::Gs,
                &gs_selector,
                raw_gs_selector,
                cs_selector.rpl,
            )?;

            // CS validation (Bochs )
            if (raw_cs_selector & 0xfffc) != 0 {
                match self.fetch_raw_descriptor(&cs_selector) {
                    Ok((dword1, dword2)) => {
                        let mut cs_descriptor = self.parse_descriptor(dword1, dword2)?;
                        if cs_descriptor.valid == 0
                            || !cs_descriptor.segment
                            || is_data_segment(cs_descriptor.r#type)
                        {
                            tracing::error!("task_switch: CS not valid executable seg");
                            return self.exception(Exception::Ts, raw_cs_selector & 0xfffc);
                        }
                        if is_code_segment_non_conforming(cs_descriptor.r#type)
                            && cs_descriptor.dpl != cs_selector.rpl
                        {
                            tracing::error!("task_switch: non-conforming CS.dpl!=CS.RPL");
                            return self.exception(Exception::Ts, raw_cs_selector & 0xfffc);
                        }
                        if !is_code_segment_non_conforming(cs_descriptor.r#type)
                            && cs_descriptor.dpl > cs_selector.rpl
                        {
                            tracing::error!("task_switch: conforming CS.dpl>RPL");
                            return self.exception(Exception::Ts, raw_cs_selector & 0xfffc);
                        }
                        if !cs_descriptor.p {
                            tracing::error!("task_switch: CS.p==0");
                            return self.exception(Exception::Np, raw_cs_selector & 0xfffc);
                        }
                        self.touch_segment(&cs_selector, &mut cs_descriptor)?;
                        self.sregs[BxSegregs::Cs as usize].cache = cs_descriptor;
                    }
                    Err(_) => {
                        tracing::error!("task_switch: bad CS fetch");
                        return self.exception(Exception::Ts, raw_cs_selector & 0xfffc);
                    }
                }
            } else {
                tracing::error!("task_switch: CS NULL");
                return self.exception(Exception::Ts, raw_cs_selector & 0xfffc);
            }

            // Bochs  — updateFetchModeMask() after CS reload
            self.update_fetch_mode_mask();
            self.invalidate_prefetch_q();
            // Alignment check depends on new CPL (Bochs )
            self.handle_alignment_check();
        }

        // Set speculative RSP before error-code push (Bochs )
        self.speculative_rsp = true;
        self.prev_rsp = self.esp() as u64;

        // Push error code if needed (Bochs )
        if push_error {
            if tss_descriptor.r#type >= 9 {
                // 386 TSS
                self.push_32(error_code)?;
            } else {
                // 286 TSS
                self.push_16(error_code as u16)?;
            }
        }

        // Check TSS T (debug trap) bit — 386 TSS only (Bochs )
        if tss_descriptor.r#type >= 9 && (trap_word & 0x1) != 0 {
            self.debug_trap |= Self::BX_DEBUG_TRAP_TASK_SWITCH_BIT;
            self.async_event = 1;
            tracing::info!("task_switch: T bit set in new TSS");
        }

        // Instruction pointer must be in CS limit, else #GP(0) (Bochs )
        let cs_limit = self.sregs[BxSegregs::Cs as usize]
            .cache
            .u
            .segment_limit_scaled();
        if new_eip > cs_limit {
            tracing::error!(
                "task_switch: EIP ({:#x}) > CS.limit ({:#x})",
                new_eip,
                cs_limit
            );
            self.speculative_rsp = false;
            return self.exception(Exception::Gp, 0);
        }

        // RSP commit (Bochs )
        self.speculative_rsp = false;

        tracing::debug!(
            "task_switch(): completed, new CS={:#06x} EIP={:#010x} SS={:#06x} ESP={:#010x}",
            raw_cs_selector,
            new_eip,
            raw_ss_selector,
            new_esp
        );

        Ok(())
    }

    /// Load a data segment selector during task switch
    /// Based on BX_CPU_C::task_switch_load_selector in 
    fn task_switch_load_selector(
        &mut self,
        seg: BxSegregs,
        selector: &BxSelector,
        raw_selector: u16,
        cs_rpl: u8,
    ) -> Result<()> {
        if (raw_selector & 0xfffc) != 0 {
            match self.fetch_raw_descriptor(selector) {
                Ok((dword1, dword2)) => {
                    let mut descriptor = self.parse_descriptor(dword1, dword2)?;

                    // AR byte must indicate data or readable code segment
                    if !descriptor.segment
                        || (is_code_segment(descriptor.r#type)
                            && !is_code_segment_readable(descriptor.r#type))
                    {
                        tracing::error!("task_switch({:?}): not data or readable code", seg);
                        return self.exception(Exception::Ts, raw_selector & 0xfffc);
                    }

                    // If data or non-conforming code, RPL and CPL must be <= DPL
                    if (is_data_segment(descriptor.r#type)
                        || is_code_segment_non_conforming(descriptor.r#type))
                        && (selector.rpl > descriptor.dpl || cs_rpl > descriptor.dpl) {
                            tracing::error!("task_switch({:?}): RPL & CPL must be <= DPL", seg);
                            return self.exception(Exception::Ts, raw_selector & 0xfffc);
                        }

                    if !descriptor.p {
                        tracing::error!("task_switch({:?}): descriptor not present", seg);
                        return self.exception(Exception::Np, raw_selector & 0xfffc);
                    }

                    self.touch_segment(selector, &mut descriptor)?;
                    self.sregs[seg as usize].cache = descriptor;
                }
                Err(_) => {
                    tracing::error!("task_switch({:?}): bad selector fetch", seg);
                    return self.exception(Exception::Ts, raw_selector & 0xfffc);
                }
            }
        }
        // NULL selector is OK, leave cache invalid
        Ok(())
    }
}
