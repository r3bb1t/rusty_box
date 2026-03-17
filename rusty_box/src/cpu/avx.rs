//! AVX/AVX2/AVX-512 instruction handlers for VEX.256 and EVEX operations
//!
//! Implements the subset of VEX.256 and EVEX instructions used by the Linux kernel,
//! primarily from blake2s_compress_avx512 and similar optimized routines.
//!
//! VEX.L=0 (128-bit) instructions are handled by the existing SSE handlers.
//! This file handles VEX.L=1 (256-bit) and EVEX-specific instructions.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::{BxPackedXmmRegister, BxPackedYmmRegister},
};

// AMX stub types (moved from old avx/amx.rs)
#[derive(Debug, Default)]
pub struct TILECFG {
    pub rows: u32,
    pub bytes_per_row: u32,
}

#[derive(Debug, Default)]
pub struct AMX {
    pub palette_id: u32,
    pub start_row: u32,
    pub tilecfg: [TILECFG; 8],
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // VZEROUPPER / VZEROALL (VEX.0F 77)
    // ========================================================================

    /// VZEROUPPER — Zero upper 128 bits of all YMM registers.
    /// Bochs avx.cc: for i in 0..nregs { vmm[i].ymm128(1) = 0; }
    pub(super) fn vzeroupper(&mut self, _instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let nregs = if self.long64_mode() { 16 } else { 8 };
        for i in 0..nregs {
            // Clear upper 128 bits (ymm128[1]) and ZMM upper 256 bits
            unsafe {
                self.vmm[i].zmm128[1] = BxPackedXmmRegister { xmm64u: [0, 0] };
                self.vmm[i].zmm128[2] = BxPackedXmmRegister { xmm64u: [0, 0] };
                self.vmm[i].zmm128[3] = BxPackedXmmRegister { xmm64u: [0, 0] };
            }
        }
        Ok(())
    }

    /// VZEROALL — Zero all YMM registers (all 256 bits).
    /// Bochs avx.cc: for i in 0..nregs { vmm[i] = 0; }
    pub(super) fn vzeroall(&mut self, _instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let nregs = if self.long64_mode() { 16 } else { 8 };
        for i in 0..nregs {
            self.vmm[i].clear();
        }
        Ok(())
    }

    // ========================================================================
    // VEX.L-aware dispatch wrappers
    // These check VEX.L and dispatch to 128-bit (SSE) or 256-bit (AVX) handlers
    // ========================================================================

