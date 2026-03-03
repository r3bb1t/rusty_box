use alloc::format;

use super::{
    cpu::Exception,
    decoder::BxSegregs,
    descriptor::{
        BxDescriptor, BxSelector, DescriptorGate, DescriptorSegment, DescriptorTaskGate,
        SystemAndGateDescriptorEnum, SEG_VALID_CACHE,
    },
    Result,
};
use crate::config::BxAddress;

pub fn parse_selector(raw_selector: u16, selector: &mut BxSelector) {
    selector.value = raw_selector;
    selector.index = raw_selector >> 3;
    selector.ti = (raw_selector >> 2) & 0x01;
    // Note: bochs uses implicit cast
    selector.rpl = raw_selector as u8 & 0x03;
}

impl<I: super::cpuid::BxCpuIdTrait> super::cpu::BxCpuC<'_, I> {
    /// Fetch raw descriptor from GDT or LDT
    /// Based on BX_CPU_C::fetch_raw_descriptor in segment_ctrl_pro.cc:536
    pub(super) fn fetch_raw_descriptor(&self, selector: &BxSelector) -> Result<(u32, u32)> {
        let index = selector.index as u32;
        let offset: BxAddress;

        if selector.ti == 0 {
            // GDT
            let index_offset = (index as u32) * 8 + 7;
            if index_offset > self.gdtr.limit as u32 {
                tracing::debug!(
                    "fetch_raw_descriptor: GDT: index ({}) {} > limit ({})",
                    index_offset,
                    index,
                    self.gdtr.limit
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
            }
            offset = self.gdtr.base + (index as u64 * 8);
        } else {
            // LDT
            if self.ldtr.cache.valid == 0 {
                tracing::debug!("fetch_raw_descriptor: LDTR.valid=0");
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
            }
            let ldt_limit = unsafe { self.ldtr.cache.u.segment.limit_scaled };
            let index_offset = (index as u32) * 8 + 7;
            if index_offset > ldt_limit {
                tracing::debug!(
                    "fetch_raw_descriptor: LDT: index ({}) {} > limit ({})",
                    index_offset,
                    index,
                    ldt_limit
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: 0,
                });
            }
            offset = unsafe { self.ldtr.cache.u.segment.base } + (index as u64 * 8);
        }

        // Read descriptor as qword (64 bits = 2 dwords)
        let qword = self.system_read_qword(offset)?;
        let dword1 = (qword & 0xFFFFFFFF) as u32;
        let dword2 = ((qword >> 32) & 0xFFFFFFFF) as u32;

        Ok((dword1, dword2))
    }

    /// Parse descriptor from two dwords
    /// Based on parse_descriptor in segment_ctrl_pro.cc:419
    pub(super) fn parse_descriptor(&self, dword1: u32, dword2: u32) -> Result<BxDescriptor> {
        let ar_byte = (dword2 >> 8) & 0xFF;

        // Extract AR byte fields
        let p = (ar_byte >> 7) != 0;
        let dpl = ((ar_byte >> 5) & 0x03) as u8;
        let segment = (ar_byte >> 4) & 0x01 != 0;
        let r#type = (ar_byte & 0x0F) as u8;

        let mut descriptor = BxDescriptor {
            valid: 0,
            p,
            dpl,
            segment,
            r#type,
            u: super::descriptor::Descriptor {
                segment: DescriptorSegment {
                    base: 0,
                    limit_scaled: 0,
                    g: false,
                    d_b: false,
                    l: false,
                    avl: false,
                },
            },
        };

        if segment {
            // Data/code segment descriptor
            // limit[15:0] from dword1 bits[15:0]; limit[19:16] from dword2 bits[19:16]
            // dword2 & 0x000F0000 already has limit[19:16] in the correct bit positions
            let limit = (dword1 & 0xFFFF) | (dword2 & 0x000F0000);
            let mut base = ((dword1 >> 16) as u64) | (((dword2 & 0xFF) as u64) << 16);
            base |= (dword2 & 0xFF000000) as u64;

            let g = (dword2 & 0x00800000) != 0;
            let d_b = (dword2 & 0x00400000) != 0;
            let avl = (dword2 & 0x00100000) != 0;

            let limit_scaled = if g { (limit << 12) | 0xFFF } else { limit };

            descriptor.u.segment = DescriptorSegment {
                base,
                limit_scaled,
                g,
                d_b,
                l: false, // TODO: Support 64-bit mode
                avl,
            };

            descriptor.valid = super::descriptor::SEG_VALID_CACHE;
        } else {
            // System/gate descriptor
            match r#type {
                0x4 | 0x6 | 0x7 => {
                    // 286 call/interrupt/trap gate
                    let param_count = (dword2 & 0x1F) as u8;
                    let dest_selector = (dword1 >> 16) as u16;
                    let dest_offset = (dword1 & 0xFFFF) as u32;

                    descriptor.u.gate = DescriptorGate {
                        param_count,
                        dest_selector,
                        dest_offset,
                    };
                    descriptor.valid = super::descriptor::SEG_VALID_CACHE;
                }
                0xC | 0xE | 0xF => {
                    // 386 call/interrupt/trap gate
                    let param_count = (dword2 & 0x1F) as u8;
                    let dest_selector = (dword1 >> 16) as u16;
                    let dest_offset = ((dword2 & 0xFFFF0000) | (dword1 & 0xFFFF)) as u32;

                    descriptor.u.gate = DescriptorGate {
                        param_count,
                        dest_selector,
                        dest_offset,
                    };
                    descriptor.valid = super::descriptor::SEG_VALID_CACHE;
                }
                0x5 => {
                    // Task gate
                    let tss_selector = (dword1 >> 16) as u16;
                    descriptor.u.task_gate = DescriptorTaskGate { tss_selector };
                    descriptor.valid = super::descriptor::SEG_VALID_CACHE;
                }
                0x2 | 0x1 | 0x3 | 0x9 | 0xB => {
                    // LDT, TSS descriptors
                    let limit = (dword1 & 0xFFFF) | (dword2 & 0x000F0000);
                    let mut base = ((dword1 >> 16) as u64) | (((dword2 & 0xFF) as u64) << 16);
                    base |= (dword2 & 0xFF000000) as u64;

                    let g = (dword2 & 0x00800000) != 0;
                    let d_b = (dword2 & 0x00400000) != 0;
                    let avl = (dword2 & 0x00100000) != 0;

                    let limit_scaled = if g { (limit << 12) | 0xFFF } else { limit };

                    descriptor.u.segment = DescriptorSegment {
                        base,
                        limit_scaled,
                        g,
                        d_b,
                        l: false,
                        avl,
                    };
                    descriptor.valid = super::descriptor::SEG_VALID_CACHE;
                }
                _ => {
                    // Reserved - invalid
                    descriptor.valid = 0;
                }
            }
        }

        Ok(descriptor)
    }

    // system_read_byte/word/dword/qword are defined in access.rs

    /// Get SS and ESP from TSS for given privilege level
    /// Based on BX_CPU_C::get_SS_ESP_from_TSS in tasking.cc:887
    pub(super) fn get_ss_esp_from_tss(&self, pl: u8) -> Result<(u16, u32)> {
        // Check if TR is valid
        if self.tr.cache.valid == 0 {
            tracing::error!("get_ss_esp_from_tss: TR.cache invalid");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: 0,
            });
        }

        // Check TSS type (386 or 286)
        let tss_type = self.tr.cache.r#type;
        if tss_type == 0x9 || tss_type == 0xB {
            // 32-bit TSS
            let tss_stackaddr = (8 * pl as u32) + 4;
            let limit_scaled = unsafe { self.tr.cache.u.segment.limit_scaled };
            if (tss_stackaddr + 7) > limit_scaled {
                tracing::error!("get_ss_esp_from_tss(386): TSSstackaddr > TSS.LIMIT");
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Ts,
                    error_code: 0,
                });
            }
            let tss_base = unsafe { self.tr.cache.u.segment.base };
            let ss = self.system_read_word(tss_base + tss_stackaddr as u64 + 4)?;
            let esp = self.system_read_dword(tss_base + tss_stackaddr as u64)?;
            Ok((ss, esp))
        } else if tss_type == 0x1 || tss_type == 0x3 {
            // 16-bit TSS
            let tss_stackaddr = (4 * pl as u32) + 2;
            let limit_scaled = unsafe { self.tr.cache.u.segment.limit_scaled };
            if (tss_stackaddr + 3) > limit_scaled {
                tracing::error!("get_ss_esp_from_tss(286): TSSstackaddr > TSS.LIMIT");
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Ts,
                    error_code: 0,
                });
            }
            let tss_base = unsafe { self.tr.cache.u.segment.base };
            let ss = self.system_read_word(tss_base + tss_stackaddr as u64 + 2)?;
            let esp = self.system_read_word(tss_base + tss_stackaddr as u64)? as u32;
            Ok((ss, esp))
        } else {
            tracing::error!("get_ss_esp_from_tss: TR is bogus type ({:#x})", tss_type);
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: 0,
            });
        }
    }

    /// Write word to new stack at given privilege level.
    ///
    /// Based on BX_CPU_C::write_new_stack_word in access.cc.
    /// Bochs calls `access_write_linear(laddr, ...)` which performs a full
    /// TLB lookup + page table walk. We use `system_write_word` which
    /// translates the linear address through paging (CPL=0 supervisor write).
    pub(super) fn write_new_stack_word(
        &mut self,
        seg: &super::descriptor::BxSegmentReg,
        addr: u32,
        _dpl: u8,
        value: u16,
    ) -> Result<()> {
        let seg_base = unsafe { seg.cache.u.segment.base };
        let laddr = (seg_base + addr as u64) & 0xFFFFFFFF;
        self.system_write_word(laddr, value)
    }

    /// Write dword to new stack at given privilege level.
    ///
    /// Based on BX_CPU_C::write_new_stack_dword in access.cc.
    /// Uses `system_write_dword` for proper paging translation.
    pub(super) fn write_new_stack_dword(
        &mut self,
        seg: &super::descriptor::BxSegmentReg,
        addr: u32,
        _dpl: u8,
        value: u32,
    ) -> Result<()> {
        let seg_base = unsafe { seg.cache.u.segment.base };
        let laddr = (seg_base + addr as u64) & 0xFFFFFFFF;
        self.system_write_dword(laddr, value)
    }

    /// Load SS segment register
    /// Based on BX_CPU_C::load_ss in segment_ctrl_pro.cc:519
    pub(super) fn load_ss(
        &mut self,
        selector: &mut BxSelector,
        descriptor: &mut BxDescriptor,
        cpl: u8,
    ) -> Result<()> {
        // Add cpl to the selector value
        selector.value = (selector.value & 0xFFFC) | cpl as u16;
        selector.rpl = cpl;

        // Touch segment if not null selector
        if (selector.value & 0xFFFC) != 0 {
            self.touch_segment(selector, descriptor)?;
        }

        self.sregs[BxSegregs::Ss as usize].selector = selector.clone();
        self.sregs[BxSegregs::Ss as usize].cache = descriptor.clone();
        self.sregs[BxSegregs::Ss as usize].cache.valid = super::descriptor::SEG_VALID_CACHE;

        // Invalidate stack cache (matches original line 533)
        // Note: invalidate_stack_cache is defined in init.rs
        self.invalidate_stack_cache();

        Ok(())
    }

    /// Touch segment - set accessed bit in descriptor
    /// Based on BX_CPU_C::touch_segment in segment_ctrl_pro.cc:502
    pub(super) fn touch_segment(
        &mut self,
        selector: &BxSelector,
        descriptor: &mut BxDescriptor,
    ) -> Result<()> {
        use super::descriptor::is_segment_accessed;

        // Check if segment is already accessed
        if !is_segment_accessed(descriptor.r#type) {
            // Get AR byte and set accessed bit
            let mut ar_byte = descriptor.get_ar_byte();
            ar_byte |= 1; // Set accessed bit
            descriptor.set_ar_byte(ar_byte);
            descriptor.r#type |= 1; // Update type field

            // Write AR byte back to GDT/LDT (should be done with locked RMW)
            // For now, use system_write_byte
            let offset = if selector.ti == 0 {
                // GDT
                self.gdtr.base + (selector.index as u64 * 8) + 5
            } else {
                // LDT
                let ldt_base = unsafe { self.ldtr.cache.u.segment.base };
                ldt_base + (selector.index as u64 * 8) + 5
            };

            self.system_write_byte(offset, ar_byte)?;
        }

        Ok(())
    }

    // system_write_byte/word/dword are defined in access.rs

    /// Check code segment descriptor validity
    /// Based on BX_CPU_C::check_cs in ctrl_xfer_pro.cc:29
    pub(super) fn check_cs(
        &mut self,
        descriptor: &BxDescriptor,
        cs_raw: u16,
        check_rpl: u8,
        check_cpl: u8,
    ) -> Result<()> {
        use super::descriptor::{is_code_segment_non_conforming, is_data_segment};
        // Mirrors Bochs ctrl_xfer_pro.cc:29 — calls exception() directly with cs_raw & 0xfffc

        // Descriptor must be valid and a code segment
        if descriptor.valid == 0 || !descriptor.segment || is_data_segment(descriptor.r#type) {
            tracing::error!("check_cs({:#06x}): not a valid code segment!", cs_raw);
            return self.exception(Exception::Gp, cs_raw & 0xfffc);
        }

        // Non-conforming code segment: DPL must = CPL
        if is_code_segment_non_conforming(descriptor.r#type) {
            if descriptor.dpl != check_cpl {
                tracing::error!(
                    "check_cs({:#06x}): non-conforming code seg descriptor dpl != cpl, dpl={}, cpl={}",
                    cs_raw, descriptor.dpl, check_cpl
                );
                return self.exception(Exception::Gp, cs_raw & 0xfffc);
            }

            // RPL must be <= CPL
            if check_rpl > check_cpl {
                tracing::error!(
                    "check_cs({:#06x}): non-conforming code seg selector rpl > cpl, rpl={}, cpl={}",
                    cs_raw,
                    check_rpl,
                    check_cpl
                );
                return self.exception(Exception::Gp, cs_raw & 0xfffc);
            }
        } else {
            // Conforming code segment: DPL must be <= CPL
            if descriptor.dpl > check_cpl {
                tracing::error!(
                    "check_cs({:#06x}): conforming code seg descriptor dpl > cpl, dpl={}, cpl={}",
                    cs_raw,
                    descriptor.dpl,
                    check_cpl
                );
                return self.exception(Exception::Gp, cs_raw & 0xfffc);
            }
        }

        // Code segment must be present — #NP (not #GP) for missing segment
        if !descriptor.p {
            tracing::error!("check_cs({:#06x}): code segment not present!", cs_raw);
            return self.exception(Exception::Np, cs_raw & 0xfffc);
        }

        Ok(())
    }

    /// Load CS segment register
    /// Based on BX_CPU_C::load_cs in ctrl_xfer_pro.cc:80
    pub(super) fn load_cs(
        &mut self,
        selector: &mut BxSelector,
        descriptor: &mut BxDescriptor,
        cpl: u8,
    ) -> Result<()> {
        // Add cpl to the selector value
        selector.value = (selector.value & 0xFFFC) | cpl as u16;
        selector.rpl = cpl;

        // Touch segment (set accessed bit)
        self.touch_segment(selector, descriptor)?;

        // Update CS segment register
        self.sregs[BxSegregs::Cs as usize].selector = selector.clone();
        self.sregs[BxSegregs::Cs as usize].cache = descriptor.clone();
        self.sregs[BxSegregs::Cs as usize].cache.valid = super::descriptor::SEG_VALID_CACHE;

        // Update user privilege level flag (Bochs cpu.h:5501)
        self.user_pl = cpl == 3;

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        Ok(())
    }

    /// Branch to far code segment
    /// Based on BX_CPU_C::branch_far in ctrl_xfer_pro.cc:115
    pub(super) fn branch_far(
        &mut self,
        selector: &mut BxSelector,
        descriptor: &mut BxDescriptor,
        rip: u64,
        cpl: u8,
    ) -> Result<()> {
        // Mask RIP to 32 bits for legacy mode
        let rip = rip & 0xFFFFFFFF;

        // Check RIP is within segment limit
        let limit = unsafe { descriptor.u.segment.limit_scaled };
        if rip as u32 > limit {
            tracing::error!("branch_far: RIP {:#010x} > limit {:#010x}", rip, limit);
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
        }

        // Load CS with new descriptor
        self.load_cs(selector, descriptor, cpl)?;

        // Update RIP
        self.set_rip(rip);

        Ok(())
    }

    /// Jump to protected mode code segment
    /// Based on BX_CPU_C::jump_protected in jmp_far.cc:30
    pub(super) fn jump_protected(&mut self, cs_raw: u16, disp: u64) -> Result<()> {
        tracing::trace!("jump_protected: cs={:#06x}, disp={:#010x}", cs_raw, disp);

        // Selector must not be null
        if (cs_raw & 0xFFFC) == 0 {
            tracing::error!("jump_protected: cs == 0");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: 0,
            });
        }

        // Parse selector
        let mut selector = BxSelector::default();
        parse_selector(cs_raw, &mut selector);

        tracing::info!(
            "jump_protected: selector index={}, ti={}, rpl={}",
            selector.index,
            selector.ti,
            selector.rpl
        );

        // Fetch descriptor from GDT/LDT
        let (dword1, dword2) = self.fetch_raw_descriptor(&selector)?;
        let mut descriptor = self.parse_descriptor(dword1, dword2)?;

        tracing::info!("jump_protected: descriptor segment={}, type={:#x}, dpl={}, p={}, base={:#010x}, limit={:#010x}",
                      descriptor.segment, descriptor.r#type, descriptor.dpl, descriptor.p,
                      unsafe { descriptor.u.segment.base }, unsafe { descriptor.u.segment.limit_scaled });

        if descriptor.segment {
            // Code segment descriptor
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            self.check_cs(&descriptor, cs_raw, selector.rpl, cpl)?;
            self.branch_far(&mut selector, &mut descriptor, disp, cpl)?;
            Ok(())
        } else {
            // System descriptor — Based on Bochs jmp_far.cc:59-127
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;

            // DPL checks for gates
            if descriptor.dpl < cpl {
                tracing::error!("jump_protected: gate.dpl < CPL");
                return self.exception(Exception::Gp, cs_raw & 0xfffc);
            }
            if descriptor.dpl < selector.rpl {
                tracing::error!("jump_protected: gate.dpl < selector.rpl");
                return self.exception(Exception::Gp, cs_raw & 0xfffc);
            }

            match descriptor.r#type {
                0x1 | 0x9 => {
                    // Available 286/386 TSS — JMP to TSS
                    tracing::debug!("jump_protected: JMP to TSS type={:#x}", descriptor.r#type);
                    if descriptor.valid == 0 || selector.ti != 0 {
                        tracing::error!("jump_protected: bad TSS selector");
                        return self.exception(Exception::Gp, cs_raw & 0xfffc);
                    }
                    if !descriptor.p {
                        tracing::error!("jump_protected: TSS not present");
                        return self.exception(Exception::Np, cs_raw & 0xfffc);
                    }
                    let (dword1, dword2) = self.fetch_raw_descriptor(&selector)?;
                    self.task_switch(
                        &selector,
                        &descriptor,
                        super::tasking::BX_TASK_FROM_JUMP,
                        dword1,
                        dword2,
                        false,
                        0,
                    )?;
                    Ok(())
                }
                0x5 => {
                    // Task gate
                    tracing::debug!("jump_protected: JMP via task gate");
                    self.task_gate_jmp(&selector, &descriptor)?;
                    Ok(())
                }
                0x4 | 0xC => {
                    // 286/386 call gate — JMP through call gate
                    tracing::debug!(
                        "jump_protected: JMP via call gate type={:#x}",
                        descriptor.r#type
                    );
                    self.jmp_call_gate(&selector, &descriptor)?;
                    Ok(())
                }
                _ => {
                    tracing::error!(
                        "jump_protected: unsupported system descriptor type {:#x}",
                        descriptor.r#type
                    );
                    self.exception(Exception::Gp, cs_raw & 0xfffc)
                }
            }
        }
    }

    /// Load segment register (handles both real and protected mode)
    /// Based on BX_CPU_C::load_seg_reg in segment_ctrl_pro.cc:28-177
    pub(super) fn load_seg_reg(&mut self, seg: BxSegregs, new_value: u16) -> Result<()> {
        // V8086 mode: use real-mode style loading (Bochs segment_ctrl_pro.cc:156-177)
        if self.v8086_mode() {
            self.load_seg_reg_real_mode(seg, new_value);
            return Ok(());
        }

        if !self.real_mode() {
            // Protected mode
            if seg as usize == BxSegregs::Ss as usize {
                // Special handling for SS
                let mut selector = BxSelector::default();
                parse_selector(new_value, &mut selector);

                // Null selector check
                if (new_value & 0xfffc) == 0 {
                    tracing::error!("load_seg_reg(SS): loading null selector");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: 0,
                    });
                }

                // Fetch descriptor from GDT/LDT
                let (dword1, dword2) = self.fetch_raw_descriptor(&selector)?;

                // Check selector RPL must equal CPL
                let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
                if selector.rpl != cpl {
                    tracing::error!("load_seg_reg(SS): rpl != CPL");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: 0,
                    });
                }

                let mut descriptor = self.parse_descriptor(dword1, dword2)?;

                if descriptor.valid == 0 {
                    tracing::error!("load_seg_reg(SS): valid bit cleared");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: 0,
                    });
                }

                // AR byte must indicate a writable data segment
                if !descriptor.segment ||
                   descriptor.r#type >= 8 || // IS_CODE_SEGMENT
                   (descriptor.r#type & 2) == 0
                // IS_DATA_SEGMENT_WRITEABLE
                {
                    tracing::error!("load_seg_reg(SS): not writable data segment");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: 0,
                    });
                }

                // DPL must equal CPL
                if descriptor.dpl != cpl {
                    tracing::error!("load_seg_reg(SS): dpl != CPL");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: 0,
                    });
                }

                // Segment must be PRESENT
                if !descriptor.p {
                    tracing::error!("load_seg_reg(SS): not present");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Ss,
                        error_code: 0,
                    });
                }

                self.touch_segment(&selector, &mut descriptor)?;

                // Load SS with selector and descriptor (this sets D_B bit!)
                self.load_ss(&mut selector, &mut descriptor, cpl)?;

                tracing::debug!(
                    "load_seg_reg(SS): loaded selector {:#06x}, d_b={}",
                    new_value,
                    unsafe { self.sregs[BxSegregs::Ss as usize].cache.u.segment.d_b }
                );

                return Ok(());
            } else if matches!(
                seg,
                BxSegregs::Ds | BxSegregs::Es | BxSegregs::Fs | BxSegregs::Gs
            ) {
                // Handling for DS, ES, FS, GS

                // Null selector is allowed for these segments
                if (new_value & 0xfffc) == 0 {
                    self.load_null_selector(seg, new_value);
                    return Ok(());
                }

                let mut selector = BxSelector::default();
                parse_selector(new_value, &mut selector);

                let (dword1, dword2) = self.fetch_raw_descriptor(&selector)?;
                let mut descriptor = self.parse_descriptor(dword1, dword2)?;

                if descriptor.valid == 0 {
                    tracing::error!(
                        "load_seg_reg({:?}, {:#06x}): invalid segment",
                        seg,
                        new_value
                    );
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: 0,
                    });
                }

                // AR byte must indicate data or readable code segment
                let is_code = descriptor.r#type >= 8;
                let is_readable = (descriptor.r#type & 2) != 0;
                if !descriptor.segment || (is_code && !is_readable) {
                    tracing::error!(
                        "load_seg_reg({:?}, {:#06x}): not data or readable code",
                        seg,
                        new_value
                    );
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: 0,
                    });
                }

                // If data or non-conforming code, RPL and CPL must be <= DPL
                let is_data = descriptor.r#type < 8;
                let is_conforming = (descriptor.r#type & 4) != 0;
                if is_data || !is_conforming {
                    let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
                    if selector.rpl > descriptor.dpl || cpl > descriptor.dpl {
                        tracing::error!(
                            "load_seg_reg({:?}, {:#06x}): RPL & CPL must be <= DPL",
                            seg,
                            new_value
                        );
                        return Err(super::error::CpuError::BadVector {
                            vector: Exception::Gp,
                            error_code: 0,
                        });
                    }
                }

                // Segment must be PRESENT
                if !descriptor.p {
                    tracing::error!(
                        "load_seg_reg({:?}, {:#06x}): segment not present",
                        seg,
                        new_value
                    );
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Np,
                        error_code: 0,
                    });
                }

                self.touch_segment(&selector, &mut descriptor)?;

                // Load segment register with selector and descriptor
                let seg_idx = seg as usize;
                self.sregs[seg_idx].selector = selector;
                self.sregs[seg_idx].cache = descriptor;
                self.sregs[seg_idx].cache.valid = super::descriptor::SEG_VALID_CACHE;

                tracing::debug!(
                    "load_seg_reg({:?}): loaded selector {:#06x}",
                    seg,
                    new_value
                );

                return Ok(());
            } else {
                tracing::error!("load_seg_reg(): invalid segment register {:?}", seg);
                return Err(super::error::CpuError::UnimplementedOpcode {
                    opcode: format!("load_seg_reg for segment {:?}", seg),
                });
            }
        }

        // Real mode or v8086 mode
        self.load_seg_reg_real_mode(seg, new_value);
        Ok(())
    }

    /// Load null selector for data segments (DS, ES, FS, GS)
    /// Based on BX_CPU_C::load_null_selector in segment_ctrl_pro.cc:212-234
    pub(super) fn load_null_selector(&mut self, seg: BxSegregs, value: u16) {
        let seg_idx = seg as usize;

        // Set selector fields
        self.sregs[seg_idx].selector.value = value;
        self.sregs[seg_idx].selector.index = 0;
        self.sregs[seg_idx].selector.ti = 0;
        self.sregs[seg_idx].selector.rpl = (value & 3) as u8;

        // Clear cache - Bochs segment_ctrl_pro.cc:221-231
        self.sregs[seg_idx].cache.valid = 0; // Invalidate null selector
        self.sregs[seg_idx].cache.p = false;
        self.sregs[seg_idx].cache.dpl = 0;
        self.sregs[seg_idx].cache.segment = true; // Data/code segment
        self.sregs[seg_idx].cache.r#type = 0;

        // Zero segment descriptor fields
        unsafe {
            self.sregs[seg_idx].cache.u.segment.base = 0;
            self.sregs[seg_idx].cache.u.segment.limit_scaled = 0;
            self.sregs[seg_idx].cache.u.segment.g = false;
            self.sregs[seg_idx].cache.u.segment.d_b = false;
            self.sregs[seg_idx].cache.u.segment.avl = false;
        }

        tracing::debug!(
            "load_null_selector({:?}): selector {:#06x}, cleared all cache fields",
            seg,
            value
        );
    }

    /// LLDT - Load Local Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:374-476
    pub(super) fn lldt_ew(&mut self, instr: &super::decoder::Instruction) -> Result<()> {
        // Must be in protected mode (catches both real mode and v8086)
        // Based on Bochs protect_ctrl.cc:376
        if !self.protected_mode() {
            tracing::debug!("LLDT: not recognized outside protected mode");
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        // CPL must be 0
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            tracing::error!("LLDT: CPL != 0");
            self.exception(Exception::Gp, 0)?;
            return Ok(());
        }

        // Read selector from register or memory
        let raw_selector = if instr.mod_c0() {
            let src = instr.src() as usize;
            self.get_gpr16(src)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr32(instr);
            self.read_virtual_word(seg, eaddr)?
        };

        // If selector is NULL, invalidate and done
        if (raw_selector & 0xfffc) == 0 {
            self.ldtr.selector.value = raw_selector;
            self.ldtr.cache.valid = 0;
            tracing::trace!("LLDT: NULL selector, invalidated");
            return Ok(());
        }

        // Parse selector
        let mut selector = BxSelector::default();
        parse_selector(raw_selector, &mut selector);

        // Selector must point into GDT (TI must be 0)
        if selector.ti != 0 {
            tracing::error!("LLDT: selector.ti != 0");
            self.exception(Exception::Gp, raw_selector & 0xfffc)?;
            return Ok(());
        }

        // Fetch descriptor from GDT
        let (dword1, dword2) = self.fetch_raw_descriptor(&selector)?;

        // Parse descriptor
        let descriptor = self.parse_descriptor(dword1, dword2)?;

        // Check if it's an LDT descriptor
        if descriptor.valid == 0
            || descriptor.segment
            || descriptor.r#type != SystemAndGateDescriptorEnum::BxSysSegmentLdt as u8
        {
            tracing::error!("LLDT: doesn't point to an LDT descriptor!");
            self.exception(Exception::Gp, raw_selector & 0xfffc)?;
            return Ok(());
        }

        // Check if present
        if !descriptor.p {
            tracing::error!("LLDT: LDT descriptor not present!");
            self.exception(Exception::Np, raw_selector & 0xfffc)?;
            return Ok(());
        }

        // Load LDTR
        self.ldtr.selector = selector;
        self.ldtr.cache = descriptor;
        self.ldtr.cache.valid = SEG_VALID_CACHE;

        tracing::trace!("LLDT: loaded selector={:#06x}", raw_selector);
        Ok(())
    }

    /// LTR - Load Task Register
    /// Based on Bochs protect_ctrl.cc:478-564
    pub(super) fn ltr_ew(&mut self, instr: &super::decoder::Instruction) -> Result<()> {
        // Must be in protected mode (catches both real mode and v8086)
        // Based on Bochs protect_ctrl.cc:480
        if !self.protected_mode() {
            tracing::debug!("LTR: not recognized outside protected mode");
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        // CPL must be 0
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            tracing::error!("LTR: CPL != 0");
            self.exception(Exception::Gp, 0)?;
            return Ok(());
        }

        // Read selector from register or memory
        let raw_selector = if instr.mod_c0() {
            let src = instr.src() as usize;
            self.get_gpr16(src)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr32(instr);
            self.read_virtual_word(seg, eaddr)?
        };

        // NULL selector not allowed for LTR
        if (raw_selector & 0xfffc) == 0 {
            tracing::error!("LTR: loading with NULL selector!");
            self.exception(Exception::Gp, 0)?;
            return Ok(());
        }

        // Parse selector
        let mut selector = BxSelector::default();
        parse_selector(raw_selector, &mut selector);

        // Selector must point into GDT (TI must be 0)
        if selector.ti != 0 {
            tracing::error!("LTR: selector.ti != 0");
            self.exception(Exception::Gp, raw_selector & 0xfffc)?;
            return Ok(());
        }

        // Fetch descriptor from GDT
        let (dword1, dword2) = self.fetch_raw_descriptor(&selector)?;

        // Parse descriptor
        let mut descriptor = self.parse_descriptor(dword1, dword2)?;

        // Check if it's an available TSS descriptor (type 1=16-bit avail, 9=32-bit avail)
        let tss_type = descriptor.r#type;
        if descriptor.valid == 0
            || descriptor.segment
            || (tss_type != SystemAndGateDescriptorEnum::BxSysSegmentAvail286Tss as u8
                && tss_type != SystemAndGateDescriptorEnum::BxSysSegmentAvail386Tss as u8)
        {
            tracing::error!(
                "LTR: doesn't point to an available TSS descriptor! type={}",
                tss_type
            );
            self.exception(Exception::Gp, raw_selector & 0xfffc)?;
            return Ok(());
        }

        // Check if present
        if !descriptor.p {
            tracing::error!("LTR: TSS descriptor not present!");
            self.exception(Exception::Np, raw_selector & 0xfffc)?;
            return Ok(());
        }

        // Mark TSS as busy in the descriptor
        descriptor.r#type |= 0x02; // Set busy bit

        // Save selector index before move
        let selector_index = selector.index;

        // Load TR
        self.tr.selector = selector;
        self.tr.cache = descriptor;
        self.tr.cache.valid = SEG_VALID_CACHE;

        // Also mark as busy in GDT (write back dword2 with busy bit)
        // Based on Bochs protect_ctrl.cc:558-561 — uses system_write
        let gdt_offset = self.gdtr.base + (selector_index as u64 * 8) + 4;
        let new_dword2 = dword2 | 0x0200; // Set busy bit in access byte
        let phys_addr = self.translate_linear_system_write(gdt_offset)?;
        self.mem_write_dword(phys_addr, new_dword2);

        tracing::trace!("LTR: loaded selector={:#06x}, marked busy", raw_selector);
        Ok(())
    }

    // =========================================================================
    // validate_seg_regs — invalidate DS/ES/FS/GS on privilege change
    // Based on Bochs segment_ctrl_pro.cc:238-272
    // =========================================================================

    /// Invalidate a single segment register if DPL < CPL and type is data or non-conforming code
    fn validate_seg_reg(&mut self, seg: usize) {
        use super::descriptor::{is_code_segment_non_conforming, is_data_segment};
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        let cache = &self.sregs[seg].cache;
        if cache.dpl < cpl {
            if cache.valid == 0
                || !cache.segment
                || is_data_segment(cache.r#type)
                || is_code_segment_non_conforming(cache.r#type)
            {
                self.sregs[seg].selector.value = 0;
                self.sregs[seg].cache.valid = 0;
            }
        }
    }

    /// Validate ES/DS/FS/GS after privilege level change
    /// Based on Bochs segment_ctrl_pro.cc:265-272
    pub(super) fn validate_seg_regs(&mut self) {
        self.validate_seg_reg(BxSegregs::Es as usize);
        self.validate_seg_reg(BxSegregs::Ds as usize);
        self.validate_seg_reg(BxSegregs::Fs as usize);
        self.validate_seg_reg(BxSegregs::Gs as usize);
    }

    // =========================================================================
    // call_protected — protected mode far CALL
    // Based on Bochs call_far.cc:29-228
    // =========================================================================

    /// Protected mode far CALL
    /// Handles code segments and call gates (same-priv and inner-priv)
    pub(super) fn call_protected(&mut self, cs_raw: u16, disp: u32, os32: bool) -> Result<()> {
        if (cs_raw & 0xfffc) == 0 {
            tracing::error!("call_protected: CS selector null");
            return self.exception(Exception::Gp, 0);
        }

        let mut cs_selector = BxSelector::default();
        parse_selector(cs_raw, &mut cs_selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, cs_raw & 0xfffc),
        };
        let mut cs_descriptor = self.parse_descriptor(dword1, dword2)?;

        if cs_descriptor.valid == 0 {
            tracing::error!("call_protected: invalid CS descriptor");
            return self.exception(Exception::Gp, cs_raw & 0xfffc);
        }

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;

        if cs_descriptor.segment {
            // ── Normal code segment ──
            self.check_cs(&cs_descriptor, cs_raw, cs_selector.rpl, cpl)?;

            let temp_rsp = if self.is_stack_32bit() {
                self.esp()
            } else {
                self.sp() as u32
            };

            let ss_seg = self.sregs[BxSegregs::Ss as usize].clone();

            if os32 {
                self.write_new_stack_dword(
                    &ss_seg,
                    temp_rsp.wrapping_sub(4),
                    cs_descriptor.dpl,
                    self.sregs[BxSegregs::Cs as usize].selector.value as u32,
                )?;
                self.write_new_stack_dword(
                    &ss_seg,
                    temp_rsp.wrapping_sub(8),
                    cs_descriptor.dpl,
                    self.eip(),
                )?;
                self.branch_far(&mut cs_selector, &mut cs_descriptor, disp as u64, cpl)?;
                if self.is_stack_32bit() {
                    self.set_esp(temp_rsp.wrapping_sub(8));
                } else {
                    self.set_sp((temp_rsp.wrapping_sub(8)) as u16);
                }
            } else {
                self.write_new_stack_word(
                    &ss_seg,
                    temp_rsp.wrapping_sub(2),
                    cs_descriptor.dpl,
                    self.sregs[BxSegregs::Cs as usize].selector.value,
                )?;
                self.write_new_stack_word(
                    &ss_seg,
                    temp_rsp.wrapping_sub(4),
                    cs_descriptor.dpl,
                    self.get_ip(),
                )?;
                self.branch_far(&mut cs_selector, &mut cs_descriptor, disp as u64, cpl)?;
                if self.is_stack_32bit() {
                    self.set_esp(temp_rsp.wrapping_sub(4));
                } else {
                    self.set_sp((temp_rsp.wrapping_sub(4)) as u16);
                }
            }
            return Ok(());
        }

        // ── System descriptor (gate) ──
        // Check DPL >= CPL and DPL >= RPL
        if cs_descriptor.dpl < cpl {
            tracing::error!("call_protected: gate.dpl < CPL");
            return self.exception(Exception::Gp, cs_raw & 0xfffc);
        }
        if cs_descriptor.dpl < cs_selector.rpl {
            tracing::error!("call_protected: gate.dpl < selector.rpl");
            return self.exception(Exception::Gp, cs_raw & 0xfffc);
        }

        match cs_descriptor.r#type {
            0x5 => {
                // Task gate
                self.task_gate_call(&cs_selector, &cs_descriptor)?;
            }
            0x4 | 0xC => {
                // 286/386 call gate
                self.call_gate(&cs_selector, &cs_descriptor, os32)?;
            }
            0x1 | 0x9 => {
                // Available TSS — CALL causes task switch
                if !cs_descriptor.p {
                    tracing::error!("call_protected: TSS not present");
                    return self.exception(Exception::Np, cs_raw & 0xfffc);
                }
                self.task_switch(
                    &cs_selector,
                    &cs_descriptor,
                    super::tasking::BX_TASK_FROM_CALL,
                    dword1,
                    dword2,
                    false,
                    0,
                )?;
            }
            _ => {
                tracing::error!(
                    "call_protected: unsupported system descriptor type {:#x}",
                    cs_descriptor.r#type
                );
                return self.exception(Exception::Gp, cs_raw & 0xfffc);
            }
        }

        Ok(())
    }

    /// Call through a call gate (same or inner privilege)
    /// Based on Bochs call_far.cc call_gate() function
    fn call_gate(
        &mut self,
        _gate_selector: &BxSelector,
        gate_descriptor: &BxDescriptor,
        _os32: bool,
    ) -> Result<()> {
        use super::descriptor::{
            is_code_segment, is_code_segment_non_conforming, is_data_segment,
            is_data_segment_writable,
        };

        // Gate must be present
        if !gate_descriptor.p {
            tracing::error!("call_gate: gate not present");
            return self.exception(Exception::Np, _gate_selector.value & 0xfffc);
        }

        // Get CS:EIP from gate
        let gate_cs_raw = unsafe { gate_descriptor.u.gate.dest_selector };
        let new_eip = unsafe { gate_descriptor.u.gate.dest_offset };

        if (gate_cs_raw & 0xfffc) == 0 {
            tracing::error!("call_gate: CS selector null");
            return self.exception(Exception::Gp, 0);
        }

        let mut gate_cs_selector = BxSelector::default();
        parse_selector(gate_cs_raw, &mut gate_cs_selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor(&gate_cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, gate_cs_raw & 0xfffc),
        };
        let mut cs_descriptor = self.parse_descriptor(dword1, dword2)?;

        // Must be code segment
        if cs_descriptor.valid == 0
            || !cs_descriptor.segment
            || is_data_segment(cs_descriptor.r#type)
        {
            tracing::error!("call_gate: not code segment");
            return self.exception(Exception::Gp, gate_cs_raw & 0xfffc);
        }

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;

        // Check: non-conforming and DPL < CPL → more privilege (inner privilege call)
        if is_code_segment_non_conforming(cs_descriptor.r#type) && cs_descriptor.dpl < cpl {
            // ── CALL GATE TO MORE PRIVILEGE ──
            tracing::debug!(
                "call_gate: to MORE privilege (DPL={} < CPL={})",
                cs_descriptor.dpl,
                cpl
            );

            // Get new SS:ESP from TSS
            let (ss_for_cpl_x, esp_for_cpl_x) = self.get_ss_esp_from_tss(cs_descriptor.dpl)?;

            if (ss_for_cpl_x & 0xfffc) == 0 {
                tracing::error!("call_gate: new SS null");
                return self.exception(Exception::Ts, 0);
            }

            let mut ss_selector = BxSelector::default();
            parse_selector(ss_for_cpl_x, &mut ss_selector);

            let (ss_dw1, ss_dw2) = match self.fetch_raw_descriptor(&ss_selector) {
                Ok(v) => v,
                Err(_) => return self.exception(Exception::Ts, ss_for_cpl_x & 0xfffc),
            };
            let mut ss_descriptor = self.parse_descriptor(ss_dw1, ss_dw2)?;

            // Validate new SS
            if ss_selector.rpl != cs_descriptor.dpl {
                return self.exception(Exception::Ts, ss_for_cpl_x & 0xfffc);
            }
            if ss_descriptor.dpl != cs_descriptor.dpl {
                return self.exception(Exception::Ts, ss_for_cpl_x & 0xfffc);
            }
            if ss_descriptor.valid == 0
                || !ss_descriptor.segment
                || is_code_segment(ss_descriptor.r#type)
                || !is_data_segment_writable(ss_descriptor.r#type)
            {
                return self.exception(Exception::Ts, ss_for_cpl_x & 0xfffc);
            }
            if !ss_descriptor.p {
                return self.exception(Exception::Ss, ss_for_cpl_x & 0xfffc);
            }

            let param_count = unsafe { gate_descriptor.u.gate.param_count } & 0x1f;

            // Save return SS:ESP and CS:EIP
            let return_ss = self.sregs[BxSegregs::Ss as usize].selector.value;
            let return_esp = if self.is_stack_32bit() {
                self.esp()
            } else {
                self.sp() as u32
            };
            let return_cs = self.sregs[BxSegregs::Cs as usize].selector.value;
            let return_eip = self.eip();

            // Prepare new stack segment
            let mut new_stack = self.sregs[BxSegregs::Ss as usize].clone();
            new_stack.selector = ss_selector.clone();
            new_stack.cache = ss_descriptor.clone();
            new_stack.selector.rpl = cs_descriptor.dpl;
            new_stack.selector.value =
                (new_stack.selector.value & 0xfffc) | new_stack.selector.rpl as u16;

            let is_386_gate = gate_descriptor.r#type == 0xC;

            if unsafe { ss_descriptor.u.segment.d_b } {
                let mut temp_esp = esp_for_cpl_x;

                if is_386_gate {
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_esp.wrapping_sub(4),
                        cs_descriptor.dpl,
                        return_ss as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_esp.wrapping_sub(8),
                        cs_descriptor.dpl,
                        return_esp,
                    )?;
                    temp_esp = temp_esp.wrapping_sub(8);

                    for n in (1..=param_count as u32).rev() {
                        temp_esp = temp_esp.wrapping_sub(4);
                        let param = self.stack_read_dword(return_esp.wrapping_add((n - 1) * 4))?;
                        self.write_new_stack_dword(&new_stack, temp_esp, cs_descriptor.dpl, param)?;
                    }

                    self.write_new_stack_dword(
                        &new_stack,
                        temp_esp.wrapping_sub(4),
                        cs_descriptor.dpl,
                        return_cs as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_esp.wrapping_sub(8),
                        cs_descriptor.dpl,
                        return_eip,
                    )?;
                    temp_esp = temp_esp.wrapping_sub(8);
                } else {
                    self.write_new_stack_word(
                        &new_stack,
                        temp_esp.wrapping_sub(2),
                        cs_descriptor.dpl,
                        return_ss,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_esp.wrapping_sub(4),
                        cs_descriptor.dpl,
                        return_esp as u16,
                    )?;
                    temp_esp = temp_esp.wrapping_sub(4);

                    for n in (1..=param_count as u32).rev() {
                        temp_esp = temp_esp.wrapping_sub(2);
                        let param =
                            self.stack_read_word(return_esp.wrapping_add((n - 1) * 2))? as u16;
                        self.write_new_stack_word(&new_stack, temp_esp, cs_descriptor.dpl, param)?;
                    }

                    self.write_new_stack_word(
                        &new_stack,
                        temp_esp.wrapping_sub(2),
                        cs_descriptor.dpl,
                        return_cs,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_esp.wrapping_sub(4),
                        cs_descriptor.dpl,
                        return_eip as u16,
                    )?;
                    temp_esp = temp_esp.wrapping_sub(4);
                }

                // Load new SS and CS
                let new_cpl = cs_descriptor.dpl;
                self.load_ss(&mut ss_selector, &mut ss_descriptor, new_cpl)?;
                self.load_cs(&mut gate_cs_selector, &mut cs_descriptor, new_cpl)?;
                self.set_eip(new_eip);
                self.set_esp(temp_esp);
            } else {
                let mut temp_sp = esp_for_cpl_x as u16;

                if is_386_gate {
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_sp.wrapping_sub(4) as u32,
                        cs_descriptor.dpl,
                        return_ss as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_sp.wrapping_sub(8) as u32,
                        cs_descriptor.dpl,
                        return_esp,
                    )?;
                    temp_sp = temp_sp.wrapping_sub(8);

                    for n in (1..=param_count as u32).rev() {
                        temp_sp = temp_sp.wrapping_sub(4);
                        let param = self.stack_read_dword(return_esp.wrapping_add((n - 1) * 4))?;
                        self.write_new_stack_dword(
                            &new_stack,
                            temp_sp as u32,
                            cs_descriptor.dpl,
                            param,
                        )?;
                    }

                    self.write_new_stack_dword(
                        &new_stack,
                        temp_sp.wrapping_sub(4) as u32,
                        cs_descriptor.dpl,
                        return_cs as u32,
                    )?;
                    self.write_new_stack_dword(
                        &new_stack,
                        temp_sp.wrapping_sub(8) as u32,
                        cs_descriptor.dpl,
                        return_eip,
                    )?;
                    temp_sp = temp_sp.wrapping_sub(8);
                } else {
                    self.write_new_stack_word(
                        &new_stack,
                        temp_sp.wrapping_sub(2) as u32,
                        cs_descriptor.dpl,
                        return_ss,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_sp.wrapping_sub(4) as u32,
                        cs_descriptor.dpl,
                        return_esp as u16,
                    )?;
                    temp_sp = temp_sp.wrapping_sub(4);

                    for n in (1..=param_count as u32).rev() {
                        temp_sp = temp_sp.wrapping_sub(2);
                        let param =
                            self.stack_read_word(return_esp.wrapping_add((n - 1) * 2))? as u16;
                        self.write_new_stack_word(
                            &new_stack,
                            temp_sp as u32,
                            cs_descriptor.dpl,
                            param,
                        )?;
                    }

                    self.write_new_stack_word(
                        &new_stack,
                        temp_sp.wrapping_sub(2) as u32,
                        cs_descriptor.dpl,
                        return_cs,
                    )?;
                    self.write_new_stack_word(
                        &new_stack,
                        temp_sp.wrapping_sub(4) as u32,
                        cs_descriptor.dpl,
                        return_eip as u16,
                    )?;
                    temp_sp = temp_sp.wrapping_sub(4);
                }

                let new_cpl = cs_descriptor.dpl;
                self.load_ss(&mut ss_selector, &mut ss_descriptor, new_cpl)?;
                self.load_cs(&mut gate_cs_selector, &mut cs_descriptor, new_cpl)?;
                self.set_eip(new_eip);
                self.set_sp(temp_sp);
            }
        } else {
            // ── CALL GATE TO SAME PRIVILEGE ──
            tracing::debug!("call_gate: to SAME privilege");

            if gate_descriptor.r#type == 0xC {
                // 386 call gate
                self.push_32(self.sregs[BxSegregs::Cs as usize].selector.value as u32)?;
                self.push_32(self.eip())?;
            } else {
                // 286 call gate
                self.push_16(self.sregs[BxSegregs::Cs as usize].selector.value)?;
                self.push_16(self.get_ip())?;
            }

            self.branch_far(
                &mut gate_cs_selector,
                &mut cs_descriptor,
                new_eip as u64,
                cpl,
            )?;
        }

        Ok(())
    }

    /// Handle task gate for CALL/JMP
    /// Based on Bochs jmp_far.cc task_gate()
    fn task_gate_call(
        &mut self,
        selector: &BxSelector,
        gate_descriptor: &BxDescriptor,
    ) -> Result<()> {
        if !gate_descriptor.p {
            tracing::error!("task_gate: not present");
            return self.exception(Exception::Np, selector.value & 0xfffc);
        }

        let raw_tss_selector = unsafe { gate_descriptor.u.task_gate.tss_selector };
        let mut tss_selector = BxSelector::default();
        parse_selector(raw_tss_selector, &mut tss_selector);

        if tss_selector.ti != 0 {
            tracing::error!("task_gate: tss_selector.ti=1");
            return self.exception(Exception::Gp, raw_tss_selector & 0xfffc);
        }

        let (dword1, dword2) = match self.fetch_raw_descriptor(&tss_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, raw_tss_selector & 0xfffc),
        };
        let tss_descriptor = self.parse_descriptor(dword1, dword2)?;

        if tss_descriptor.valid == 0 || tss_descriptor.segment {
            tracing::error!("task_gate: TSS descriptor invalid");
            return self.exception(Exception::Gp, raw_tss_selector & 0xfffc);
        }
        if tss_descriptor.r#type != 0x1 && tss_descriptor.r#type != 0x9 {
            tracing::error!("task_gate: TSS not available type");
            return self.exception(Exception::Gp, raw_tss_selector & 0xfffc);
        }
        if !tss_descriptor.p {
            tracing::error!("task_gate: TSS not present");
            return self.exception(Exception::Np, raw_tss_selector & 0xfffc);
        }

        self.task_switch(
            &tss_selector,
            &tss_descriptor,
            super::tasking::BX_TASK_FROM_CALL,
            dword1,
            dword2,
            false,
            0,
        )
    }

    // =========================================================================
    // return_protected — protected mode far RET
    // Based on Bochs ret_far.cc:29-268
    // =========================================================================

    /// Protected mode far RET
    pub(super) fn return_protected(&mut self, pop_bytes: u16, os32: bool) -> Result<()> {
        let temp_rsp = if self.is_stack_32bit() {
            self.esp()
        } else {
            self.sp() as u32
        };

        let (raw_cs_raw, return_eip, stack_param_offset) = if os32 {
            let eip = self.stack_read_dword(temp_rsp)?;
            let cs = self.stack_read_dword(temp_rsp.wrapping_add(4))? as u16;
            (cs, eip, 8u32)
        } else {
            let ip = self.stack_read_word(temp_rsp)? as u32;
            let cs = self.stack_read_word(temp_rsp.wrapping_add(2))?;
            (cs, ip, 4u32)
        };

        if (raw_cs_raw & 0xfffc) == 0 {
            tracing::error!("return_protected: CS selector null");
            return self.exception(Exception::Gp, 0);
        }

        let mut cs_selector = BxSelector::default();
        parse_selector(raw_cs_raw, &mut cs_selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, raw_cs_raw & 0xfffc),
        };
        let mut cs_descriptor = self.parse_descriptor(dword1, dword2)?;

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cs_selector.rpl < cpl {
            tracing::error!("return_protected: CS.rpl < CPL");
            return self.exception(Exception::Gp, raw_cs_raw & 0xfffc);
        }

        // check_cs validates code segment, DPL, and presence
        // Bochs check_cs calls exception() directly (Gp for type/DPL, Np for not-present)
        self.check_cs(&cs_descriptor, raw_cs_raw, 0, cs_selector.rpl)?;

        if cs_selector.rpl == cpl {
            // ── Same privilege return ──
            tracing::debug!(
                "return_protected: same-priv return CS={:#06x} EIP={:#010x}",
                raw_cs_raw,
                return_eip
            );

            self.branch_far(&mut cs_selector, &mut cs_descriptor, return_eip as u64, cpl)?;

            if self.is_stack_32bit() {
                self.set_esp(
                    self.esp()
                        .wrapping_add(stack_param_offset)
                        .wrapping_add(pop_bytes as u32),
                );
            } else {
                self.set_sp(
                    self.sp()
                        .wrapping_add(stack_param_offset as u16)
                        .wrapping_add(pop_bytes),
                );
            }
        } else {
            // ── Outer privilege return ──
            tracing::debug!(
                "return_protected: outer-priv return CS={:#06x} EIP={:#010x}",
                raw_cs_raw,
                return_eip
            );

            let (raw_ss_raw, return_rsp) = if os32 {
                let ss = self.stack_read_word(
                    temp_rsp
                        .wrapping_add(stack_param_offset)
                        .wrapping_add(pop_bytes as u32)
                        .wrapping_add(4),
                )?;
                let rsp = self.stack_read_dword(
                    temp_rsp
                        .wrapping_add(stack_param_offset)
                        .wrapping_add(pop_bytes as u32),
                )?;
                (ss, rsp)
            } else {
                let ss = self.stack_read_word(
                    temp_rsp
                        .wrapping_add(stack_param_offset)
                        .wrapping_add(pop_bytes as u32)
                        .wrapping_add(2),
                )?;
                let rsp = self.stack_read_word(
                    temp_rsp
                        .wrapping_add(stack_param_offset)
                        .wrapping_add(pop_bytes as u32),
                )? as u32;
                (ss, rsp)
            };

            if (raw_ss_raw & 0xfffc) == 0 {
                tracing::error!("return_protected: SS selector null");
                return self.exception(Exception::Gp, 0);
            }

            let mut ss_selector = BxSelector::default();
            parse_selector(raw_ss_raw, &mut ss_selector);

            let (ss_dw1, ss_dw2) = match self.fetch_raw_descriptor(&ss_selector) {
                Ok(v) => v,
                Err(_) => {
                    return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
                }
            };
            let mut ss_descriptor = self.parse_descriptor(ss_dw1, ss_dw2)?;

            // Validate SS
            if ss_selector.rpl != cs_selector.rpl {
                tracing::error!("return_protected: SS.rpl != CS.rpl");
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }
            if ss_descriptor.valid == 0
                || !ss_descriptor.segment
                || ss_descriptor.r#type >= 8 // code segment
                || (ss_descriptor.r#type & 2) == 0
            // not writable
            {
                tracing::error!("return_protected: SS not writable data");
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }
            if ss_descriptor.dpl != cs_selector.rpl {
                tracing::error!("return_protected: SS.dpl != CS.rpl");
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }
            if !ss_descriptor.p {
                tracing::error!("return_protected: SS not present");
                return self.exception(Exception::Ss, raw_ss_raw & 0xfffc);
            }

            // Load new CS
            let new_cpl = cs_selector.rpl;
            self.branch_far(
                &mut cs_selector,
                &mut cs_descriptor,
                return_eip as u64,
                new_cpl,
            )?;

            // Load new SS
            self.load_ss(&mut ss_selector, &mut ss_descriptor, new_cpl)?;

            if unsafe { ss_descriptor.u.segment.d_b } {
                self.set_esp(return_rsp.wrapping_add(pop_bytes as u32));
            } else {
                self.set_sp((return_rsp as u16).wrapping_add(pop_bytes));
            }

            // Invalidate DS/ES/FS/GS if no longer accessible at new privilege level
            self.validate_seg_regs();
        }

        Ok(())
    }

    // =========================================================================
    // JMP call gate — JMP through a call gate (no stack frame push)
    // Based on Bochs jmp_far.cc:180-221 jmp_call_gate()
    // =========================================================================

    fn jmp_call_gate(
        &mut self,
        _selector: &BxSelector,
        gate_descriptor: &BxDescriptor,
    ) -> Result<()> {
        if !gate_descriptor.p {
            tracing::error!("jmp_call_gate: gate not present");
            return self.exception(Exception::Np, _selector.value & 0xfffc);
        }

        let gate_cs_raw = unsafe { gate_descriptor.u.gate.dest_selector };
        if (gate_cs_raw & 0xfffc) == 0 {
            tracing::error!("jmp_call_gate: CS selector null");
            return self.exception(Exception::Gp, 0);
        }

        let mut gate_cs_selector = BxSelector::default();
        parse_selector(gate_cs_raw, &mut gate_cs_selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor(&gate_cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, gate_cs_raw & 0xfffc),
        };
        let mut cs_descriptor = self.parse_descriptor(dword1, dword2)?;

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        self.check_cs(&cs_descriptor, gate_cs_raw, 0, cpl)?;

        let temp_eip = unsafe { gate_descriptor.u.gate.dest_offset };
        self.branch_far(
            &mut gate_cs_selector,
            &mut cs_descriptor,
            temp_eip as u64,
            cpl,
        )?;
        Ok(())
    }

    // =========================================================================
    // Task gate for JMP — JMP through a task gate
    // Based on Bochs jmp_far.cc:129-178 task_gate()
    // =========================================================================

    fn task_gate_jmp(
        &mut self,
        selector: &BxSelector,
        gate_descriptor: &BxDescriptor,
    ) -> Result<()> {
        if !gate_descriptor.p {
            tracing::error!("task_gate_jmp: not present");
            return self.exception(Exception::Np, selector.value & 0xfffc);
        }

        let raw_tss_selector = unsafe { gate_descriptor.u.task_gate.tss_selector };
        let mut tss_selector = BxSelector::default();
        parse_selector(raw_tss_selector, &mut tss_selector);

        if tss_selector.ti != 0 {
            tracing::error!("task_gate_jmp: tss_selector.ti=1");
            return self.exception(Exception::Gp, raw_tss_selector & 0xfffc);
        }

        let (dword1, dword2) = match self.fetch_raw_descriptor(&tss_selector) {
            Ok(v) => v,
            Err(_) => {
                return self.exception(Exception::Gp, raw_tss_selector & 0xfffc);
            }
        };
        let tss_descriptor = self.parse_descriptor(dword1, dword2)?;

        if tss_descriptor.valid == 0 || tss_descriptor.segment {
            tracing::error!("task_gate_jmp: bad TSS descriptor");
            return self.exception(Exception::Gp, raw_tss_selector & 0xfffc);
        }
        if tss_descriptor.r#type != 0x1 && tss_descriptor.r#type != 0x9 {
            tracing::error!("task_gate_jmp: TSS not available");
            return self.exception(Exception::Gp, raw_tss_selector & 0xfffc);
        }
        if !tss_descriptor.p {
            tracing::error!("task_gate_jmp: TSS not present");
            return self.exception(Exception::Np, raw_tss_selector & 0xfffc);
        }

        self.task_switch(
            &tss_selector,
            &tss_descriptor,
            super::tasking::BX_TASK_FROM_JUMP,
            dword1,
            dword2,
            false,
            0,
        )
    }
}
