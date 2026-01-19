use super::{
    cpu::Exception,
    decoder::BxSegregs,
    descriptor::{BxDescriptor, BxSelector, DescriptorGate, DescriptorSegment, DescriptorTaskGate},
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
            let limit = (dword1 & 0xFFFF) | ((dword2 & 0x000F0000) << 16);
            let mut base = ((dword1 >> 16) as u64) | (((dword2 & 0xFF) as u64) << 16);
            base |= ((dword2 & 0xFF000000) as u64) << 8;
            
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
                    let limit = (dword1 & 0xFFFF) | ((dword2 & 0x000F0000) << 16);
                    let mut base = ((dword1 >> 16) as u64) | (((dword2 & 0xFF) as u64) << 16);
                    base |= ((dword2 & 0xFF000000) as u64) << 8;
                    
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

    /// Read qword from system address space (bypasses some checks)
    /// Based on BX_CPU_C::system_read_qword in access.cc:617
    pub(super) fn system_read_qword(&self, laddr: BxAddress) -> Result<u64> {
        // For now, use simple memory read - in full implementation this would
        // go through TLB and address translation
        let lo = self.mem_read_dword(laddr) as u64;
        let hi = self.mem_read_dword(laddr + 4) as u64;
        Ok(lo | (hi << 32))
    }

    /// Read word from system address space (bypasses some checks)
    /// Based on BX_CPU_C::system_read_word in access.cc:585
    pub(super) fn system_read_word(&self, laddr: BxAddress) -> Result<u16> {
        // For now, use simple memory read
        Ok(self.mem_read_word(laddr))
    }

    /// Read dword from system address space (bypasses some checks)
    /// Based on BX_CPU_C::system_read_dword in access.cc:600
    pub(super) fn system_read_dword(&self, laddr: BxAddress) -> Result<u32> {
        // For now, use simple memory read
        Ok(self.mem_read_dword(laddr))
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
}
