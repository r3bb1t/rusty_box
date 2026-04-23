use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Bit-mix hash of icount for pseudo-random values (splitmix64 variant).
    fn hw_rand64(&self) -> u64 {
        let mut x = self.icount.wrapping_mul(0x517cc1b727220a95);
        x ^= x >> 33;
        x = x.wrapping_mul(0x4cf5ad432745937f);
        x ^= x >> 33;
        x
    }

    /// Clear OSZAPC then set CF=1 (hardware always ready).
    #[inline]
    fn clear_flags_set_cf(&mut self) {
        self.set_of(false); self.set_sf(false); self.set_zf(false);
        self.set_af(false); self.set_pf(false); self.set_cf(false);
        self.set_cf(true);
    }

    // ── RDRAND ──────────────────────────────────────────────────────

    /// RDRAND r16  (0F C7 /6, opsize 16)
    /// Bochs BX_WRITE_16BIT_REG: only writes low 16 bits, preserves 63:16.
    pub fn rdrand_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        self.clear_flags_set_cf();
        let val = self.hw_rand64() as u16;
        self.set_gpr16(instr.dst() as usize, val);
        Ok(())
    }

    /// RDRAND r32  (0F C7 /6, opsize 32)
    pub fn rdrand_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.clear_flags_set_cf();
        let val = self.hw_rand64() as u32;
        self.set_gpr32(instr.dst() as usize, val);
        Ok(())
    }

    /// RDRAND r64  (0F C7 /6, opsize 64, REX.W)
    pub fn rdrand_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.clear_flags_set_cf();
        let val = self.hw_rand64();
        self.set_gpr64(instr.dst() as usize, val);
        Ok(())
    }

    // ── RDSEED ──────────────────────────────────────────────────────

    /// RDSEED r16  (0F C7 /7, opsize 16)
    /// Bochs BX_WRITE_16BIT_REG: only writes low 16 bits, preserves 63:16.
    pub fn rdseed_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        self.clear_flags_set_cf();
        let val = self.hw_rand64() as u16;
        self.set_gpr16(instr.dst() as usize, val);
        Ok(())
    }

    /// RDSEED r32  (0F C7 /7, opsize 32)
    pub fn rdseed_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.clear_flags_set_cf();
        let val = self.hw_rand64() as u32;
        self.set_gpr32(instr.dst() as usize, val);
        Ok(())
    }

    /// RDSEED r64  (0F C7 /7, opsize 64, REX.W)
    pub fn rdseed_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.clear_flags_set_cf();
        let val = self.hw_rand64();
        self.set_gpr64(instr.dst() as usize, val);
        Ok(())
    }
}