    /// VMOVDQU load — VEX.L=0: XMM <- M128, VEX.L=1: YMM <- M256
    /// Also handles register form (mod=11): dst_reg <- src_reg
    pub(super) fn vmovdqu_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            // Register form: copy src1 (rm) to dst (nnn)
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.write_xmm_reg(instr.dst(), val);
            }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                let val = self.v_read_ymmword(seg, eaddr)?;
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.v_read_xmmword(seg, eaddr)?;
                self.write_xmm_reg(instr.dst(), val);
            }
        }
        Ok(())
    }

    /// VMOVDQU store — VEX.L=0: M128 <- XMM, VEX.L=1: M256 <- YMM
    /// Also handles register form (mod=11): dst_reg <- src_reg
    pub(super) fn vmovdqu_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            // Register form: copy src1 (nnn) to dst (rm)
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.write_xmm_reg(instr.dst(), val);
            }
        } else {
            // Memory form: store to memory
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.v_write_ymmword(seg, eaddr, &val)?;
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.v_write_xmmword(seg, eaddr, &val)?;
            }
        }
        Ok(())
    }

    /// VMOVDQA/VMOVAPS/VMOVAPD register-to-register — VEX.L aware
    pub(super) fn vmovdqa_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let val = self.read_ymm_reg(instr.src1());
            self.write_ymm_reg(instr.dst(), val);
        } else {
            let val = self.read_xmm_reg(instr.src1());
            self.write_xmm_reg(instr.dst(), val);
        }
        Ok(())
    }

    /// VMOVDQA/VMOVAPS load — VEX.L=0: XMM <- M128, VEX.L=1: YMM <- M256 (aligned)
    /// Also handles register form (mod=11): dst_reg <- src_reg
    pub(super) fn vmovdqa_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            // Register form: copy src1 (rm) to dst (nnn)
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.write_xmm_reg(instr.dst(), val);
            }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                let val = self.v_read_ymmword(seg, eaddr)?;
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.v_read_xmmword_aligned(seg, eaddr)?;
                self.write_xmm_reg(instr.dst(), val);
            }
        }
        Ok(())
    }

    /// VMOVDQA store — VEX.L=0: M128 <- XMM, VEX.L=1: M256 <- YMM (aligned)
    /// Also handles register form (mod=11): dst_reg <- src_reg
    pub(super) fn vmovdqa_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            // Register form: copy src1 (nnn) to dst (rm)
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.write_ymm_reg(instr.dst(), val);
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.write_xmm_reg(instr.dst(), val);
            }
        } else {
            // Memory form: store to memory
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                let val = self.read_ymm_reg(instr.src1());
                self.v_write_ymmword(seg, eaddr, &val)?;
            } else {
                let val = self.read_xmm_reg(instr.src1());
                self.v_write_xmmword_aligned(seg, eaddr, &val)?;
            }
        }
        Ok(())
    }

    /// VMOVUPS/VMOVUPD load — VEX.L aware unaligned
    pub(super) fn vmovups_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vmovdqu_load(instr) // same behavior
    }

    /// VMOVUPS/VMOVUPD store — VEX.L aware unaligned
    pub(super) fn vmovups_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.vmovdqu_store(instr) // same behavior
    }

    // ========================================================================
    // VEX.L-aware packed integer arithmetic
    // ========================================================================

    /// VPADDD — Packed Add Dwords (VEX.L aware)
    /// dst = src1 + src2 (element-wise 32-bit add)
    pub(super) fn vpaddd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            // 256-bit: 8 dwords
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] = src1.ymm32u[i].wrapping_add(src2.ymm32u[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            // 128-bit: 4 dwords
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] = src1.xmm32u[i].wrapping_add(src2.xmm32u[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPXOR / VPXORD — Packed XOR (VEX.L aware)
    /// dst = src1 ^ src2
    pub(super) fn vpxor(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    result.ymm64u[i] = src1.ymm64u[i] ^ src2.ymm64u[i];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    result.xmm64u[i] = src1.xmm64u[i] ^ src2.xmm64u[i];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPAND / VPANDD — Packed AND (VEX.L aware)
    pub(super) fn vpand(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    result.ymm64u[i] = src1.ymm64u[i] & src2.ymm64u[i];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    result.xmm64u[i] = src1.xmm64u[i] & src2.xmm64u[i];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPEQD — Packed Compare Equal Dwords (VEX.L aware)
    /// dst[i] = (src1[i] == src2[i]) ? 0xFFFFFFFF : 0
    pub(super) fn vpcmpeqd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] =
                        if src1.ymm32u[i] == src2.ymm32u[i] { 0xFFFF_FFFF } else { 0 };
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] =
                        if src1.xmm32u[i] == src2.xmm32u[i] { 0xFFFF_FFFF } else { 0 };
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSHUFD — Packed Shuffle Dwords (VEX.L aware)
    /// dst[i] = src[imm8[i*2+1:i*2]] for each dword lane
    pub(super) fn vpshufd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let imm = instr.ib();
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane
                for i in 0..4 {
                    let sel = ((imm >> (i * 2)) & 0x3) as usize;
                    result.ymm32u[i] = src.ymm32u[sel];
                }
                // Upper 128-bit lane (operates independently)
                for i in 0..4 {
                    let sel = ((imm >> (i * 2)) & 0x3) as usize;
                    result.ymm32u[4 + i] = src.ymm32u[4 + sel];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    let sel = ((imm >> (i * 2)) & 0x3) as usize;
                    result.xmm32u[i] = src.xmm32u[sel];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // AVX-512 specific instructions (EVEX only)
    // ========================================================================

    /// VPERMI2D — Full Permute of Dwords from Two Sources
    /// EVEX.66.0F38.W0 76 /r
    /// For each dword element i in dest:
    ///   index = dest[i] (low bits select from concatenation of src1:src2)
    ///   result[i] = (src1:src2)[index]
    /// where src1 = VEX.vvvv, src2 = r/m
    pub(super) fn vpermi2d(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv

        if instr.get_vl() >= 1 {
            // 256-bit: 8 dwords, index bits 2:0 select from 16-element pool (8+8)
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let indices = self.read_ymm_reg(dst_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };

            // Concatenate src1 (elements 0-7) and src2 (elements 8-15)
            unsafe {
                let num_elements = 8usize; // 256-bit / 32-bit = 8 elements
                let index_mask = (num_elements * 2 - 1) as u32; // 0xF for 16-element pool
                for i in 0..num_elements {
                    let idx = (indices.ymm32u[i] & index_mask) as usize;
                    if idx < num_elements {
                        result.ymm32u[i] = src1.ymm32u[idx];
                    } else {
                        result.ymm32u[i] = src2.ymm32u[idx - num_elements];
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            // 128-bit: 4 dwords, index bits 2:0 select from 8-element pool (4+4)
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let indices = self.read_xmm_reg(dst_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };

            unsafe {
                let num_elements = 4usize;
                let index_mask = (num_elements * 2 - 1) as u32; // 0x7 for 8-element pool
                for i in 0..num_elements {
                    let idx = (indices.xmm32u[i] & index_mask) as usize;
                    if idx < num_elements {
                        result.xmm32u[i] = src1.xmm32u[idx];
                    } else {
                        result.xmm32u[i] = src2.xmm32u[idx - num_elements];
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPRORD — Packed Rotate Right Dwords by immediate
    /// EVEX.66.0F.W0 72 /0 ib
    /// Operands: dst=VEX.vvvv (src2), src=rm (src1), imm8
    pub(super) fn vprord(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv — for EVEX group opcodes, dst is in vvvv
        let count = (instr.ib() & 31) as u32; // rotate count mod 32

        // For EVEX group opcodes, rm (source) is in dst(), nnn is opcode extension
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] = src.ymm32u[i].rotate_right(count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] = src.xmm32u[i].rotate_right(count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPROLD — Packed Rotate Left Dwords by immediate
    /// EVEX.66.0F.W0 72 /1 ib
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vprold(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv — for EVEX group opcodes, dst is in vvvv
        let count = (instr.ib() & 31) as u32; // rotate count mod 32

        // For EVEX group opcodes, rm (source) is in dst(), nnn is opcode extension
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] = src.ymm32u[i].rotate_left(count);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] = src.xmm32u[i].rotate_left(count);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMOVZXBD — Packed Zero-Extend Bytes to Dwords (VEX.L aware)
    /// VEX.128: 4 bytes → 4 dwords (XMM)
    /// VEX.256: 8 bytes → 8 dwords (YMM)
    pub(super) fn vpmovzxbd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();

        if instr.get_vl() >= 1 {
            // 256-bit: read 8 bytes, zero-extend each to dword
            let src_bytes: [u8; 8] = if instr.mod_c0() {
                // Register form: lower 8 bytes of XMM
                let src = self.read_xmm_reg(instr.src1());
                let mut bytes = [0u8; 8];
                unsafe {
                    bytes.copy_from_slice(&src.xmmubyte[..8]);
                }
                bytes
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                let q = self.v_read_qword(seg, eaddr)?;
                q.to_le_bytes()
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] = src_bytes[i] as u32;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            // 128-bit: read 4 bytes, zero-extend each to dword
            let src_bytes: [u8; 4] = if instr.mod_c0() {
                let src = self.read_xmm_reg(instr.src1());
                let mut bytes = [0u8; 4];
                unsafe {
                    bytes.copy_from_slice(&src.xmmubyte[..4]);
                }
                bytes
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                let d = self.v_read_dword(seg, eaddr)?;
                d.to_le_bytes()
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] = src_bytes[i] as u32;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKLDQ — Unpack and Interleave Low Dwords (VEX.L aware)
    /// 128-bit: dst = [src1[0], src2[0], src1[1], src2[1]]
    /// 256-bit: same per 128-bit lane
    pub(super) fn vpunpckldq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower lane
                result.ymm32u[0] = src1.ymm32u[0];
                result.ymm32u[1] = src2.ymm32u[0];
                result.ymm32u[2] = src1.ymm32u[1];
                result.ymm32u[3] = src2.ymm32u[1];
                // Upper lane
                result.ymm32u[4] = src1.ymm32u[4];
                result.ymm32u[5] = src2.ymm32u[4];
                result.ymm32u[6] = src1.ymm32u[5];
                result.ymm32u[7] = src2.ymm32u[5];
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                result.xmm32u[0] = src1.xmm32u[0];
                result.xmm32u[1] = src2.xmm32u[0];
                result.xmm32u[2] = src1.xmm32u[1];
                result.xmm32u[3] = src2.xmm32u[1];
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKHDQ — Unpack and Interleave High Dwords (VEX.L aware)
    pub(super) fn vpunpckhdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower lane: high dwords
                result.ymm32u[0] = src1.ymm32u[2];
                result.ymm32u[1] = src2.ymm32u[2];
                result.ymm32u[2] = src1.ymm32u[3];
                result.ymm32u[3] = src2.ymm32u[3];
                // Upper lane
                result.ymm32u[4] = src1.ymm32u[6];
                result.ymm32u[5] = src2.ymm32u[6];
                result.ymm32u[6] = src1.ymm32u[7];
                result.ymm32u[7] = src2.ymm32u[7];
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                result.xmm32u[0] = src1.xmm32u[2];
                result.xmm32u[1] = src2.xmm32u[2];
                result.xmm32u[2] = src1.xmm32u[3];
                result.xmm32u[3] = src2.xmm32u[3];
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBD — Packed Subtract Dwords (VEX.L aware)
    pub(super) fn vpsubd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] = src1.ymm32u[i].wrapping_sub(src2.ymm32u[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] = src1.xmm32u[i].wrapping_sub(src2.xmm32u[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLD — Packed Shift Left Logical Dwords by immediate (VEX.L aware)
    /// Used in EVEX as EVEX.66.0F.W0 72 /6 ib
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpslld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv — for EVEX group opcodes, dst is in vvvv
        let count = instr.ib() as u32;

        // For EVEX group opcodes, rm (source) is in dst(), nnn is opcode extension
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 32 {
                unsafe {
                    for i in 0..8 {
                        result.ymm32u[i] = src.ymm32u[i] << count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 32 {
                unsafe {
                    for i in 0..4 {
                        result.xmm32u[i] = src.xmm32u[i] << count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLD — Packed Shift Right Logical Dwords by immediate (VEX.L aware)
    /// Used in EVEX as EVEX.66.0F.W0 72 /2 ib
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsrld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv — for EVEX group opcodes, dst is in vvvv
        let count = instr.ib() as u32;

        // For EVEX group opcodes, rm (source) is in dst(), nnn is opcode extension
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 32 {
                unsafe {
                    for i in 0..8 {
                        result.ymm32u[i] = src.ymm32u[i] >> count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 32 {
                unsafe {
                    for i in 0..4 {
                        result.xmm32u[i] = src.xmm32u[i] >> count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPOR / VPORD — Packed OR (VEX.L aware)
    pub(super) fn vpor(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    result.ymm64u[i] = src1.ymm64u[i] | src2.ymm64u[i];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    result.xmm64u[i] = src1.xmm64u[i] | src2.xmm64u[i];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VINSERTF128 / VINSERTI128 — Insert 128-bit value into 256-bit register
    /// VEX.256.66.0F3A.W0 18 /r ib (VINSERTF128)
    /// VEX.256.66.0F3A.W0 38 /r ib (VINSERTI128)
    /// Matches Bochs VINSERTF128_VdqHdqWdqIbR (avx.cc:478)
    /// Both instructions perform the identical operation — integer vs float is
    /// only a naming distinction.
    /// dst = src1 (VEX.vvvv) with 128-bit lane[imm8[0]] replaced by src2 (rm)
    pub(super) fn vinsert_f128_i128(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        // Read the full 256-bit source (VEX.vvvv)
        let mut result = self.read_ymm_reg(instr.src2());
        let imm = instr.ib();

        // Read the 128-bit value to insert (rm — register or memory)
        let src2 = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_xmmword(seg, eaddr)?
        };

        // Insert into the selected 128-bit lane
        // For VEX.256: offset = imm8 & 1 (only 2 lanes)
        let offset = (imm & 1) as usize;
        let base = offset * 2; // index into ymm64u array
        unsafe {
            result.ymm64u[base] = src2.xmm64u[0];
            result.ymm64u[base + 1] = src2.xmm64u[1];
        }

        self.write_ymm_reg(instr.dst(), result);
        Ok(())
    }

    /// VEXTRACTI128 — Extract 128-bit integer value from 256-bit register
    /// VEX.256.66.0F3A.W0 39 /r ib
    /// If imm8[0]=0: dst = src[127:0]; if imm8[0]=1: dst = src[255:128]
    /// Our decoder: dst() = nnn (source YMM), src1() = rm (destination XMM)
    pub(super) fn vextracti128(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_idx = instr.dst(); // nnn — source YMM register
        let imm = instr.ib();
        let src = self.read_ymm_reg(src_idx);
        let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
        if (imm & 1) != 0 {
            // Extract upper 128 bits
            unsafe {
                result.xmm64u[0] = src.ymm64u[2];
                result.xmm64u[1] = src.ymm64u[3];
            }
        } else {
            // Extract lower 128 bits
            unsafe {
                result.xmm64u[0] = src.ymm64u[0];
                result.xmm64u[1] = src.ymm64u[1];
            }
        }

        if instr.mod_c0() {
            self.write_xmm_reg(instr.src1(), result); // rm = destination
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_write_xmmword(seg, eaddr, &result)?;
        }
        Ok(())
    }

    /// VPERM2I128 — Permute 128-bit integer values from two 256-bit sources
    /// VEX.256.66.0F3A.W0 46 /r ib
    /// Matches Bochs VPERM2F128_VdqHdqWdqIbR (avx.cc:543)
    /// For each 128-bit half (n=0,1): select from imm8 bits [n*4+3:n*4]
    ///   bit 3: zero that half
    ///   bit 1: select op2 (else op1)
    ///   bit 0: select which 128-bit half of chosen source
    pub(super) fn vperm2i128(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_ymm_reg(instr.src2()); // VEX.vvvv
        let op2 = if instr.mod_c0() {
            self.read_ymm_reg(instr.src1()) // rm
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_ymmword(seg, eaddr)?
        };
        let mut order = instr.ib();
        let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };

        for n in 0..2u8 {
            let base = (n as usize) * 2; // index into ymm64u (0 or 2)
            if (order & 0x8) != 0 {
                // Zero this 128-bit half
                unsafe {
                    result.ymm64u[base] = 0;
                    result.ymm64u[base + 1] = 0;
                }
            } else {
                let src = if (order & 0x2) != 0 { &op2 } else { &op1 };
                let half = (order & 0x1) as usize; // which 128-bit half of source
                let src_base = half * 2;
                unsafe {
                    result.ymm64u[base] = src.ymm64u[src_base];
                    result.ymm64u[base + 1] = src.ymm64u[src_base + 1];
                }
            }
            order >>= 4;
        }

        self.write_ymm_reg(instr.dst(), result);
        Ok(())
    }

    /// VPSHUFB — Packed Shuffle Bytes (VEX.L aware, 3-operand VEX encoding)
    /// VEX.128/256.66.0F38 00 /r
    /// Matches Bochs VPSHUFB (avx512.cc:702) — per-lane byte shuffle
    /// dst[i] = (mask[i] & 0x80) ? 0 : data[mask[i] & 0xF]  (within each 128-bit lane)
    pub(super) fn vpshufb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let data_idx = instr.src2(); // VEX.vvvv — data source

        if instr.get_vl() >= 1 {
            // 256-bit: two independent 128-bit lane shuffles
            let data = self.read_ymm_reg(data_idx);
            let mask = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane (bytes 0-15)
                for i in 0..16usize {
                    let m = mask.ymmubyte[i];
                    if (m & 0x80) != 0 {
                        result.ymmubyte[i] = 0;
                    } else {
                        result.ymmubyte[i] = data.ymmubyte[(m & 0xf) as usize];
                    }
                }
                // Upper 128-bit lane (bytes 16-31) — shuffles within upper lane only
                for i in 16..32usize {
                    let m = mask.ymmubyte[i];
                    if (m & 0x80) != 0 {
                        result.ymmubyte[i] = 0;
                    } else {
                        result.ymmubyte[i] = data.ymmubyte[16 + (m & 0xf) as usize];
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            // 128-bit: single lane shuffle
            let data = self.read_xmm_reg(data_idx);
            let mask = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16usize {
                    let m = mask.xmmubyte[i];
                    if (m & 0x80) != 0 {
                        result.xmmubyte[i] = 0;
                    } else {
                        result.xmmubyte[i] = data.xmmubyte[(m & 0xf) as usize];
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // =========================================================================
    // VPALIGNR — Packed Align Right (AVX/AVX2)
    // Bochs: avx2.cc VPALIGNR_VdqHdqWdqIbR
    // =========================================================================

    /// VPALIGNR — Packed Align Right (VEX.L aware, 3-operand)
    /// VEX.128/256.66.0F3A 0F /r ib
    /// Per 128-bit lane: result = [src1:src2] >> (imm8 * 8), where src1 is high.
    /// Bochs: op1 = src1 (vvv), op2 = src2 (rm); xmm_palignr(&op2, &op1, imm8);
    ///        write op2 to dst.
    pub(super) fn vpalignr(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let shift = instr.ib() as usize;

        if instr.get_vl() >= 1 {
            // 256-bit: two independent 128-bit lane align-right operations
            let op1 = self.read_ymm_reg(instr.src2()); // VEX.vvvv = high part
            let op2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            // Process each 128-bit lane independently
            for lane in 0..2usize {
                let base = lane * 16;
                Self::palignr_lane(
                    &op1, &op2, base, shift, &mut result,
                );
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            // 128-bit: single lane
            let op1 = self.read_xmm_reg(instr.src2()); // VEX.vvvv = high part
            let op2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            // Concatenate [op1:op2] (32 bytes, but only 16 bytes each)
            // and extract 16 bytes starting at byte offset `shift`
            if shift >= 32 {
                // All zeros — result already zeroed
            } else if shift >= 16 {
                // Only op1 bytes contribute, shifted right
                let s = shift - 16;
                unsafe {
                    for i in 0..(16 - s) {
                        result.xmmubyte[i] = op1.xmmubyte[i + s];
                    }
                }
            } else {
                // Both op2 and op1 contribute
                unsafe {
                    for i in 0..16usize {
                        let src_idx = i + shift;
                        if src_idx < 16 {
                            result.xmmubyte[i] = op2.xmmubyte[src_idx];
                        } else {
                            result.xmmubyte[i] = op1.xmmubyte[src_idx - 16];
                        }
                    }
                }
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// Helper: PALIGNR for one 128-bit lane within a YMM register.
    /// op1[base..base+16] is high, op2[base..base+16] is low.
    fn palignr_lane(
        op1: &BxPackedYmmRegister,
        op2: &BxPackedYmmRegister,
        base: usize,
        shift: usize,
        result: &mut BxPackedYmmRegister,
    ) {
        if shift >= 32 {
            // All zeros — result bytes already 0
        } else if shift >= 16 {
            let s = shift - 16;
            unsafe {
                for i in 0..(16 - s) {
                    result.ymmubyte[base + i] = op1.ymmubyte[base + i + s];
                }
            }
        } else {
            unsafe {
                for i in 0..16usize {
                    let src_idx = i + shift;
                    if src_idx < 16 {
                        result.ymmubyte[base + i] = op2.ymmubyte[base + src_idx];
                    } else {
                        result.ymmubyte[base + i] = op1.ymmubyte[base + src_idx - 16];
                    }
                }
            }
        }
    }

    // =========================================================================
    // VPBLENDD — Blend Packed Dwords (AVX2)
    // Bochs: VPBLENDD_VdqHdqWdqIbR → uses same logic as VBLENDPS
    // =========================================================================

    /// VPBLENDD — Blend packed dwords by immediate mask (VEX.L aware)
    /// VEX.128/256.66.0F3A.W0 02 /r ib
    /// For each dword lane i: dst[i] = (imm8 & (1<<i)) ? src2[i] : src1[i]
    /// src1 = VEX.vvvv (src2()), src2 = rm (src1())
    pub(super) fn vpblendd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let imm8 = instr.ib();
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv

        if instr.get_vl() >= 1 {
            // 256-bit: 8 dwords
            let src1 = self.read_ymm_reg(src1_idx);
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8usize {
                    if (imm8 & (1 << i)) != 0 {
                        result.ymm32u[i] = src2.ymm32u[i];
                    } else {
                        result.ymm32u[i] = src1.ymm32u[i];
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            // 128-bit: 4 dwords (only bits 0-3 of imm8 matter)
            let src1 = self.read_xmm_reg(src1_idx);
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4usize {
                    if (imm8 & (1 << i)) != 0 {
                        result.xmm32u[i] = src2.xmm32u[i];
                    } else {
                        result.xmm32u[i] = src1.xmm32u[i];
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // =========================================================================
    // VPBROADCAST — Broadcast scalar to all elements (AVX2)
    // Bochs: avx2.cc VPBROADCASTB/W/D/Q
    // =========================================================================

    /// VPBROADCASTB — broadcast byte from XMM[0] to all bytes of dst
    pub(super) fn vpbroadcastb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_byte = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmmubyte[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_byte(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe { for i in 0..32 { result.ymmubyte[i] = src_byte; } }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe { for i in 0..16 { result.xmmubyte[i] = src_byte; } }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPBROADCASTW — broadcast word from XMM[0] to all words of dst
    pub(super) fn vpbroadcastw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_word = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm16u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_word(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe { for i in 0..16 { result.ymm16u[i] = src_word; } }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe { for i in 0..8 { result.xmm16u[i] = src_word; } }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPBROADCASTD — broadcast dword from XMM[0] to all dwords of dst
    pub(super) fn vpbroadcastd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_dword = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm32u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_dword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe { for i in 0..8 { result.ymm32u[i] = src_dword; } }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe { for i in 0..4 { result.xmm32u[i] = src_dword; } }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPBROADCASTQ — broadcast qword from XMM[0] to all qwords of dst
    pub(super) fn vpbroadcastq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let src_qword = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm64u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe { for i in 0..4 { result.ymm64u[i] = src_qword; } }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe { for i in 0..2 { result.xmm64u[i] = src_qword; } }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VPERMD — Permute dwords in YMM using index from another YMM (AVX2)
    /// Bochs: avx2.cc V256_VPERMD_VdqHdqWdq
    pub(super) fn vpermd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let idx = self.read_ymm_reg(instr.src2());  // VEX.vvvv = index
        let src = if instr.mod_c0() {
            self.read_ymm_reg(instr.src1())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_ymmword(seg, eaddr)?
        };
        let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
        unsafe {
            for i in 0..8 {
                let sel = (idx.ymm32u[i] & 7) as usize;
                result.ymm32u[i] = src.ymm32u[sel];
            }
        }
        self.write_ymm_reg(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Unpack byte/word/qword variants
    // ========================================================================

    /// VPUNPCKLBW — Unpack and Interleave Low Bytes (VEX.L aware)
    /// Per 128-bit lane: result[2i] = src1[i], result[2i+1] = src2[i] for i in 0..8
    pub(super) fn vpunpcklbw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane
                for i in 0..8usize {
                    result.ymmubyte[i * 2] = src1.ymmubyte[i];
                    result.ymmubyte[i * 2 + 1] = src2.ymmubyte[i];
                }
                // Upper 128-bit lane
                for i in 0..8usize {
                    result.ymmubyte[16 + i * 2] = src1.ymmubyte[16 + i];
                    result.ymmubyte[16 + i * 2 + 1] = src2.ymmubyte[16 + i];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8usize {
                    result.xmmubyte[i * 2] = src1.xmmubyte[i];
                    result.xmmubyte[i * 2 + 1] = src2.xmmubyte[i];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKHBW — Unpack and Interleave High Bytes (VEX.L aware)
    /// Per 128-bit lane: result[2i] = src1[8+i], result[2i+1] = src2[8+i] for i in 0..8
    pub(super) fn vpunpckhbw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane (high bytes 8..16)
                for i in 0..8usize {
                    result.ymmubyte[i * 2] = src1.ymmubyte[8 + i];
                    result.ymmubyte[i * 2 + 1] = src2.ymmubyte[8 + i];
                }
                // Upper 128-bit lane (high bytes 24..32)
                for i in 0..8usize {
                    result.ymmubyte[16 + i * 2] = src1.ymmubyte[24 + i];
                    result.ymmubyte[16 + i * 2 + 1] = src2.ymmubyte[24 + i];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8usize {
                    result.xmmubyte[i * 2] = src1.xmmubyte[8 + i];
                    result.xmmubyte[i * 2 + 1] = src2.xmmubyte[8 + i];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKLWD — Unpack and Interleave Low Words (VEX.L aware)
    /// Per 128-bit lane: result[2i] = src1[i], result[2i+1] = src2[i] for i in 0..4
    pub(super) fn vpunpcklwd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane
                for i in 0..4usize {
                    result.ymm16u[i * 2] = src1.ymm16u[i];
                    result.ymm16u[i * 2 + 1] = src2.ymm16u[i];
                }
                // Upper 128-bit lane
                for i in 0..4usize {
                    result.ymm16u[8 + i * 2] = src1.ymm16u[8 + i];
                    result.ymm16u[8 + i * 2 + 1] = src2.ymm16u[8 + i];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4usize {
                    result.xmm16u[i * 2] = src1.xmm16u[i];
                    result.xmm16u[i * 2 + 1] = src2.xmm16u[i];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKHWD — Unpack and Interleave High Words (VEX.L aware)
    /// Per 128-bit lane: result[2i] = src1[4+i], result[2i+1] = src2[4+i] for i in 0..4
    pub(super) fn vpunpckhwd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane (high words 4..8)
                for i in 0..4usize {
                    result.ymm16u[i * 2] = src1.ymm16u[4 + i];
                    result.ymm16u[i * 2 + 1] = src2.ymm16u[4 + i];
                }
                // Upper 128-bit lane (high words 12..16)
                for i in 0..4usize {
                    result.ymm16u[8 + i * 2] = src1.ymm16u[12 + i];
                    result.ymm16u[8 + i * 2 + 1] = src2.ymm16u[12 + i];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4usize {
                    result.xmm16u[i * 2] = src1.xmm16u[4 + i];
                    result.xmm16u[i * 2 + 1] = src2.xmm16u[4 + i];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKLQDQ — Unpack and Interleave Low Qwords (VEX.L aware)
    /// Per 128-bit lane: result[0] = src1[0], result[1] = src2[0]
    pub(super) fn vpunpcklqdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower lane
                result.ymm64u[0] = src1.ymm64u[0];
                result.ymm64u[1] = src2.ymm64u[0];
                // Upper lane
                result.ymm64u[2] = src1.ymm64u[2];
                result.ymm64u[3] = src2.ymm64u[2];
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                result.xmm64u[0] = src1.xmm64u[0];
                result.xmm64u[1] = src2.xmm64u[0];
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPUNPCKHQDQ — Unpack and Interleave High Qwords (VEX.L aware)
    /// Per 128-bit lane: result[0] = src1[1], result[1] = src2[1]
    pub(super) fn vpunpckhqdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower lane
                result.ymm64u[0] = src1.ymm64u[1];
                result.ymm64u[1] = src2.ymm64u[1];
                // Upper lane
                result.ymm64u[2] = src1.ymm64u[3];
                result.ymm64u[3] = src2.ymm64u[3];
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                result.xmm64u[0] = src1.xmm64u[1];
                result.xmm64u[1] = src2.xmm64u[1];
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed integer add/sub (byte, word, qword widths)
    // ========================================================================

    /// VPADDQ — Packed Add Qwords (VEX.L aware)
    /// dst[i] = vvvv[i] + src[i] (element-wise 64-bit wrapping add)
    pub(super) fn vpaddq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    result.ymm64u[i] = src1.ymm64u[i].wrapping_add(src2.ymm64u[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    result.xmm64u[i] = src1.xmm64u[i].wrapping_add(src2.xmm64u[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDW — Packed Add Words (VEX.L aware)
    /// dst[i] = vvvv[i] + src[i] (element-wise 16-bit wrapping add)
    pub(super) fn vpaddw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16u[i] = src1.ymm16u[i].wrapping_add(src2.ymm16u[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16u[i] = src1.xmm16u[i].wrapping_add(src2.xmm16u[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDB — Packed Add Bytes (VEX.L aware)
    /// dst[i] = vvvv[i] + src[i] (element-wise 8-bit wrapping add)
    pub(super) fn vpaddb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..32 {
                    result.ymmubyte[i] = src1.ymmubyte[i].wrapping_add(src2.ymmubyte[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16 {
                    result.xmmubyte[i] = src1.xmmubyte[i].wrapping_add(src2.xmmubyte[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBQ — Packed Subtract Qwords (VEX.L aware)
    /// dst[i] = vvvv[i] - src[i] (element-wise 64-bit wrapping sub)
    pub(super) fn vpsubq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    result.ymm64u[i] = src1.ymm64u[i].wrapping_sub(src2.ymm64u[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    result.xmm64u[i] = src1.xmm64u[i].wrapping_sub(src2.xmm64u[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBW — Packed Subtract Words (VEX.L aware)
    /// dst[i] = vvvv[i] - src[i] (element-wise 16-bit wrapping sub)
    pub(super) fn vpsubw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16u[i] = src1.ymm16u[i].wrapping_sub(src2.ymm16u[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16u[i] = src1.xmm16u[i].wrapping_sub(src2.xmm16u[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBB — Packed Subtract Bytes (VEX.L aware)
    /// dst[i] = vvvv[i] - src[i] (element-wise 8-bit wrapping sub)
    pub(super) fn vpsubb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..32 {
                    result.ymmubyte[i] = src1.ymmubyte[i].wrapping_sub(src2.ymmubyte[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16 {
                    result.xmmubyte[i] = src1.xmmubyte[i].wrapping_sub(src2.xmmubyte[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed logical: VPANDN
    // ========================================================================

    /// VPANDN — Packed AND NOT (VEX.L aware)
    /// dst[i] = NOT(vvvv[i]) AND src[i]
    pub(super) fn vpandn(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    result.ymm64u[i] = !src1.ymm64u[i] & src2.ymm64u[i];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    result.xmm64u[i] = !src1.xmm64u[i] & src2.xmm64u[i];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed multiply
    // ========================================================================

    /// VPMULUDQ — Unsigned Multiply Dwords to Qwords (VEX.L aware)
    /// dst_q[i] = (vvvv_d[i*2] as u64) * (src_d[i*2] as u64)
    /// Uses even-numbered dwords only, produces qword results
    pub(super) fn vpmuludq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    result.ymm64u[i] =
                        (src1.ymm32u[i * 2] as u64) * (src2.ymm32u[i * 2] as u64);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    result.xmm64u[i] =
                        (src1.xmm32u[i * 2] as u64) * (src2.xmm32u[i * 2] as u64);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULDQ — Signed Multiply Dwords to Qwords (VEX.L aware)
    /// dst_q[i] = (vvvv_d[i*2] as i32 as i64) * (src_d[i*2] as i32 as i64)
    /// Uses even-numbered dwords only (signed), produces qword results
    pub(super) fn vpmuldq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    let a = src1.ymm32s[i * 2] as i64;
                    let b = src2.ymm32s[i * 2] as i64;
                    result.ymm64u[i] = (a * b) as u64;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    let a = src1.xmm32s[i * 2] as i64;
                    let b = src2.xmm32s[i * 2] as i64;
                    result.xmm64u[i] = (a * b) as u64;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULLD — Packed Multiply Low Dwords (VEX.L aware)
    /// dst[i] = (vvvv[i] as i32).wrapping_mul(src[i] as i32) as u32
    pub(super) fn vpmulld(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] =
                        (src1.ymm32s[i] as i64 * src2.ymm32s[i] as i64) as u32;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] =
                        (src1.xmm32s[i] as i64 * src2.xmm32s[i] as i64) as u32;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULLW — Packed Multiply Low Words (VEX.L aware)
    /// dst[i] = low 16 bits of (vvvv[i] * src[i])
    pub(super) fn vpmullw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    let prod = (src1.ymm16s[i] as i32) * (src2.ymm16s[i] as i32);
                    result.ymm16u[i] = prod as u16;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    let prod = (src1.xmm16s[i] as i32) * (src2.xmm16s[i] as i32);
                    result.xmm16u[i] = prod as u16;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULHW — Packed Multiply High Words Signed (VEX.L aware)
    /// dst[i] = high 16 bits of (vvvv[i] as i16 * src[i] as i16)
    pub(super) fn vpmulhw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    let prod = (src1.ymm16s[i] as i32) * (src2.ymm16s[i] as i32);
                    result.ymm16u[i] = (prod >> 16) as u16;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    let prod = (src1.xmm16s[i] as i32) * (src2.xmm16s[i] as i32);
                    result.xmm16u[i] = (prod >> 16) as u16;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULHUW — Packed Multiply High Words Unsigned (VEX.L aware)
    /// dst[i] = high 16 bits of (vvvv[i] as u16 * src[i] as u16)
    pub(super) fn vpmulhuw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    let prod = (src1.ymm16u[i] as u32) * (src2.ymm16u[i] as u32);
                    result.ymm16u[i] = (prod >> 16) as u16;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    let prod = (src1.xmm16u[i] as u32) * (src2.xmm16u[i] as u32);
                    result.xmm16u[i] = (prod >> 16) as u16;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPMULHRSW — Packed Multiply High with Round and Scale (VEX.L aware)
    /// Bochs simd_int.h:988-993: result[i] = (((src1[i] * src2[i]) >> 14) + 1) >> 1
    pub(super) fn vpmulhrsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    let t = ((src1.ymm16s[i] as i32 * src2.ymm16s[i] as i32) >> 14) + 1;
                    result.ymm16u[i] = (t >> 1) as u16;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    let t = ((src1.xmm16s[i] as i32 * src2.xmm16s[i] as i32) >> 14) + 1;
                    result.xmm16u[i] = (t >> 1) as u16;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed shift by register (count from low 64 bits of XMM source)
    // ========================================================================

    /// VPSRLQ — Packed Shift Right Logical Qwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 63, result is zero.
    pub(super) fn vpsrlq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        // Shift count from ModRM source (register or memory)
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm64u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 64 {
                let count = count as u32;
                unsafe {
                    for i in 0..4 {
                        result.ymm64u[i] = src1.ymm64u[i] >> count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 64 {
                let count = count as u32;
                unsafe {
                    for i in 0..2 {
                        result.xmm64u[i] = src1.xmm64u[i] >> count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLD — Packed Shift Left Logical Dwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 31, result is zero.
    pub(super) fn vpslld_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm64u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 32 {
                let count = count as u32;
                unsafe {
                    for i in 0..8 {
                        result.ymm32u[i] = src1.ymm32u[i] << count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 32 {
                let count = count as u32;
                unsafe {
                    for i in 0..4 {
                        result.xmm32u[i] = src1.xmm32u[i] << count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLQ — Packed Shift Left Logical Qwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 63, result is zero.
    pub(super) fn vpsllq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm64u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 64 {
                let count = count as u32;
                unsafe {
                    for i in 0..4 {
                        result.ymm64u[i] = src1.ymm64u[i] << count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 64 {
                let count = count as u32;
                unsafe {
                    for i in 0..2 {
                        result.xmm64u[i] = src1.xmm64u[i] << count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLW — Packed Shift Right Logical Words by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 15, result is zero.
    pub(super) fn vpsrlw_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm64u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 16 {
                let count = count as u32;
                unsafe {
                    for i in 0..16 {
                        result.ymm16u[i] = src1.ymm16u[i] >> count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 16 {
                let count = count as u32;
                unsafe {
                    for i in 0..8 {
                        result.xmm16u[i] = src1.xmm16u[i] >> count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLD — Packed Shift Right Logical Dwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 31, result is zero.
    pub(super) fn vpsrld_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm64u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 32 {
                let count = count as u32;
                unsafe {
                    for i in 0..8 {
                        result.ymm32u[i] = src1.ymm32u[i] >> count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 32 {
                let count = count as u32;
                unsafe {
                    for i in 0..4 {
                        result.xmm32u[i] = src1.xmm32u[i] >> count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRAW — Packed Shift Right Arithmetic Words by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 15, count is clamped to 15.
    pub(super) fn vpsraw_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count_raw = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm64u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        let count = if count_raw > 15 { 15u32 } else { count_raw as u32 };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16u[i] = (src1.ymm16s[i] >> count) as u16;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16u[i] = (src1.xmm16s[i] >> count) as u16;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRAD — Packed Shift Right Arithmetic Dwords by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 31, count is clamped to 31.
    pub(super) fn vpsrad_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count_raw = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm64u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        let count = if count_raw > 31 { 31u32 } else { count_raw as u32 };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] = (src1.ymm32s[i] >> count) as u32;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] = (src1.xmm32s[i] >> count) as u32;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLW — Packed Shift Left Logical Words by XMM count (VEX.L aware)
    /// Count is from bits [63:0] of src XMM. If count > 15, result is zero.
    pub(super) fn vpsllw_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        let count = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            unsafe { src.xmm64u[0] }
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        if instr.get_vl() >= 1 {
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 16 {
                let count = count as u32;
                unsafe {
                    for i in 0..16 {
                        result.ymm16u[i] = src1.ymm16u[i] << count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 16 {
                let count = count as u32;
                unsafe {
                    for i in 0..8 {
                        result.xmm16u[i] = src1.xmm16u[i] << count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed shift by immediate
    // ========================================================================

    /// VPSRLQ — Packed Shift Right Logical Qwords by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsrlq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count = instr.ib() as u32;
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 64 {
                unsafe {
                    for i in 0..4 {
                        result.ymm64u[i] = src.ymm64u[i] >> count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 64 {
                unsafe {
                    for i in 0..2 {
                        result.xmm64u[i] = src.xmm64u[i] >> count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLQ — Packed Shift Left Logical Qwords by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsllq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count = instr.ib() as u32;
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 64 {
                unsafe {
                    for i in 0..4 {
                        result.ymm64u[i] = src.ymm64u[i] << count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 64 {
                unsafe {
                    for i in 0..2 {
                        result.xmm64u[i] = src.xmm64u[i] << count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLW — Packed Shift Right Logical Words by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsrlw_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count = instr.ib() as u32;
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 16 {
                unsafe {
                    for i in 0..16 {
                        result.ymm16u[i] = src.ymm16u[i] >> count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 16 {
                unsafe {
                    for i in 0..8 {
                        result.xmm16u[i] = src.xmm16u[i] >> count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLW — Packed Shift Left Logical Words by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsllw_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count = instr.ib() as u32;
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            if count < 16 {
                unsafe {
                    for i in 0..16 {
                        result.ymm16u[i] = src.ymm16u[i] << count;
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            if count < 16 {
                unsafe {
                    for i in 0..8 {
                        result.xmm16u[i] = src.xmm16u[i] << count;
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRAW — Packed Shift Right Arithmetic Words by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    /// Arithmetic shift sign-extends; count clamped to 15 if > 15.
    pub(super) fn vpsraw_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count_raw = instr.ib() as u32;
        let count = if count_raw > 15 { 15 } else { count_raw };
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16u[i] = (src.ymm16s[i] >> count) as u16;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16u[i] = (src.xmm16s[i] >> count) as u16;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRAD — Packed Shift Right Arithmetic Dwords by immediate (VEX.L aware)
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    /// Arithmetic shift sign-extends; count clamped to 31 if > 31.
    pub(super) fn vpsrad_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let count_raw = instr.ib() as u32;
        let count = if count_raw > 31 { 31 } else { count_raw };
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] = (src.ymm32s[i] >> count) as u32;
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] = (src.xmm32s[i] >> count) as u32;
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSLLDQ — Packed Shift Left Double Quadword by immediate (VEX.L aware)
    /// Byte-granularity left shift of each 128-bit lane. Immediate = byte count (0-15).
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpslldq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let shift = instr.ib() as usize;
        let shift = if shift > 15 { 16 } else { shift };
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane
                for i in 0..16usize {
                    if i >= shift {
                        result.ymmubyte[i] = src.ymmubyte[i - shift];
                    }
                    // else remains 0 (zero-fill from the right)
                }
                // Upper 128-bit lane
                for i in 0..16usize {
                    if i >= shift {
                        result.ymmubyte[16 + i] = src.ymmubyte[16 + i - shift];
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16usize {
                    if i >= shift {
                        result.xmmubyte[i] = src.xmmubyte[i - shift];
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSRLDQ — Packed Shift Right Double Quadword by immediate (VEX.L aware)
    /// Byte-granularity right shift of each 128-bit lane. Immediate = byte count (0-15).
    /// Operands: dst=VEX.vvvv (src2), src=rm (dst), imm8
    pub(super) fn vpsrldq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.src2(); // VEX.vvvv
        let shift = instr.ib() as usize;
        let shift = if shift > 15 { 16 } else { shift };
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane
                for i in 0..16usize {
                    if i + shift < 16 {
                        result.ymmubyte[i] = src.ymmubyte[i + shift];
                    }
                    // else remains 0 (zero-fill from the left)
                }
                // Upper 128-bit lane
                for i in 0..16usize {
                    if i + shift < 16 {
                        result.ymmubyte[16 + i] = src.ymmubyte[16 + i + shift];
                    }
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.dst())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16usize {
                    if i + shift < 16 {
                        result.xmmubyte[i] = src.xmmubyte[i + shift];
                    }
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Packed compare
    // ========================================================================

    /// VPCMPEQB — Packed Compare Equal Bytes (VEX.L aware)
    /// dst[i] = (vvvv[i] == src[i]) ? 0xFF : 0x00
    pub(super) fn vpcmpeqb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..32 {
                    result.ymmubyte[i] =
                        if src1.ymmubyte[i] == src2.ymmubyte[i] { 0xFF } else { 0x00 };
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16 {
                    result.xmmubyte[i] =
                        if src1.xmmubyte[i] == src2.xmmubyte[i] { 0xFF } else { 0x00 };
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPEQW — Packed Compare Equal Words (VEX.L aware)
    /// dst[i] = (vvvv[i] == src[i]) ? 0xFFFF : 0x0000
    pub(super) fn vpcmpeqw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16u[i] =
                        if src1.ymm16u[i] == src2.ymm16u[i] { 0xFFFF } else { 0x0000 };
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16u[i] =
                        if src1.xmm16u[i] == src2.xmm16u[i] { 0xFFFF } else { 0x0000 };
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPEQQ — Packed Compare Equal Qwords (VEX.L aware)
    /// dst[i] = (vvvv[i] == src[i]) ? 0xFFFF_FFFF_FFFF_FFFF : 0
    pub(super) fn vpcmpeqq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    result.ymm64u[i] =
                        if src1.ymm64u[i] == src2.ymm64u[i] { 0xFFFF_FFFF_FFFF_FFFF } else { 0 };
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    result.xmm64u[i] =
                        if src1.xmm64u[i] == src2.xmm64u[i] { 0xFFFF_FFFF_FFFF_FFFF } else { 0 };
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPGTB — Packed Compare Greater Than Bytes, signed (VEX.L aware)
    /// dst[i] = ((vvvv[i] as i8) > (src[i] as i8)) ? 0xFF : 0x00
    pub(super) fn vpcmpgtb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..32 {
                    result.ymmubyte[i] =
                        if src1.ymm_sbyte[i] > src2.ymm_sbyte[i] { 0xFF } else { 0x00 };
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16 {
                    result.xmmubyte[i] =
                        if src1.xmm_sbyte[i] > src2.xmm_sbyte[i] { 0xFF } else { 0x00 };
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPGTW — Packed Compare Greater Than Words, signed (VEX.L aware)
    /// dst[i] = ((vvvv[i] as i16) > (src[i] as i16)) ? 0xFFFF : 0x0000
    pub(super) fn vpcmpgtw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16u[i] =
                        if src1.ymm16s[i] > src2.ymm16s[i] { 0xFFFF } else { 0x0000 };
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16u[i] =
                        if src1.xmm16s[i] > src2.xmm16s[i] { 0xFFFF } else { 0x0000 };
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPGTD — Packed Compare Greater Than Dwords, signed (VEX.L aware)
    /// dst[i] = ((vvvv[i] as i32) > (src[i] as i32)) ? 0xFFFFFFFF : 0
    pub(super) fn vpcmpgtd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..8 {
                    result.ymm32u[i] =
                        if src1.ymm32s[i] > src2.ymm32s[i] { 0xFFFF_FFFF } else { 0 };
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..4 {
                    result.xmm32u[i] =
                        if src1.xmm32s[i] > src2.xmm32s[i] { 0xFFFF_FFFF } else { 0 };
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPCMPGTQ — Packed Compare Greater Than Qwords, signed (VEX.L aware)
    /// dst[i] = ((vvvv[i] as i64) > (src[i] as i64)) ? 0xFFFF_FFFF_FFFF_FFFF : 0
    pub(super) fn vpcmpgtq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..4 {
                    result.ymm64u[i] =
                        if src1.ymm64s[i] > src2.ymm64s[i] { 0xFFFF_FFFF_FFFF_FFFF } else { 0 };
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..2 {
                    result.xmm64u[i] =
                        if src1.xmm64s[i] > src2.xmm64s[i] { 0xFFFF_FFFF_FFFF_FFFF } else { 0 };
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // VPMOVMSKB — Move Byte Mask to GPR
    // ========================================================================

    /// VPMOVMSKB — Extract MSB of each byte, packed into GPR (VEX.L aware)
    /// Result is a bitmask: bit i = MSB of byte i in source XMM/YMM
    pub(super) fn vpmovmskb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_gpr = instr.dst() as usize;
        if instr.get_vl() >= 1 {
            // 256-bit: 32 bytes -> 32-bit mask
            let src = self.read_ymm_reg(instr.src1());
            let mut mask: u32 = 0;
            unsafe {
                for i in 0..32 {
                    if (src.ymmubyte[i] & 0x80) != 0 {
                        mask |= 1u32 << i;
                    }
                }
            }
            self.set_gpr32(dst_gpr, mask);
        } else {
            // 128-bit: 16 bytes -> 16-bit mask (zero-extended to 32/64)
            let src = self.read_xmm_reg(instr.src1());
            let mut mask: u32 = 0;
            unsafe {
                for i in 0..16 {
                    if (src.xmmubyte[i] & 0x80) != 0 {
                        mask |= 1u32 << i;
                    }
                }
            }
            self.set_gpr32(dst_gpr, mask);
        }
        Ok(())
    }

    // ========================================================================
    // VPSHUFHW / VPSHUFLW — Shuffle high/low words within 64-bit lanes
    // ========================================================================

    /// VPSHUFHW — Shuffle High Words within 64-bit lanes (VEX.L aware)
    /// In each 128-bit lane: words 0-3 are copied unchanged, words 4-7 are shuffled
    /// by imm8[1:0], imm8[3:2], imm8[5:4], imm8[7:6]
    pub(super) fn vpshufhw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let imm = instr.ib();
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane
                // Words 0-3 copied unchanged
                for i in 0..4 {
                    result.ymm16u[i] = src.ymm16u[i];
                }
                // Words 4-7 shuffled from high half of lower lane (words 4-7)
                for i in 0..4 {
                    let sel = ((imm >> (i * 2)) & 0x3) as usize;
                    result.ymm16u[4 + i] = src.ymm16u[4 + sel];
                }
                // Upper 128-bit lane
                // Words 8-11 copied unchanged
                for i in 0..4 {
                    result.ymm16u[8 + i] = src.ymm16u[8 + i];
                }
                // Words 12-15 shuffled from high half of upper lane (words 12-15)
                for i in 0..4 {
                    let sel = ((imm >> (i * 2)) & 0x3) as usize;
                    result.ymm16u[12 + i] = src.ymm16u[12 + sel];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                // Words 0-3 copied unchanged
                for i in 0..4 {
                    result.xmm16u[i] = src.xmm16u[i];
                }
                // Words 4-7 shuffled
                for i in 0..4 {
                    let sel = ((imm >> (i * 2)) & 0x3) as usize;
                    result.xmm16u[4 + i] = src.xmm16u[4 + sel];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSHUFLW — Shuffle Low Words within 64-bit lanes (VEX.L aware)
    /// In each 128-bit lane: words 4-7 are copied unchanged, words 0-3 are shuffled
    /// by imm8[1:0], imm8[3:2], imm8[5:4], imm8[7:6]
    pub(super) fn vpshuflw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let imm = instr.ib();
        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                // Lower 128-bit lane
                // Words 0-3 shuffled from low half of lower lane (words 0-3)
                for i in 0..4 {
                    let sel = ((imm >> (i * 2)) & 0x3) as usize;
                    result.ymm16u[i] = src.ymm16u[sel];
                }
                // Words 4-7 copied unchanged
                for i in 0..4 {
                    result.ymm16u[4 + i] = src.ymm16u[4 + i];
                }
                // Upper 128-bit lane
                // Words 8-11 shuffled from low half of upper lane (words 8-11)
                for i in 0..4 {
                    let sel = ((imm >> (i * 2)) & 0x3) as usize;
                    result.ymm16u[8 + i] = src.ymm16u[8 + sel];
                }
                // Words 12-15 copied unchanged
                for i in 0..4 {
                    result.ymm16u[12 + i] = src.ymm16u[12 + i];
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                // Words 0-3 shuffled
                for i in 0..4 {
                    let sel = ((imm >> (i * 2)) & 0x3) as usize;
                    result.xmm16u[i] = src.xmm16u[sel];
                }
                // Words 4-7 copied unchanged
                for i in 0..4 {
                    result.xmm16u[4 + i] = src.xmm16u[4 + i];
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    // ========================================================================
    // Saturating packed add/sub
    // ========================================================================

    /// VPADDSB — Packed Add Signed Saturating Bytes (VEX.L aware)
    pub(super) fn vpaddsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..32 {
                    result.ymm_sbyte[i] = src1.ymm_sbyte[i].saturating_add(src2.ymm_sbyte[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16 {
                    result.xmm_sbyte[i] = src1.xmm_sbyte[i].saturating_add(src2.xmm_sbyte[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDSW — Packed Add Signed Saturating Words (VEX.L aware)
    pub(super) fn vpaddsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16s[i] = src1.ymm16s[i].saturating_add(src2.ymm16s[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16s[i] = src1.xmm16s[i].saturating_add(src2.xmm16s[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBSB — Packed Subtract Signed Saturating Bytes (VEX.L aware)
    pub(super) fn vpsubsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..32 {
                    result.ymm_sbyte[i] = src1.ymm_sbyte[i].saturating_sub(src2.ymm_sbyte[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16 {
                    result.xmm_sbyte[i] = src1.xmm_sbyte[i].saturating_sub(src2.xmm_sbyte[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBSW — Packed Subtract Signed Saturating Words (VEX.L aware)
    pub(super) fn vpsubsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16s[i] = src1.ymm16s[i].saturating_sub(src2.ymm16s[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16s[i] = src1.xmm16s[i].saturating_sub(src2.xmm16s[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDUSB — Packed Add Unsigned Saturating Bytes (VEX.L aware)
    pub(super) fn vpaddusb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..32 {
                    result.ymmubyte[i] = src1.ymmubyte[i].saturating_add(src2.ymmubyte[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16 {
                    result.xmmubyte[i] = src1.xmmubyte[i].saturating_add(src2.xmmubyte[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPADDUSW — Packed Add Unsigned Saturating Words (VEX.L aware)
    pub(super) fn vpaddusw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16u[i] = src1.ymm16u[i].saturating_add(src2.ymm16u[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16u[i] = src1.xmm16u[i].saturating_add(src2.xmm16u[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBUSB — Packed Subtract Unsigned Saturating Bytes (VEX.L aware)
    pub(super) fn vpsubusb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..32 {
                    result.ymmubyte[i] = src1.ymmubyte[i].saturating_sub(src2.ymmubyte[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..16 {
                    result.xmmubyte[i] = src1.xmmubyte[i].saturating_sub(src2.xmmubyte[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }

    /// VPSUBUSW — Packed Subtract Unsigned Saturating Words (VEX.L aware)
    pub(super) fn vpsubusw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let src1_idx = instr.src2(); // VEX.vvvv
        if instr.get_vl() >= 1 {
            let src2 = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_ymmword(seg, eaddr)?
            };
            let src1 = self.read_ymm_reg(src1_idx);
            let mut result = BxPackedYmmRegister { ymm64u: [0; 4] };
            unsafe {
                for i in 0..16 {
                    result.ymm16u[i] = src1.ymm16u[i].saturating_sub(src2.ymm16u[i]);
                }
            }
            self.write_ymm_reg(dst_idx, result);
        } else {
            let src2 = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1())
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_xmmword(seg, eaddr)?
            };
            let src1 = self.read_xmm_reg(src1_idx);
            let mut result = BxPackedXmmRegister { xmm64u: [0; 2] };
            unsafe {
                for i in 0..8 {
                    result.xmm16u[i] = src1.xmm16u[i].saturating_sub(src2.xmm16u[i]);
                }
            }
            self.write_xmm_reg(dst_idx, result);
        }
        Ok(())
    }
}
