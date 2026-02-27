//! Bit scan instructions: BSF, BSR
//! Matching Bochs bit32.cc
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{Instruction, BxSegregs},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // BSF / BSR — Bit Scan Forward / Reverse (0F BC / 0F BD)
    // =========================================================================

    /// BSF r32, r/m32 — Bit Scan Forward (0F BC /r)
    /// Scans src from bit 0 upward for first set bit. Sets dst = bit index.
    /// If src == 0, ZF=1 and dst is undefined; otherwise ZF=0.
    /// Bochs: logical32.cc BSF_GdEd
    pub fn bsf_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_dword(seg, eaddr)?
        };
        if op2 == 0 {
            // ZF=1, dst undefined
            self.eflags |= 1 << 6; // ZF
        } else {
            let idx = op2.trailing_zeros();
            self.set_gpr32(instr.dst() as usize, idx);
            self.eflags &= !(1 << 6); // ZF=0
        }
        Ok(())
    }

    /// BSR r32, r/m32 — Bit Scan Reverse (0F BD /r)
    /// Scans src from MSB downward for first set bit. Sets dst = bit index.
    /// If src == 0, ZF=1 and dst is undefined; otherwise ZF=0.
    /// Bochs: logical32.cc BSR_GdEd
    pub fn bsr_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_dword(seg, eaddr)?
        };
        if op2 == 0 {
            self.eflags |= 1 << 6; // ZF=1
        } else {
            let idx = 31 - op2.leading_zeros();
            self.set_gpr32(instr.dst() as usize, idx);
            self.eflags &= !(1 << 6); // ZF=0
        }
        Ok(())
    }

    /// BSF r16, r/m16 — Bit Scan Forward 16-bit (0F BC /r with 66h prefix)
    pub fn bsf_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr16(instr.src() as usize) as u32
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_word(seg, eaddr)? as u32
        };
        if op2 == 0 {
            self.eflags |= 1 << 6;
        } else {
            let idx = op2.trailing_zeros();
            self.set_gpr16(instr.dst() as usize, idx as u16);
            self.eflags &= !(1 << 6);
        }
        Ok(())
    }

    /// BSR r16, r/m16 — Bit Scan Reverse 16-bit
    pub fn bsr_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = if instr.mod_c0() {
            self.get_gpr16(instr.src() as usize) as u32
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_word(seg, eaddr)? as u32
        };
        if op2 == 0 {
            self.eflags |= 1 << 6;
        } else {
            let idx = 15 - op2.leading_zeros();
            self.set_gpr16(instr.dst() as usize, idx as u16);
            self.eflags &= !(1 << 6);
        }
        Ok(())
    }
}
