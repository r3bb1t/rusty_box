//! XMM/YMM/ZMM register types and MXCSR for SSE/AVX/AVX-512
//!
//! Based on Bochs cpu/simd_int.h and cpu/xmm.h
//! Uses unions for free reinterpretation matching Bochs semantics.

use crate::cpu::{BxCpuC, BxCpuIdTrait};

pub(super) const MXCSR_RESET: u32 = 0x0000_1F80; // All exceptions masked, round-nearest
pub(super) const MXCSR_MASK: u32 = 0x0000_FFBF; // Valid bits mask (no bit 6 DAZ on older CPUs)

// ============================================================================
// XMM register (128-bit) — matches Bochs bx_xmm_reg_t
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy)]
pub union BxPackedXmmRegister {
    pub xmm_sbyte: [i8; 16],
    pub xmm16s: [i16; 8],
    pub xmm32s: [i32; 4],
    pub xmm64s: [i64; 2],
    pub xmmubyte: [u8; 16],
    pub xmm16u: [u16; 8],
    pub xmm32u: [u32; 4],
    pub xmm64u: [u64; 2],
    pub xmm32f: [f32; 4],
    pub xmm64f: [f64; 2],
    // Raw bytes for bulk copy
    pub raw: [u8; 16],
}

impl Default for BxPackedXmmRegister {
    fn default() -> Self {
        Self { xmm64u: [0, 0] }
    }
}

impl core::fmt::Debug for BxPackedXmmRegister {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (hi, lo) = unsafe { (self.xmm64u[1], self.xmm64u[0]) };
        write!(f, "XMM({:016x}:{:016x})", hi, lo)
    }
}

pub type BxXmmReg = BxPackedXmmRegister;

// ============================================================================
// YMM register (256-bit) — matches Bochs bx_ymm_reg_t
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy)]
pub union BxPackedYmmRegister {
    pub ymm_sbyte: [i8; 32],
    pub ymm16s: [i16; 16],
    pub ymm32s: [i32; 8],
    pub ymm64s: [i64; 4],
    pub ymmubyte: [u8; 32],
    pub ymm16u: [u16; 16],
    pub ymm32u: [u32; 8],
    pub ymm64u: [u64; 4],
    pub ymm32f: [f32; 8],
    pub ymm64f: [f64; 4],
    pub ymm128: [BxPackedXmmRegister; 2],
    pub raw: [u8; 32],
}

impl Default for BxPackedYmmRegister {
    fn default() -> Self {
        Self {
            ymm64u: [0, 0, 0, 0],
        }
    }
}

impl core::fmt::Debug for BxPackedYmmRegister {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "YMM(...)")
    }
}

pub type BxYmmReg = BxPackedYmmRegister;

// ============================================================================
// ZMM register (512-bit) — matches Bochs bx_zmm_reg_t
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy)]
pub union BxPackedZmmRegister {
    pub zmm_sbyte: [i8; 64],
    pub zmm16s: [i16; 32],
    pub zmm32s: [i32; 16],
    pub zmm64s: [i64; 8],
    pub zmmubyte: [u8; 64],
    pub zmm16u: [u16; 32],
    pub zmm32u: [u32; 16],
    pub zmm64u: [u64; 8],
    pub zmm32f: [f32; 16],
    pub zmm64f: [f64; 8],
    pub zmm128: [BxPackedXmmRegister; 4],
    pub zmm256: [BxPackedYmmRegister; 2],
    pub raw: [u8; 64],
}

impl BxPackedZmmRegister {
    pub(super) fn clear(&mut self) {
        *self = Default::default();
    }
}

impl Default for BxPackedZmmRegister {
    fn default() -> Self {
        Self {
            zmm64u: [0, 0, 0, 0, 0, 0, 0, 0],
        }
    }
}

impl core::fmt::Debug for BxPackedZmmRegister {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ZMM(...)")
    }
}

pub type BxZmmReg = BxPackedZmmRegister;
pub type BxPackedAvxRegister = BxPackedZmmRegister;

// ============================================================================
// MXCSR — SSE control/status register
// ============================================================================

/// MXCSR bit definitions (matching Bochs)
pub(super) const MXCSR_IE: u32 = 1 << 0; // Invalid operation exception
pub(super) const MXCSR_DE: u32 = 1 << 1; // Denormal exception
pub(super) const MXCSR_ZE: u32 = 1 << 2; // Zero-divide exception
pub(super) const MXCSR_OE: u32 = 1 << 3; // Overflow exception
pub(super) const MXCSR_UE: u32 = 1 << 4; // Underflow exception
pub(super) const MXCSR_PE: u32 = 1 << 5; // Precision exception
pub(super) const MXCSR_DAZ: u32 = 1 << 6; // Denormals Are Zeros
pub(super) const MXCSR_IM: u32 = 1 << 7; // Invalid operation mask
pub(super) const MXCSR_DM: u32 = 1 << 8; // Denormal mask
pub(super) const MXCSR_ZM: u32 = 1 << 9; // Zero-divide mask
pub(super) const MXCSR_OM: u32 = 1 << 10; // Overflow mask
pub(super) const MXCSR_UM: u32 = 1 << 11; // Underflow mask
pub(super) const MXCSR_PM: u32 = 1 << 12; // Precision mask
pub(super) const MXCSR_RC: u32 = 0x6000; // Rounding control (bits 13-14)
pub(super) const MXCSR_FZ: u32 = 1 << 15; // Flush to Zero

