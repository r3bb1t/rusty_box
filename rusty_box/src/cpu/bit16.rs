//! 16-bit bit scan and bit test instructions
//! Matching Bochs bit16.cc
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // BSF / BSR — Bit Scan Forward / Reverse 16-bit (0F BC / 0F BD with 66h)
    // =========================================================================

    /// BSF r16, r/m16 — Bit Scan Forward 16-bit (0F BC /r with 66h prefix)
    /// Bochs bit16.cc: SET_FLAGS_OSZAPC_LOGIC_16(val_16); clear_ZF();
    pub fn bsf_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr16(instr.src() as usize) as u32
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_word(seg, eaddr)? as u32
        };
        if op2 == 0 {
            self.set_zf(true);
        } else {
            let idx = op2.trailing_zeros();
            self.set_flags_oszapc_logic_16(idx as u16);
            self.set_zf(false);
            self.set_gpr16(instr.dst() as usize, idx as u16);
        }
        Ok(())
    }

    /// BSR r16, r/m16 — Bit Scan Reverse 16-bit
    /// Bochs bit16.cc: SET_FLAGS_OSZAPC_LOGIC_16(val_16); clear_ZF();
    /// BUG FIX: was `15 - op2.leading_zeros()` which wraps for u32 with upper 16 bits zero.
    /// Correct: `31 - op2.leading_zeros()` since op2 is zero-extended u32.
    pub fn bsr_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr16(instr.src() as usize) as u32
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_word(seg, eaddr)? as u32
        };
        if op2 == 0 {
            self.set_zf(true);
        } else {
            let idx = 31 - op2.leading_zeros();
            self.set_flags_oszapc_logic_16(idx as u16);
            self.set_zf(false);
            self.set_gpr16(instr.dst() as usize, idx as u16);
        }
        Ok(())
    }
}
