//! 64-bit bit scan and bit test instructions: BSF, BSR, BT, BTS, BTR, BTC
//! Matching Bochs bit64.cc and logical64.cc (BT variants)
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // BSF / BSR — Bit Scan Forward / Reverse (64-bit)
    // Matching Bochs bit64.cc BSF_GqEq / BSR_GqEq
    // =========================================================================

    /// BSF r64, r/m64 — Bit Scan Forward
    pub fn bsf_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)?
        };
        if op2 == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            let idx = op2.trailing_zeros() as u64;
            self.set_flags_oszapc_logic_64(idx);
            self.eflags.remove(EFlags::ZF);
            self.set_gpr64(instr.dst() as usize, idx);
        }
        Ok(())
    }

    /// BSR r64, r/m64 — Bit Scan Reverse
    pub fn bsr_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)?
        };
        if op2 == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            let idx = (63 - op2.leading_zeros()) as u64;
            self.set_flags_oszapc_logic_64(idx);
            self.eflags.remove(EFlags::ZF);
            self.set_gpr64(instr.dst() as usize, idx);
        }
        Ok(())
    }

    // =========================================================================
    // BT / BTS / BTR / BTC — Bit Test (64-bit, register index)
    // Matching Bochs bit64.cc BT_EqGq / BTS_EqGq / BTR_EqGq / BTC_EqGq
    // =========================================================================

    /// BT r/m64, r64 — Bit Test (CF = bit at index)
    pub fn bt_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr64(instr.src() as usize);

        if instr.mod_c0() {
            let op1 = self.get_gpr64(instr.dst() as usize);
            let bit_index = op2 & 0x3F;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
        } else {
            // Memory form: bit index can extend beyond the qword
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            let displacement = (op2 as i64 >> 6) * 8;
            let addr = eaddr.wrapping_add(displacement as u64);
            let op1 = self.read_virtual_qword_64(seg, addr)?;
            let bit_index = op2 & 0x3F;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
        }
        Ok(())
    }

    /// BTS r/m64, r64 — Bit Test and Set
    pub fn bts_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr64(instr.src() as usize);

        if instr.mod_c0() {
            let dst_reg = instr.dst() as usize;
            let op1 = self.get_gpr64(dst_reg);
            let bit_index = op2 & 0x3F;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.set_gpr64(dst_reg, op1 | (1u64 << bit_index));
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            let displacement = (op2 as i64 >> 6) * 8;
            let addr = eaddr.wrapping_add(displacement as u64);
            let op1 = self.read_rmw_virtual_qword_64(seg, addr)?;
            let bit_index = op2 & 0x3F;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.write_rmw_virtual_qword_back_64(op1 | (1u64 << bit_index));
        }
        Ok(())
    }

    /// BTR r/m64, r64 — Bit Test and Reset
    pub fn btr_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr64(instr.src() as usize);

        if instr.mod_c0() {
            let dst_reg = instr.dst() as usize;
            let op1 = self.get_gpr64(dst_reg);
            let bit_index = op2 & 0x3F;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.set_gpr64(dst_reg, op1 & !(1u64 << bit_index));
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            let displacement = (op2 as i64 >> 6) * 8;
            let addr = eaddr.wrapping_add(displacement as u64);
            let op1 = self.read_rmw_virtual_qword_64(seg, addr)?;
            let bit_index = op2 & 0x3F;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.write_rmw_virtual_qword_back_64(op1 & !(1u64 << bit_index));
        }
        Ok(())
    }

    /// BTC r/m64, r64 — Bit Test and Complement
    pub fn btc_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr64(instr.src() as usize);

        if instr.mod_c0() {
            let dst_reg = instr.dst() as usize;
            let op1 = self.get_gpr64(dst_reg);
            let bit_index = op2 & 0x3F;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.set_gpr64(dst_reg, op1 ^ (1u64 << bit_index));
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            let displacement = (op2 as i64 >> 6) * 8;
            let addr = eaddr.wrapping_add(displacement as u64);
            let op1 = self.read_rmw_virtual_qword_64(seg, addr)?;
            let bit_index = op2 & 0x3F;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.write_rmw_virtual_qword_back_64(op1 ^ (1u64 << bit_index));
        }
        Ok(())
    }

    // =========================================================================
    // BT / BTS / BTR / BTC — Bit Test with immediate (64-bit)
    // Matching Bochs bit64.cc BT_EqIb / BTS_EqIb / BTR_EqIb / BTC_EqIb
    // =========================================================================

    /// BT r/m64, imm8 — Bit Test with immediate
    pub fn bt_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit_index = (instr.ib() & 0x3F) as u64;
        let op1 = if instr.mod_c0() {
            self.get_gpr64(instr.dst() as usize)
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)?
        };
        if (op1 >> bit_index) & 1 != 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        Ok(())
    }

    /// BTS r/m64, imm8 — Bit Test and Set with immediate
    pub fn bts_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit_index = (instr.ib() & 0x3F) as u64;
        if instr.mod_c0() {
            let dst_reg = instr.dst() as usize;
            let op1 = self.get_gpr64(dst_reg);
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.set_gpr64(dst_reg, op1 | (1u64 << bit_index));
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.read_rmw_virtual_qword_64(seg, eaddr)?;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.write_rmw_virtual_qword_back_64(op1 | (1u64 << bit_index));
        }
        Ok(())
    }

    /// BTR r/m64, imm8 — Bit Test and Reset with immediate
    pub fn btr_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit_index = (instr.ib() & 0x3F) as u64;
        if instr.mod_c0() {
            let dst_reg = instr.dst() as usize;
            let op1 = self.get_gpr64(dst_reg);
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.set_gpr64(dst_reg, op1 & !(1u64 << bit_index));
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.read_rmw_virtual_qword_64(seg, eaddr)?;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.write_rmw_virtual_qword_back_64(op1 & !(1u64 << bit_index));
        }
        Ok(())
    }

    /// BTC r/m64, imm8 — Bit Test and Complement with immediate
    pub fn btc_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit_index = (instr.ib() & 0x3F) as u64;
        if instr.mod_c0() {
            let dst_reg = instr.dst() as usize;
            let op1 = self.get_gpr64(dst_reg);
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.set_gpr64(dst_reg, op1 ^ (1u64 << bit_index));
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.read_rmw_virtual_qword_64(seg, eaddr)?;
            if (op1 >> bit_index) & 1 != 0 {
                self.eflags.insert(EFlags::CF);
            } else {
                self.eflags.remove(EFlags::CF);
            }
            self.write_rmw_virtual_qword_back_64(op1 ^ (1u64 << bit_index));
        }
        Ok(())
    }

    // =========================================================================
    // POPCNT — Population Count (64-bit) (F3 REX.W 0F B8 /r)
    // Bochs: bit64.cc POPCNT_GqEqR / POPCNT_GqEqM
    // =========================================================================

    /// POPCNT r64, r/m64 — count set bits
    pub fn popcnt_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)?
        };
        let result = op2.count_ones() as u64;
        self.set_gpr64(instr.dst() as usize, result);

        // POPCNT clears OF, SF, AF, CF, PF; sets ZF if result is 0
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::CF | EFlags::PF);
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    // =========================================================================
    // LZCNT — Leading Zero Count (64-bit) (F3 REX.W 0F BD /r)
    // =========================================================================

    /// LZCNT r64, r/m64 — count leading zeros
    pub fn lzcnt_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)?
        };
        let result = op2.leading_zeros() as u64;
        self.set_gpr64(instr.dst() as usize, result);

        // CF = (op2 == 0), ZF = (result == 0 i.e. op2 has bit 63 set)
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::PF);
        if op2 == 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }

    // =========================================================================
    // MOVBE — Move Big-Endian (64-bit) (0F 38 F0 / 0F 38 F1 with REX.W)
    // Matching Bochs bit.cc MOVBE_GqMq / MOVBE_MqGq
    // =========================================================================

    /// MOVBE r64, m64 — load qword with byte swap
    pub fn movbe_gq_mq(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let val = self.read_virtual_qword_64(seg, eaddr)?;
        self.set_gpr64(instr.dst() as usize, val.swap_bytes());
        Ok(())
    }

    /// MOVBE m64, r64 — store qword with byte swap
    pub fn movbe_mq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        let val = self.get_gpr64(instr.dst() as usize);
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        self.write_virtual_qword_64(seg, eaddr, val.swap_bytes())?;
        Ok(())
    }

    // =========================================================================
    // TZCNT — Trailing Zero Count (64-bit) (F3 REX.W 0F BC /r)
    // =========================================================================

    /// TZCNT r64, r/m64 — count trailing zeros
    pub fn tzcnt_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)?
        };
        let result = op2.trailing_zeros() as u64;
        self.set_gpr64(instr.dst() as usize, result);

        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::PF);
        if op2 == 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            self.eflags.remove(EFlags::ZF);
        }
        Ok(())
    }
}
