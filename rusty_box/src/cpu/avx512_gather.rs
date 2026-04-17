//! AVX-512F gather instruction stubs
//!
//! Gather instructions load elements from memory using per-element indices
//! from a vector register. Each element's address = base + index[i] * scale.
//!
//! These are STUB implementations that zero the destination and clear the
//! opmask register (safe behavior matching post-gather opmask semantics).
//! Full VSIB decoding is not yet implemented in the decoder, so these
//! handlers cannot compute correct per-element addresses.
//!
//! Mirrors Bochs `cpu/avx/gather.cc`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    xmm::BxPackedZmmRegister,
};

/// Number of 32-bit elements per vector length: VL0=4, VL1=8, VL2=16
#[inline]
fn dword_elements(vl: u8) -> usize {
    match vl {
        0 => 4,   // 128-bit
        1 => 8,   // 256-bit
        _ => 16,  // 512-bit
    }
}

/// Number of 64-bit elements per vector length: VL0=2, VL1=4, VL2=8
#[inline]
fn qword_elements(vl: u8) -> usize {
    match vl {
        0 => 2,
        1 => 4,
        _ => 8,
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // VPGATHERDD — Gather packed dwords using dword indices
    // EVEX.0F38.W0 90 /r (VSIB)
    // ========================================================================

    /// VPGATHERDD Vdq{k}, [base + Hdq*scale]
    ///
    /// Gathers dword elements from memory at addresses computed from a base
    /// address plus dword indices scaled by a factor. Only elements whose
    /// corresponding opmask bit is set are loaded; others are zeroed (or
    /// merged). After the gather, the opmask register is cleared to 0.
    ///
    /// STUB: zeros destination and clears opmask (VSIB not yet decoded).
    pub fn evex_vpgatherdd(&mut self, instr: &Instruction) -> super::Result<()> {
        tracing::warn!("EVEX VPGATHERDD: stub — VSIB gather not fully implemented");
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_reg = instr.dst();

        // Zero the destination register
        let result = BxPackedZmmRegister::default();
        let dst = &mut self.vmm[dst_reg as usize];
        for i in 0..nelements {
            dst.set_zmm32u(i, result.zmm32u(i));
        }
        for i in nelements..16 {
            dst.set_zmm32u(i, 0);
        }

        // Clear opmask register (Intel spec: mask is zeroed after gather)
        let k = instr.opmask();
        if k != 0 {
            self.bx_write_opmask(k as usize, 0);
        }

        Ok(())
    }

    // ========================================================================
    // VPGATHERDQ — Gather packed qwords using dword indices
    // EVEX.0F38.W1 90 /r (VSIB)
    // ========================================================================

    /// VPGATHERDQ Vdq{k}, [base + Hdq*scale]
    ///
    /// Gathers qword elements from memory at addresses computed from a base
    /// address plus dword indices (half as many indices as output elements)
    /// scaled by a factor. After the gather, the opmask register is cleared.
    ///
    /// STUB: zeros destination and clears opmask (VSIB not yet decoded).
    pub fn evex_vpgatherdq(&mut self, instr: &Instruction) -> super::Result<()> {
        tracing::warn!("EVEX VPGATHERDQ: stub — VSIB gather not fully implemented");
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_reg = instr.dst();

        // Zero the destination register
        let dst = &mut self.vmm[dst_reg as usize];
        for i in 0..nelements {
            dst.set_zmm64u(i, 0);
        }
        for i in nelements..8 {
            dst.set_zmm64u(i, 0);
        }

        // Clear opmask register
        let k = instr.opmask();
        if k != 0 {
            self.bx_write_opmask(k as usize, 0);
        }

        Ok(())
    }

    // ========================================================================
    // VPGATHERQD — Gather packed dwords using qword indices
    // EVEX.0F38.W0 91 /r (VSIB)
    // ========================================================================

    /// VPGATHERQD Vdq{k}, [base + Hdq*scale]
    ///
    /// Gathers dword elements from memory at addresses computed from a base
    /// address plus qword indices scaled by a factor. The number of elements
    /// gathered is half the qword index count (since indices are 64-bit but
    /// data is 32-bit). After the gather, the opmask register is cleared.
    ///
    /// STUB: zeros destination and clears opmask (VSIB not yet decoded).
    pub fn evex_vpgatherqd(&mut self, instr: &Instruction) -> super::Result<()> {
        tracing::warn!("EVEX VPGATHERQD: stub — VSIB gather not fully implemented");
        let vl = instr.get_vl();
        // QD: qword indices but dword data — number of elements is based on
        // the index width. For VL=512, there are 8 qword indices producing
        // 8 dword results (written to the lower half of the destination).
        let nelements = qword_elements(vl);
        let dst_reg = instr.dst();

        // Zero the entire destination register
        let dst = &mut self.vmm[dst_reg as usize];
        for i in 0..16 {
            dst.set_zmm32u(i, 0);
        }

        // Clear opmask register
        let k = instr.opmask();
        if k != 0 {
            self.bx_write_opmask(k as usize, 0);
        }

        // Suppress unused variable warning — nelements will be used when
        // VSIB is fully implemented
        let _ = nelements;

        Ok(())
    }

    // ========================================================================
    // VPGATHERQQ — Gather packed qwords using qword indices
    // EVEX.0F38.W1 91 /r (VSIB)
    // ========================================================================

    /// VPGATHERQQ Vdq{k}, [base + Hdq*scale]
    ///
    /// Gathers qword elements from memory at addresses computed from a base
    /// address plus qword indices scaled by a factor. After the gather, the
    /// opmask register is cleared to 0.
    ///
    /// STUB: zeros destination and clears opmask (VSIB not yet decoded).
    pub fn evex_vpgatherqq(&mut self, instr: &Instruction) -> super::Result<()> {
        tracing::warn!("EVEX VPGATHERQQ: stub — VSIB gather not fully implemented");
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_reg = instr.dst();

        // Zero the destination register
        let dst = &mut self.vmm[dst_reg as usize];
        for i in 0..nelements {
            dst.set_zmm64u(i, 0);
        }
        for i in nelements..8 {
            dst.set_zmm64u(i, 0);
        }

        // Clear opmask register
        let k = instr.opmask();
        if k != 0 {
            self.bx_write_opmask(k as usize, 0);
        }

        Ok(())
    }
}
