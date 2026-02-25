use super::{
    cpu::{CpuMode, Exception},
    decoder::BxSegregs,
    descriptor::{BxDescriptor, BxSelector, DescriptorGate, DescriptorSegment, DescriptorTaskGate,
                 SystemAndGateDescriptorEnum, SEG_VALID_CACHE},
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
                tracing::error!("fetch_raw_descriptor: GDT: index ({}) {} > limit ({})", index_offset, index, self.gdtr.limit);
                return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
            }
            offset = self.gdtr.base + (index as u64 * 8);
        } else {
            // LDT
            if self.ldtr.cache.valid == 0 {
                tracing::error!("fetch_raw_descriptor: LDTR.valid=0");
                return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
            }
            let ldt_limit = unsafe { self.ldtr.cache.u.segment.limit_scaled };
            let index_offset = (index as u32) * 8 + 7;
            if index_offset > ldt_limit {
                tracing::error!("fetch_raw_descriptor: LDT: index ({}) {} > limit ({})", index_offset, index, ldt_limit);
                return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
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

            let limit_scaled = if g {
                (limit << 12) | 0xFFF
            } else {
                limit
            };

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
                    
                    let limit_scaled = if g {
                        (limit << 12) | 0xFFF
                    } else {
                        limit
                    };
                    
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

    /// Read qword from system address space.
    /// Based on BX_CPU_C::system_read_qword in access.cc:617
    /// Translates linear address through paging when CR0.PG is set.
    pub(super) fn system_read_qword(&self, laddr: BxAddress) -> Result<u64> {
        let paddr_lo = self.translate_linear_system_read(laddr)?;
        let paddr_hi = self.translate_linear_system_read(laddr + 4)?;
        let lo = self.mem_read_dword(paddr_lo) as u64;
        let hi = self.mem_read_dword(paddr_hi) as u64;
        Ok(lo | (hi << 32))
    }

    /// Read word from system address space.
    /// Based on BX_CPU_C::system_read_word in access.cc:585
    pub(super) fn system_read_word(&self, laddr: BxAddress) -> Result<u16> {
        let paddr = self.translate_linear_system_read(laddr)?;
        Ok(self.mem_read_word(paddr))
    }

    /// Read dword from system address space.
    /// Based on BX_CPU_C::system_read_dword in access.cc:600
    pub(super) fn system_read_dword(&self, laddr: BxAddress) -> Result<u32> {
        let paddr = self.translate_linear_system_read(laddr)?;
        Ok(self.mem_read_dword(paddr))
    }

    /// Get SS and ESP from TSS for given privilege level
    /// Based on BX_CPU_C::get_SS_ESP_from_TSS in tasking.cc:887
    pub(super) fn get_ss_esp_from_tss(&self, pl: u8) -> Result<(u16, u32)> {
        // Check if TR is valid
        if self.tr.cache.valid == 0 {
            tracing::error!("get_ss_esp_from_tss: TR.cache invalid");
            return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
        }

        // Check TSS type (386 or 286)
        let tss_type = self.tr.cache.r#type;
        if tss_type == 0x9 || tss_type == 0xB {
            // 32-bit TSS
            let tss_stackaddr = (8 * pl as u32) + 4;
            let limit_scaled = unsafe { self.tr.cache.u.segment.limit_scaled };
            if (tss_stackaddr + 7) > limit_scaled {
                tracing::error!("get_ss_esp_from_tss(386): TSSstackaddr > TSS.LIMIT");
                return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
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
                return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
            }
            let tss_base = unsafe { self.tr.cache.u.segment.base };
            let ss = self.system_read_word(tss_base + tss_stackaddr as u64 + 2)?;
            let esp = self.system_read_word(tss_base + tss_stackaddr as u64)? as u32;
            Ok((ss, esp))
        } else {
            tracing::error!("get_ss_esp_from_tss: TR is bogus type ({:#x})", tss_type);
            return Err(super::error::CpuError::BadVector { vector: Exception::Ts });
        }
    }

    /// Write word to new stack at given privilege level
    /// Based on BX_CPU_C::write_new_stack_word in access.cc
    /// This writes to a stack segment at a different privilege level
    pub(super) fn write_new_stack_word(
        &mut self,
        seg: &super::descriptor::BxSegmentReg,
        addr: u32,
        _dpl: u8,
        value: u16,
    ) -> Result<()> {
        // Get linear address from new stack segment
        let seg_base = unsafe { seg.cache.u.segment.base };
        let laddr = (seg_base + addr as u64) & 0xFFFFFFFF;
        
        // Write through system memory access (bypasses normal checks)
        // For now, use direct memory write
        self.mem_write_word(laddr, value);
        Ok(())
    }

    /// Write dword to new stack at given privilege level
    /// Based on BX_CPU_C::write_new_stack_dword in access.cc
    /// This writes to a stack segment at a different privilege level
    pub(super) fn write_new_stack_dword(
        &mut self,
        seg: &super::descriptor::BxSegmentReg,
        addr: u32,
        _dpl: u8,
        value: u32,
    ) -> Result<()> {
        // Get linear address from new stack segment
        let seg_base = unsafe { seg.cache.u.segment.base };
        let laddr = (seg_base + addr as u64) & 0xFFFFFFFF;
        
        // Write through system memory access
        self.mem_write_dword(laddr, value);
        Ok(())
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

    /// Write byte to system address space (bypasses some checks)
    /// Based on BX_CPU_C::system_write_byte in access.cc
    pub(super) fn system_write_byte(&mut self, laddr: BxAddress, data: u8) -> Result<()> {
        // For now, use simple memory write
        self.mem_write_byte(laddr, data);
        Ok(())
    }

    /// Write word to system address space (bypasses some checks)
    /// Based on BX_CPU_C::system_write_word in access.cc:572
    pub(super) fn system_write_word(&mut self, laddr: BxAddress, data: u16) -> Result<()> {
        // For now, use simple memory write
        self.mem_write_word(laddr, data);
        Ok(())
    }

    /// Write dword to system address space (bypasses some checks)
    /// Based on BX_CPU_C::system_write_dword in access.cc:588
    pub(super) fn system_write_dword(&mut self, laddr: BxAddress, data: u32) -> Result<()> {
        // For now, use simple memory write
        self.mem_write_dword(laddr, data);
        Ok(())
    }

    /// Check code segment descriptor validity
    /// Based on BX_CPU_C::check_cs in ctrl_xfer_pro.cc:29
    pub(super) fn check_cs(
        &self,
        descriptor: &BxDescriptor,
        cs_raw: u16,
        check_rpl: u8,
        check_cpl: u8,
    ) -> Result<()> {
        use super::descriptor::{is_code_segment_non_conforming, is_data_segment};

        // Descriptor must be valid and a code segment
        if descriptor.valid == 0 || !descriptor.segment || is_data_segment(descriptor.r#type) {
            tracing::error!("check_cs({:#06x}): not a valid code segment!", cs_raw);
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
            });
        }

        // Non-conforming code segment: DPL must = CPL
        if is_code_segment_non_conforming(descriptor.r#type) {
            if descriptor.dpl != check_cpl {
                tracing::error!(
                    "check_cs({:#06x}): non-conforming code seg descriptor dpl != cpl, dpl={}, cpl={}",
                    cs_raw, descriptor.dpl, check_cpl
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                });
            }

            // RPL must be <= CPL
            if check_rpl > check_cpl {
                tracing::error!(
                    "check_cs({:#06x}): non-conforming code seg selector rpl > cpl, rpl={}, cpl={}",
                    cs_raw, check_rpl, check_cpl
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                });
            }
        } else {
            // Conforming code segment: DPL must be <= CPL
            if descriptor.dpl > check_cpl {
                tracing::error!(
                    "check_cs({:#06x}): conforming code seg descriptor dpl > cpl, dpl={}, cpl={}",
                    cs_raw, descriptor.dpl, check_cpl
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                });
            }
        }

        // Code segment must be present
        if !descriptor.p {
            tracing::error!("check_cs({:#06x}): code segment not present!", cs_raw);
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Np,
            });
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
    pub(super) fn jump_protected(
        &mut self,
        cs_raw: u16,
        disp: u64,
    ) -> Result<()> {
        tracing::trace!("jump_protected: cs={:#06x}, disp={:#010x}", cs_raw, disp);

        // Selector must not be null
        if (cs_raw & 0xFFFC) == 0 {
            tracing::error!("jump_protected: cs == 0");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
            });
        }

        // Parse selector
        let mut selector = BxSelector::default();
        parse_selector(cs_raw, &mut selector);

        tracing::info!("jump_protected: selector index={}, ti={}, rpl={}",
                      selector.index, selector.ti, selector.rpl);

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
            // System descriptor (call gate, task gate, TSS)
            // For now, return error - these are complex and rarely used during BIOS boot
            tracing::error!(
                "jump_protected: system descriptor type {:#x} not yet implemented",
                descriptor.r#type
            );
            Err(super::error::CpuError::UnimplementedOpcode {
                opcode: format!("jump_protected with system descriptor type {:#x}", descriptor.r#type),
            })
        }
    }

    /// Load segment register (handles both real and protected mode)
    /// Based on BX_CPU_C::load_seg_reg in segment_ctrl_pro.cc:28-177
    pub(super) fn load_seg_reg(&mut self, seg: BxSegregs, new_value: u16) -> Result<()> {
        if !self.real_mode() {
            // Protected mode
            if seg as usize == BxSegregs::Ss as usize {
                // Special handling for SS
                let mut selector = BxSelector::default();
                parse_selector(new_value, &mut selector);

                // Null selector check
                if (new_value & 0xfffc) == 0 {
                    tracing::error!("load_seg_reg(SS): loading null selector");
                    return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
                }

                // Fetch descriptor from GDT/LDT
                let (dword1, dword2) = self.fetch_raw_descriptor(&selector)?;

                // Check selector RPL must equal CPL
                let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
                if selector.rpl != cpl {
                    tracing::error!("load_seg_reg(SS): rpl != CPL");
                    return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
                }

                let mut descriptor = self.parse_descriptor(dword1, dword2)?;

                if descriptor.valid == 0 {
                    tracing::error!("load_seg_reg(SS): valid bit cleared");
                    return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
                }

                // AR byte must indicate a writable data segment
                if !descriptor.segment ||
                   descriptor.r#type >= 8 || // IS_CODE_SEGMENT
                   (descriptor.r#type & 2) == 0 // IS_DATA_SEGMENT_WRITEABLE
                {
                    tracing::error!("load_seg_reg(SS): not writable data segment");
                    return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
                }

                // DPL must equal CPL
                if descriptor.dpl != cpl {
                    tracing::error!("load_seg_reg(SS): dpl != CPL");
                    return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
                }

                // Segment must be PRESENT
                if !descriptor.p {
                    tracing::error!("load_seg_reg(SS): not present");
                    return Err(super::error::CpuError::BadVector { vector: Exception::Ss });
                }

                self.touch_segment(&selector, &mut descriptor)?;

                // Load SS with selector and descriptor (this sets D_B bit!)
                self.load_ss(&mut selector, &mut descriptor, cpl)?;

                tracing::debug!("load_seg_reg(SS): loaded selector {:#06x}, d_b={}",
                    new_value, unsafe { self.sregs[BxSegregs::Ss as usize].cache.u.segment.d_b });

                return Ok(());
            } else if matches!(seg, BxSegregs::Ds | BxSegregs::Es | BxSegregs::Fs | BxSegregs::Gs) {
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
                    tracing::error!("load_seg_reg({:?}, {:#06x}): invalid segment", seg, new_value);
                    return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
                }

                // AR byte must indicate data or readable code segment
                let is_code = descriptor.r#type >= 8;
                let is_readable = (descriptor.r#type & 2) != 0;
                if !descriptor.segment || (is_code && !is_readable) {
                    tracing::error!("load_seg_reg({:?}, {:#06x}): not data or readable code", seg, new_value);
                    return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
                }

                // If data or non-conforming code, RPL and CPL must be <= DPL
                let is_data = descriptor.r#type < 8;
                let is_conforming = (descriptor.r#type & 4) != 0;
                if is_data || !is_conforming {
                    let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
                    if selector.rpl > descriptor.dpl || cpl > descriptor.dpl {
                        tracing::error!("load_seg_reg({:?}, {:#06x}): RPL & CPL must be <= DPL", seg, new_value);
                        return Err(super::error::CpuError::BadVector { vector: Exception::Gp });
                    }
                }

                // Segment must be PRESENT
                if !descriptor.p {
                    tracing::error!("load_seg_reg({:?}, {:#06x}): segment not present", seg, new_value);
                    return Err(super::error::CpuError::BadVector { vector: Exception::Np });
                }

                self.touch_segment(&selector, &mut descriptor)?;

                // Load segment register with selector and descriptor
                let seg_idx = seg as usize;
                self.sregs[seg_idx].selector = selector;
                self.sregs[seg_idx].cache = descriptor;
                self.sregs[seg_idx].cache.valid = super::descriptor::SEG_VALID_CACHE;

                tracing::debug!("load_seg_reg({:?}): loaded selector {:#06x}", seg, new_value);

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

        tracing::debug!("load_null_selector({:?}): selector {:#06x}, cleared all cache fields", seg, value);
    }

    /// LLDT - Load Local Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:374-476
    pub(super) fn lldt_ew(&mut self, instr: &super::decoder::BxInstructionGenerated) -> Result<()> {
        // Must be in protected mode
        if self.real_mode() {
            tracing::error!("LLDT: not recognized in real mode");
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
        if descriptor.valid == 0 || descriptor.segment
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
    pub(super) fn ltr_ew(&mut self, instr: &super::decoder::BxInstructionGenerated) -> Result<()> {
        // Must be in protected mode
        if self.real_mode() {
            tracing::error!("LTR: not recognized in real mode");
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
        if descriptor.valid == 0 || descriptor.segment
            || (tss_type != SystemAndGateDescriptorEnum::BxSysSegmentAvail286Tss as u8
                && tss_type != SystemAndGateDescriptorEnum::BxSysSegmentAvail386Tss as u8)
        {
            tracing::error!("LTR: doesn't point to an available TSS descriptor! type={}", tss_type);
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
        let gdt_offset = self.gdtr.base + (selector_index as u64 * 8) + 4;
        let new_dword2 = dword2 | 0x0200; // Set busy bit in access byte
        let phys_addr = self.translate_linear_system_read(gdt_offset)?;
        self.mem_write_dword(phys_addr, new_dword2);

        tracing::trace!("LTR: loaded selector={:#06x}, marked busy", raw_selector);
        Ok(())
    }
}
