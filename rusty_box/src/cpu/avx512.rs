//! AVX-512 Foundation (AVX-512F) instruction handlers
//!
//! Implements core 512-bit vector operations with opmask support.
//! Handlers work for 128/256/512-bit via `get_vl()` (EVEX.L'L field).
//!
//! Mirrors Bochs `cpu/avx/avx512.cc`, `avx512_move.cc`, `avx512_pfp.cc`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
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

/// Byte size for vector length: VL0=16, VL1=32, VL2=64
#[inline]
fn vl_bytes(vl: u8) -> usize {
    match vl {
        0 => 16,
        1 => 32,
        _ => 64,
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

/// Write ZMM register, zeroing upper bits beyond VL
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
            // else: merge masking — keep original value
        }
        // Zero upper elements beyond VL (EVEX always clears upper)
        for i in nelements..16 {
            dst.zmm32u[i] = 0;
        }
    }
}

/// Write ZMM register for qword operations
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
    // VMOVDQU32/64 — Unaligned move (EVEX-encoded)
    // ========================================================================

    /// VMOVDQU32 Vdq{k}, Wdq — EVEX.0F.W0 6F (load, register form)
    pub fn evex_vmovdqu32_load_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU32 Vdq{k}, Mdq — EVEX.0F.W0 6F (load, memory form)
    pub fn evex_vmovdqu32_load_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let bytes = vl_bytes(vl);
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut src = BxPackedZmmRegister { zmm64u: [0; 8] };
        // Read bytes from memory
        for i in 0..(bytes / 4) {
            let val = self.v_read_dword(seg, laddr + (i * 4) as u64)?;
            unsafe { src.zmm32u[i] = val; }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU32 Wdq{k}, Vdq — EVEX.0F.W0 7F (store, register form)
    pub fn evex_vmovdqu32_store_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU32 Mdq{k}, Vdq — EVEX.0F.W0 7F (store, memory form)
    pub fn evex_vmovdqu32_store_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let bytes = vl_bytes(vl);
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        for i in 0..(bytes / 4) {
            if (mask >> i) & 1 != 0 {
                let val = unsafe { src.zmm32u[i] };
                self.v_write_dword(seg, laddr + (i * 4) as u64, val)?;
            }
        }
        Ok(())
    }

    /// VMOVDQU64 — same as VMOVDQU32 but with qword masking granularity
    /// EVEX.0F.W1 6F (load), 7F (store)
    pub fn evex_vmovdqu64_load_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    pub fn evex_vmovdqu64_load_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let bytes = vl_bytes(vl);
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut src = BxPackedZmmRegister { zmm64u: [0; 8] };
        for i in 0..(bytes / 8) {
            let val = if self.long64_mode() {
                self.read_virtual_qword_64(seg, laddr + (i * 8) as u64)?
            } else {
                self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64
                    | ((self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64) << 32)
            };
            unsafe { src.zmm64u[i] = val; }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    pub fn evex_vmovdqu64_store_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vmovdqu32_store_r(instr) // register form is identical
    }

    pub fn evex_vmovdqu64_store_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let bytes = vl_bytes(vl);
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        for i in 0..(bytes / 8) {
            if (mask >> i) & 1 != 0 {
                let val = unsafe { src.zmm64u[i] };
                if self.long64_mode() {
                    self.write_virtual_qword_64(seg, laddr + (i * 8) as u64, val)?;
                } else {
                    self.v_write_dword(seg, laddr + (i * 8) as u64, val as u32)?;
                    self.v_write_dword(seg, laddr + (i * 8 + 4) as u64, (val >> 32) as u32)?;
                }
            }
        }
        Ok(())
    }

    // ========================================================================
    // VPADDD/Q — Packed integer add (EVEX-encoded)
    // ========================================================================

    /// VPADDD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 FE
    pub fn evex_vpaddd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let val = self.v_read_dword(seg, laddr + (i * 4) as u64)?;
                unsafe { tmp.zmm32u[i] = val; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = src1.zmm32u[i].wrapping_add(src2.zmm32u[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPADDQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 D4
    pub fn evex_vpaddq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let lo = self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
                let hi = self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
                unsafe { tmp.zmm64u[i] = lo | (hi << 32); }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = src1.zmm64u[i].wrapping_add(src2.zmm64u[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSUBD/Q — Packed integer subtract
    // ========================================================================

    /// VPSUBD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 FA
    pub fn evex_vpsubd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let val = self.v_read_dword(seg, laddr + (i * 4) as u64)?;
                unsafe { tmp.zmm32u[i] = val; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = src1.zmm32u[i].wrapping_sub(src2.zmm32u[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSUBQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 FB
    pub fn evex_vpsubq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let lo = self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
                let hi = self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
                unsafe { tmp.zmm64u[i] = lo | (hi << 32); }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = src1.zmm64u[i].wrapping_sub(src2.zmm64u[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPXORD/Q, VPORD/Q, VPANDD/Q, VPANDND/Q — Packed bitwise logical
    // ========================================================================

    /// VPXORD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 EF
    pub fn evex_vpxord(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = src1.zmm32u[i] ^ src2.zmm32u[i];
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPXORQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 EF
    pub fn evex_vpxorq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let lo = self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
                let hi = self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
                unsafe { tmp.zmm64u[i] = lo | (hi << 32); }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = src1.zmm64u[i] ^ src2.zmm64u[i];
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPORD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 EB
    pub fn evex_vpord(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = src1.zmm32u[i] | src2.zmm32u[i];
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPANDD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 DB
    pub fn evex_vpandd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = src1.zmm32u[i] & src2.zmm32u[i];
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPANDND Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 DF
    pub fn evex_vpandnd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = (!src1.zmm32u[i]) & src2.zmm32u[i];
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPBROADCASTD/Q — Broadcast scalar to all elements
    // ========================================================================

    /// VPBROADCASTD Vdq{k}, Wd — EVEX.66.0F38.W0 58
    pub fn evex_vpbroadcastd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let scalar = if instr.mod_c0() {
            unsafe { read_zmm(self, instr.src()).zmm32u[0] }
        } else {
            let laddr = self.resolve_addr(instr);
            self.v_read_dword(BxSegregs::from(instr.seg()), laddr)?
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = scalar;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPBROADCASTQ Vdq{k}, Wq — EVEX.66.0F38.W1 59
    pub fn evex_vpbroadcastq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let scalar = if instr.mod_c0() {
            unsafe { read_zmm(self, instr.src()).zmm64u[0] }
        } else {
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            if self.long64_mode() {
                self.read_virtual_qword_64(seg, laddr as u64)?
            } else {
                let lo = self.v_read_dword(seg, laddr)? as u64;
                let hi = self.v_read_dword(seg, laddr + 4)? as u64;
                lo | (hi << 32)
            }
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = scalar;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPBROADCASTD Vdq{k}, Gd — EVEX.66.0F38.W0 7C (broadcast from GPR)
    pub fn evex_vpbroadcastd_gpr(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let scalar = self.get_gpr32(instr.src() as usize);
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = scalar;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPBROADCASTQ Vdq{k}, Gq — EVEX.66.0F38.W1 7C (broadcast from GPR)
    pub fn evex_vpbroadcastq_gpr(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let scalar = self.get_gpr64(instr.src() as usize);
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = scalar;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPTERNLOGD/Q — Bitwise ternary logic (3-input truth table)
    // Most commonly used AVX-512F instruction — replaces AND/OR/XOR combos
    // ========================================================================

    /// VPTERNLOGD Vdq{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 25
    pub fn evex_vpternlogd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_reg = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let imm8 = instr.ib() as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = dst_reg.zmm32u[i];
                let b = src1.zmm32u[i];
                let c = src2.zmm32u[i];
                // For each bit position, compute truth table lookup
                // bit_index = (a_bit << 2) | (b_bit << 1) | c_bit
                // result_bit = (imm8 >> bit_index) & 1
                let mut r = 0u32;
                for bit in 0..32 {
                    let a_bit = (a >> bit) & 1;
                    let b_bit = (b >> bit) & 1;
                    let c_bit = (c >> bit) & 1;
                    let idx = (a_bit << 2) | (b_bit << 1) | c_bit;
                    r |= ((imm8 >> idx) & 1) << bit;
                }
                result.zmm32u[i] = r;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPTERNLOGQ Vdq{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W1 25
    pub fn evex_vpternlogq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_reg = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let lo = self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
                let hi = self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
                unsafe { tmp.zmm64u[i] = lo | (hi << 32); }
            }
            tmp
        };
        let imm8 = instr.ib() as u64;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = dst_reg.zmm64u[i];
                let b = src1.zmm64u[i];
                let c = src2.zmm64u[i];
                let mut r = 0u64;
                for bit in 0..64 {
                    let a_bit = (a >> bit) & 1;
                    let b_bit = (b >> bit) & 1;
                    let c_bit = (c >> bit) & 1;
                    let idx = (a_bit << 2) | (b_bit << 1) | c_bit;
                    r |= ((imm8 >> idx) & 1) << bit;
                }
                result.zmm64u[i] = r;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSLLD/Q, VPSRLD/Q — Packed shift by immediate
    // ========================================================================

    /// VPSLLD Vdq{k}, Hdq, Ib — EVEX.66.0F.W0 72 /6
    pub fn evex_vpslld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = if count >= 32 { 0 } else { src.zmm32u[i] << count };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRLD Vdq{k}, Hdq, Ib — EVEX.66.0F.W0 72 /2
    pub fn evex_vpsrld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = if count >= 32 { 0 } else { src.zmm32u[i] >> count };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRAD Vdq{k}, Hdq, Ib — EVEX.66.0F.W0 72 /4 (arithmetic right shift)
    pub fn evex_vpsrad_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = if count >= 32 {
                    ((src.zmm32u[i] as i32) >> 31) as u32
                } else {
                    ((src.zmm32u[i] as i32) >> count) as u32
                };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSLLQ Vdq{k}, Hdq, Ib — EVEX.66.0F.W1 73 /6
    pub fn evex_vpsllq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = if count >= 64 { 0 } else { src.zmm64u[i] << count };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRLQ Vdq{k}, Hdq, Ib — EVEX.66.0F.W1 73 /2
    pub fn evex_vpsrlq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = if count >= 64 { 0 } else { src.zmm64u[i] >> count };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRAQ Vdq{k}, Hdq, Ib — EVEX.66.0F.W1 72 /4 (arithmetic right shift qword)
    pub fn evex_vpsraq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = if count >= 64 {
                    ((src.zmm64u[i] as i64) >> 63) as u64
                } else {
                    ((src.zmm64u[i] as i64) >> count) as u64
                };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VEXTRACTI32x4 / VINSERTI32x4 — Extract/Insert 128-bit lane
    // ========================================================================

    /// VEXTRACTI32x4 Wdq{k}, Vdq, Ib — EVEX.66.0F3A.W0 39
    pub fn evex_vextracti32x4(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = read_zmm(self, instr.src());
        let imm = (instr.ib() & 0x03) as usize; // lane index (0-3)
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            // Copy 128-bit lane
            result.zmm32u[0] = src.zmm32u[imm * 4];
            result.zmm32u[1] = src.zmm32u[imm * 4 + 1];
            result.zmm32u[2] = src.zmm32u[imm * 4 + 2];
            result.zmm32u[3] = src.zmm32u[imm * 4 + 3];
        }
        if instr.mod_c0() {
            // Register form — write 128 bits, zero upper
            let mask = read_opmask_for_write(self, instr);
            let zmask = instr.is_zero_masking() != 0;
            write_zmm_masked(self, instr.dst(), &result, mask, zmask, 0); // VL=0 (128-bit)
        } else {
            // Memory form — write 16 bytes
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let mask = read_opmask_for_write(self, instr);
            for i in 0..4u64 {
                if (mask >> i) & 1 != 0 {
                    let val = unsafe { result.zmm32u[i as usize] };
                    self.v_write_dword(seg, laddr + i * 4, val)?;
                }
            }
        }
        Ok(())
    }

    /// VINSERTI32x4 Vdq{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 38
    pub fn evex_vinserti32x4(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let imm = (instr.ib() & 0x03) as usize;
        // Start with src1 (the full vector)
        let mut result = read_zmm(self, instr.src1());
        // Read 128-bit insert value
        let insert = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..4 {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        // Insert 128-bit lane
        unsafe {
            result.zmm32u[imm * 4] = insert.zmm32u[0];
            result.zmm32u[imm * 4 + 1] = insert.zmm32u[1];
            result.zmm32u[imm * 4 + 2] = insert.zmm32u[2];
            result.zmm32u[imm * 4 + 3] = insert.zmm32u[3];
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSHUFB — Packed shuffle bytes (EVEX)
    // ========================================================================

    /// VPSHUFB Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 00
    pub fn evex_vpshufb(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let bytes = vl_bytes(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..bytes {
                unsafe { tmp.zmmubyte[i] = self.v_read_byte(seg, laddr + i as u64)?; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        // Per-lane shuffle: each 128-bit lane independently
        let lanes = bytes / 16;
        for lane in 0..lanes {
            let base = lane * 16;
            for i in 0..16 {
                unsafe {
                    let ctrl = src2.zmmubyte[base + i];
                    if ctrl & 0x80 != 0 {
                        result.zmmubyte[base + i] = 0;
                    } else {
                        let idx = (ctrl & 0x0F) as usize;
                        result.zmmubyte[base + i] = src1.zmmubyte[base + idx];
                    }
                }
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        // Byte-granularity masking
        let nelements = bytes;
        unsafe {
            let dst = &mut self.vmm[instr.dst() as usize];
            for i in 0..nelements {
                if (mask >> i) & 1 != 0 {
                    dst.zmmubyte[i] = result.zmmubyte[i];
                } else if zmask {
                    dst.zmmubyte[i] = 0;
                }
            }
            for i in nelements..64 {
                dst.zmmubyte[i] = 0;
            }
        }
        Ok(())
    }

    // ========================================================================
    // VPSHUFD — Shuffle packed dwords by immediate
    // ========================================================================

    /// VPSHUFD Vdq{k}, Wdq, Ib — EVEX.66.0F.W0 70
    pub fn evex_vpshufd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let imm8 = instr.ib();
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        let lanes = nelements / 4;
        for lane in 0..lanes {
            let base = lane * 4;
            unsafe {
                for j in 0..4 {
                    let sel = ((imm8 >> (j * 2)) & 0x03) as usize;
                    result.zmm32u[base + j] = src.zmm32u[base + sel];
                }
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSLLD/Q, VPSRLD/Q by XMM count (shift by register)
    // ========================================================================

    /// VPSLLD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 F2 (shift left by XMM[63:0])
    pub fn evex_vpslld_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count = unsafe { count_reg.zmm64u[0] } as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = if count >= 32 { 0 } else { src.zmm32u[i] << count };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRLD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 D2 (shift right by XMM[63:0])
    pub fn evex_vpsrld_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count = unsafe { count_reg.zmm64u[0] } as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = if count >= 32 { 0 } else { src.zmm32u[i] >> count };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRAD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 E2 (arithmetic shift right by XMM[63:0])
    pub fn evex_vpsrad_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count = unsafe { count_reg.zmm64u[0] } as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = if count >= 32 {
                    ((src.zmm32u[i] as i32) >> 31) as u32
                } else {
                    ((src.zmm32u[i] as i32) >> count) as u32
                };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSLLQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 F3
    pub fn evex_vpsllq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count = unsafe { count_reg.zmm64u[0] } as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = if count >= 64 { 0 } else { src.zmm64u[i] << count };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRLQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 D3
    pub fn evex_vpsrlq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count = unsafe { count_reg.zmm64u[0] } as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = if count >= 64 { 0 } else { src.zmm64u[i] >> count };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRAQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 E2 (arithmetic shift right qword)
    pub fn evex_vpsraq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count = unsafe { count_reg.zmm64u[0] } as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = if count >= 64 {
                    ((src.zmm64u[i] as i64) >> 63) as u64
                } else {
                    ((src.zmm64u[i] as i64) >> count) as u64
                };
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPCMPD/Q — Packed compare producing opmask result
    // ========================================================================

    /// VPCMPD Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 1F
    pub fn evex_vpcmpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let imm3 = instr.ib() & 0x07;
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        unsafe {
            for i in 0..nelements {
                let a = src1.zmm32u[i] as i32;
                let b = src2.zmm32u[i] as i32;
                let cmp = match imm3 {
                    0 => a == b,        // EQ
                    1 => a < b,         // LT
                    2 => a <= b,        // LE
                    3 => false,         // FALSE
                    4 => a != b,        // NEQ
                    5 => a >= b,        // NLT (GE)
                    6 => a > b,         // NLE (GT)
                    _ => true,          // TRUE
                };
                if cmp && ((write_mask >> i) & 1 != 0) {
                    result |= 1 << i;
                }
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPCMPUD Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 1E (unsigned compare)
    pub fn evex_vpcmpud(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let imm3 = instr.ib() & 0x07;
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        unsafe {
            for i in 0..nelements {
                let a = src1.zmm32u[i];
                let b = src2.zmm32u[i];
                let cmp = match imm3 {
                    0 => a == b,
                    1 => a < b,
                    2 => a <= b,
                    3 => false,
                    4 => a != b,
                    5 => a >= b,
                    6 => a > b,
                    _ => true,
                };
                if cmp && ((write_mask >> i) & 1 != 0) {
                    result |= 1 << i;
                }
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // VPMULLD — Packed multiply low dword
    // ========================================================================

    /// VPMULLD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 40
    pub fn evex_vpmulld(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = src1.zmm32u[i].wrapping_mul(src2.zmm32u[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMINSD/VPMAXSD — Packed min/max signed dword
    // ========================================================================

    /// VPMINSD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 39
    pub fn evex_vpminsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = src1.zmm32u[i] as i32;
                let b = src2.zmm32u[i] as i32;
                result.zmm32u[i] = if a < b { a } else { b } as u32;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPMAXSD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 3D
    pub fn evex_vpmaxsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
            }
            tmp
        };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = src1.zmm32u[i] as i32;
                let b = src2.zmm32u[i] as i32;
                result.zmm32u[i] = if a > b { a } else { b } as u32;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}
