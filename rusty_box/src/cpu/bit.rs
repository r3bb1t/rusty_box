//! Bit manipulation instructions: SETcc
//! Matching Bochs bit.cc
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{Instruction, BxSegregs},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // SETcc Eb — Set byte on condition (0F 90..9F)
    // =========================================================================

    /// Write val (0 or 1) to r/m8 operand. Used by SETcc.
    fn setcc_eb(&mut self, instr: &Instruction, val: u8) -> super::Result<()> {
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let ext = instr.extend8bit_l();
            self.write_8bit_regx(dst, ext, val);
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.write_virtual_byte(seg, eaddr, val)?;
        }
        Ok(())
    }

    pub fn seto_eb(&mut self, instr: &Instruction) -> super::Result<()>   { self.setcc_eb(instr, self.get_of() as u8) }
    pub fn setno_eb(&mut self, instr: &Instruction) -> super::Result<()>  { self.setcc_eb(instr, (!self.get_of()) as u8) }
    pub fn setb_eb(&mut self, instr: &Instruction) -> super::Result<()>   { self.setcc_eb(instr, self.get_cf() as u8) }
    pub fn setnb_eb(&mut self, instr: &Instruction) -> super::Result<()>  { self.setcc_eb(instr, (!self.get_cf()) as u8) }
    pub fn setz_eb(&mut self, instr: &Instruction) -> super::Result<()>   { self.setcc_eb(instr, self.get_zf() as u8) }
    pub fn setnz_eb(&mut self, instr: &Instruction) -> super::Result<()>  { self.setcc_eb(instr, (!self.get_zf()) as u8) }
    pub fn setbe_eb(&mut self, instr: &Instruction) -> super::Result<()>  { self.setcc_eb(instr, (self.get_cf() || self.get_zf()) as u8) }
    pub fn setnbe_eb(&mut self, instr: &Instruction) -> super::Result<()> { self.setcc_eb(instr, (!self.get_cf() && !self.get_zf()) as u8) }
    pub fn sets_eb(&mut self, instr: &Instruction) -> super::Result<()>   { self.setcc_eb(instr, self.get_sf() as u8) }
    pub fn setns_eb(&mut self, instr: &Instruction) -> super::Result<()>  { self.setcc_eb(instr, (!self.get_sf()) as u8) }
    pub fn setp_eb(&mut self, instr: &Instruction) -> super::Result<()>   { self.setcc_eb(instr, self.get_pf() as u8) }
    pub fn setnp_eb(&mut self, instr: &Instruction) -> super::Result<()>  { self.setcc_eb(instr, (!self.get_pf()) as u8) }
    pub fn setl_eb(&mut self, instr: &Instruction) -> super::Result<()>   { self.setcc_eb(instr, (self.get_sf() != self.get_of()) as u8) }
    pub fn setnl_eb(&mut self, instr: &Instruction) -> super::Result<()>  { self.setcc_eb(instr, (self.get_sf() == self.get_of()) as u8) }
    pub fn setle_eb(&mut self, instr: &Instruction) -> super::Result<()>  { self.setcc_eb(instr, (self.get_zf() || (self.get_sf() != self.get_of())) as u8) }
    pub fn setnle_eb(&mut self, instr: &Instruction) -> super::Result<()> { self.setcc_eb(instr, (!self.get_zf() && (self.get_sf() == self.get_of())) as u8) }
}
