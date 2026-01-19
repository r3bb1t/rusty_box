//! Task switching implementation
//!
//! Based on Bochs cpu/tasking.cc
//! Copyright (C) 2001-2014 The Bochs Project

use super::{
    cpu::Exception,
    cpuid::BxCpuIdTrait,
    decoder::BxSegregs,
    descriptor::{BxDescriptor, BxSelector},
    segment_ctrl_pro::parse_selector,
    Result,
};

// Task switch source constants (matches Bochs)
const BX_TASK_FROM_JUMP: u32 = 0x0;
const BX_TASK_FROM_CALL: u32 = 0x1;
const BX_TASK_FROM_INT: u32 = 0x2;
const BX_TASK_FROM_IRET: u32 = 0x3;

impl<I: BxCpuIdTrait> super::cpu::BxCpuC<'_, I> {
    /// Perform task switch
    /// Based on BX_CPU_C::task_switch in tasking.cc:113
    pub(super) fn task_switch(
        &mut self,
        tss_selector: &BxSelector,
        tss_descriptor: &BxDescriptor,
        source: u32, // BX_TASK_FROM_*
        _dword1: u32,
        dword2: u32,
        _push_error: bool,
        _error_code: u32,
    ) -> Result<()> {
        tracing::debug!("task_switch(): ENTER, source={}", source);

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        // STEP 2: The processor performs limit-checking on the target TSS
        // Gather info about new TSS (matches lines 158-164)
        let new_tss_max = if tss_descriptor.r#type <= 3 {
            0x2B // 286 TSS
        } else {
            0x67 // 386 TSS
        };

        let nbase32 = unsafe { tss_descriptor.u.segment.base } as u32;
        let new_tss_limit = unsafe { tss_descriptor.u.segment.limit_scaled };

        if new_tss_limit < new_tss_max {
            tracing::error!("task_switch(): new TSS limit ({}) < {}", new_tss_limit, new_tss_max);
            return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
        }

        // Gather info about old TSS (matches lines 196-210)
        let old_tss_max = if self.tr.cache.r#type <= 3 {
            0x29
        } else {
            0x5F
        };

        let obase32 = unsafe { self.tr.cache.u.segment.base } as u32;
        let old_tss_limit = unsafe { self.tr.cache.u.segment.limit_scaled };

        if old_tss_limit < old_tss_max {
            tracing::error!("task_switch(): old TSS limit ({}) < {}", old_tss_limit, old_tss_max);
            return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
        }

        // Step 3: If JMP or IRET, clear busy bit in old task TSS descriptor (matches lines 243-249)
        if source == BX_TASK_FROM_JUMP || source == BX_TASK_FROM_IRET {
            let laddr = (self.gdtr.base + (self.tr.selector.index as u64 * 8) + 4) as u32;
            let mut temp32 = self.system_read_dword(laddr as u64)?;
            temp32 &= !0x200; // Clear busy bit
            self.system_write_dword(laddr as u64, temp32)?;
        }

        // STEP 5: Save the current task state in the TSS (matches lines 269-332)
        // For now, use simplified register access - full implementation would need proper register getters
        if self.tr.cache.r#type <= 3 {
            // 286 TSS - save 16-bit registers
            // TODO: Implement proper register access methods
            tracing::warn!("task_switch(): 286 TSS save not yet fully implemented");
        } else {
            // 386 TSS - save 32-bit registers
            // TODO: Implement proper register access methods
            tracing::warn!("task_switch(): 386 TSS save not yet fully implemented");
        }

        // effect on link field of new task (matches lines 334-339)
        if source == BX_TASK_FROM_CALL || source == BX_TASK_FROM_INT {
            // set to selector of old task's TSS
            // Use system_write_word (which writes dword with low 16 bits)
            self.system_write_word(nbase32 as u64, self.tr.selector.value)?;
        }

        // STEP 6: The new-task state is loaded from the TSS (matches lines 341-411)
        let (new_eip, new_eflags, new_eax, new_ecx, new_edx, new_ebx, new_esp, new_ebp, new_esi, new_edi,
             raw_es_selector, raw_cs_selector, raw_ss_selector, raw_ds_selector, raw_fs_selector, raw_gs_selector, raw_ldt_selector) =
            if tss_descriptor.r#type <= 3 {
                // 286 TSS
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
                (new_eip, new_eflags, new_eax, new_ecx, new_edx, new_ebx, new_esp, new_ebp, new_esi, new_edi,
                 raw_es, raw_cs, raw_ss, raw_ds, 0, 0, raw_ldt)
            } else {
                // 386 TSS
                let new_cr3 = if self.cr0.pg() {
                    self.system_read_dword((nbase32 + 0x1c) as u64)?
                } else {
                    0
                };
                // TODO: Set CR3 if changed
                if new_cr3 != 0 && self.cr0.pg() {
                    // For now, just log - full CR3 handling would require paging updates
                    tracing::debug!("task_switch(): CR3 change to {:#x} (not yet fully implemented)", new_cr3);
                }

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
                (new_eip, new_eflags, new_eax, new_ecx, new_edx, new_ebx, new_esp, new_ebp, new_esi, new_edi,
                 raw_es, raw_cs, raw_ss, raw_ds, raw_fs, raw_gs, raw_ldt)
            };

        // Step 7: If CALL, interrupt, or JMP, set busy flag in new task's TSS descriptor (matches lines 416-423)
        if source != BX_TASK_FROM_IRET {
            let laddr = (self.gdtr.base + (tss_selector.index as u64 * 8) + 4) as u32;
            let mut new_dword2 = dword2;
            new_dword2 |= 0x200; // Set busy bit
            self.system_write_dword(laddr as u64, new_dword2)?;
        }

        // Step 8: Load the task register with the segment selector and descriptor for the new task TSS (matches lines 464-469)
        self.tr.selector = tss_selector.clone();
        self.tr.cache = tss_descriptor.clone();
        self.tr.cache.r#type |= 2; // mark TSS in TR as busy

        // Step 9: Set TS flag in CR0 (matches line 472)
        // TODO: Set CR0.TS flag when CR0 access is implemented

        // Step 10: If call or interrupt, set the NT flag in the eflags (matches lines 481-484)
        let mut final_eflags = new_eflags;
        if source == BX_TASK_FROM_CALL || source == BX_TASK_FROM_INT {
            final_eflags |= 1 << 14; // Set NT flag
        }

        // Step 11: Load the new task (dynamic) state from new TSS (matches lines 486-503)
        self.set_eip(new_eip);
        // TODO: Implement proper register setters
        // For now, just set EIP and EFLAGS - register loading would need proper access methods
        self.eflags = final_eflags;
        tracing::debug!("task_switch(): Loaded new EIP={:#x}, EFLAGS={:#x}", new_eip, final_eflags);

        // Fill in selectors for all segment registers (matches lines 508-523)
        let mut cs_selector = BxSelector::default();
        parse_selector(raw_cs_selector, &mut cs_selector);
        self.sregs[BxSegregs::Cs as usize].selector = cs_selector;
        
        let mut ss_selector = BxSelector::default();
        parse_selector(raw_ss_selector, &mut ss_selector);
        self.sregs[BxSegregs::Ss as usize].selector = ss_selector;
        
        let mut ds_selector = BxSelector::default();
        parse_selector(raw_ds_selector, &mut ds_selector);
        self.sregs[BxSegregs::Ds as usize].selector = ds_selector;
        
        let mut es_selector = BxSelector::default();
        parse_selector(raw_es_selector, &mut es_selector);
        self.sregs[BxSegregs::Es as usize].selector = es_selector;
        
        if raw_fs_selector != 0 {
            let mut fs_selector = BxSelector::default();
            parse_selector(raw_fs_selector, &mut fs_selector);
            self.sregs[BxSegregs::Fs as usize].selector = fs_selector;
        }
        
        if raw_gs_selector != 0 {
            let mut gs_selector = BxSelector::default();
            parse_selector(raw_gs_selector, &mut gs_selector);
            self.sregs[BxSegregs::Gs as usize].selector = gs_selector;
        }
        
        if raw_ldt_selector != 0 {
            let mut ldt_selector = BxSelector::default();
            parse_selector(raw_ldt_selector, &mut ldt_selector);
            self.ldtr.selector = ldt_selector;
        }

        // Start out with invalid descriptor caches (matches lines 525-533)
        self.ldtr.cache.valid = 0;
        self.sregs[BxSegregs::Cs as usize].cache.valid = 0;
        self.sregs[BxSegregs::Ss as usize].cache.valid = 0;
        self.sregs[BxSegregs::Ds as usize].cache.valid = 0;
        self.sregs[BxSegregs::Es as usize].cache.valid = 0;
        self.sregs[BxSegregs::Fs as usize].cache.valid = 0;
        self.sregs[BxSegregs::Gs as usize].cache.valid = 0;

        // TODO: Load and validate segment descriptors (lines 574-944 in original)
        // This is a simplified version - full implementation would validate all segments
        tracing::debug!("task_switch(): Task switch completed (simplified implementation)");

        Ok(())
    }
}
