//! Protected mode control instructions
//!
//! Based on Bochs protect_ctrl.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements LGDT, SGDT, LIDT, SIDT, SLDT, STR, ARPL, LAR, LSL, VERR, VERW
//! (LLDT and LTR are in segment_ctrl_pro.rs)

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    descriptor::SegTypeBits,
    eflags::EFlags,
    Result,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// LGDT - Load Global Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:831-864
    pub fn lgdt_ms(&mut self, instr: &Instruction) -> Result<()> {
        // CPL must be 0 (Bochs protect_ctrl.cc:836-839)
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            tracing::debug!("LGDT: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        // Bochs: (eaddr + 2) & i->asize_mask() — mask for 16-bit address wrap
        let asize_mask: u64 = if self.long64_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFF_FFFF
        };
        let limit = self.v_read_word(seg, eaddr)?;
        let mut base = self.v_read_dword(seg, eaddr.wrapping_add(2) & asize_mask)? as u64;

        // In 16-bit operand size mode, mask base to 24 bits (80286 compatibility)
        // Based on Bochs protect_ctrl.cc:858
        if instr.os32_l() == 0 {
            base &= 0x00FFFFFF;
        }

        self.gdtr.base = base;
        self.gdtr.limit = limit;
        Ok(())
    }

    /// SGDT - Store Global Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:763-795
    pub fn sgdt_ms(&mut self, instr: &Instruction) -> Result<()> {
        // UMIP check (Bochs protect_ctrl.cc:767-772) — CR4.UMIP and CPL!=0 → #GP(0)
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                tracing::debug!("SGDT: CPL != 0 causes #GP when CR4.UMIP set");
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let asize_mask: u64 = if self.long64_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFF_FFFF
        };
        self.v_write_word(seg, eaddr, self.gdtr.limit)?;
        self.v_write_dword(
            seg,
            eaddr.wrapping_add(2) & asize_mask,
            self.gdtr.base as u32,
        )?;
        Ok(())
    }

    /// LIDT - Load Interrupt Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:866-898
    pub fn lidt_ms(&mut self, instr: &Instruction) -> Result<()> {
        // CPL must be 0 (Bochs protect_ctrl.cc:871-874)
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            tracing::debug!("LIDT: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let asize_mask: u64 = if self.long64_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFF_FFFF
        };
        let limit = self.v_read_word(seg, eaddr)?;
        let mut base = self.v_read_dword(seg, eaddr.wrapping_add(2) & asize_mask)? as u64;

        // In 16-bit operand size mode, mask base to 24 bits
        // Based on Bochs protect_ctrl.cc:893
        if instr.os32_l() == 0 {
            base &= 0x00FFFFFF;
        }

        self.idtr.base = base;
        self.idtr.limit = limit;
        Ok(())
    }

    /// SIDT - Store Interrupt Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:797-829
    pub fn sidt_ms(&mut self, instr: &Instruction) -> Result<()> {
        // UMIP check (Bochs protect_ctrl.cc:799-803) — CR4.UMIP and CPL!=0 → #GP(0)
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                tracing::debug!("SIDT: CPL != 0 causes #GP when CR4.UMIP set");
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let asize_mask: u64 = if self.long64_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFF_FFFF
        };
        self.v_write_word(seg, eaddr, self.idtr.limit)?;
        self.v_write_dword(
            seg,
            eaddr.wrapping_add(2) & asize_mask,
            self.idtr.base as u32,
        )?;
        Ok(())
    }

    /// SLDT - Store Local Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:286-328
    pub fn sldt_ew(&mut self, instr: &Instruction) -> Result<()> {
        if !self.protected_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        // UMIP check (Bochs protect_ctrl.cc:293-297) — CR4.UMIP and CPL!=0 → #GP(0)
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        let val = self.ldtr.selector.value;
        if instr.mod_c0() {
            // Register destination: Group 6 (0F 00) now uses group convention: dst()=rm
            if instr.os32_l() != 0 {
                self.set_gpr32(instr.dst() as usize, val as u32);
            } else {
                self.set_gpr16(instr.dst() as usize, val);
            }
        } else {
            // Memory destination — always write 16-bit
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_write_word(seg, eaddr, val)?;
        }
        Ok(())
    }

    /// SMSW — Store Machine Status Word
    /// Based on Bochs crregs.cc:916-961
    pub fn smsw_ew(&mut self, instr: &Instruction) -> Result<()> {
        // UMIP check (Bochs crregs.cc:918-925) — CR4.UMIP and CPL!=0 → #GP(0)
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        let msw = self.cr0.get32();

        if instr.mod_c0() {
            // Register form: writes 32-bit value (Bochs crregs.cc:928-935)
            // For Group 7 (0F 01): b1=0x101, (b1 & 0x0F)==0x01 → Ed,Gd branch: DST=rm, SRC1=nnn
            // So dst() = rm = actual register. Matches Bochs: BX_WRITE_32BIT_REGZ(i->dst(), val)
            if instr.os32_l() != 0 {
                self.set_gpr32(instr.dst() as usize, msw);
            } else {
                self.set_gpr16(instr.dst() as usize, msw as u16);
            }
        } else {
            // Memory form: always writes 16-bit (Bochs crregs.cc:937-958)
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_write_word(seg, eaddr, msw as u16)?;
        }
        Ok(())
    }

    /// STR - Store Task Register
    /// Based on Bochs protect_ctrl.cc:330-372
    pub fn str_ew(&mut self, instr: &Instruction) -> Result<()> {
        if !self.protected_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        // UMIP check (Bochs protect_ctrl.cc:337-341) — CR4.UMIP and CPL!=0 → #GP(0)
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        let val = self.tr.selector.value;
        if instr.mod_c0() {
            // Register destination: Group 6 (0F 00) now uses group convention: dst()=rm
            if instr.os32_l() != 0 {
                self.set_gpr32(instr.dst() as usize, val as u32);
            } else {
                self.set_gpr16(instr.dst() as usize, val);
            }
        } else {
            // Memory destination — always write 16-bit
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_write_word(seg, eaddr, val)?;
        }
        Ok(())
    }

    /// ARPL — Adjust Requested Privilege Level
    /// Based on Bochs protect_ctrl.cc:31-68
    pub(super) fn arpl_ew_gw(&mut self, instr: &Instruction) -> Result<()> {
        if !self.protected_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        // Decoder convention for 0x63 (ARPL Ew,Gw): dst()=nnn=Gw, src1()=rm=Ew
        // Bochs: i->dst()=Ew(rm), i->src()=Gw(nnn) — opposite of our decoder!
        // So: op1_16 = Ew = src1()=rm, op2_16 = Gw = dst()=nnn
        let op1_16: u16;
        if instr.mod_c0() {
            // Register form: op1 comes from rm = src1()
            op1_16 = self.get_gpr16(instr.src1() as usize);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            op1_16 = self.v_read_word(seg, eaddr)?;
        }
        // op2_16 = Gw = nnn = dst()
        let op2_16 = self.get_gpr16(instr.dst() as usize);

        if (op1_16 & 0x03) < (op2_16 & 0x03) {
            // Adjust RPL field and set ZF
            let new_op1 = (op1_16 & 0xfffc) | (op2_16 & 0x03);
            if instr.mod_c0() {
                // Write back to rm = src1()
                self.set_gpr16(instr.src1() as usize, new_op1);
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_write_word(seg, eaddr, new_op1)?;
            }
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    /// Non-throwing fetch_raw_descriptor2 — returns None on failure
    /// Based on BX_CPU_C::fetch_raw_descriptor2 in segment_ctrl_pro.cc:570-596
    fn fetch_raw_descriptor2_nt(
        &mut self,
        selector: &super::descriptor::BxSelector,
    ) -> Option<(u32, u32)> {
        let index = selector.index as u64;
        if selector.ti == 0 {
            // GDT
            if index * 8 + 7 > self.gdtr.limit as u64 {
                return None;
            }
            let offset = self.gdtr.base + index * 8;
            let qword = self.system_read_qword(offset).ok()?;
            Some((
                (qword & 0xFFFFFFFF) as u32,
                ((qword >> 32) & 0xFFFFFFFF) as u32,
            ))
        } else {
            // LDT
            if self.ldtr.cache.valid == 0 {
                return None;
            }
            let ldt_limit = unsafe { self.ldtr.cache.u.segment.limit_scaled as u64 };
            if index * 8 + 7 > ldt_limit {
                return None;
            }
            let ldt_base = unsafe { self.ldtr.cache.u.segment.base };
            let offset = ldt_base + index * 8;
            let qword = self.system_read_qword(offset).ok()?;
            Some((
                (qword & 0xFFFFFFFF) as u32,
                ((qword >> 32) & 0xFFFFFFFF) as u32,
            ))
        }
    }

    /// LAR — Load Access Rights
    /// Based on Bochs protect_ctrl.cc:70-181
    pub(super) fn lar_gv_ew(&mut self, instr: &Instruction) -> Result<()> {
        if !self.protected_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let raw_selector: u16;
        if instr.mod_c0() {
            raw_selector = self.get_gpr16(instr.src1() as usize);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            raw_selector = self.v_read_word(seg, eaddr)?;
        }

        // Null selector → clear ZF
        if (raw_selector & 0xfffc) == 0 {
            self.eflags.remove(EFlags::ZF);
            return Ok(());
        }

        let mut selector = super::descriptor::BxSelector::default();
        super::segment_ctrl_pro::parse_selector(raw_selector, &mut selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor2_nt(&selector) {
            Some(v) => v,
            None => {
                tracing::debug!("LAR: failed to fetch descriptor");
                self.eflags.remove(EFlags::ZF);
                return Ok(());
            }
        };

        let descriptor = match self.parse_descriptor(dword1, dword2) {
            Ok(d) => d,
            Err(_) => {
                self.eflags.remove(EFlags::ZF);
                return Ok(());
            }
        };

        if descriptor.valid == 0 {
            tracing::debug!("LAR: descriptor not valid");
            self.eflags.remove(EFlags::ZF);
            return Ok(());
        }

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;

        if descriptor.segment {
            // Normal code/data segment
            // Conforming code segments ignore DPL
            let is_code = SegTypeBits::from_raw(descriptor.r#type).contains(SegTypeBits::CODE);
            let is_conforming = SegTypeBits::from_raw(descriptor.r#type).contains(SegTypeBits::CONFORMING);
            if !(is_code && is_conforming) {
                if descriptor.dpl < cpl || descriptor.dpl < selector.rpl {
                    self.eflags.remove(EFlags::ZF);
                    return Ok(());
                }
            }
        } else {
            // System/gate segment — only certain types accepted
            // Based on Bochs protect_ctrl.cc:134-168
            match descriptor.r#type {
                // 286/386 TSS, LDT, call gate, task gate — accepted
                0x1 | 0x3 | 0x4 | 0x5 => {
                    // 286 types not accepted in long mode — skip (we're 32-bit)
                }
                0x2 | 0x9 | 0xB | 0xC => {
                    // LDT, 386 TSS (avail/busy), 386 call gate
                }
                _ => {
                    tracing::debug!("LAR: not accepted descriptor type {}", descriptor.r#type);
                    self.eflags.remove(EFlags::ZF);
                    return Ok(());
                }
            }
            if descriptor.dpl < cpl || descriptor.dpl < selector.rpl {
                self.eflags.remove(EFlags::ZF);
                return Ok(());
            }
        }

        // All checks passed — set ZF and write access rights to dst
        self.eflags.insert(EFlags::ZF);
        if instr.os32_l() != 0 {
            // 32-bit: masked by 00FFFF00 (Bochs protect_ctrl.cc:174)
            self.set_gpr32(instr.dst() as usize, dword2 & 0x00ffff00);
        } else {
            // 16-bit: lower byte of access rights
            self.set_gpr16(instr.dst() as usize, (dword2 & 0xff00) as u16);
        }
        Ok(())
    }

    /// LSL — Load Segment Limit
    /// Based on Bochs protect_ctrl.cc:183-283
    pub(super) fn lsl_gv_ew(&mut self, instr: &Instruction) -> Result<()> {
        if !self.protected_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let raw_selector: u16;
        if instr.mod_c0() {
            raw_selector = self.get_gpr16(instr.src1() as usize);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            raw_selector = self.v_read_word(seg, eaddr)?;
        }

        // Null selector → clear ZF
        if (raw_selector & 0xfffc) == 0 {
            self.eflags.remove(EFlags::ZF);
            return Ok(());
        }

        let mut selector = super::descriptor::BxSelector::default();
        super::segment_ctrl_pro::parse_selector(raw_selector, &mut selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor2_nt(&selector) {
            Some(v) => v,
            None => {
                tracing::debug!("LSL: failed to fetch descriptor");
                self.eflags.remove(EFlags::ZF);
                return Ok(());
            }
        };

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        let descriptor_dpl = ((dword2 >> 13) & 0x03) as u8;
        let limit32: u32;

        if (dword2 & 0x00001000) == 0 {
            // System segment (S bit = 0)
            let seg_type = (dword2 >> 8) & 0x0f;
            match seg_type {
                // 286/386 TSS (avail/busy) — not accepted in long mode; skip 286 check (32-bit)
                0x1 | 0x3 | 0x2 | 0x9 | 0xB => {
                    // Privilege check
                    if descriptor_dpl < cpl || descriptor_dpl < selector.rpl {
                        self.eflags.remove(EFlags::ZF);
                        return Ok(());
                    }
                    // Compute byte-granular limit
                    let raw_limit = (dword1 & 0x0000ffff) | (dword2 & 0x000f0000);
                    limit32 = if (dword2 & 0x00800000) != 0 {
                        (raw_limit << 12) | 0x00000fff
                    } else {
                        raw_limit
                    };
                }
                _ => {
                    // Remaining types not accepted for LSL
                    self.eflags.remove(EFlags::ZF);
                    return Ok(());
                }
            }
        } else {
            // Data/code segment (S bit = 1)
            let raw_limit = (dword1 & 0x0000ffff) | (dword2 & 0x000f0000);
            limit32 = if (dword2 & 0x00800000) != 0 {
                (raw_limit << 12) | 0x00000fff
            } else {
                raw_limit
            };
            // Non-conforming code segments need privilege check
            // (dword2 & 0x00000c00) == 0x00000c00 means conforming code (bits 10+11 both set)
            if (dword2 & 0x00000c00) != 0x00000c00 {
                if descriptor_dpl < cpl || descriptor_dpl < selector.rpl {
                    self.eflags.remove(EFlags::ZF);
                    return Ok(());
                }
            }
        }

        // All checks passed
        self.eflags.insert(EFlags::ZF);
        if instr.os32_l() != 0 {
            self.set_gpr32(instr.dst() as usize, limit32);
        } else {
            self.set_gpr16(instr.dst() as usize, limit32 as u16);
        }
        Ok(())
    }

    /// VERR — Verify Segment for Reading
    /// Based on Bochs protect_ctrl.cc:597-687
    pub(super) fn verr_ew(&mut self, instr: &Instruction) -> Result<()> {
        if !self.protected_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let raw_selector: u16;
        if instr.mod_c0() {
            // Group 6 (0F 00): dst()=rm (Bochs: i->dst())
            raw_selector = self.get_gpr16(instr.dst() as usize);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            raw_selector = self.v_read_word(seg, eaddr)?;
        }

        // Null selector → clear ZF
        if (raw_selector & 0xfffc) == 0 {
            tracing::debug!("VERR: null selector");
            self.eflags.remove(EFlags::ZF);
            return Ok(());
        }

        let mut selector = super::descriptor::BxSelector::default();
        super::segment_ctrl_pro::parse_selector(raw_selector, &mut selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor2_nt(&selector) {
            Some(v) => v,
            None => {
                tracing::debug!("VERR: not within descriptor table");
                self.eflags.remove(EFlags::ZF);
                return Ok(());
            }
        };

        let descriptor = match self.parse_descriptor(dword1, dword2) {
            Ok(d) => d,
            Err(_) => {
                self.eflags.remove(EFlags::ZF);
                return Ok(());
            }
        };

        // System segment → inaccessible
        if !descriptor.segment {
            tracing::debug!("VERR: system descriptor");
            self.eflags.remove(EFlags::ZF);
            return Ok(());
        }

        if descriptor.valid == 0 {
            tracing::debug!("VERR: valid bit cleared");
            self.eflags.remove(EFlags::ZF);
            return Ok(());
        }

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        let is_code = SegTypeBits::from_raw(descriptor.r#type).contains(SegTypeBits::CODE);

        if is_code {
            // Code segment: readable conforming segments ignore DPL
            let is_conforming = SegTypeBits::from_raw(descriptor.r#type).contains(SegTypeBits::CONFORMING);
            let is_readable = SegTypeBits::from_raw(descriptor.r#type).contains(SegTypeBits::READABLE);
            if is_conforming && is_readable {
                tracing::debug!("VERR: conforming readable code, OK");
                self.eflags.insert(EFlags::ZF);
                return Ok(());
            }
            if !is_readable {
                tracing::debug!("VERR: code not readable");
                self.eflags.remove(EFlags::ZF);
                return Ok(());
            }
            // Readable non-conforming code segment
            if descriptor.dpl < cpl || descriptor.dpl < selector.rpl {
                tracing::debug!("VERR: non-conforming code not within priv level");
                self.eflags.remove(EFlags::ZF);
            } else {
                self.eflags.insert(EFlags::ZF);
            }
        } else {
            // Data segment
            if descriptor.dpl < cpl || descriptor.dpl < selector.rpl {
                tracing::debug!("VERR: data seg not within priv level");
                self.eflags.remove(EFlags::ZF);
            } else {
                self.eflags.insert(EFlags::ZF);
            }
        }
        Ok(())
    }

    /// VERW — Verify Segment for Writing
    /// Based on Bochs protect_ctrl.cc:689-761
    pub(super) fn verw_ew(&mut self, instr: &Instruction) -> Result<()> {
        if !self.protected_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        let raw_selector: u16;
        if instr.mod_c0() {
            // Group 6 (0F 00): dst()=rm (Bochs: i->dst())
            raw_selector = self.get_gpr16(instr.dst() as usize);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            raw_selector = self.v_read_word(seg, eaddr)?;
        }

        // Null selector → clear ZF
        if (raw_selector & 0xfffc) == 0 {
            tracing::debug!("VERW: null selector");
            self.eflags.remove(EFlags::ZF);
            return Ok(());
        }

        let mut selector = super::descriptor::BxSelector::default();
        super::segment_ctrl_pro::parse_selector(raw_selector, &mut selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor2_nt(&selector) {
            Some(v) => v,
            None => {
                tracing::debug!("VERW: not within descriptor table");
                self.eflags.remove(EFlags::ZF);
                return Ok(());
            }
        };

        let descriptor = match self.parse_descriptor(dword1, dword2) {
            Ok(d) => d,
            Err(_) => {
                self.eflags.remove(EFlags::ZF);
                return Ok(());
            }
        };

        // System segment or code segment → inaccessible for write
        let is_code = SegTypeBits::from_raw(descriptor.r#type).contains(SegTypeBits::CODE);
        if !descriptor.segment || is_code {
            tracing::debug!("VERW: system seg or code");
            self.eflags.remove(EFlags::ZF);
            return Ok(());
        }

        if descriptor.valid == 0 {
            tracing::debug!("VERW: valid bit cleared");
            self.eflags.remove(EFlags::ZF);
            return Ok(());
        }

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        let is_writable = SegTypeBits::from_raw(descriptor.r#type).contains(SegTypeBits::READABLE);

        if is_writable {
            if descriptor.dpl < cpl || descriptor.dpl < selector.rpl {
                tracing::debug!("VERW: writable data seg not within priv level");
                self.eflags.remove(EFlags::ZF);
            } else {
                self.eflags.insert(EFlags::ZF);
            }
        } else {
            tracing::debug!("VERW: data seg not writable");
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    // =========================================================================
    // LGDT/LIDT/SGDT/SIDT — 64-bit mode variants
    // Matching Bochs protect_ctrl.cc LGDT64_Ms / LIDT64_Ms / SGDT64_Ms / SIDT64_Ms
    // In 64-bit mode: base is 8 bytes (not 4), no 24-bit truncation
    // =========================================================================

    /// LGDT64 - Load GDT in 64-bit mode (10-byte pseudo-descriptor)
    /// Bochs protect_ctrl.cc LGDT64_Ms — uses system_read (paging-aware)
    pub fn lgdt_op64_ms(&mut self, instr: &Instruction) -> Result<()> {
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = self.get_laddr64(seg as usize, eaddr);
        let limit = self.system_read_word(laddr)?;
        let base = self.system_read_qword(laddr.wrapping_add(2))?;
        if !self.is_canonical(base) {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        self.gdtr.limit = limit;
        self.gdtr.base = base;
        Ok(())
    }

    /// LIDT64 - Load IDT in 64-bit mode (10-byte pseudo-descriptor)
    /// Bochs protect_ctrl.cc LIDT64_Ms — uses system_read (paging-aware)
    pub fn lidt_op64_ms(&mut self, instr: &Instruction) -> Result<()> {
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = self.get_laddr64(seg as usize, eaddr);
        let limit = self.system_read_word(laddr)?;
        let base = self.system_read_qword(laddr.wrapping_add(2))?;
        if !self.is_canonical(base) {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        self.idtr.limit = limit;
        self.idtr.base = base;

        Ok(())
    }

    /// SGDT64 - Store GDT in 64-bit mode (10-byte pseudo-descriptor)
    /// Bochs protect_ctrl.cc SGDT64_Ms — uses system_write (paging-aware)
    pub fn sgdt_op64_ms(&mut self, instr: &Instruction) -> Result<()> {
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = self.get_laddr64(seg as usize, eaddr);
        self.system_write_word(laddr, self.gdtr.limit)?;
        self.system_write_qword(laddr.wrapping_add(2), self.gdtr.base)?;
        Ok(())
    }

    /// SIDT64 - Store IDT in 64-bit mode (10-byte pseudo-descriptor)
    /// Bochs protect_ctrl.cc SIDT64_Ms — uses system_write (paging-aware)
    pub fn sidt_op64_ms(&mut self, instr: &Instruction) -> Result<()> {
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = self.get_laddr64(seg as usize, eaddr);
        self.system_write_word(laddr, self.idtr.limit)?;
        self.system_write_qword(laddr.wrapping_add(2), self.idtr.base)?;
        Ok(())
    }

    // =========================================================================
    // LSS/LFS/LGS — 64-bit mode far pointer load
    // Matching Bochs protect_ctrl.cc LSS_GqMp / LFS_GqMp / LGS_GqMp
    // =========================================================================

    pub fn lss_gq_mp(&mut self, instr: &Instruction) -> Result<()> {
        self.load_far_pointer64(instr, BxSegregs::Ss)
    }

    pub fn lfs_gq_mp(&mut self, instr: &Instruction) -> Result<()> {
        self.load_far_pointer64(instr, BxSegregs::Fs)
    }

    pub fn lgs_gq_mp(&mut self, instr: &Instruction) -> Result<()> {
        self.load_far_pointer64(instr, BxSegregs::Gs)
    }

    fn load_far_pointer64(
        &mut self,
        instr: &Instruction,
        target_seg: BxSegregs,
    ) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let laddr = self.get_laddr64(seg as usize, eaddr);

        // Read 64-bit offset + 16-bit selector
        let offset = self.mem_read_qword(laddr);
        let selector = self.mem_read_word(laddr.wrapping_add(8));

        self.load_seg_reg(target_seg, selector)?;
        self.set_gpr64(instr.dst() as usize, offset);
        Ok(())
    }
}