#[derive(Debug, Default, Clone, Copy)]
pub struct BxMxcsr {
    pub(crate) mxcsr: u32,
}

impl BxMxcsr {
    /// Get rounding control mode (0=nearest, 1=down, 2=up, 3=truncate)
    #[inline]
    pub fn rounding_mode(&self) -> u8 {
        ((self.mxcsr >> 13) & 3) as u8
    }

    /// Check if Flush-to-Zero is enabled
    #[inline]
    pub fn flush_to_zero(&self) -> bool {
        (self.mxcsr & MXCSR_FZ) != 0
    }

    /// Check if Denormals-Are-Zeros is enabled
    #[inline]
    pub fn daz(&self) -> bool {
        (self.mxcsr & MXCSR_DAZ) != 0
    }

    /// Check if an exception is masked
    #[inline]
    pub fn is_masked(&self, exception_bit: u32) -> bool {
        // Mask bits are 6 positions above the exception flag bits
        (self.mxcsr & (exception_bit << 7)) != 0
    }
}

// ============================================================================
// CPU helper methods for XMM register access
// ============================================================================

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// Clear a vector register (all 512 bits to zero)
    #[allow(non_snake_case)]
    pub(super) fn BX_CLEAR_AVX_REG(&mut self, index: usize) {
        self.vmm[index].clear()
    }

    /// Read XMM register (lower 128 bits of vmm[index])
    #[inline]
    pub(super) fn read_xmm_reg(&self, index: u8) -> BxPackedXmmRegister {
        unsafe { self.vmm[index as usize].zmm128[0] }
    }

    /// Write XMM register (writes lower 128 bits, clears upper bits for VEX-encoded SSE)
    #[inline]
    pub(super) fn write_xmm_reg(&mut self, index: u8, val: BxPackedXmmRegister) {
        let i = index as usize;
        self.vmm[i].clear();
        unsafe {
            self.vmm[i].zmm128[0] = val;
        }
    }

    /// Write XMM register preserving upper bits (for legacy SSE without VEX)
    #[inline]
    pub(super) fn write_xmm_reg_lo128(&mut self, index: u8, val: BxPackedXmmRegister) {
        unsafe {
            self.vmm[index as usize].zmm128[0] = val;
        }
    }

    /// Read low qword of XMM register
    #[inline]
    pub(super) fn xmm_lo_qword(&self, index: u8) -> u64 {
        unsafe { self.vmm[index as usize].zmm64u[0] }
    }

    /// Read high qword of XMM register
    #[inline]
    pub(super) fn xmm_hi_qword(&self, index: u8) -> u64 {
        unsafe { self.vmm[index as usize].zmm64u[1] }
    }

    /// Write low qword of XMM register (preserves high qword)
    #[inline]
    pub(super) fn write_xmm_lo_qword(&mut self, index: u8, val: u64) {
        unsafe {
            self.vmm[index as usize].zmm64u[0] = val;
        }
    }

    /// Write high qword of XMM register (preserves low qword)
    #[inline]
    pub(super) fn write_xmm_hi_qword(&mut self, index: u8, val: u64) {
        unsafe {
            self.vmm[index as usize].zmm64u[1] = val;
        }
    }

    /// Read low dword of XMM register
    #[inline]
    pub(super) fn xmm_lo_dword(&self, index: u8) -> u32 {
        unsafe { self.vmm[index as usize].zmm32u[0] }
    }

    /// Write low dword of XMM register (preserves rest)
    #[inline]
    pub(super) fn write_xmm_lo_dword(&mut self, index: u8, val: u32) {
        unsafe {
            self.vmm[index as usize].zmm32u[0] = val;
        }
    }

    /// Prepare for SSE instruction — check CR0.EM, CR4.OSFXSR, CR0.TS
    /// Returns Ok(()) if SSE is available, or raises #UD/#NM exception.
    /// Bochs: BX_CPU_C::prepareSSE() / bx_no_sse checks
    #[inline]
    pub(super) fn prepare_sse(&mut self) -> super::Result<()> {
        if self.cr0.em() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if !self.cr4.osfxsr() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if self.cr0.ts() {
            return self.exception(super::cpu::Exception::Nm, 0);
        }
        Ok(())
    }
}
