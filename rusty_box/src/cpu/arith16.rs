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
}
