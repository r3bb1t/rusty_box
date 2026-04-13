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
    /// Based on BX_CPU_C::fetch_raw_descriptor in segment_ctrl_pro.cc
    pub(super) fn fetch_raw_descriptor(&mut self, selector: &BxSelector) -> Result<(u32, u32)> {
        let index = selector.index as u32;
        let offset: BxAddress = if selector.ti == 0 {
            // GDT
            let index_offset = index * 8 + 7;
            if index_offset > self.gdtr.limit as u32 {
                tracing::debug!(
                    "fetch_raw_descriptor: GDT: index ({}) {} > limit ({}) GDTR.base={:#x}",
                    index_offset,
                    index,
                    self.gdtr.limit,
                    self.gdtr.base
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: (selector.value & 0xfffc),
                });
            }
            self.gdtr.base + (index as u64 * 8)
        } else {
            // LDT
            if self.ldtr.cache.valid == 0 {
                tracing::debug!("fetch_raw_descriptor: LDTR.valid=0");
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: (selector.value & 0xfffc),
                });
            }
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            let ldt_limit = self.ldtr.cache.u.segment_limit_scaled();
            let index_offset = index * 8 + 7;
            if index_offset > ldt_limit {
                tracing::debug!(
                    "fetch_raw_descriptor: LDT: index ({}) {} > limit ({})",
                    index_offset,
                    index,
                    ldt_limit
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: (selector.value & 0xfffc),
                });
            }
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            self.ldtr.cache.u.segment_base() + (index as u64 * 8)
        };

        // Read descriptor as qword (64 bits = 2 dwords)
        let qword = self.system_read_qword(offset)?;
        let dword1 = (qword & 0xFFFFFFFF) as u32;
        let dword2 = ((qword >> 32) & 0xFFFFFFFF) as u32;

        Ok((dword1, dword2))
    }

    /// Parse descriptor from two dwords
    /// Based on parse_descriptor in segment_ctrl_pro.cc
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
            u: super::descriptor::Descriptor::Segment(DescriptorSegment {
                base: 0,
                limit_scaled: 0,
                g: false,
                d_b: false,
                l: false,
                avl: false,
            }),
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

            descriptor.u = super::descriptor::Descriptor::Segment(DescriptorSegment {
                base,
                limit_scaled,
                g,
                d_b,
                l: (dword2 & 0x00200000) != 0,
                avl,
            });

            descriptor.valid = super::descriptor::SEG_VALID_CACHE;
        } else {
            // System/gate descriptor
            match r#type {
                0x4 | 0x6 | 0x7 => {
                    // 286 call/interrupt/trap gate
                    let param_count = (dword2 & 0x1F) as u8;
                    let dest_selector = (dword1 >> 16) as u16;
                    let dest_offset = dword1 & 0xFFFF ;

                    descriptor.u = super::descriptor::Descriptor::Gate(DescriptorGate {
                        param_count,
                        dest_selector,
                        dest_offset,
                    });
                    descriptor.valid = super::descriptor::SEG_VALID_CACHE;
                }
                0xC | 0xE | 0xF => {
                    // 386 call/interrupt/trap gate
                    let param_count = (dword2 & 0x1F) as u8;
                    let dest_selector = (dword1 >> 16) as u16;
                    let dest_offset = (dword2 & 0xFFFF0000) | (dword1 & 0xFFFF) ;

                    descriptor.u = super::descriptor::Descriptor::Gate(DescriptorGate {
                        param_count,
                        dest_selector,
                        dest_offset,
                    });
                    descriptor.valid = super::descriptor::SEG_VALID_CACHE;
                }
                0x5 => {
                    // Task gate
                    let tss_selector = (dword1 >> 16) as u16;
                    descriptor.u = super::descriptor::Descriptor::TaskGate(DescriptorTaskGate { tss_selector });
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

                    descriptor.u = super::descriptor::Descriptor::Segment(DescriptorSegment {
                        base,
                        limit_scaled,
                        g,
                        d_b,
                        l: false,
                        avl,
                    });
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
    /// Based on BX_CPU_C::get_SS_ESP_from_TSS in tasking.cc
    pub(super) fn get_ss_esp_from_tss(&mut self, pl: u8) -> Result<(u16, u32)> {
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
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            let limit_scaled = self.tr.cache.u.segment_limit_scaled();
            if (tss_stackaddr + 7) > limit_scaled {
                tracing::error!("get_ss_esp_from_tss(386): TSSstackaddr > TSS.LIMIT");
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Ts,
                    error_code: 0,
                });
            }
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            let tss_base = self.tr.cache.u.segment_base();
            let ss = self.system_read_word(tss_base + tss_stackaddr as u64 + 4)?;
            let esp = self.system_read_dword(tss_base + tss_stackaddr as u64)?;
            Ok((ss, esp))
        } else if tss_type == 0x1 || tss_type == 0x3 {
            // 16-bit TSS
            let tss_stackaddr = (4 * pl as u32) + 2;
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            let limit_scaled = self.tr.cache.u.segment_limit_scaled();
            if (tss_stackaddr + 3) > limit_scaled {
                tracing::error!("get_ss_esp_from_tss(286): TSSstackaddr > TSS.LIMIT");
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Ts,
                    error_code: 0,
                });
            }
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            let tss_base = self.tr.cache.u.segment_base();
            let ss = self.system_read_word(tss_base + tss_stackaddr as u64 + 2)?;
            let esp = self.system_read_word(tss_base + tss_stackaddr as u64)? as u32;
            Ok((ss, esp))
        } else {
            tracing::error!("get_ss_esp_from_tss: TR is bogus type ({:#x})", tss_type);
            Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: 0,
            })
        }
    }

    /// Get RSP from TSS for a given privilege level (64-bit long mode).
    /// Based on BX_CPU_C::get_RSP_from_TSS in tasking.cc
    pub(super) fn get_rsp_from_tss(&mut self, pl: u8) -> Result<u64> {
        if self.tr.cache.valid == 0 {
            tracing::error!("get_rsp_from_tss: TR.cache invalid");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Ts,
                error_code: 0,
            });
        }

        // 64-bit TSS: RSP fields at offsets 4, 12, 20 for PL 0, 1, 2
        let tss_stackaddr = (8 * pl as u32) + 4;
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        let limit_scaled = self.tr.cache.u.segment_limit_scaled();
        if (tss_stackaddr + 7) > limit_scaled {
            tracing::debug!("get_rsp_from_tss: TSSstackaddr > TSS.LIMIT");
            let err_code = self.tr.selector.value & 0xfffc;
            self.exception(Exception::Ts, err_code)?;
            unreachable!("exception() always returns Err");
        }

        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        let tss_base = self.tr.cache.u.segment_base();
        let rsp = self.system_read_qword(tss_base + tss_stackaddr as u64)?;

        if !self.is_canonical(rsp) {
            tracing::error!("get_rsp_from_tss: canonical address failure {:#018x}", rsp);
            let err_code = self.sregs[BxSegregs::Ss as usize].selector.value & 0xfffc;
            self.exception(Exception::Ss, err_code)?;
            unreachable!("exception() always returns Err");
        }

        Ok(rsp)
    }

    /// Fetch 16-byte (128-bit) raw descriptor from GDT/LDT for 64-bit system descriptors.
    /// Returns (dword1, dword2, dword3) where dword3 is the upper 32 bits of the 64-bit base.
    /// Based on BX_CPU_C::fetch_raw_descriptor_64 in segment_ctrl_pro.cc
    pub(super) fn fetch_raw_descriptor_64(&mut self, selector: &BxSelector) -> Result<(u32, u32, u32)> {
        let index = selector.index as u32;
        let offset: u64 = if selector.ti == 0 {
            // GDT — need 16 bytes (index*8 + 15)
            let index_offset = index * 8 + 15;
            if index_offset > self.gdtr.limit as u32 {
                tracing::error!(
                    "fetch_raw_descriptor_64: GDT: index ({}) {} > limit ({})",
                    index_offset,
                    index,
                    self.gdtr.limit
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: selector.value & 0xfffc,
                });
            }
            self.gdtr.base + (index as u64 * 8)
        } else {
            // LDT
            if self.ldtr.cache.valid == 0 {
                tracing::error!("fetch_raw_descriptor_64: LDTR.valid=0");
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: selector.value & 0xfffc,
                });
            }
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            let ldt_limit = self.ldtr.cache.u.segment_limit_scaled();
            let index_offset = index * 8 + 15;
            if index_offset > ldt_limit {
                tracing::error!(
                    "fetch_raw_descriptor_64: LDT: index ({}) {} > limit ({})",
                    index_offset,
                    index,
                    ldt_limit
                );
                return Err(super::error::CpuError::BadVector {
                    vector: Exception::Gp,
                    error_code: selector.value & 0xfffc,
                });
            }
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            self.ldtr.cache.u.segment_base() + (index as u64 * 8)
        };

        // Read two qwords (16 bytes total = 128-bit descriptor)
        let raw_descriptor1 = self.system_read_qword(offset)?;
        let raw_descriptor2 = self.system_read_qword(offset + 8)?;

        // Check that extended attributes in dword4 don't have type bits set
        if raw_descriptor2 & 0x00001F0000000000u64 != 0 {
            tracing::error!("fetch_raw_descriptor_64: extended attributes DWORD4 TYPE != 0");
            return Err(super::error::CpuError::BadVector {
                vector: Exception::Gp,
                error_code: selector.value & 0xfffc,
            });
        }

        let dword1 = (raw_descriptor1 & 0xFFFFFFFF) as u32;
        let dword2 = ((raw_descriptor1 >> 32) & 0xFFFFFFFF) as u32;
        let dword3 = (raw_descriptor2 & 0xFFFFFFFF) as u32;

        Ok((dword1, dword2, dword3))
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
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        let seg_base = seg.cache.u.segment_base();
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
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        let seg_base = seg.cache.u.segment_base();
        let laddr = (seg_base + addr as u64) & 0xFFFFFFFF;
        self.system_write_dword(laddr, value)
    }

    /// Write qword to new stack at given privilege level (64-bit linear address variant).
    ///
    /// Based on BX_CPU_C::write_new_stack_qword(bx_address laddr, ...) in access.cc.
    /// Used in long mode where the linear address is 64-bit and there is no segment base.
    pub(super) fn write_new_stack_qword_64(
        &mut self,
        laddr: u64,
        _dpl: u8,
        value: u64,
    ) -> Result<()> {
        self.system_write_qword(laddr, value)
    }

    /// Load SS segment register
    /// Based on BX_CPU_C::load_ss in segment_ctrl_pro.cc
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
    /// Based on BX_CPU_C::touch_segment in segment_ctrl_pro.cc
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
                // SAFETY: segment cache populated during segment load; union read matches descriptor type
                let ldt_base = self.ldtr.cache.u.segment_base();
                ldt_base + (selector.index as u64 * 8) + 5
            };

            self.system_write_byte(offset, ar_byte)?;
        }

        Ok(())
    }

    // system_write_byte/word/dword are defined in access.rs

    /// Check code segment descriptor validity
    /// Based on BX_CPU_C::check_cs in ctrl_xfer_pro.cc
    pub(super) fn check_cs(
        &mut self,
        descriptor: &BxDescriptor,
        cs_raw: u16,
        check_rpl: u8,
        check_cpl: u8,
    ) -> Result<()> {
        use super::descriptor::{is_code_segment_non_conforming, is_data_segment};
        // Mirrors Bochs ctrl_xfer_pro.cc — calls exception() directly with cs_raw & 0xfffc

        // Descriptor must be valid and a code segment
        if descriptor.valid == 0 || !descriptor.segment || is_data_segment(descriptor.r#type) {
            tracing::error!("check_cs({:#06x}): not a valid code segment!", cs_raw);
            return self.exception(Exception::Gp, cs_raw & 0xfffc);
        }

        // Bochs ctrl_xfer_pro.cc — L+D_B both set is invalid in long mode
        if self.long_mode()
            && descriptor.u.segment_l() && descriptor.u.segment_d_b() {
                tracing::error!(
                    "check_cs({:#06x}): Both CS.L and CS.D_B bits enabled!",
                    cs_raw
                );
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
    /// Based on BX_CPU_C::load_cs in ctrl_xfer_pro.cc
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

        // Verify consistency: CS.val RPL bits must match CS.rpl field
        debug_assert_eq!(
            self.sregs[BxSegregs::Cs as usize].selector.value & 3,
            self.sregs[BxSegregs::Cs as usize].selector.rpl as u16,
            "load_cs: CS.val RPL bits ({}) != CS.rpl field ({}) after load!",
            self.sregs[BxSegregs::Cs as usize].selector.value & 3,
            self.sregs[BxSegregs::Cs as usize].selector.rpl
        );

        // Bochs ctrl_xfer_pro.cc — handleCpuModeChange() in long mode
        // Updates cpu_mode (Long64 vs LongCompat) based on CS.L bit
        if self.long_mode() {
            self.handle_cpu_mode_change();
        }

        // Bochs ctrl_xfer_pro.cc — updateFetchModeMask() after CS load.
        // Updates icache hash (d_b, long64) and user_pl.
        self.update_fetch_mode_mask();

        // Bochs ctrl_xfer_pro.cc — handleAlignmentCheck() after CS load.
        self.handle_alignment_check();

        // Invalidate prefetch queue
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        Ok(())
    }

    /// Branch to far code segment
    /// Based on BX_CPU_C::branch_far in ctrl_xfer_pro.cc
    pub(super) fn branch_far(
        &mut self,
        selector: &mut BxSelector,
        descriptor: &mut BxDescriptor,
        rip: u64,
        cpl: u8,
    ) -> Result<()> {
        // Bochs ctrl_xfer_pro.cc
        // In long mode with a 64-bit code segment, do canonical check instead of limit check
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        if self.long_mode() && descriptor.u.segment_l() {
            if !self.is_canonical(rip) {
                tracing::error!("branch_far: canonical RIP violation {:#018x}", rip);
                return self.exception(Exception::Gp, 0);
            }
        } else {
            // Legacy mode: mask RIP to 32 bits and check segment limit
            let rip_masked = rip & 0xFFFFFFFF;
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            let limit = descriptor.u.segment_limit_scaled();
            if rip_masked as u32 > limit {
                tracing::error!(
                    "branch_far: RIP {:#010x} > limit {:#010x}",
                    rip_masked,
                    limit
                );
                return self.exception(Exception::Gp, 0);
            }
        }

        // Load CS with new descriptor
        self.load_cs(selector, descriptor, cpl)?;

        // Update RIP
        // In long mode with L=1, RIP is full 64-bit; otherwise mask to 32 bits
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        if self.long_mode() && descriptor.u.segment_l() {
            self.set_rip(rip);
        } else {
            self.set_rip(rip & 0xFFFFFFFF);
        }

        Ok(())
    }

    /// Jump to protected mode code segment
    /// Based on BX_CPU_C::jump_protected in jmp_far.cc
    pub(super) fn jump_protected(&mut self, cs_raw: u16, disp: u64) -> Result<()> {


        // Selector must not be null
        if (cs_raw & 0xFFFC) == 0 {
            tracing::debug!("jump_protected: null selector cs={:#06x}", cs_raw);
            return self.exception(Exception::Gp, 0);
        }

        // Parse selector
        let mut selector = BxSelector::default();
        parse_selector(cs_raw, &mut selector);

        tracing::debug!(
            "jump_protected: selector index={}, ti={}, rpl={}, GDTR base={:#010x} limit={:#06x}",
            selector.index,
            selector.ti,
            selector.rpl,
            self.gdtr.base,
            self.gdtr.limit
        );

        // Fetch descriptor from GDT/LDT
        let (dword1, dword2) = match self.fetch_raw_descriptor(&selector) {
            Ok(v) => v,
            Err(super::error::CpuError::BadVector { vector, error_code }) => {
                return self.exception(vector, error_code);
            }
            Err(e) => return Err(e),
        };
        let mut descriptor = self.parse_descriptor(dword1, dword2)?;

        tracing::info!("jump_protected: descriptor segment={}, type={:#x}, dpl={}, p={}, base={:#010x}, limit={:#010x}",
                      descriptor.segment, descriptor.r#type, descriptor.dpl, descriptor.p,
                      descriptor.u.segment_base(), descriptor.u.segment_limit_scaled());

        if descriptor.segment {
            // Code segment descriptor
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            self.check_cs(&descriptor, cs_raw, selector.rpl, cpl)?;
            self.branch_far(&mut selector, &mut descriptor, disp, cpl)?;
            Ok(())
        } else {
            // System descriptor — Based on Bochs jmp_far.cc
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

            // In long mode, only 386 call gates (type 0xC) are allowed
            // Bochs jmp_far.cc
            if self.long_mode() {
                if descriptor.r#type != 0xC {
                    tracing::error!(
                        "jump_protected: gate type {:#x} unsupported in long mode",
                        descriptor.r#type
                    );
                    return self.exception(Exception::Gp, cs_raw & 0xfffc);
                }
                if !descriptor.p {
                    tracing::error!("jump_protected: call gate not present!");
                    return self.exception(Exception::Np, cs_raw & 0xfffc);
                }
                return self.jmp_call_gate64(&selector);
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
    /// Based on BX_CPU_C::load_seg_reg in segment_ctrl_pro.cc
    pub(super) fn load_seg_reg(&mut self, seg: BxSegregs, new_value: u16) -> Result<()> {
        // V8086 mode: use real-mode style loading (Bochs segment_ctrl_pro.cc)
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

                let err_code = new_value & 0xFFFC;

                // Null selector check — Bochs segment_ctrl_pro.cc
                if (new_value & 0xfffc) == 0 {
                    // Bochs: long64 mode allows null SS when CPL != 3 and RPL == CPL
                    if self.long64_mode() {
                        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
                        if cpl != 3 && selector.rpl == cpl {
                            self.load_null_selector(seg, new_value);
                            return Ok(());
                        }
                    }
                    tracing::error!("load_seg_reg(SS): loading null selector");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: err_code,
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
                        error_code: err_code,
                    });
                }

                let mut descriptor = self.parse_descriptor(dword1, dword2)?;

                if descriptor.valid == 0 {
                    tracing::error!("load_seg_reg(SS): valid bit cleared");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: err_code,
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
                        error_code: err_code,
                    });
                }

                // DPL must equal CPL
                if descriptor.dpl != cpl {
                    tracing::error!("load_seg_reg(SS): dpl != CPL");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: err_code,
                    });
                }

                // Segment must be PRESENT
                if !descriptor.p {
                    tracing::error!("load_seg_reg(SS): not present");
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Ss,
                        error_code: err_code,
                    });
                }

                self.touch_segment(&selector, &mut descriptor)?;

                // Load SS with selector and descriptor (this sets D_B bit!)
                self.load_ss(&mut selector, &mut descriptor, cpl)?;

                tracing::debug!(
                    "load_seg_reg(SS): loaded selector {:#06x}, d_b={}",
                    new_value,
                    self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b()
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
                let err_code = new_value & 0xFFFC;

                if descriptor.valid == 0 {
                    tracing::error!(
                        "load_seg_reg({:?}, {:#06x}): invalid segment",
                        seg,
                        new_value
                    );
                    return Err(super::error::CpuError::BadVector {
                        vector: Exception::Gp,
                        error_code: err_code,
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
                        error_code: err_code,
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
                            error_code: err_code,
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
                        error_code: err_code,
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
    /// Based on BX_CPU_C::load_null_selector in segment_ctrl_pro.cc
    pub(super) fn load_null_selector(&mut self, seg: BxSegregs, value: u16) {
        let seg_idx = seg as usize;

        // Set selector fields
        self.sregs[seg_idx].selector.value = value;
        self.sregs[seg_idx].selector.index = 0;
        self.sregs[seg_idx].selector.ti = 0;
        self.sregs[seg_idx].selector.rpl = (value & 3) as u8;

        // Clear cache - Bochs segment_ctrl_pro.cc
        self.sregs[seg_idx].cache.valid = 0; // Invalidate null selector
        self.sregs[seg_idx].cache.p = false;
        self.sregs[seg_idx].cache.dpl = 0;
        self.sregs[seg_idx].cache.segment = true; // Data/code segment
        self.sregs[seg_idx].cache.r#type = 0;

        // Zero segment descriptor fields — Bochs segment_ctrl_pro.cc
        self.sregs[seg_idx].cache.u.set_segment_base(0);
        self.sregs[seg_idx].cache.u.set_segment_limit_scaled(0);
        self.sregs[seg_idx].cache.u.set_segment_g(false);
        self.sregs[seg_idx].cache.u.set_segment_d_b(false);
        self.sregs[seg_idx].cache.u.set_segment_l(false);
        self.sregs[seg_idx].cache.u.set_segment_avl(false);

        // Bochs segment_ctrl_pro.cc — invalidate stack cache after null SS load
        if seg == BxSegregs::Ss {
            self.invalidate_stack_cache();
        }

        tracing::debug!(
            "load_null_selector({:?}): selector {:#06x}, cleared all cache fields",
            seg,
            value
        );
    }

    /// LLDT - Load Local Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc
    pub(super) fn lldt_ew(&mut self, instr: &super::decoder::Instruction) -> Result<()> {
        // Must be in protected mode (catches both real mode and v8086)
        // Based on Bochs protect_ctrl.cc
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
        // Group 6 (0F 00): dst()=rm (Bochs: i->dst())
        let raw_selector = if instr.mod_c0() {
            self.get_gpr16(instr.dst() as usize)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_word(seg, eaddr)?
        };

        // If selector is NULL, invalidate and done
        if (raw_selector & 0xfffc) == 0 {
            self.ldtr.selector.value = raw_selector;
            self.ldtr.cache.valid = 0;

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
        // In long mode, system descriptors are 16 bytes (Bochs protect_ctrl.cc)
        let mut dword3 = 0u32;
        let (dword1, dword2) = if self.long64_mode() {
            let (d1, d2, d3) = self.fetch_raw_descriptor_64(&selector)?;
            dword3 = d3;
            (d1, d2)
        } else {
            self.fetch_raw_descriptor(&selector)?
        };

        // Parse descriptor
        let mut descriptor = self.parse_descriptor(dword1, dword2)?;

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

        // In long mode, extend base to 64 bits and check canonical
        // Bochs protect_ctrl.cc
        if self.long64_mode() {
            descriptor.u.set_segment_base(descriptor.u.segment_base() | (dword3 as u64) << 32);
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            if !self.is_canonical(descriptor.u.segment_base()) {
                tracing::error!("LLDT: non-canonical LDT descriptor base!");
                self.exception(Exception::Gp, raw_selector & 0xfffc)?;
                return Ok(());
            }
        }

        // Load LDTR
        self.ldtr.selector = selector;
        self.ldtr.cache = descriptor;
        self.ldtr.cache.valid = SEG_VALID_CACHE;


        Ok(())
    }

    /// LTR - Load Task Register
    /// Based on Bochs protect_ctrl.cc
    pub(super) fn ltr_ew(&mut self, instr: &super::decoder::Instruction) -> Result<()> {
        // Must be in protected mode (catches both real mode and v8086)
        // Based on Bochs protect_ctrl.cc
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
        // Group 6 (0F 00): dst()=rm (Bochs: i->dst())
        let raw_selector = if instr.mod_c0() {
            self.get_gpr16(instr.dst() as usize)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_word(seg, eaddr)?
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
        // In long mode, system descriptors are 16 bytes (Bochs protect_ctrl.cc)
        let mut dword3 = 0u32;
        let (dword1, dword2) = if self.long64_mode() {
            let (d1, d2, d3) = self.fetch_raw_descriptor_64(&selector)?;
            dword3 = d3;
            (d1, d2)
        } else {
            self.fetch_raw_descriptor(&selector)?
        };

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

        // In long mode, must be 386 TSS (Bochs protect_ctrl.cc)
        if self.long_mode()
            && tss_type != SystemAndGateDescriptorEnum::BxSysSegmentAvail386Tss as u8
        {
            tracing::error!("LTR: doesn't point to an available TSS386 descriptor in long mode!");
            self.exception(Exception::Gp, raw_selector & 0xfffc)?;
            return Ok(());
        }

        // Check if present
        if !descriptor.p {
            tracing::error!("LTR: TSS descriptor not present!");
            self.exception(Exception::Np, raw_selector & 0xfffc)?;
            return Ok(());
        }

        // In long mode, extend base to 64 bits and check canonical
        // Bochs protect_ctrl.cc
        if self.long64_mode() {
            descriptor.u.set_segment_base(descriptor.u.segment_base() | (dword3 as u64) << 32);
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            if !self.is_canonical(descriptor.u.segment_base()) {
                tracing::error!("LTR: non-canonical TSS descriptor base!");
                self.exception(Exception::Gp, raw_selector & 0xfffc)?;
                return Ok(());
            }
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
        // Based on Bochs protect_ctrl.cc — uses system_write
        let gdt_offset = self.gdtr.base + (selector_index as u64 * 8) + 4;
        let new_dword2 = dword2 | 0x0200; // Set busy bit in access byte
        let phys_addr = self.translate_linear_system_write(gdt_offset)?;
        self.mem_write_dword(phys_addr, new_dword2);


        Ok(())
    }

    // =========================================================================
    // validate_seg_regs — invalidate DS/ES/FS/GS on privilege change
    // Based on Bochs segment_ctrl_pro.cc
    // =========================================================================

    /// Invalidate a single segment register if DPL < CPL and type is data or non-conforming code
    fn validate_seg_reg(&mut self, seg: usize) {
        use super::descriptor::{is_code_segment_non_conforming, is_data_segment};
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        let cache = &self.sregs[seg].cache;
        if cache.dpl < cpl
            && (cache.valid == 0
                || !cache.segment
                || is_data_segment(cache.r#type)
                || is_code_segment_non_conforming(cache.r#type))
            {
                self.sregs[seg].selector.value = 0;
                self.sregs[seg].cache.valid = 0;
            }
    }

    /// Validate ES/DS/FS/GS after privilege level change
    /// Based on Bochs segment_ctrl_pro.cc
    pub(super) fn validate_seg_regs(&mut self) {
        self.validate_seg_reg(BxSegregs::Es as usize);
        self.validate_seg_reg(BxSegregs::Ds as usize);
        self.validate_seg_reg(BxSegregs::Fs as usize);
        self.validate_seg_reg(BxSegregs::Gs as usize);
    }

    // =========================================================================
    // call_protected — protected mode far CALL
    // Based on Bochs call_far.cc
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
        // SAFETY: descriptor type verified as gate before union access
        let gate_cs_raw = gate_descriptor.u.gate_dest_selector();
        // SAFETY: descriptor type verified as gate before union access
        let new_eip = gate_descriptor.u.gate_dest_offset();

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

            // SAFETY: descriptor type verified as gate before union access
            let param_count = gate_descriptor.u.gate_param_count() & 0x1f;

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

            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            if ss_descriptor.u.segment_d_b() {
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
                            self.stack_read_word(return_esp.wrapping_add((n - 1) * 2))?;
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
                            self.stack_read_word(return_esp.wrapping_add((n - 1) * 2))?;
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

        // SAFETY: descriptor type verified as task gate before union access
        let raw_tss_selector = gate_descriptor.u.task_gate_tss_selector();
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
    // Based on Bochs ret_far.cc
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

            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            if ss_descriptor.u.segment_d_b() {
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
    // Based on Bochs jmp_far.cc jmp_call_gate()
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

        // SAFETY: descriptor type verified as gate before union access
        let gate_cs_raw = gate_descriptor.u.gate_dest_selector();
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

        // SAFETY: descriptor type verified as gate before union access
        let temp_eip = gate_descriptor.u.gate_dest_offset();
        self.branch_far(
            &mut gate_cs_selector,
            &mut cs_descriptor,
            temp_eip as u64,
            cpl,
        )?;
        Ok(())
    }

    // =========================================================================
    // JMP call gate 64-bit — JMP through a 64-bit call gate (long mode)
    // Based on Bochs jmp_far.cc jmp_call_gate64()
    // =========================================================================

    fn jmp_call_gate64(&mut self, gate_selector: &BxSelector) -> Result<()> {
        use super::descriptor::is_data_segment;

        tracing::debug!("jmp_call_gate64: jump to CALL GATE 64");

        let (dword1, dword2, dword3) = self.fetch_raw_descriptor_64(gate_selector)?;
        let gate_descriptor = self.parse_descriptor(dword1, dword2)?;

        // SAFETY: descriptor type verified as gate before union access
        let dest_selector = gate_descriptor.u.gate_dest_selector();
        // selector must not be null else #GP(0)
        if (dest_selector & 0xfffc) == 0 {
            tracing::error!("jmp_call_gate64: selector in gate null");
            return self.exception(Exception::Gp, 0);
        }

        let mut cs_selector = BxSelector::default();
        parse_selector(dest_selector, &mut cs_selector);

        let (dw1, dw2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, dest_selector & 0xfffc),
        };
        let mut cs_descriptor = self.parse_descriptor(dw1, dw2)?;

        // Find the RIP from the gate_descriptor: high 32 bits from dword3, low 32 from gate offset
        // SAFETY: descriptor type verified as gate before union access
        let gate_offset_lo = gate_descriptor.u.gate_dest_offset();
        let new_rip = ((dword3 as u64) << 32) | (gate_offset_lo as u64);

        // AR byte of selected descriptor must indicate code segment, else #GP(code segment selector)
        if cs_descriptor.valid == 0
            || !cs_descriptor.segment
            || is_data_segment(cs_descriptor.r#type)
        {
            tracing::error!("jmp_call_gate64: not code segment in 64-bit call gate");
            return self.exception(Exception::Gp, dest_selector & 0xfffc);
        }

        // In long mode, only 64-bit call gates are allowed, and they must point
        // to 64-bit code segments (L=1, D=0), else #GP(selector)
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        if !cs_descriptor.is_long64_segment() || cs_descriptor.u.segment_d_b() {
            tracing::error!("jmp_call_gate64: not 64-bit code segment in 64-bit call gate");
            return self.exception(Exception::Gp, dest_selector & 0xfffc);
        }

        // check code-segment descriptor
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        self.check_cs(&cs_descriptor, dest_selector, 0, cpl)?;

        // and transfer the control
        self.branch_far(&mut cs_selector, &mut cs_descriptor, new_rip, cpl)?;
        Ok(())
    }

    // =========================================================================
    // Task gate for JMP — JMP through a task gate
    // Based on Bochs jmp_far.cc task_gate()
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

        // SAFETY: descriptor type verified as task gate before union access
        let raw_tss_selector = gate_descriptor.u.task_gate_tss_selector();
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

    // =========================================================================
    // 64-bit long mode: call_protected_64
    // Based on Bochs call_far.cc (the long_mode path)
    // =========================================================================

    /// Protected mode far call for 64-bit mode.
    /// Based on Bochs call_far.cc call_protected() with long_mode() paths.
    ///
    /// In long mode, the call_protected function handles:
    /// 1. Normal code segment: push return CS:RIP, branch_far
    /// 2. 64-bit call gates (type 0xC only allowed in long mode)
    pub(super) fn call_protected_64(
        &mut self,
        instr: &super::decoder::Instruction,
        cs_raw: u16,
        disp: u64,
    ) -> Result<()> {
        if (cs_raw & 0xfffc) == 0 {
            tracing::debug!("call_protected_64: CS selector null");
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
            tracing::error!("call_protected_64: invalid CS descriptor");
            return self.exception(Exception::Gp, cs_raw & 0xfffc);
        }

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;

        if cs_descriptor.segment {
            // Normal code segment
            self.check_cs(&cs_descriptor, cs_raw, cs_selector.rpl, cpl)?;

            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            if self.long_mode() && cs_descriptor.u.segment_l() {
                // Moving to 64-bit long mode code segment — use 64-bit RSP directly
                let temp_rsp = self.rsp();

                if instr.os64_l() != 0 {
                    self.write_new_stack_qword_64(
                        temp_rsp.wrapping_sub(8),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Cs as usize].selector.value as u64,
                    )?;
                    self.write_new_stack_qword_64(
                        temp_rsp.wrapping_sub(16),
                        cs_descriptor.dpl,
                        self.rip(),
                    )?;
                    self.branch_far(&mut cs_selector, &mut cs_descriptor, disp, cpl)?;
                    self.set_rsp(temp_rsp.wrapping_sub(16));
                } else if instr.os32_l() != 0 {
                    self.write_new_stack_qword_64(
                        temp_rsp.wrapping_sub(4),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Cs as usize].selector.value as u64,
                    )?;
                    self.write_new_stack_qword_64(
                        temp_rsp.wrapping_sub(8),
                        cs_descriptor.dpl,
                        self.eip() as u64,
                    )?;
                    self.branch_far(&mut cs_selector, &mut cs_descriptor, disp, cpl)?;
                    self.set_rsp(temp_rsp.wrapping_sub(8));
                } else {
                    self.write_new_stack_qword_64(
                        temp_rsp.wrapping_sub(2),
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Cs as usize].selector.value as u64,
                    )?;
                    self.write_new_stack_qword_64(
                        temp_rsp.wrapping_sub(4),
                        cs_descriptor.dpl,
                        self.get_ip() as u64,
                    )?;
                    self.branch_far(&mut cs_selector, &mut cs_descriptor, disp, cpl)?;
                    self.set_rsp(temp_rsp.wrapping_sub(4));
                }
            } else {
                // Legacy mode within long mode (compatibility sub-mode)
                // SAFETY: segment cache populated during segment load; union read matches descriptor type
                let temp_rsp = if self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b()
                {
                    self.esp() as u64
                } else {
                    self.sp() as u64
                };

                let ss_seg = self.sregs[BxSegregs::Ss as usize].clone();

                if instr.os32_l() != 0 {
                    self.write_new_stack_dword(
                        &ss_seg,
                        temp_rsp.wrapping_sub(4) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Cs as usize].selector.value as u32,
                    )?;
                    self.write_new_stack_dword(
                        &ss_seg,
                        temp_rsp.wrapping_sub(8) as u32,
                        cs_descriptor.dpl,
                        self.eip(),
                    )?;
                    self.branch_far(&mut cs_selector, &mut cs_descriptor, disp, cpl)?;
                    // SAFETY: segment cache populated during segment load; union read matches descriptor type
                    if self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b() {
                        self.set_esp(temp_rsp.wrapping_sub(8) as u32);
                    } else {
                        self.set_sp(temp_rsp.wrapping_sub(8) as u16);
                    }
                } else {
                    self.write_new_stack_word(
                        &ss_seg,
                        temp_rsp.wrapping_sub(2) as u32,
                        cs_descriptor.dpl,
                        self.sregs[BxSegregs::Cs as usize].selector.value,
                    )?;
                    self.write_new_stack_word(
                        &ss_seg,
                        temp_rsp.wrapping_sub(4) as u32,
                        cs_descriptor.dpl,
                        self.get_ip(),
                    )?;
                    self.branch_far(&mut cs_selector, &mut cs_descriptor, disp, cpl)?;
                    // SAFETY: segment cache populated during segment load; union read matches descriptor type
                    if self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b() {
                        self.set_esp(temp_rsp.wrapping_sub(4) as u32);
                    } else {
                        self.set_sp(temp_rsp.wrapping_sub(4) as u16);
                    }
                }
            }
            return Ok(());
        }

        // System/gate descriptor
        let gate_descriptor = cs_descriptor;
        let gate_selector = cs_selector;

        // DPL checks
        if gate_descriptor.dpl < cpl {
            tracing::error!("call_protected_64: descriptor.dpl < CPL");
            return self.exception(Exception::Gp, cs_raw & 0xfffc);
        }
        if gate_descriptor.dpl < gate_selector.rpl {
            tracing::error!("call_protected_64: descriptor.dpl < selector.rpl");
            return self.exception(Exception::Gp, cs_raw & 0xfffc);
        }

        // In long mode, only 386 call gates (type 0xC) are allowed
        if gate_descriptor.r#type != 0xC {
            tracing::error!(
                "call_protected_64: gate type {:#x} unsupported in long mode",
                gate_descriptor.r#type
            );
            return self.exception(Exception::Gp, cs_raw & 0xfffc);
        }
        if !gate_descriptor.p {
            tracing::error!("call_protected_64: call gate not present");
            return self.exception(Exception::Np, cs_raw & 0xfffc);
        }

        // call_gate64 implementation (inline from Bochs call_far.cc)
        self.call_gate64(&gate_selector)
    }

    /// 64-bit call gate implementation.
    /// Based on Bochs call_far.cc call_gate64()
    fn call_gate64(&mut self, gate_selector: &BxSelector) -> Result<()> {
        use super::descriptor::{is_code_segment_non_conforming, is_data_segment};

        tracing::debug!("call_gate64: CALL 64bit call gate");

        let (dword1, dword2, dword3) = self.fetch_raw_descriptor_64(gate_selector)?;
        let gate_descriptor = self.parse_descriptor(dword1, dword2)?;

        // SAFETY: descriptor type verified as gate before union access
        let dest_selector = gate_descriptor.u.gate_dest_selector();
        if (dest_selector & 0xfffc) == 0 {
            tracing::error!("call_gate64: selector in gate null");
            return self.exception(Exception::Gp, 0);
        }

        let mut cs_selector = BxSelector::default();
        parse_selector(dest_selector, &mut cs_selector);

        let (dw1, dw2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, dest_selector & 0xfffc),
        };
        let mut cs_descriptor = self.parse_descriptor(dw1, dw2)?;

        // Compute the full 64-bit RIP from the gate descriptor
        // SAFETY: descriptor type verified as gate before union access
        let gate_offset_lo = gate_descriptor.u.gate_dest_offset();
        let new_rip = ((dword3 as u64) << 32) | (gate_offset_lo as u64);

        // AR byte must indicate code segment, DPL <= CPL
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cs_descriptor.valid == 0
            || !cs_descriptor.segment
            || is_data_segment(cs_descriptor.r#type)
            || cs_descriptor.dpl > cpl
        {
            tracing::error!("call_gate64: selected descriptor is not code");
            return self.exception(Exception::Gp, dest_selector & 0xfffc);
        }

        // Must be a 64-bit code segment (L=1, D=0)
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        if !cs_descriptor.is_long64_segment() || cs_descriptor.u.segment_d_b() {
            tracing::error!("call_gate64: not 64-bit code segment in call gate 64");
            return self.exception(Exception::Gp, dest_selector & 0xfffc);
        }

        // Code segment must be present
        if !cs_descriptor.p {
            tracing::error!("call_gate64: code segment not present");
            return self.exception(Exception::Np, dest_selector & 0xfffc);
        }

        let old_cs = self.sregs[BxSegregs::Cs as usize].selector.value as u64;
        let old_rip = self.rip();

        // CALL GATE TO MORE PRIVILEGE
        if is_code_segment_non_conforming(cs_descriptor.r#type) && cs_descriptor.dpl < cpl {
            tracing::debug!("CALL GATE64 TO MORE PRIVILEGE LEVEL");

            let rsp_for_cpl_x = self.get_rsp_from_tss(cs_descriptor.dpl)?;
            let old_ss = self.sregs[BxSegregs::Ss as usize].selector.value as u64;
            let old_rsp = self.rsp();

            // Push old stack long pointer onto new stack
            self.write_new_stack_qword_64(
                rsp_for_cpl_x.wrapping_sub(8),
                cs_descriptor.dpl,
                old_ss,
            )?;
            self.write_new_stack_qword_64(
                rsp_for_cpl_x.wrapping_sub(16),
                cs_descriptor.dpl,
                old_rsp,
            )?;
            // Push long pointer to return address onto new stack
            self.write_new_stack_qword_64(
                rsp_for_cpl_x.wrapping_sub(24),
                cs_descriptor.dpl,
                old_cs,
            )?;
            self.write_new_stack_qword_64(
                rsp_for_cpl_x.wrapping_sub(32),
                cs_descriptor.dpl,
                old_rip,
            )?;
            let new_rsp = rsp_for_cpl_x.wrapping_sub(32);

            // Load CS:RIP (guaranteed to be in 64 bit mode)
            let dest_dpl = cs_descriptor.dpl;
            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_rip, dest_dpl)?;

            // Set up null SS descriptor
            self.load_null_selector(BxSegregs::Ss, dest_dpl as u16);

            self.set_rsp(new_rsp);
        } else {
            // CALL GATE64 TO SAME PRIVILEGE
            tracing::debug!("CALL GATE64 TO SAME PRIVILEGE");

            // Push to 64-bit stack
            self.write_new_stack_qword_64(self.rsp().wrapping_sub(8), cpl, old_cs)?;
            self.write_new_stack_qword_64(self.rsp().wrapping_sub(16), cpl, old_rip)?;

            // Load CS:RIP (guaranteed to be in 64 bit mode)
            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_rip, cpl)?;

            self.set_rsp(self.rsp().wrapping_sub(16));
        }

        Ok(())
    }

    // =========================================================================
    // 64-bit long mode: return_protected_64
    // Based on Bochs ret_far.cc return_protected() with long mode paths
    // =========================================================================

    /// Protected mode far return for 64-bit mode.
    /// Based on Bochs ret_far.cc return_protected() with all X86_64 paths.
    pub(super) fn return_protected_64(
        &mut self,
        instr: &super::decoder::Instruction,
        pop_bytes: u16,
    ) -> Result<()> {
        let temp_rsp: u64 = if self.long64_mode() {
            self.rsp()
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        } else if self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b() {
            self.esp() as u64
        } else {
            self.sp() as u64
        };

        let (raw_cs_selector, return_rip, stack_param_offset): (u16, u64, u64) =
            if instr.os64_l() != 0 {
                let cs = self.stack_read_qword(temp_rsp.wrapping_add(8))? as u16;
                let rip = self.stack_read_qword(temp_rsp)?;
                (cs, rip, 16)
            } else if instr.os32_l() != 0 {
                // Bochs ret_far.cc: CS at temp_RSP+4, RIP at temp_RSP+0
                let cs = self.stack_read_dword((temp_rsp as u32).wrapping_add(4))? as u16;
                let rip = self.stack_read_dword(temp_rsp as u32)? as u64;
                (cs, rip, 8)
            } else {
                let cs = self.stack_read_word((temp_rsp as u32).wrapping_add(2))?;
                let rip = self.stack_read_word(temp_rsp as u32)? as u64;
                (cs, rip, 4)
            };

        if (raw_cs_selector & 0xfffc) == 0 {
            tracing::error!("return_protected_64: CS selector null");
            return self.exception(Exception::Gp, 0);
        }

        let mut cs_selector = BxSelector::default();
        parse_selector(raw_cs_selector, &mut cs_selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, raw_cs_selector & 0xfffc),
        };
        let mut cs_descriptor = self.parse_descriptor(dword1, dword2)?;

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cs_selector.rpl < cpl {
            tracing::error!("return_protected_64: CS.rpl < CPL");
            return self.exception(Exception::Gp, raw_cs_selector & 0xfffc);
        }

        // check_cs validates code segment, DPL, and presence
        self.check_cs(&cs_descriptor, raw_cs_selector, 0, cs_selector.rpl)?;

        // RETURN TO SAME PRIVILEGE LEVEL
        if cs_selector.rpl == cpl {
            tracing::debug!("return_protected_64: return to SAME PRIVILEGE LEVEL");

            self.branch_far(&mut cs_selector, &mut cs_descriptor, return_rip, cpl)?;

            if self.long64_mode() {
                self.set_rsp(
                    self.rsp()
                        .wrapping_add(stack_param_offset)
                        .wrapping_add(pop_bytes as u64),
                );
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            } else if self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b() {
                let val = self
                    .esp()
                    .wrapping_add(stack_param_offset as u32)
                    .wrapping_add(pop_bytes as u32);
                self.set_esp(val);
            } else {
                let val = self
                    .sp()
                    .wrapping_add(stack_param_offset as u16)
                    .wrapping_add(pop_bytes);
                self.set_sp(val);
            }
        } else {
            // RETURN TO OUTER PRIVILEGE LEVEL
            tracing::debug!("return_protected_64: return to OUTER PRIVILEGE LEVEL");

            let (raw_ss_selector, return_rsp): (u16, u64) = if instr.os64_l() != 0 {
                let ss = self
                    .stack_read_qword(temp_rsp.wrapping_add(24).wrapping_add(pop_bytes as u64))?
                    as u16;
                let rsp =
                    self.stack_read_qword(temp_rsp.wrapping_add(16).wrapping_add(pop_bytes as u64))?;
                (ss, rsp)
            } else if instr.os32_l() != 0 {
                let ss = self.stack_read_word(
                    (temp_rsp as u32)
                        .wrapping_add(12)
                        .wrapping_add(pop_bytes as u32),
                )?;
                let rsp = self.stack_read_dword(
                    (temp_rsp as u32)
                        .wrapping_add(8)
                        .wrapping_add(pop_bytes as u32),
                )? as u64;
                (ss, rsp)
            } else {
                let ss = self.stack_read_word(
                    (temp_rsp as u32)
                        .wrapping_add(6)
                        .wrapping_add(pop_bytes as u32),
                )?;
                let rsp = self.stack_read_word(
                    (temp_rsp as u32)
                        .wrapping_add(4)
                        .wrapping_add(pop_bytes as u32),
                )? as u64;
                (ss, rsp)
            };

            let mut ss_selector = BxSelector::default();
            parse_selector(raw_ss_selector, &mut ss_selector);

            let mut ss_descriptor = BxDescriptor::default();

            if (raw_ss_selector & 0xfffc) == 0 {
                // Null SS is allowed in long mode if it's a 64-bit code segment
                // and not returning to ring 3
                if self.long_mode() {
                    if !cs_descriptor.is_long64_segment() || cs_selector.rpl == 3 {
                        tracing::error!("return_protected_64: SS selector null");
                        return self.exception(Exception::Gp, 0);
                    }
                } else {
                    tracing::error!("return_protected_64: SS selector null");
                    return self.exception(Exception::Gp, 0);
                }
            } else {
                let (ss_dw1, ss_dw2) = match self.fetch_raw_descriptor(&ss_selector) {
                    Ok(v) => v,
                    Err(_) => {
                        return self.exception(Exception::Gp, raw_ss_selector & 0xfffc);
                    }
                };
                ss_descriptor = self.parse_descriptor(ss_dw1, ss_dw2)?;

                if ss_selector.rpl != cs_selector.rpl {
                    tracing::error!("return_protected_64: ss.rpl != cs.rpl");
                    return self.exception(Exception::Gp, raw_ss_selector & 0xfffc);
                }
                if ss_descriptor.valid == 0
                    || !ss_descriptor.segment
                    || ss_descriptor.r#type >= 8 // code segment
                    || (ss_descriptor.r#type & 2) == 0
                // not writable
                {
                    tracing::error!("return_protected_64: SS AR byte not writable data");
                    return self.exception(Exception::Gp, raw_ss_selector & 0xfffc);
                }
                if ss_descriptor.dpl != cs_selector.rpl {
                    tracing::error!("return_protected_64: SS.dpl != cs.rpl");
                    return self.exception(Exception::Gp, raw_ss_selector & 0xfffc);
                }
                if !ss_descriptor.p {
                    tracing::error!("return_protected_64: ss.present == 0");
                    return self.exception(Exception::Ss, raw_ss_selector & 0xfffc);
                }
            }

            // Load new CS
            let cs_rpl = cs_selector.rpl;
            self.branch_far(&mut cs_selector, &mut cs_descriptor, return_rip, cs_rpl)?;

            if (raw_ss_selector & 0xfffc) != 0 {
                // Load SS:RSP from stack
                self.load_ss(&mut ss_selector, &mut ss_descriptor, cs_rpl)?;
            } else {
                // In 64-bit mode with null SS
                self.load_null_selector(BxSegregs::Ss, raw_ss_selector);
            }

            if self.long64_mode() {
                self.set_rsp(return_rsp.wrapping_add(pop_bytes as u64));
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            } else if ss_descriptor.u.segment_d_b() {
                self.set_esp((return_rsp as u32).wrapping_add(pop_bytes as u32));
            } else {
                self.set_sp((return_rsp as u16).wrapping_add(pop_bytes));
            }

            // Validate segment registers for privilege change
            self.validate_seg_regs();
        }

        Ok(())
    }

    // =========================================================================
    // 64-bit long mode: long_iret
    // Based on Bochs iret.cc long_iret()
    // =========================================================================

    /// Long mode IRET implementation.
    /// Based on Bochs iret.cc long_iret()
    pub(super) fn long_iret(&mut self, instr: &super::decoder::Instruction) -> Result<()> {
        use super::eflags::EFlags;

        tracing::debug!("LONG MODE IRET");

        if self.eflags.contains(EFlags::NT) {
            tracing::error!("iret64: return from nested task in x86-64 mode!");
            return self.exception(Exception::Gp, 0);
        }

        // Determine temp_RSP based on mode
        let temp_rsp: u64 = if self.long64_mode() {
            self.rsp()
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        } else if self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b() {
            self.esp() as u64
        } else {
            self.sp() as u64
        };

        // Read RIP, CS, EFLAGS from stack based on operand size
        let (new_eflags, raw_cs_selector, new_rip, top_nbytes_same): (u32, u16, u64, u64) =
            if instr.os64_l() != 0 {
                let eflags = self.stack_read_qword(temp_rsp.wrapping_add(16))? as u32;
                let cs = self.stack_read_qword(temp_rsp.wrapping_add(8))? as u16;
                let rip = self.stack_read_qword(temp_rsp)?;
                (eflags, cs, rip, 24)
            } else if instr.os32_l() != 0 {
                let eflags = self.stack_read_dword((temp_rsp as u32).wrapping_add(8))?;
                let cs = self.stack_read_dword((temp_rsp as u32).wrapping_add(4))? as u16;
                let rip = self.stack_read_dword(temp_rsp as u32)? as u64;
                (eflags, cs, rip, 12)
            } else {
                let eflags = self.stack_read_word((temp_rsp as u32).wrapping_add(4))? as u32;
                let cs = self.stack_read_word((temp_rsp as u32).wrapping_add(2))?;
                let rip = self.stack_read_word(temp_rsp as u32)? as u64;
                (eflags, cs, rip, 6)
            };

        // Ignore VM flag in long mode
        let new_eflags = new_eflags & !EFlags::VM.bits();

        let mut cs_selector = BxSelector::default();
        parse_selector(raw_cs_selector, &mut cs_selector);

        // Return CS selector must be non-null
        if (raw_cs_selector & 0xfffc) == 0 {
            tracing::error!("iret64: return CS selector null");
            return self.exception(Exception::Gp, 0);
        }

        // Fetch and parse CS descriptor
        let (dword1, dword2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, raw_cs_selector & 0xfffc),
        };
        let mut cs_descriptor = self.parse_descriptor(dword1, dword2)?;

        // Return CS selector RPL must be >= CPL
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cs_selector.rpl < cpl {
            tracing::error!("iret64: return selector RPL < CPL");
            return self.exception(Exception::Gp, raw_cs_selector & 0xfffc);
        }

        // Check code-segment descriptor
        self.check_cs(&cs_descriptor, raw_cs_selector, 0, cs_selector.rpl)?;

        // INTERRUPT RETURN TO SAME PRIVILEGE LEVEL (only when not os64)
        if cs_selector.rpl == cpl && instr.os64_l() == 0 {
            tracing::debug!("LONG MODE INTERRUPT RETURN TO SAME PRIVILEGE LEVEL");

            // Load CS:RIP from stack
            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_rip, cpl)?;

            // Compute change mask for EFLAGS
            let mut change_mask = EFlags::OSZAPC
                .union(EFlags::TF)
                .union(EFlags::DF)
                .union(EFlags::NT)
                .union(EFlags::RF)
                .union(EFlags::ID)
                .union(EFlags::AC);
            let iopl = self.eflags.iopl();
            if cpl <= iopl {
                change_mask = change_mask.union(EFlags::IF_);
            }
            if cpl == 0 {
                change_mask = change_mask
                    .union(EFlags::VIP)
                    .union(EFlags::VIF)
                    .union(EFlags::IOPL_MASK);
            }

            let mut change_mask_val = change_mask.bits();
            if instr.os32_l() == 0 {
                // 16 bit
                change_mask_val &= 0xffff;
            }

            self.write_eflags(new_eflags, change_mask_val);

            // We are NOT in 64-bit mode for this path
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            if self.sregs[BxSegregs::Ss as usize].cache.u.segment_d_b() {
                self.set_esp(self.esp().wrapping_add(top_nbytes_same as u32));
            } else {
                self.set_sp(self.sp().wrapping_add(top_nbytes_same as u16));
            }
        } else {
            // INTERRUPT RETURN TO OUTER PRIVILEGE LEVEL or 64-BIT MODE
            tracing::debug!("LONG MODE INTERRUPT RETURN TO OUTER PRIVILEGE LEVEL or 64 BIT MODE");

            // Read SS and RSP from stack
            let (raw_ss_selector, new_rsp): (u16, u64) = if instr.os64_l() != 0 {
                let ss = self.stack_read_qword(temp_rsp.wrapping_add(32))? as u16;
                let rsp = self.stack_read_qword(temp_rsp.wrapping_add(24))?;
                (ss, rsp)
            } else if instr.os32_l() != 0 {
                let ss = self.stack_read_dword((temp_rsp as u32).wrapping_add(16))? as u16;
                let rsp = self.stack_read_dword((temp_rsp as u32).wrapping_add(12))? as u64;
                (ss, rsp)
            } else {
                let ss = self.stack_read_word((temp_rsp as u32).wrapping_add(8))?;
                let rsp = self.stack_read_word((temp_rsp as u32).wrapping_add(6))? as u64;
                (ss, rsp)
            };

            let mut ss_selector = BxSelector::default();
            let mut ss_descriptor = BxDescriptor::default();

            if (raw_ss_selector & 0xfffc) == 0 {
                if !cs_descriptor.is_long64_segment() || cs_selector.rpl == 3 {
                    tracing::error!("iret64: SS selector null");
                    return self.exception(Exception::Gp, 0);
                }
            } else {
                parse_selector(raw_ss_selector, &mut ss_selector);

                if ss_selector.rpl != cs_selector.rpl {
                    tracing::error!("iret64: SS.rpl != CS.rpl");
                    return self.exception(Exception::Gp, raw_ss_selector & 0xfffc);
                }

                let (ss_dw1, ss_dw2) = match self.fetch_raw_descriptor(&ss_selector) {
                    Ok(v) => v,
                    Err(_) => {
                        return self.exception(Exception::Gp, raw_ss_selector & 0xfffc);
                    }
                };
                ss_descriptor = self.parse_descriptor(ss_dw1, ss_dw2)?;

                if ss_descriptor.valid == 0
                    || !ss_descriptor.segment
                    || ss_descriptor.r#type >= 8 // code segment
                    || (ss_descriptor.r#type & 2) == 0
                // not writable
                {
                    tracing::error!("iret64: SS AR byte not writable or code segment");
                    return self.exception(Exception::Gp, raw_ss_selector & 0xfffc);
                }
                if ss_descriptor.dpl != cs_selector.rpl {
                    tracing::error!("iret64: SS.dpl != CS selector RPL");
                    return self.exception(Exception::Gp, raw_ss_selector & 0xfffc);
                }
                if !ss_descriptor.p {
                    tracing::error!("iret64: SS not present!");
                    return self.exception(Exception::Np, raw_ss_selector & 0xfffc);
                }
            }

            let prev_cpl = cpl;

            // Compute change mask for EFLAGS
            let mut change_mask = EFlags::OSZAPC
                .union(EFlags::TF)
                .union(EFlags::DF)
                .union(EFlags::NT)
                .union(EFlags::RF)
                .union(EFlags::ID)
                .union(EFlags::AC);
            let iopl = self.eflags.iopl();
            if prev_cpl <= iopl {
                change_mask = change_mask.union(EFlags::IF_);
            }
            if prev_cpl == 0 {
                change_mask = change_mask
                    .union(EFlags::VIP)
                    .union(EFlags::VIF)
                    .union(EFlags::IOPL_MASK);
            }

            let mut change_mask_val = change_mask.bits();
            if instr.os32_l() == 0 && instr.os64_l() == 0 {
                // 16 bit
                change_mask_val &= 0xffff;
            }

            // Set CPL to the RPL of the return CS selector
            let cs_rpl = cs_selector.rpl;
            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_rip, cs_rpl)?;

            // Write EFLAGS
            self.write_eflags(new_eflags, change_mask_val);

            if (raw_ss_selector & 0xfffc) != 0 {
                // Load SS:RSP from stack
                self.load_ss(&mut ss_selector, &mut ss_descriptor, cs_rpl)?;
            } else {
                // We are in 64-bit mode with null SS
                self.load_null_selector(BxSegregs::Ss, raw_ss_selector);
            }

            if self.long64_mode() {
                self.set_rsp(new_rsp);
            // SAFETY: segment cache populated during segment load; union read matches descriptor type
            } else if ss_descriptor.u.segment_d_b() {
                self.set_esp(new_rsp as u32);
            } else {
                self.set_sp(new_rsp as u16);
            }

            if prev_cpl != self.sregs[BxSegregs::Cs as usize].selector.rpl {
                self.validate_seg_regs();
            }
        }

        Ok(())
    }
}
