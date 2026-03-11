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
            // TODO: 32-byte alignment check for YMM
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
    /// dst[i] = rotate_right(src[i], imm8)
    pub(super) fn vprord(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst(); // VEX.vvvv for EVEX group opcodes
        let count = (instr.ib() & 31) as u32; // rotate count mod 32

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
                for i in 0..8 {
                    result.ymm32u[i] = src.ymm32u[i].rotate_right(count);
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
                    result.xmm32u[i] = src.xmm32u[i].rotate_right(count);
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
    pub(super) fn vpslld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst(); // VEX.vvvv for group opcodes
        let count = instr.ib() as u32;

        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
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
                self.read_xmm_reg(instr.src1())
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
    pub(super) fn vpsrld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let dst_idx = instr.dst();
        let count = instr.ib() as u32;

        if instr.get_vl() >= 1 {
            let src = if instr.mod_c0() {
                self.read_ymm_reg(instr.src1())
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
                self.read_xmm_reg(instr.src1())
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
}
