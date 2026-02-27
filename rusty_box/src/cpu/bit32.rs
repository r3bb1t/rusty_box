//! 32-bit bit scan instructions: BSF, BSR
//! Matching Bochs bit32.cc
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{Instruction, BxSegregs},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // BSF / BSR — Bit Scan Forward / Reverse (0F BC / 0F BD)
    // =========================================================================

    /// BSF r32, r/m32 — Bit Scan Forward (0F BC /r)
    /// Bochs bit32.cc: SET_FLAGS_OSZAPC_LOGIC_32(val_32); clear_ZF();
    pub fn bsf_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_dword(seg, eaddr)?
        };
        if op2 == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            let idx = op2.trailing_zeros();
            self.set_flags_oszapc_logic_32(idx);
            self.eflags.remove(EFlags::ZF);
            self.set_gpr32(instr.dst() as usize, idx);
        }
        Ok(())
    }

    /// BSR r32, r/m32 — Bit Scan Reverse (0F BD /r)
    /// Bochs bit32.cc: SET_FLAGS_OSZAPC_LOGIC_32(val_32); clear_ZF();
    pub fn bsr_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_dword(seg, eaddr)?
        };
        if op2 == 0 {
            self.eflags.insert(EFlags::ZF);
        } else {
            let idx = 31 - op2.leading_zeros();
            self.set_flags_oszapc_logic_32(idx);
            self.eflags.remove(EFlags::ZF);
            self.set_gpr32(instr.dst() as usize, idx);
        }
        Ok(())
    }

}
