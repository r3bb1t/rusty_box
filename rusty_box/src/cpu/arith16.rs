use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // INC/DEC instructions
    // =========================================================================

    /// INC r16
    pub fn inc_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.wrapping_add(1);
        self.set_gpr16(dst, result);
        self.set_flags_oszap_inc_16(result, op1);
        tracing::trace!("INC r16: {:#06x} + 1 = {:#06x}", op1, result);
    }

    /// DEC r16
    pub fn dec_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.wrapping_sub(1);
        self.set_gpr16(dst, result);
        self.set_flags_oszap_dec_16(result, op1);
        tracing::trace!("DEC r16: {:#06x} - 1 = {:#06x}", op1, result);
    }

    /// INC r/m16 (memory form) — matches Bochs INC_EwM
    pub fn inc_ew_m(&mut self, instr: &BxInstructionGenerated) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1, laddr) = self.read_rmw_virtual_word(seg, eaddr)?;
        let result = op1.wrapping_add(1);
        self.write_rmw_linear_word(laddr, result);
        self.set_flags_oszap_inc_16(result, op1);
        Ok(())
    }

    /// DEC r/m16 (memory form) — matches Bochs DEC_EwM
    pub fn dec_ew_m(&mut self, instr: &BxInstructionGenerated) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1, laddr) = self.read_rmw_virtual_word(seg, eaddr)?;
        let result = op1.wrapping_sub(1);
        self.write_rmw_linear_word(laddr, result);
        self.set_flags_oszap_dec_16(result, op1);
        Ok(())
    }

    /// INC r/m16 — unified dispatch
    pub fn inc_ew(&mut self, instr: &BxInstructionGenerated) -> super::Result<()> {
        if instr.mod_c0() { self.inc_ew_r(instr); Ok(()) } else { self.inc_ew_m(instr) }
    }

    /// DEC r/m16 — unified dispatch
    pub fn dec_ew(&mut self, instr: &BxInstructionGenerated) -> super::Result<()> {
        if instr.mod_c0() { self.dec_ew_r(instr); Ok(()) } else { self.dec_ew_m(instr) }
    }
}
