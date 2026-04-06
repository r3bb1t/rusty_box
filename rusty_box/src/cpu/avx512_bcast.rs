#![allow(unused_unsafe)]

//! AVX-512F broadcast instruction handlers
//!
//! Implements EVEX-encoded broadcast operations:
//! - VBROADCASTSS/SD (scalar float/double broadcast)
//! - VPBROADCASTB/W (scalar byte/word broadcast)
//! - VBROADCASTI32x4/64x2/32x8/64x4 (sub-vector integer broadcast)
//! - VBROADCASTF32x4/64x2/32x8/64x4 (sub-vector FP broadcast, bitwise identical)
//!
//! Mirrors Bochs `cpu/avx/avx512.cc` broadcast handlers.

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

/// Number of 16-bit elements per vector length: VL0=8, VL1=16, VL2=32
#[inline]
fn word_elements(vl: u8) -> usize {
    match vl {
        0 => 8,
        1 => 16,
        _ => 32,
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
        // SAFETY: opmask register union always valid for rrx (full 64-bit) access
        unsafe { cpu.opmask[k as usize].rrx() }
    }
}

/// Read ZMM register as a ZMM-width value
#[inline]
fn read_zmm<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, reg: u8) -> BxPackedZmmRegister {
    cpu.vmm[reg as usize]
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
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..nelements {
        if (mask >> i) & 1 != 0 {
            dst.set_zmm32u(i, result.zmm32u(i));
        } else if zero_masking {
            dst.set_zmm32u(i, 0);
        }
        // else: merge masking — keep original value
    }
    // Zero upper elements beyond VL (EVEX always clears upper)
    for i in nelements..16 {
        dst.set_zmm32u(i, 0);
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
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..nelements {
        if (mask >> i) & 1 != 0 {
            dst.set_zmm64u(i, result.zmm64u(i));
        } else if zero_masking {
            dst.set_zmm64u(i, 0);
        }
    }
    // Zero upper elements beyond VL
    for i in nelements..8 {
        dst.set_zmm64u(i, 0);
    }
}

/// Write ZMM register with word masking granularity
fn write_zmm_masked_w<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nelements = word_elements(vl);
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..nelements {
        if (mask >> i) & 1 != 0 {
            dst.set_zmm16u(i, result.zmm16u(i));
        } else if zero_masking {
            dst.set_zmm16u(i, 0);
        }
    }
    // Zero upper elements beyond VL
    for i in nelements..32 {
        dst.set_zmm16u(i, 0);
    }
}

/// Write ZMM register with byte masking granularity
fn write_zmm_masked_b<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let bytes = vl_bytes(vl);
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..bytes {
        if (mask >> i) & 1 != 0 {
            dst.set_zmmubyte(i, result.zmmubyte(i));
        } else if zero_masking {
            dst.set_zmmubyte(i, 0);
        }
    }
    // Zero upper bytes beyond VL
    for i in bytes..64 {
        dst.set_zmmubyte(i, 0);
    }
}

/// Read 128-bit (16-byte) block from memory into a raw byte array.
fn read_mem_128<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    seg: BxSegregs,
    laddr: u64,
) -> super::Result<[u8; 16]> {
    let mut buf = [0u8; 16];
    for i in 0..4 {
        let val = cpu.v_read_dword(seg, laddr + (i * 4) as u64)?;
        let bytes = val.to_le_bytes();
        buf[i * 4..i * 4 + 4].copy_from_slice(&bytes);
    }
    Ok(buf)
}

