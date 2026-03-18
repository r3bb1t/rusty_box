//! AVX-512F/DQ extract and insert instruction handlers for all lane sizes.
//!
//! Implements VEXTRACTI/F64x2, VEXTRACTI/F32x8, VEXTRACTI/F64x4,
//! VINSERTI/F64x2, VINSERTI/F32x8, VINSERTI/F64x4, and VEXTRACTPS.
//!
//! Mirrors Bochs `cpu/avx/avx512.cc` extract/insert operations.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedZmmRegister,
};

/// Byte size for vector length
#[inline]
fn vl_bytes(vl: u8) -> usize {
    match vl { 0 => 16, 1 => 32, _ => 64 }
}

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

/// Read opmask value for masking. k0 returns all-ones (no masking).
#[inline]
fn read_opmask_for_write<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, instr: &Instruction) -> u64 {
    let k = instr.opmask();
    if k == 0 {
        u64::MAX // k0 = all elements active
    } else {
        unsafe { cpu.opmask[k as usize].rrx }
    }
}

/// Read ZMM register as a ZMM-width value
#[inline]
fn read_zmm<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, reg: u8) -> BxPackedZmmRegister {
    unsafe { cpu.vmm[reg as usize] }
}

/// Write ZMM register with dword masking granularity, zeroing upper bits beyond VL
fn write_zmm_masked<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nelements = dword_elements(vl);
    unsafe {
        let dst = &mut cpu.vmm[reg as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                dst.zmm32u[i] = result.zmm32u[i];
            } else if zero_masking {
                dst.zmm32u[i] = 0;
            }
        }
        // Zero upper elements beyond VL (EVEX always clears upper)
        for i in nelements..16 {
            dst.zmm32u[i] = 0;
        }
    }
}

