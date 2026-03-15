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
    // VEX.L-aware dispatch wrappers
    // These check VEX.L and dispatch to 128-bit (SSE) or 256-bit (AVX) handlers
    // ========================================================================

    /// VMOVDQU load — VEX.L=0: XMM <- M128, VEX.L=1: YMM <- M256
    pub(super) fn vmovdqu_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        if instr.get_vl() >= 1 {
            let val = self.v_read_ymmword(seg, eaddr)?;
            self.write_ymm_reg(instr.dst(), val);
        } else {
            let val = self.v_read_xmmword(seg, eaddr)?;
            self.write_xmm_reg(instr.dst(), val);
        }
        Ok(())
    }

    /// VMOVDQU store — VEX.L=0: M128 <- XMM, VEX.L=1: M256 <- YMM
    pub(super) fn vmovdqu_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        if instr.get_vl() >= 1 {
            let val = self.read_ymm_reg(instr.src1());
            self.v_write_ymmword(seg, eaddr, &val)?;
        } else {
            let val = self.read_xmm_reg(instr.src1());
            self.v_write_xmmword(seg, eaddr, &val)?;
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
    pub(super) fn vmovdqa_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        if instr.get_vl() >= 1 {
            // TODO: 32-byte alignment check for YMM — VMOVDQA requires 32-byte aligned
            // access for 256-bit operands. Should raise #GP(0) if eaddr % 32 != 0.
            // v_read_ymmword_aligned does not exist yet; using unaligned read as fallback.
            let val = self.v_read_ymmword(seg, eaddr)?;
            self.write_ymm_reg(instr.dst(), val);
        } else {
            let val = self.v_read_xmmword_aligned(seg, eaddr)?;
            self.write_xmm_reg(instr.dst(), val);
        }
        Ok(())
    }

    /// VMOVDQA store — VEX.L=0: M128 <- XMM, VEX.L=1: M256 <- YMM (aligned)
    pub(super) fn vmovdqa_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        if instr.get_vl() >= 1 {
            // TODO: 32-byte alignment check for YMM — VMOVDQA requires 32-byte aligned
            // access for 256-bit operands. Should raise #GP(0) if eaddr % 32 != 0.
            // v_write_ymmword_aligned does not exist yet; using unaligned write as fallback.
            let val = self.read_ymm_reg(instr.src1());
            self.v_write_ymmword(seg, eaddr, &val)?;
        } else {
            let val = self.read_xmm_reg(instr.src1());
            self.v_write_xmmword_aligned(seg, eaddr, &val)?;
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
}
