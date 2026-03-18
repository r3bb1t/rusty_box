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
}