/// Write ZMM register with qword masking granularity
fn write_zmm_masked_q<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nelements = qword_elements(vl);
    unsafe {
        let dst = &mut cpu.vmm[reg as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                dst.zmm64u[i] = result.zmm64u[i];
            } else if zero_masking {
                dst.zmm64u[i] = 0;
            }
        }
        // Zero upper elements beyond VL
        for i in nelements..8 {
            dst.zmm64u[i] = 0;
        }
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // VEXTRACTI64x2 / VEXTRACTF64x2 — Extract 128-bit lane (qword masking)
    // EVEX.66.0F3A.W1 39 /r ib
    // ========================================================================

    /// VEXTRACTI64x2 Wdq{k}, Vdq, Ib — Extract 128-bit lane with qword masking.
    /// imm8[1:0] selects which 128-bit lane from the source register.
    /// Register form: write 128 bits to XMM + zero upper.
    /// Memory form: write 16 bytes with per-qword masking.
    pub fn evex_vextracti64x2(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let num_lanes = vl_bytes(vl) / 16;
        let imm = (instr.ib() as usize) & (num_lanes - 1);
        // Extract 128-bit lane (2 qwords)
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            result.zmm64u[0] = src.zmm64u[imm * 2];
            result.zmm64u[1] = src.zmm64u[imm * 2 + 1];
        }
        if instr.mod_c0() {
            // Register form — write 128 bits with qword masking, zero upper
            let mask = read_opmask_for_write(self, instr);
            let zmask = instr.is_zero_masking() != 0;
            write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, 0); // VL=0 (128-bit)
        } else {
            // Memory form — write 16 bytes with per-qword masking
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let mask = read_opmask_for_write(self, instr);
            for i in 0..2u64 {
                if (mask >> i) & 1 != 0 {
                    let val = unsafe { result.zmm64u[i as usize] };
                    self.v_write_qword(seg, laddr + i * 8, val)?;
                }
            }
        }
        Ok(())
    }

    /// VEXTRACTF64x2 — Bitwise identical to VEXTRACTI64x2 (float naming only).
    pub fn evex_vextractf64x2(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vextracti64x2(instr)
    }

    // ========================================================================
    // VEXTRACTI32x8 / VEXTRACTF32x8 — Extract 256-bit half (dword masking)
    // EVEX.66.0F3A.W0 3B /r ib
    // ========================================================================

    /// VEXTRACTI32x8 Wqq{k}, Vdq, Ib — Extract 256-bit half with dword masking.
    /// imm8 bit 0 selects which 256-bit half (0=lower, 1=upper).
    /// Register form: write 256 bits to YMM + zero upper.
    /// Memory form: write 32 bytes with per-dword masking.
    pub fn evex_vextracti32x8(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_zmm(self, instr.src());
        let half = (instr.ib() & 0x01) as usize;
        // Extract 256-bit half (8 dwords)
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..8 {
                result.zmm32u[i] = src.zmm32u[half * 8 + i];
            }
        }
        if instr.mod_c0() {
            // Register form — write 256 bits with dword masking, zero upper
            let mask = read_opmask_for_write(self, instr);
            let zmask = instr.is_zero_masking() != 0;
            write_zmm_masked(self, instr.dst(), &result, mask, zmask, 1); // VL=1 (256-bit)
        } else {
            // Memory form — write 32 bytes with per-dword masking
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let mask = read_opmask_for_write(self, instr);
            for i in 0..8u64 {
                if (mask >> i) & 1 != 0 {
                    let val = unsafe { result.zmm32u[i as usize] };
                    self.v_write_dword(seg, laddr + i * 4, val)?;
                }
            }
        }
        Ok(())
    }

    /// VEXTRACTF32x8 — Bitwise identical to VEXTRACTI32x8 (float naming only).
    pub fn evex_vextractf32x8(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vextracti32x8(instr)
    }

    // ========================================================================
    // VEXTRACTI64x4 / VEXTRACTF64x4 — Extract 256-bit half (qword masking)
    // EVEX.66.0F3A.W1 3B /r ib
    // ========================================================================

    /// VEXTRACTI64x4 Wqq{k}, Vdq, Ib — Extract 256-bit half with qword masking.
    /// imm8 bit 0 selects which 256-bit half (0=lower, 1=upper).
    /// Register form: write 256 bits to YMM + zero upper.
    /// Memory form: write 32 bytes with per-qword masking.
    pub fn evex_vextracti64x4(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_zmm(self, instr.src());
        let half = (instr.ib() & 0x01) as usize;
        // Extract 256-bit half (4 qwords)
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..4 {
                result.zmm64u[i] = src.zmm64u[half * 4 + i];
            }
        }
        if instr.mod_c0() {
            // Register form — write 256 bits with qword masking, zero upper
            let mask = read_opmask_for_write(self, instr);
            let zmask = instr.is_zero_masking() != 0;
            write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, 1); // VL=1 (256-bit)
        } else {
            // Memory form — write 32 bytes with per-qword masking
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let mask = read_opmask_for_write(self, instr);
            for i in 0..4u64 {
                if (mask >> i) & 1 != 0 {
                    let val = unsafe { result.zmm64u[i as usize] };
                    self.v_write_qword(seg, laddr + i * 8, val)?;
                }
            }
        }
        Ok(())
    }

    /// VEXTRACTF64x4 — Bitwise identical to VEXTRACTI64x4 (float naming only).
    pub fn evex_vextractf64x4(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vextracti64x4(instr)
    }

    // ========================================================================
    // VINSERTI64x2 / VINSERTF64x2 — Insert 128-bit lane (qword masking)
    // EVEX.66.0F3A.W1 38 /r ib
    // ========================================================================

    /// VINSERTI64x2 Vdq{k}, Hdq, Wdq, Ib — Insert 128-bit lane with qword masking.
    /// imm8[1:0] selects which 128-bit lane position in the destination.
    /// src1 = full vector (VEX.vvvv), src2 = 128-bit value to insert (rm).
    pub fn evex_vinserti64x2(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let num_lanes = vl_bytes(vl) / 16;
        let imm = (instr.ib() as usize) & (num_lanes - 1);
        // Start with src1 (the full vector from VEX.vvvv)
        let mut result = read_zmm(self, instr.src1());
        // Read 128-bit insert value (2 qwords)
        let insert = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..2 {
                let val = self.v_read_qword(seg, laddr + (i * 8) as u64)?;
                unsafe { tmp.zmm64u[i] = val; }
            }
            tmp
        };
        // Insert 128-bit lane (2 qwords)
        unsafe {
            result.zmm64u[imm * 2] = insert.zmm64u[0];
            result.zmm64u[imm * 2 + 1] = insert.zmm64u[1];
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VINSERTF64x2 — Bitwise identical to VINSERTI64x2 (float naming only).
    pub fn evex_vinsertf64x2(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vinserti64x2(instr)
    }

    // ========================================================================
    // VINSERTI32x8 / VINSERTF32x8 — Insert 256-bit half (dword masking)
    // EVEX.66.0F3A.W0 3A /r ib
    // ========================================================================

    /// VINSERTI32x8 Vdq{k}, Hdq, Wqq, Ib — Insert 256-bit half with dword masking.
    /// imm8 bit 0 selects which 256-bit half position (0=lower, 1=upper).
    /// src1 = full 512-bit vector (VEX.vvvv), src2 = 256-bit value to insert (rm).
    pub fn evex_vinserti32x8(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let half = (instr.ib() & 0x01) as usize;
        // Start with src1 (the full vector)
        let mut result = read_zmm(self, instr.src1());
        // Read 256-bit insert value (8 dwords)
        let insert = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..8 {
                let val = self.v_read_dword(seg, laddr + (i * 4) as u64)?;
                unsafe { tmp.zmm32u[i] = val; }
            }
            tmp
        };
        // Insert 256-bit half (8 dwords)
        unsafe {
            for i in 0..8 {
                result.zmm32u[half * 8 + i] = insert.zmm32u[i];
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VINSERTF32x8 — Bitwise identical to VINSERTI32x8 (float naming only).
    pub fn evex_vinsertf32x8(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vinserti32x8(instr)
    }

    // ========================================================================
    // VINSERTI64x4 / VINSERTF64x4 — Insert 256-bit half (qword masking)
    // EVEX.66.0F3A.W1 3A /r ib
    // ========================================================================

    /// VINSERTI64x4 Vdq{k}, Hdq, Wqq, Ib — Insert 256-bit half with qword masking.
    /// imm8 bit 0 selects which 256-bit half position (0=lower, 1=upper).
    /// src1 = full 512-bit vector (VEX.vvvv), src2 = 256-bit value to insert (rm).
    pub fn evex_vinserti64x4(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let half = (instr.ib() & 0x01) as usize;
        // Start with src1 (the full vector)
        let mut result = read_zmm(self, instr.src1());
        // Read 256-bit insert value (4 qwords)
        let insert = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..4 {
                let val = self.v_read_qword(seg, laddr + (i * 8) as u64)?;
                unsafe { tmp.zmm64u[i] = val; }
            }
            tmp
        };
        // Insert 256-bit half (4 qwords)
        unsafe {
            for i in 0..4 {
                result.zmm64u[half * 4 + i] = insert.zmm64u[i];
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VINSERTF64x4 — Bitwise identical to VINSERTI64x4 (float naming only).
    pub fn evex_vinsertf64x4(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vinserti64x4(instr)
    }

    // ========================================================================
    // VEXTRACTPS — Extract single float from XMM to GPR/memory
    // EVEX.66.0F3A.WIG 17 /r ib
    // ========================================================================

    /// VEXTRACTPS Ed, Vdq, Ib — Extract a single 32-bit float element from XMM.
    /// imm8[1:0] selects which dword element (0-3) from the source XMM register.
    /// Register form: write 32-bit value to GPR (zero-extended to 64 bits in 64-bit mode).
    /// Memory form: write 4 bytes to memory.
    pub fn evex_vextractps(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_zmm(self, instr.src());
        let sel = (instr.ib() & 0x03) as usize;
        let val = unsafe { src.zmm32u[sel] };
        if instr.mod_c0() {
            // Register form — write dword to GPR (zero-extended)
            self.set_gpr64(instr.dst() as usize, val as u64);
        } else {
            // Memory form — write 4 bytes
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_write_dword(seg, laddr, val)?;
        }
        Ok(())
    }
}