/// Read 256-bit (32-byte) block from memory into a raw byte array.
fn read_mem_256<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    seg: BxSegregs,
    laddr: u64,
) -> super::Result<[u8; 32]> {
    let mut buf = [0u8; 32];
    for i in 0..8 {
        let val = cpu.v_read_dword(seg, laddr + (i * 4) as u64)?;
        let bytes = val.to_le_bytes();
        buf[i * 4..i * 4 + 4].copy_from_slice(&bytes);
    }
    Ok(buf)
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // VBROADCASTSS — Broadcast single-precision float
    // ========================================================================

    /// VBROADCASTSS Vdq{k}, Wss — EVEX.66.0F38.W0 18
    ///
    /// Broadcast dword element [0] from XMM source (register) or memory dword
    /// to all dword elements in destination. Semantically for FP but bitwise
    /// identical to VPBROADCASTD.
    pub fn evex_vbroadcastss(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let scalar = if instr.mod_c0() {
            read_zmm(self, instr.src()).zmm32u(0)
        } else {
            let laddr = self.resolve_addr(instr);
            self.v_read_dword(BxSegregs::from(instr.seg()), laddr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VBROADCASTSD — Broadcast double-precision float
    // ========================================================================

    /// VBROADCASTSD Vdq{k}, Wsd — EVEX.66.0F38.W1 19
    ///
    /// Broadcast qword element [0] from XMM source (register) or memory qword
    /// to all qword elements in destination.
    pub fn evex_vbroadcastsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let scalar = if instr.mod_c0() {
            read_zmm(self, instr.src()).zmm64u(0)
        } else {
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            if self.long64_mode() {
                self.read_virtual_qword_64(seg, laddr)?
            } else {
                let lo = self.v_read_dword(seg, laddr)? as u64;
                let hi = self.v_read_dword(seg, laddr + 4)? as u64;
                lo | (hi << 32)
            }
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VBROADCASTI32x4 / VBROADCASTF32x4 — Broadcast 128 bits (dword masking)
    // ========================================================================

    /// VBROADCASTI32x4 Vdq{k}, Mdq — EVEX.66.0F38.W0 5A
    ///
    /// Load 128 bits from memory and replicate to all 128-bit lanes.
    /// VL=256: 2 copies, VL=512: 4 copies. Dword masking granularity.
    /// Memory-only (no register form).
    pub fn evex_vbroadcasti32x4(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let src128 = read_mem_128(self, seg, laddr)?;

        let mut result = BxPackedZmmRegister::default();
        let num_lanes = vl_bytes(vl) / 16;
        for lane in 0..num_lanes {
            let base = lane * 16;
            result.raw_mut()[base..base + 16].copy_from_slice(&src128);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VBROADCASTF32x4 Vdq{k}, Mdq — EVEX.66.0F38.W0 1A
    ///
    /// Bitwise identical to VBROADCASTI32x4 (same operation, FP semantic).
    pub fn evex_vbroadcastf32x4(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vbroadcasti32x4(instr)
    }

    // ========================================================================
    // VBROADCASTI64x2 / VBROADCASTF64x2 — Broadcast 128 bits (qword masking)
    // ========================================================================

    /// VBROADCASTI64x2 Vdq{k}, Mdq — EVEX.66.0F38.W1 5A
    ///
    /// Load 128 bits from memory and replicate to all 128-bit lanes.
    /// VL=256: 2 copies, VL=512: 4 copies. Qword masking granularity.
    /// Memory-only (no register form).
    pub fn evex_vbroadcasti64x2(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let src128 = read_mem_128(self, seg, laddr)?;

        let mut result = BxPackedZmmRegister::default();
        let num_lanes = vl_bytes(vl) / 16;
        for lane in 0..num_lanes {
            let base = lane * 16;
            result.raw_mut()[base..base + 16].copy_from_slice(&src128);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VBROADCASTF64x2 Vdq{k}, Mdq — EVEX.66.0F38.W1 1A
    ///
    /// Bitwise identical to VBROADCASTI64x2 (same operation, FP semantic).
    pub fn evex_vbroadcastf64x2(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vbroadcasti64x2(instr)
    }

    // ========================================================================
    // VBROADCASTI32x8 / VBROADCASTF32x8 — Broadcast 256 bits (dword masking)
    // ========================================================================

    /// VBROADCASTI32x8 Vdq{k}, Mdq — EVEX.66.0F38.W0 5B
    ///
    /// Load 256 bits from memory and replicate to both 256-bit halves (512-bit only).
    /// Dword masking granularity.
    /// Memory-only (no register form).
    pub fn evex_vbroadcasti32x8(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let src256 = read_mem_256(self, seg, laddr)?;

        let mut result = BxPackedZmmRegister::default();
        let num_halves = vl_bytes(vl) / 32;
        for half in 0..num_halves {
            let base = half * 32;
            result.raw_mut()[base..base + 32].copy_from_slice(&src256);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VBROADCASTF32x8 Vdq{k}, Mdq — EVEX.66.0F38.W0 1B
    ///
    /// Bitwise identical to VBROADCASTI32x8 (same operation, FP semantic).
    pub fn evex_vbroadcastf32x8(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vbroadcasti32x8(instr)
    }

    // ========================================================================
    // VBROADCASTI64x4 / VBROADCASTF64x4 — Broadcast 256 bits (qword masking)
    // ========================================================================

    /// VBROADCASTI64x4 Vdq{k}, Mdq — EVEX.66.0F38.W1 5B
    ///
    /// Load 256 bits from memory and replicate to both 256-bit halves (512-bit only).
    /// Qword masking granularity.
    /// Memory-only (no register form).
    pub fn evex_vbroadcasti64x4(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let src256 = read_mem_256(self, seg, laddr)?;

        let mut result = BxPackedZmmRegister::default();
        let num_halves = vl_bytes(vl) / 32;
        for half in 0..num_halves {
            let base = half * 32;
            result.raw_mut()[base..base + 32].copy_from_slice(&src256);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VBROADCASTF64x4 Vdq{k}, Mdq — EVEX.66.0F38.W1 1B
    ///
    /// Bitwise identical to VBROADCASTI64x4 (same operation, FP semantic).
    pub fn evex_vbroadcastf64x4(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vbroadcasti64x4(instr)
    }

    // ========================================================================
    // VPBROADCASTB — Broadcast byte to all byte positions
    // ========================================================================

    /// VPBROADCASTB Vdq{k}, Wb — EVEX.66.0F38.W0 78
    ///
    /// Broadcast byte element [0] from XMM source (register) or memory byte
    /// to all byte positions in destination.
    pub fn evex_vpbroadcastb(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nbytes = vl_bytes(vl);
        let scalar = if instr.mod_c0() {
            read_zmm(self, instr.src()).zmmubyte(0)
        } else {
            let laddr = self.resolve_addr(instr);
            self.v_read_byte(BxSegregs::from(instr.seg()), laddr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nbytes {
            result.set_zmmubyte(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPBROADCASTW — Broadcast word to all word positions
    // ========================================================================

    /// VPBROADCASTW Vdq{k}, Ww — EVEX.66.0F38.W0 79
    ///
    /// Broadcast word element [0] from XMM source (register) or memory word
    /// to all word positions in destination.
    pub fn evex_vpbroadcastw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let scalar = if instr.mod_c0() {
            read_zmm(self, instr.src()).zmm16u(0)
        } else {
            let laddr = self.resolve_addr(instr);
            self.v_read_word(BxSegregs::from(instr.seg()), laddr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}
