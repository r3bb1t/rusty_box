// 64-bit arithmetic operations: ADD, ADC, SUB, SBB, CMP, NEG, INC, DEC,
// XADD, CMPXCHG, CMPXCHG16B.
// Mirrors Bochs cpp/cpu/arith64.cc
//
// Memory access pattern (64-bit mode):
//   eaddr = self.resolve_addr64(instr)           -- u64 effective address
//   laddr = self.get_laddr64(seg_idx, eaddr)     -- u64 linear address
//   val   = self.read_rmw_linear_qword(seg, laddr) -> (u64, u64)  (value, laddr)
//   self.write_rmw_linear_qword(laddr, val)
//   val   = self.read_linear_qword(seg, laddr)   -- plain read (no write-back)
//
// Flag update helpers (update_flags_add64 / update_flags_sub64) live in
// this file as private helpers on BxCpuC, following the same
// ADD_COUT_VEC / SUB_COUT_VEC carry-vector formulas used by the 32-bit
// counterparts in cpu.rs.

use super::{BxCpuC, BxCpuIdTrait};
use crate::cpu::decoder::{BxSegregs, Instruction};
use crate::cpu::eflags::EFlags;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Flag update helpers for 64-bit arithmetic
    // =========================================================================
    // Mirrors Bochs SET_FLAGS_OSZAPC_ADD_64 / SET_FLAGS_OSZAPC_SUB_64.
    // The carry-vector formula computes the carry/borrow bit at every bit
    // position in a single expression, so ADC/SBB (which wrap the carry into
    // the result before calling here) are handled correctly.

    /// SET_FLAGS_OSZAPC_ADD_64: update all six arithmetic flags after a 64-bit add.
    pub(super) fn update_flags_add64(&mut self, op1: u64, op2: u64, res: u64) {
        // ADD_COUT_VEC: carry-out at each bit position
        let cout_vec = (op1 & op2) | ((op1 | op2) & !res);
        let cf = (cout_vec >> 63) & 1 != 0;
        let zf = res == 0;
        let sf = (res & 0x8000_0000_0000_0000) != 0;
        // GET_ADD_OVERFLOW: overflow when both operands have the same sign but the
        // result has a different sign.
        let of = ((op1 ^ res) & (op2 ^ res) & 0x8000_0000_0000_0000) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let parity = (res as u8).count_ones() % 2 == 0;

        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if parity {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    /// SET_FLAGS_OSZAPC_SUB_64: update all six arithmetic flags after a 64-bit sub.
    pub(super) fn update_flags_sub64(&mut self, op1: u64, op2: u64, res: u64) {
        // SUB_COUT_VEC: borrow at each bit position
        let cout_vec = (!op1 & op2) | ((!op1 ^ op2) & res);
        let cf = (cout_vec >> 63) & 1 != 0;
        let zf = res == 0;
        let sf = (res & 0x8000_0000_0000_0000) != 0;
        // GET_SUB_OVERFLOW: overflow when operands have different signs and the
        // result sign differs from op1.
        let of = ((op1 ^ op2) & (op1 ^ res) & 0x8000_0000_0000_0000) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let parity = (res as u8).count_ones() % 2 == 0;

        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if parity {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    /// SET_FLAGS_OSZAP_ADD_64: update OSZAP flags only (no CF) after a 64-bit add.
    /// Used by INC which must preserve CF.
    fn update_flags_oszap_add64(&mut self, op1: u64, op2: u64, res: u64) {
        let cout_vec = (op1 & op2) | ((op1 | op2) & !res);
        let zf = res == 0;
        let sf = (res & 0x8000_0000_0000_0000) != 0;
        let of = ((op1 ^ res) & (op2 ^ res) & 0x8000_0000_0000_0000) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let parity = (res as u8).count_ones() % 2 == 0;

        // Clear OSZAP but preserve CF
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::ZF | EFlags::AF | EFlags::PF);

        if parity {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    /// SET_FLAGS_OSZAP_SUB_64: update OSZAP flags only (no CF) after a 64-bit sub.
    /// Used by DEC which must preserve CF.
    fn update_flags_oszap_sub64(&mut self, op1: u64, op2: u64, res: u64) {
        let cout_vec = (!op1 & op2) | ((!op1 ^ op2) & res);
        let zf = res == 0;
        let sf = (res & 0x8000_0000_0000_0000) != 0;
        let of = ((op1 ^ op2) & (op1 ^ res) & 0x8000_0000_0000_0000) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let parity = (res as u8).count_ones() % 2 == 0;

        // Clear OSZAP but preserve CF
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::ZF | EFlags::AF | EFlags::PF);

        if parity {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    // =========================================================================
    // ADD — 64-bit
    // Bochs: ADD_EqGqM / ADD_GqEqR / ADD_GqEqM / ADD_EqIdM / ADD_EqIdR
    // =========================================================================

    /// ADD r/m64, r64 (memory form) — Bochs ADD_EqGqM
    pub fn add_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2 = self.get_gpr64(instr.src() as usize);
        let sum = op1.wrapping_add(op2);
        self.write_rmw_linear_qword(rmw_laddr, sum);
        self.update_flags_add64(op1, op2, sum);
        Ok(())
    }

    /// ADD r64, r64 (register form) — Bochs ADD_GqEqR
    pub fn add_gq_eq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.get_gpr64(instr.src() as usize);
        let sum = op1.wrapping_add(op2);
        self.set_gpr64(instr.dst() as usize, sum);
        self.update_flags_add64(op1, op2, sum);
    }

    /// ADD r64, r/m64 (memory form) — Bochs ADD_GqEqM
    pub fn add_gq_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.read_linear_qword(seg, laddr)?;
        let sum = op1.wrapping_add(op2);
        self.set_gpr64(instr.dst() as usize, sum);
        self.update_flags_add64(op1, op2, sum);
        Ok(())
    }

    /// ADD r/m64, imm32 sign-extended (memory form) — Bochs ADD_EqIdM
    /// The immediate is a sign-extended 32-bit value (i->Id() cast to Bit32s then u64).
    pub fn add_eq_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2 = instr.id() as i32 as i64 as u64; // sign-extend 32-bit imm to 64 bits
        let sum = op1.wrapping_add(op2);
        self.write_rmw_linear_qword(rmw_laddr, sum);
        self.update_flags_add64(op1, op2, sum);
        Ok(())
    }

    /// ADD r/m64, imm32 sign-extended (register form) — Bochs ADD_EqIdR
    pub fn add_eq_id_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = instr.id() as i32 as i64 as u64;
        let sum = op1.wrapping_add(op2);
        self.set_gpr64(instr.dst() as usize, sum);
        self.update_flags_add64(op1, op2, sum);
    }

    // =========================================================================
    // ADC — Add with Carry, 64-bit
    // Bochs: ADC_EqGqM / ADC_GqEqR / ADC_GqEqM / ADC_EqIdM / ADC_EqIdR
    // =========================================================================

    /// ADC r/m64, r64 (memory form) — Bochs ADC_EqGqM
    pub fn adc_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2 = self.get_gpr64(instr.src() as usize);
        let cf = self.get_cf() as u64;
        let sum = op1.wrapping_add(op2).wrapping_add(cf);
        self.write_rmw_linear_qword(rmw_laddr, sum);
        self.update_flags_add64(op1, op2, sum);
        Ok(())
    }

    /// ADC r64, r64 (register form) — Bochs ADC_GqEqR
    pub fn adc_gq_eq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.get_gpr64(instr.src() as usize);
        let cf = self.get_cf() as u64;
        let sum = op1.wrapping_add(op2).wrapping_add(cf);
        self.set_gpr64(instr.dst() as usize, sum);
        self.update_flags_add64(op1, op2, sum);
    }

    /// ADC r64, r/m64 (memory form) — Bochs ADC_GqEqM
    pub fn adc_gq_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.read_linear_qword(seg, laddr)?;
        let cf = self.get_cf() as u64;
        let sum = op1.wrapping_add(op2).wrapping_add(cf);
        self.set_gpr64(instr.dst() as usize, sum);
        self.update_flags_add64(op1, op2, sum);
        Ok(())
    }

    /// ADC r/m64, imm32 sign-extended (memory form) — Bochs ADC_EqIdM
    pub fn adc_eq_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2 = instr.id() as i32 as i64 as u64;
        let cf = self.get_cf() as u64;
        let sum = op1.wrapping_add(op2).wrapping_add(cf);
        self.write_rmw_linear_qword(rmw_laddr, sum);
        self.update_flags_add64(op1, op2, sum);
        Ok(())
    }

    /// ADC r/m64, imm32 sign-extended (register form) — Bochs ADC_EqIdR
    pub fn adc_eq_id_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = instr.id() as i32 as i64 as u64;
        let cf = self.get_cf() as u64;
        let sum = op1.wrapping_add(op2).wrapping_add(cf);
        self.set_gpr64(instr.dst() as usize, sum);
        self.update_flags_add64(op1, op2, sum);
    }

    // =========================================================================
    // SBB — Subtract with Borrow, 64-bit
    // Bochs: SBB_EqGqM / SBB_GqEqR / SBB_GqEqM / SBB_EqIdM / SBB_EqIdR
    // =========================================================================

    /// SBB r/m64, r64 (memory form) — Bochs SBB_EqGqM
    pub fn sbb_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2 = self.get_gpr64(instr.src() as usize);
        let cf = self.get_cf() as u64;
        // Bochs: diff = op1 - (op2 + CF)
        let diff = op1.wrapping_sub(op2.wrapping_add(cf));
        self.write_rmw_linear_qword(rmw_laddr, diff);
        self.update_flags_sub64(op1, op2, diff);
        Ok(())
    }

    /// SBB r64, r64 (register form) — Bochs SBB_GqEqR
    pub fn sbb_gq_eq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.get_gpr64(instr.src() as usize);
        let cf = self.get_cf() as u64;
        let diff = op1.wrapping_sub(op2.wrapping_add(cf));
        self.set_gpr64(instr.dst() as usize, diff);
        self.update_flags_sub64(op1, op2, diff);
    }

    /// SBB r64, r/m64 (memory form) — Bochs SBB_GqEqM
    pub fn sbb_gq_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.read_linear_qword(seg, laddr)?;
        let cf = self.get_cf() as u64;
        let diff = op1.wrapping_sub(op2.wrapping_add(cf));
        self.set_gpr64(instr.dst() as usize, diff);
        self.update_flags_sub64(op1, op2, diff);
        Ok(())
    }

    /// SBB r/m64, imm32 sign-extended (memory form) — Bochs SBB_EqIdM
    pub fn sbb_eq_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2 = instr.id() as i32 as i64 as u64;
        let cf = self.get_cf() as u64;
        let diff = op1.wrapping_sub(op2.wrapping_add(cf));
        self.write_rmw_linear_qword(rmw_laddr, diff);
        self.update_flags_sub64(op1, op2, diff);
        Ok(())
    }

    /// SBB r/m64, imm32 sign-extended (register form) — Bochs SBB_EqIdR
    pub fn sbb_eq_id_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = instr.id() as i32 as i64 as u64;
        let cf = self.get_cf() as u64;
        let diff = op1.wrapping_sub(op2.wrapping_add(cf));
        self.set_gpr64(instr.dst() as usize, diff);
        self.update_flags_sub64(op1, op2, diff);
    }

    // =========================================================================
    // SUB — 64-bit
    // Bochs: SUB_EqGqM / SUB_GqEqR / SUB_GqEqM / SUB_EqIdM / SUB_EqIdR
    // =========================================================================

    /// SUB r/m64, r64 (memory form) — Bochs SUB_EqGqM
    pub fn sub_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2 = self.get_gpr64(instr.src() as usize);
        let diff = op1.wrapping_sub(op2);
        self.write_rmw_linear_qword(rmw_laddr, diff);
        self.update_flags_sub64(op1, op2, diff);
        Ok(())
    }

    /// SUB r64, r64 (register form) — Bochs SUB_GqEqR
    pub fn sub_gq_eq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.get_gpr64(instr.src() as usize);
        let diff = op1.wrapping_sub(op2);
        self.set_gpr64(instr.dst() as usize, diff);
        self.update_flags_sub64(op1, op2, diff);
    }

    /// SUB r64, r/m64 (memory form) — Bochs SUB_GqEqM
    pub fn sub_gq_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.read_linear_qword(seg, laddr)?;
        let diff = op1.wrapping_sub(op2);
        self.set_gpr64(instr.dst() as usize, diff);
        self.update_flags_sub64(op1, op2, diff);
        Ok(())
    }

    /// SUB r/m64, imm32 sign-extended (memory form) — Bochs SUB_EqIdM
    pub fn sub_eq_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2 = instr.id() as i32 as i64 as u64;
        let diff = op1.wrapping_sub(op2);
        self.write_rmw_linear_qword(rmw_laddr, diff);
        self.update_flags_sub64(op1, op2, diff);
        Ok(())
    }

    /// SUB r/m64, imm32 sign-extended (register form) — Bochs SUB_EqIdR
    pub fn sub_eq_id_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = instr.id() as i32 as i64 as u64;
        let diff = op1.wrapping_sub(op2);
        self.set_gpr64(instr.dst() as usize, diff);
        self.update_flags_sub64(op1, op2, diff);
    }

    // =========================================================================
    // CMP — 64-bit (sets flags, no writeback)
    // Bochs: CMP_EqGqM / CMP_GqEqR / CMP_GqEqM / CMP_EqIdM / CMP_EqIdR
    // =========================================================================

    /// CMP r/m64, r64 (memory form) — Bochs CMP_EqGqM
    pub fn cmp_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1 = self.read_linear_qword(seg, laddr)?;
        let op2 = self.get_gpr64(instr.src() as usize);
        let diff = op1.wrapping_sub(op2);
        self.update_flags_sub64(op1, op2, diff);
        Ok(())
    }

    /// CMP r64, r64 (register form) — Bochs CMP_GqEqR
    pub fn cmp_gq_eq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.get_gpr64(instr.src() as usize);
        let diff = op1.wrapping_sub(op2);
        self.update_flags_sub64(op1, op2, diff);
    }

    /// CMP r64, r/m64 (memory form) — Bochs CMP_GqEqM
    pub fn cmp_gq_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.read_linear_qword(seg, laddr)?;
        let diff = op1.wrapping_sub(op2);
        self.update_flags_sub64(op1, op2, diff);
        Ok(())
    }

    /// CMP r/m64, imm32 sign-extended (memory form) — Bochs CMP_EqIdM
    pub fn cmp_eq_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1 = self.read_linear_qword(seg, laddr)?;
        let op2 = instr.id() as i32 as i64 as u64;
        let diff = op1.wrapping_sub(op2);
        self.update_flags_sub64(op1, op2, diff);
        Ok(())
    }

    /// CMP r/m64, imm32 sign-extended (register form) — Bochs CMP_EqIdR
    pub fn cmp_eq_id_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = instr.id() as i32 as i64 as u64;
        let diff = op1.wrapping_sub(op2);
        self.update_flags_sub64(op1, op2, diff);
    }

    // =========================================================================
    // NEG — Two's Complement Negation, 64-bit
    // Bochs: NEG_EqM / NEG_EqR
    // =========================================================================

    /// NEG r/m64 (memory form) — Bochs NEG_EqM
    /// Note: Bochs stores result back then derives op1 = -result for SET_FLAGS.
    /// We preserve the original value for the flag helper.
    pub fn neg_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1_orig, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let result = 0u64.wrapping_sub(op1_orig);
        self.write_rmw_linear_qword(rmw_laddr, result);
        // Bochs: SET_FLAGS_OSZAPC_SUB_64(0, -op1_64, op1_64)
        // After negation: op1_stored = result, -op1_stored = op1_orig.
        // Equivalent to sub(0, op1_orig, result).
        self.update_flags_sub64(0, op1_orig, result);
        Ok(())
    }

    /// NEG r64 (register form) — Bochs NEG_EqR
    pub fn neg_eq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let result = 0u64.wrapping_sub(op1);
        self.set_gpr64(instr.dst() as usize, result);
        self.update_flags_sub64(0, op1, result);
    }

    // =========================================================================
    // INC — Increment, 64-bit (preserves CF)
    // Bochs: INC_EqM / INC_EqR
    // =========================================================================

    /// INC r/m64 (memory form) — Bochs INC_EqM
    pub fn inc_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let result = op1.wrapping_add(1);
        self.write_rmw_linear_qword(rmw_laddr, result);
        // Bochs: SET_FLAGS_OSZAP_ADD_64(op1_64 - 1, 0, op1_64)
        // which is the value before increment, 0, value after.
        // With op1=pre-increment value, result=post-increment:
        self.update_flags_oszap_add64(op1, 0, result);
        Ok(())
    }

    /// INC r64 (register form) — Bochs INC_EqR
    pub fn inc_eq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let result = op1.wrapping_add(1);
        self.set_gpr64(instr.dst() as usize, result);
        // Bochs: ++BX_READ_64BIT_REG then SET_FLAGS_OSZAP_ADD_64(rrx-1, 0, rrx)
        self.update_flags_oszap_add64(op1, 0, result);
    }

    // =========================================================================
    // DEC — Decrement, 64-bit (preserves CF)
    // Bochs: DEC_EqM / DEC_EqR
    // =========================================================================

    /// DEC r/m64 (memory form) — Bochs DEC_EqM
    pub fn dec_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let result = op1.wrapping_sub(1);
        self.write_rmw_linear_qword(rmw_laddr, result);
        // Bochs: SET_FLAGS_OSZAP_SUB_64(op1_64 + 1, 0, op1_64)
        // i.e. SET_FLAGS_OSZAP_SUB_64(pre-decrement, 0, post-decrement)
        self.update_flags_oszap_sub64(op1, 0, result);
        Ok(())
    }

    /// DEC r64 (register form) — Bochs DEC_EqR
    pub fn dec_eq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let result = op1.wrapping_sub(1);
        self.set_gpr64(instr.dst() as usize, result);
        // Bochs: --BX_READ_64BIT_REG then SET_FLAGS_OSZAP_SUB_64(rrx+1, 0, rrx)
        self.update_flags_oszap_sub64(op1, 0, result);
    }

    // =========================================================================
    // XADD — Exchange and Add, 64-bit
    // Bochs: XADD_EqGqM / XADD_EqGqR
    // =========================================================================

    /// XADD r/m64, r64 (memory form) — Bochs XADD_EqGqM
    /// temp <- src + dst; src <- dst; dst <- temp
    pub fn xadd_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2 = self.get_gpr64(instr.src() as usize);
        let sum = op1.wrapping_add(op2);
        self.write_rmw_linear_qword(rmw_laddr, sum);
        // src <- original dst
        self.set_gpr64(instr.src() as usize, op1);
        self.update_flags_add64(op1, op2, sum);
        Ok(())
    }

    /// XADD r64, r64 (register form) — Bochs XADD_EqGqR
    /// For XADD AL,AL: write src first (gets op1), then dst (gets sum).
    pub fn xadd_eq_gq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.get_gpr64(instr.src() as usize);
        let sum = op1.wrapping_add(op2);
        // Write src first, then dst — so if src==dst, sum wins.
        self.set_gpr64(instr.src() as usize, op1);
        self.set_gpr64(instr.dst() as usize, sum);
        self.update_flags_add64(op1, op2, sum);
    }

    // =========================================================================
    // CMPXCHG — Compare and Exchange, 64-bit
    // Bochs: CMPXCHG_EqGqM / CMPXCHG_EqGqR
    // =========================================================================

    /// CMPXCHG r/m64, r64 (memory form) — Bochs CMPXCHG_EqGqM
    /// Compare RAX with dst; if equal, load src into dst; else load dst into RAX.
    pub fn cmpxchg_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let rax = self.get_gpr64(0); // RAX = index 0
        let diff = rax.wrapping_sub(op1);
        self.update_flags_sub64(rax, op1, diff);

        if diff == 0 {
            // dest <- src
            let src = self.get_gpr64(instr.src() as usize);
            self.write_rmw_linear_qword(rmw_laddr, src);
        } else {
            // accumulator <- dest
            self.write_rmw_linear_qword(rmw_laddr, op1);
            self.set_gpr64(0, op1); // RAX <- dest
        }
        Ok(())
    }

    /// CMPXCHG r64, r64 (register form) — Bochs CMPXCHG_EqGqR
    pub fn cmpxchg_eq_gq_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let rax = self.get_gpr64(0);
        let diff = rax.wrapping_sub(op1);
        self.update_flags_sub64(rax, op1, diff);

        if diff == 0 {
            // dest <- src
            let src = self.get_gpr64(instr.src() as usize);
            self.set_gpr64(instr.dst() as usize, src);
        } else {
            // RAX <- dest
            self.set_gpr64(0, op1);
        }
    }

    // =========================================================================
    // CMPXCHG16B — Compare and Exchange 16 Bytes
    // Bochs: CMPXCHG16B
    // =========================================================================

    /// CMPXCHG16B m128
    /// Compares RDX:RAX with m128. If equal, sets ZF and stores RCX:RBX into m128.
    /// Otherwise clears ZF and loads m128 into RDX:RAX.
    /// Bochs: read_RMW_linear_dqword_aligned_64 + write_RMW_linear_dqword.
    /// Since the operand is 16-byte aligned, both qwords are always on the same page.
    /// We translate once with write permission and do direct physical reads/writes.
    pub fn cmpxchg16b(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);

        // Bochs: #GP(0) if not 16-byte aligned
        if (laddr & 0xF) != 0 {
            self.exception(super::cpu::Exception::Gp, 0)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // 16-byte aligned → both qwords on same page. Translate once with write access.
        let paddr = self.translate_data_write(laddr)?;

        let op1_lo = self.mem_read_qword(paddr);
        let op1_hi = self.mem_read_qword(paddr + 8);

        let rax = self.get_gpr64(0); // RAX
        let rdx = self.get_gpr64(2); // RDX

        // diff = (RAX - op1_lo) | (RDX - op1_hi)
        let diff = rax.wrapping_sub(op1_lo) | rdx.wrapping_sub(op1_hi);

        if diff == 0 {
            // dest <- RCX:RBX
            let rbx = self.get_gpr64(3);
            let rcx = self.get_gpr64(1);
            self.mem_write_qword(paddr, rbx);
            self.mem_write_qword(paddr + 8, rcx);
            self.eflags.insert(EFlags::ZF);
        } else {
            // write back original (Bochs: write_RMW_linear_dqword(hi, lo))
            self.mem_write_qword(paddr, op1_lo);
            self.mem_write_qword(paddr + 8, op1_hi);
            self.eflags.remove(EFlags::ZF);
            // RAX <- op1_lo, RDX <- op1_hi
            self.set_gpr64(0, op1_lo);
            self.set_gpr64(2, op1_hi);
        }
        Ok(())
    }

    // =========================================================================
    // CDQE / CQO — Sign Extension, 64-bit
    // Bochs: CDQE / CQO (arith64.cc)
    // =========================================================================

    /// CDQE: sign-extend EAX into RAX — Bochs CDQE
    /// No flags affected.
    pub fn cdqe(&mut self, _instr: &Instruction) {
        let eax = self.get_gpr32(0); // EAX
        let rax = eax as i32 as i64 as u64; // sign-extend 32→64
        self.set_gpr64(0, rax);
    }

    /// CQO: sign-extend RAX into RDX:RAX — Bochs CQO
    /// No flags affected.
    pub fn cqo(&mut self, _instr: &Instruction) {
        let rax = self.get_gpr64(0);
        let rdx = if (rax & 0x8000_0000_0000_0000) != 0 {
            u64::MAX
        } else {
            0
        };
        self.set_gpr64(2, rdx); // RDX
    }

    // =========================================================================
    // Accumulator-immediate forms (RAX, imm32 sign-extended to 64)
    // =========================================================================

    /// ADD RAX, imm32 (sign-extended to 64) — Bochs ADD_RAXId → ADD_EqIdR
    /// Accumulator form: hardcodes RAX as destination (decoder stores rm != 0).
    pub fn add_rax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(0); // RAX
        let op2 = instr.id() as i32 as i64 as u64;
        let sum = op1.wrapping_add(op2);
        self.set_gpr64(0, sum);
        self.update_flags_add64(op1, op2, sum);
    }

    /// ADC RAX, imm32 (sign-extended to 64) — Bochs ADC_RAXId → ADC_EqIdR
    pub fn adc_rax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(0); // RAX
        let op2 = instr.id() as i32 as i64 as u64;
        let cf = self.get_cf() as u64;
        let sum = op1.wrapping_add(op2).wrapping_add(cf);
        self.set_gpr64(0, sum);
        self.update_flags_add64(op1, op2, sum);
    }

    /// SBB RAX, imm32 (sign-extended to 64) — Bochs SBB_RAXId → SBB_EqIdR
    pub fn sbb_rax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(0); // RAX
        let op2 = instr.id() as i32 as i64 as u64;
        let cf = self.get_cf() as u64;
        let diff = op1.wrapping_sub(op2.wrapping_add(cf));
        self.set_gpr64(0, diff);
        self.update_flags_sub64(op1, op2, diff);
    }

    /// SUB RAX, imm32 (sign-extended to 64) — Bochs SUB_RAXId → SUB_EqIdR
    pub fn sub_rax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(0); // RAX
        let op2 = instr.id() as i32 as i64 as u64;
        let diff = op1.wrapping_sub(op2);
        self.set_gpr64(0, diff);
        self.update_flags_sub64(op1, op2, diff);
    }

    // =========================================================================
    // Unified dispatch handlers (R vs M)
    // =========================================================================

    // --- ADD unified dispatchers ---

    /// ADD r/m64, r64 - unified dispatch (store-direction)
    pub fn add_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let op1 = self.get_gpr64(instr.dst() as usize);
            let op2 = self.get_gpr64(instr.src() as usize);
            let sum = op1.wrapping_add(op2);
            self.set_gpr64(instr.dst() as usize, sum);
            self.update_flags_add64(op1, op2, sum);
            Ok(())
        } else {
            self.add_eq_gq_m(instr)
        }
    }

    /// ADD r64, r/m64 - unified dispatch (load-direction)
    pub fn add_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.add_gq_eq_r(instr);
            Ok(())
        } else {
            self.add_gq_eq_m(instr)
        }
    }

    /// ADD r/m64, imm32 (sign-extended) - unified dispatch
    pub fn add_eq_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.add_eq_id_r(instr);
            Ok(())
        } else {
            self.add_eq_id_m(instr)
        }
    }

    /// ADD r/m64, imm8 (sign-extended) - unified dispatch
    /// The decoder already sign-extends the immediate to 32 bits, so we reuse _id forms.
    pub fn add_eqs_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.add_eq_id_r(instr);
            Ok(())
        } else {
            self.add_eq_id_m(instr)
        }
    }

    // --- ADC unified dispatchers ---

    /// ADC r/m64, r64 - unified dispatch (store-direction)
    pub fn adc_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let op1 = self.get_gpr64(instr.dst() as usize);
            let op2 = self.get_gpr64(instr.src() as usize);
            let cf = self.get_cf() as u64;
            let sum = op1.wrapping_add(op2).wrapping_add(cf);
            self.set_gpr64(instr.dst() as usize, sum);
            self.update_flags_add64(op1, op2, sum);
            Ok(())
        } else {
            self.adc_eq_gq_m(instr)
        }
    }

    /// ADC r64, r/m64 - unified dispatch (load-direction)
    pub fn adc_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.adc_gq_eq_r(instr);
            Ok(())
        } else {
            self.adc_gq_eq_m(instr)
        }
    }

    /// ADC r/m64, imm32 (sign-extended) - unified dispatch
    pub fn adc_eq_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.adc_eq_id_r(instr);
            Ok(())
        } else {
            self.adc_eq_id_m(instr)
        }
    }

    /// ADC r/m64, imm8 (sign-extended) - unified dispatch
    /// The decoder already sign-extends the immediate to 32 bits, so we reuse _id forms.
    pub fn adc_eqs_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.adc_eq_id_r(instr);
            Ok(())
        } else {
            self.adc_eq_id_m(instr)
        }
    }

    // --- SBB unified dispatchers ---

    /// SBB r/m64, r64 - unified dispatch (store-direction)
    pub fn sbb_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let op1 = self.get_gpr64(instr.dst() as usize);
            let op2 = self.get_gpr64(instr.src() as usize);
            let cf = self.get_cf() as u64;
            let diff = op1.wrapping_sub(op2.wrapping_add(cf));
            self.set_gpr64(instr.dst() as usize, diff);
            self.update_flags_sub64(op1, op2, diff);
            Ok(())
        } else {
            self.sbb_eq_gq_m(instr)
        }
    }

    /// SBB r64, r/m64 - unified dispatch (load-direction)
    pub fn sbb_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.sbb_gq_eq_r(instr);
            Ok(())
        } else {
            self.sbb_gq_eq_m(instr)
        }
    }

    /// SBB r/m64, imm32 (sign-extended) - unified dispatch
    pub fn sbb_eq_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.sbb_eq_id_r(instr);
            Ok(())
        } else {
            self.sbb_eq_id_m(instr)
        }
    }

    /// SBB r/m64, imm8 (sign-extended) - unified dispatch
    /// The decoder already sign-extends the immediate to 32 bits, so we reuse _id forms.
    pub fn sbb_eqs_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.sbb_eq_id_r(instr);
            Ok(())
        } else {
            self.sbb_eq_id_m(instr)
        }
    }

    // --- SUB unified dispatchers ---

    /// SUB r/m64, r64 - unified dispatch (store-direction)
    pub fn sub_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let op1 = self.get_gpr64(instr.dst() as usize);
            let op2 = self.get_gpr64(instr.src() as usize);
            let diff = op1.wrapping_sub(op2);
            self.set_gpr64(instr.dst() as usize, diff);
            self.update_flags_sub64(op1, op2, diff);
            Ok(())
        } else {
            self.sub_eq_gq_m(instr)
        }
    }

    /// SUB r64, r/m64 - unified dispatch (load-direction)
    pub fn sub_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.sub_gq_eq_r(instr);
            Ok(())
        } else {
            self.sub_gq_eq_m(instr)
        }
    }

    /// SUB r/m64, imm32 (sign-extended) - unified dispatch
    pub fn sub_eq_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.sub_eq_id_r(instr);
            Ok(())
        } else {
            self.sub_eq_id_m(instr)
        }
    }

    /// SUB r/m64, imm8 (sign-extended) - unified dispatch
    /// The decoder already sign-extends the immediate to 32 bits, so we reuse _id forms.
    pub fn sub_eqs_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.sub_eq_id_r(instr);
            Ok(())
        } else {
            self.sub_eq_id_m(instr)
        }
    }

    // --- CMP unified dispatchers ---

    /// CMP r/m64, imm8 (sign-extended) - unified dispatch
    /// The decoder already sign-extends the immediate to 32 bits, so we reuse _id forms.
    pub fn cmp_eqs_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_eq_id_r(instr);
            Ok(())
        } else {
            self.cmp_eq_id_m(instr)
        }
    }

    // --- NEG unified dispatcher ---

    /// NEG r/m64 - unified dispatch
    pub fn neg_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.neg_eq_r(instr);
            Ok(())
        } else {
            self.neg_eq_m(instr)
        }
    }

    // --- XADD unified dispatcher ---

    /// XADD r/m64, r64 - unified dispatch
    pub fn xadd_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xadd_eq_gq_r(instr);
            Ok(())
        } else {
            self.xadd_eq_gq_m(instr)
        }
    }

    // --- CMPXCHG unified dispatcher ---

    /// CMPXCHG r/m64, r64 - unified dispatch
    pub fn cmpxchg_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmpxchg_eq_gq_r(instr);
            Ok(())
        } else {
            self.cmpxchg_eq_gq_m(instr)
        }
    }
}
