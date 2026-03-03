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
        let eaddr = self.resolve_addr32(instr);
        // Bochs: (eaddr + 2) & i->asize_mask() — mask for 16-bit address wrap
        let asize_mask: u32 = if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFFFFFF
        };
        let limit = self.read_virtual_word(seg, eaddr)?;
        let mut base = self.read_virtual_dword(seg, eaddr.wrapping_add(2) & asize_mask)? as u64;

        // In 16-bit operand size mode, mask base to 24 bits (80286 compatibility)
        // Based on Bochs protect_ctrl.cc:858
        if instr.os32_l() == 0 {
            base &= 0x00FFFFFF;
        }

        self.gdtr.base = base;
        self.gdtr.limit = limit;
        tracing::trace!("LGDT: base={:#010x}, limit={:#06x}", base, limit);
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
        let eaddr = self.resolve_addr32(instr);
        let asize_mask: u32 = if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFFFFFF
        };
        self.write_virtual_word(seg, eaddr, self.gdtr.limit)?;
        self.write_virtual_dword(
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
        let eaddr = self.resolve_addr32(instr);
        let asize_mask: u32 = if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFFFFFF
        };
        let limit = self.read_virtual_word(seg, eaddr)?;
        let mut base = self.read_virtual_dword(seg, eaddr.wrapping_add(2) & asize_mask)? as u64;

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
        let eaddr = self.resolve_addr32(instr);
        let asize_mask: u32 = if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFFFFFF
        };
        self.write_virtual_word(seg, eaddr, self.idtr.limit)?;
        self.write_virtual_dword(
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
            // Register destination: for Group 6 (0F 00), decoder puts nnn in dst() and rm in src1()
            // The actual register operand is rm = src1() (Bochs: i->dst() = rm)
            if instr.os32_l() != 0 {
                self.set_gpr32(instr.src1() as usize, val as u32);
            } else {
                self.set_gpr16(instr.src1() as usize, val);
            }
        } else {
            // Memory destination — always write 16-bit
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr32(instr);
            self.write_virtual_word(seg, eaddr, val)?;
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
            let eaddr = self.resolve_addr32(instr);
            self.write_virtual_word(seg, eaddr, msw as u16)?;
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
            // Register destination: for Group 6 (0F 00), decoder puts nnn in dst() and rm in src1()
            // The actual register operand is rm = src1() (Bochs: i->dst() = rm)
            if instr.os32_l() != 0 {
                self.set_gpr32(instr.src1() as usize, val as u32);
            } else {
                self.set_gpr16(instr.src1() as usize, val);
            }
        } else {
            // Memory destination — always write 16-bit
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr32(instr);
            self.write_virtual_word(seg, eaddr, val)?;
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
            let eaddr = self.resolve_addr32(instr);
            op1_16 = self.read_virtual_word(seg, eaddr)?;
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
                let eaddr = self.resolve_addr32(instr);
                self.write_virtual_word(seg, eaddr, new_op1)?;
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
        &self,
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
            let eaddr = self.resolve_addr32(instr);
            raw_selector = self.read_virtual_word(seg, eaddr)?;
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
            let is_code = (descriptor.r#type & 0x8) != 0;
            let is_conforming = (descriptor.r#type & 0x4) != 0;
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
            let eaddr = self.resolve_addr32(instr);
            raw_selector = self.read_virtual_word(seg, eaddr)?;
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
            // Bochs protect_ctrl.cc:611: BX_READ_16BIT_REG(i->src())
            raw_selector = self.get_gpr16(instr.src() as usize);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr32(instr);
            raw_selector = self.read_virtual_word(seg, eaddr)?;
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
        let is_code = (descriptor.r#type & 0x8) != 0;

        if is_code {
            // Code segment: readable conforming segments ignore DPL
            let is_conforming = (descriptor.r#type & 0x4) != 0;
            let is_readable = (descriptor.r#type & 0x2) != 0;
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
            // Bochs protect_ctrl.cc:703: BX_READ_16BIT_REG(i->src())
            raw_selector = self.get_gpr16(instr.src() as usize);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr32(instr);
            raw_selector = self.read_virtual_word(seg, eaddr)?;
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
        let is_code = (descriptor.r#type & 0x8) != 0;
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
        let is_writable = (descriptor.r#type & 0x2) != 0;

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
}
