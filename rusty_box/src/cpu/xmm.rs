#![allow(dead_code)]
//! XMM/YMM/ZMM register types and MXCSR for SSE/AVX/AVX-512
//!
//! Based on Bochs cpu/simd_int.h and cpu/xmm.h.
//! Safe structs backed by byte arrays with inline accessor methods.
//! On x86 targets LLVM optimises from_le_bytes/to_le_bytes to identical code as union access.

use crate::cpu::{BxCpuC, BxCpuIdTrait};

pub(super) const MXCSR_RESET: u32 = Mxcsr::RESET.bits();
pub(super) const MXCSR_MASK: u32 = 0x0000_FFBF; // Valid bits mask (no bit 6 DAZ on older CPUs)

// ============================================================================
// XMM register (128-bit) — matches Bochs bx_xmm_reg_t
// ============================================================================

// Helper: read N bytes from a byte array at offset, interpret as little-endian value.
// These are generic building blocks used by the register accessor macros below.

/// Generate typed accessor methods for a packed-register struct backed by `self.bytes`.
/// Each invocation generates a getter `$name(i) -> $ty` and setter `set_$name(i, v: $ty)`
/// for a specific element width.
macro_rules! packed_reg_accessors {
    // Unsigned integer accessor
    (uint $name:ident, $setter:ident, $ty:ty, $width:expr) => {
        #[inline(always)]
        pub fn $name(&self, i: usize) -> $ty {
            let s = i * $width;
            <$ty>::from_le_bytes(self.bytes[s..s + $width].try_into().unwrap())
        }
        #[inline(always)]
        pub fn $setter(&mut self, i: usize, v: $ty) {
            let s = i * $width;
            self.bytes[s..s + $width].copy_from_slice(&v.to_le_bytes());
        }
    };
    // Signed integer accessor (reinterprets same bytes)
    (sint $name:ident, $setter:ident, $uname:ident, $usetter:ident, $sty:ty, $uty:ty) => {
        #[inline(always)]
        pub fn $name(&self, i: usize) -> $sty { self.$uname(i) as $sty }
        #[inline(always)]
        pub fn $setter(&mut self, i: usize, v: $sty) { self.$usetter(i, v as $uty) }
    };
    // Float accessor
    (float $name:ident, $setter:ident, $fty:ty, $width:expr) => {
        #[inline(always)]
        pub fn $name(&self, i: usize) -> $fty {
            let s = i * $width;
            <$fty>::from_le_bytes(self.bytes[s..s + $width].try_into().unwrap())
        }
        #[inline(always)]
        pub fn $setter(&mut self, i: usize, v: $fty) {
            let s = i * $width;
            self.bytes[s..s + $width].copy_from_slice(&v.to_le_bytes());
        }
    };
    // Single-byte accessor (no endianness concern)
    (byte $name:ident, $setter:ident, $sname:ident, $ssetter:ident) => {
        #[inline(always)]
        pub fn $name(&self, i: usize) -> u8 { self.bytes[i] }
        #[inline(always)]
        pub fn $setter(&mut self, i: usize, v: u8) { self.bytes[i] = v; }
        #[inline(always)]
        pub fn $sname(&self, i: usize) -> i8 { self.bytes[i] as i8 }
        #[inline(always)]
        pub fn $ssetter(&mut self, i: usize, v: i8) { self.bytes[i] = v as u8; }
    };
}

// ============================================================================
// XMM register (128-bit) — matches Bochs bx_xmm_reg_t
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
#[derive(Default)]
pub struct BxPackedXmmRegister {
    pub(crate) bytes: [u8; 16],
}


impl BxPackedXmmRegister {
    packed_reg_accessors!(uint xmm64u, set_xmm64u, u64, 8);
    packed_reg_accessors!(uint xmm32u, set_xmm32u, u32, 4);
    packed_reg_accessors!(uint xmm16u, set_xmm16u, u16, 2);
    packed_reg_accessors!(byte xmmubyte, set_xmmubyte, xmm_sbyte, set_xmm_sbyte);
    packed_reg_accessors!(sint xmm64s, set_xmm64s, xmm64u, set_xmm64u, i64, u64);
    packed_reg_accessors!(sint xmm32s, set_xmm32s, xmm32u, set_xmm32u, i32, u32);
    packed_reg_accessors!(sint xmm16s, set_xmm16s, xmm16u, set_xmm16u, i16, u16);
    packed_reg_accessors!(float xmm32f, set_xmm32f, f32, 4);
    packed_reg_accessors!(float xmm64f, set_xmm64f, f64, 8);

    /// Raw byte slice (for bulk copy / memcmp).
    #[inline(always)]
    pub fn raw(&self) -> &[u8; 16] { &self.bytes }
    #[inline(always)]
    pub fn raw_mut(&mut self) -> &mut [u8; 16] { &mut self.bytes }
}

impl core::fmt::Debug for BxPackedXmmRegister {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (hi, lo) = (self.xmm64u(1), self.xmm64u(0));
        write!(f, "XMM({:016x}:{:016x})", hi, lo)
    }
}

pub type BxXmmReg = BxPackedXmmRegister;

// ============================================================================
// YMM register (256-bit) — matches Bochs bx_ymm_reg_t
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
#[derive(Default)]
pub struct BxPackedYmmRegister {
    pub(crate) bytes: [u8; 32],
}


impl BxPackedYmmRegister {
    packed_reg_accessors!(uint ymm64u, set_ymm64u, u64, 8);
    packed_reg_accessors!(uint ymm32u, set_ymm32u, u32, 4);
    packed_reg_accessors!(uint ymm16u, set_ymm16u, u16, 2);
    packed_reg_accessors!(byte ymmubyte, set_ymmubyte, ymm_sbyte, set_ymm_sbyte);
    packed_reg_accessors!(sint ymm64s, set_ymm64s, ymm64u, set_ymm64u, i64, u64);
    packed_reg_accessors!(sint ymm32s, set_ymm32s, ymm32u, set_ymm32u, i32, u32);
    packed_reg_accessors!(sint ymm16s, set_ymm16s, ymm16u, set_ymm16u, i16, u16);
    packed_reg_accessors!(float ymm32f, set_ymm32f, f32, 4);
    packed_reg_accessors!(float ymm64f, set_ymm64f, f64, 8);

    /// View as XMM halves.
    #[inline(always)]
    pub fn ymm128(&self, i: usize) -> BxPackedXmmRegister {
        let s = i * 16;
        let mut r = BxPackedXmmRegister::default();
        r.bytes.copy_from_slice(&self.bytes[s..s + 16]);
        r
    }
    #[inline(always)]
    pub fn set_ymm128(&mut self, i: usize, v: BxPackedXmmRegister) {
        let s = i * 16;
        self.bytes[s..s + 16].copy_from_slice(&v.bytes);
    }

    #[inline(always)]
    pub fn raw(&self) -> &[u8; 32] { &self.bytes }
    #[inline(always)]
    pub fn raw_mut(&mut self) -> &mut [u8; 32] { &mut self.bytes }
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

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct BxPackedZmmRegister {
    pub(crate) bytes: [u8; 64],
}

impl Default for BxPackedZmmRegister {
    fn default() -> Self { Self { bytes: [0; 64] } }
}

impl BxPackedZmmRegister {
    packed_reg_accessors!(uint zmm64u, set_zmm64u, u64, 8);
    packed_reg_accessors!(uint zmm32u, set_zmm32u, u32, 4);
    packed_reg_accessors!(uint zmm16u, set_zmm16u, u16, 2);
    packed_reg_accessors!(byte zmmubyte, set_zmmubyte, zmm_sbyte, set_zmm_sbyte);
    packed_reg_accessors!(sint zmm64s, set_zmm64s, zmm64u, set_zmm64u, i64, u64);
    packed_reg_accessors!(sint zmm32s, set_zmm32s, zmm32u, set_zmm32u, i32, u32);
    packed_reg_accessors!(sint zmm16s, set_zmm16s, zmm16u, set_zmm16u, i16, u16);
    packed_reg_accessors!(float zmm32f, set_zmm32f, f32, 4);
    packed_reg_accessors!(float zmm64f, set_zmm64f, f64, 8);

    /// View as XMM quarters.
    #[inline(always)]
    pub fn zmm128(&self, i: usize) -> BxPackedXmmRegister {
        let s = i * 16;
        let mut r = BxPackedXmmRegister::default();
        r.bytes.copy_from_slice(&self.bytes[s..s + 16]);
        r
    }
    #[inline(always)]
    pub fn set_zmm128(&mut self, i: usize, v: BxPackedXmmRegister) {
        let s = i * 16;
        self.bytes[s..s + 16].copy_from_slice(&v.bytes);
    }

    /// View as YMM halves.
    #[inline(always)]
    pub fn zmm256(&self, i: usize) -> BxPackedYmmRegister {
        let s = i * 32;
        let mut r = BxPackedYmmRegister::default();
        r.bytes.copy_from_slice(&self.bytes[s..s + 32]);
        r
    }
    #[inline(always)]
    pub fn set_zmm256(&mut self, i: usize, v: BxPackedYmmRegister) {
        let s = i * 32;
        self.bytes[s..s + 32].copy_from_slice(&v.bytes);
    }

    pub(super) fn clear(&mut self) {
        *self = Default::default();
    }

    #[inline(always)]
    pub fn raw(&self) -> &[u8; 64] { &self.bytes }
    #[inline(always)]
    pub fn raw_mut(&mut self) -> &mut [u8; 64] { &mut self.bytes }
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

bitflags::bitflags! {
    /// MXCSR — SSE/AVX control and status register (matching Bochs)
    ///
    /// Lower 6 bits are sticky exception flags (set by hardware on exception).
    /// Bits 7-12 are the corresponding exception masks (1 = masked / suppressed).
    /// Bit 6 = DAZ (Denormals Are Zeros), bit 15 = FZ (Flush to Zero),
    /// bits 13-14 = rounding control.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct Mxcsr: u32 {
        /// Invalid-operation exception flag
        const IE  = 1 << 0;
        /// Denormal-operand exception flag
        const DE  = 1 << 1;
        /// Zero-divide exception flag
        const ZE  = 1 << 2;
        /// Overflow exception flag
        const OE  = 1 << 3;
        /// Underflow exception flag
        const UE  = 1 << 4;
        /// Precision (inexact) exception flag
        const PE  = 1 << 5;
        /// Denormals-Are-Zeros mode
        const DAZ = 1 << 6;
        /// Invalid-operation exception mask
        const IM  = 1 << 7;
        /// Denormal-operand exception mask
        const DM  = 1 << 8;
        /// Zero-divide exception mask
        const ZM  = 1 << 9;
        /// Overflow exception mask
        const OM  = 1 << 10;
        /// Underflow exception mask
        const UM  = 1 << 11;
        /// Precision exception mask
        const PM  = 1 << 12;
        /// Rounding-control bit 0 (bits 13-14 together)
        const RC0 = 1 << 13;
        /// Rounding-control bit 1
        const RC1 = 1 << 14;
        /// Flush-to-Zero mode
        const FZ  = 1 << 15;
    }
}

impl Mxcsr {
    /// Both rounding-control bits (mask = 0x6000)
    pub const RC_MASK: Mxcsr = Self::RC0.union(Self::RC1);

    /// All exception mask bits (IM|DM|ZM|OM|UM|PM)
    pub const ALL_MASKS: Mxcsr = Self::IM
        .union(Self::DM)
        .union(Self::ZM)
        .union(Self::OM)
        .union(Self::UM)
        .union(Self::PM);

    /// Reset value: all exceptions masked, round-to-nearest (= 0x1F80)
    pub const RESET: Mxcsr = Self::ALL_MASKS;

    /// Get rounding control mode (0=nearest, 1=down, 2=up, 3=truncate)
    #[inline]
    pub const fn rounding_mode(self) -> u8 {
        ((self.bits() >> 13) & 3) as u8
    }
}

// ---- Backward-compat wrapper (existing code uses BxMxcsr { mxcsr: u32 }) ----
#[derive(Debug, Default, Clone, Copy)]
pub struct BxMxcsr {
    pub(crate) mxcsr: u32,
}

impl BxMxcsr {
    /// Get the typed Mxcsr bitflags view
    #[inline]
    pub fn flags(&self) -> Mxcsr {
        Mxcsr::from_bits_retain(self.mxcsr)
    }

    /// Get rounding control mode (0=nearest, 1=down, 2=up, 3=truncate)
    #[inline]
    pub fn rounding_mode(&self) -> u8 {
        self.flags().rounding_mode()
    }

    /// Check if Flush-to-Zero is enabled
    #[inline]
    pub fn flush_to_zero(&self) -> bool {
        self.flags().contains(Mxcsr::FZ)
    }

    /// Check if Denormals-Are-Zeros is enabled
    #[inline]
    pub fn daz(&self) -> bool {
        self.flags().contains(Mxcsr::DAZ)
    }

    /// Check if an exception is masked
    #[inline]
    pub fn is_masked(&self, exception_bit: u32) -> bool {
        // Mask bits are 7 positions above the exception flag bits
        (self.mxcsr & (exception_bit << 7)) != 0
    }
}

// ---- Backward-compat constants (prefer Mxcsr::<NAME> in new code) ----
pub(super) const MXCSR_IE: u32 = Mxcsr::IE.bits();
pub(super) const MXCSR_DE: u32 = Mxcsr::DE.bits();
pub(super) const MXCSR_ZE: u32 = Mxcsr::ZE.bits();
pub(super) const MXCSR_OE: u32 = Mxcsr::OE.bits();
pub(super) const MXCSR_UE: u32 = Mxcsr::UE.bits();
pub(super) const MXCSR_PE: u32 = Mxcsr::PE.bits();
pub(super) const MXCSR_DAZ: u32 = Mxcsr::DAZ.bits();
pub(super) const MXCSR_IM: u32 = Mxcsr::IM.bits();
pub(super) const MXCSR_DM: u32 = Mxcsr::DM.bits();
pub(super) const MXCSR_ZM: u32 = Mxcsr::ZM.bits();
pub(super) const MXCSR_OM: u32 = Mxcsr::OM.bits();
pub(super) const MXCSR_UM: u32 = Mxcsr::UM.bits();
pub(super) const MXCSR_PM: u32 = Mxcsr::PM.bits();
pub(super) const MXCSR_RC: u32 = Mxcsr::RC_MASK.bits();
pub(super) const MXCSR_FZ: u32 = Mxcsr::FZ.bits();

// ============================================================================
// CPU helper methods for XMM register access
// ============================================================================

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Clear a vector register (all 512 bits to zero)
    #[allow(non_snake_case)]
    pub(super) fn BX_CLEAR_AVX_REG(&mut self, index: usize) {
        self.vmm[index].clear()
    }

    /// Read XMM register (lower 128 bits of vmm[index])
    #[inline]
    pub(super) fn read_xmm_reg(&self, index: u8) -> BxPackedXmmRegister {
        self.vmm[index as usize].zmm128(0)
    }

    /// Write XMM register (writes lower 128 bits, clears upper bits for VEX-encoded SSE)
    #[inline]
    pub(super) fn write_xmm_reg(&mut self, index: u8, val: BxPackedXmmRegister) {
        let i = index as usize;
        self.vmm[i].clear();
        self.vmm[i].set_zmm128(0, val);
    }

    /// Write XMM register preserving upper bits (for legacy SSE without VEX)
    #[inline]
    pub(super) fn write_xmm_reg_lo128(&mut self, index: u8, val: BxPackedXmmRegister) {
        self.vmm[index as usize].set_zmm128(0, val);
    }

    /// Read low qword of XMM register
    #[inline]
    pub(super) fn xmm_lo_qword(&self, index: u8) -> u64 {
        self.vmm[index as usize].zmm64u(0)
    }

    /// Read high qword of XMM register
    #[inline]
    pub(super) fn xmm_hi_qword(&self, index: u8) -> u64 {
        self.vmm[index as usize].zmm64u(1)
    }

    /// Write low qword of XMM register (preserves high qword)
    #[inline]
    pub(super) fn write_xmm_lo_qword(&mut self, index: u8, val: u64) {
        self.vmm[index as usize].set_zmm64u(0, val);
    }

    /// Write high qword of XMM register (preserves low qword)
    #[inline]
    pub(super) fn write_xmm_hi_qword(&mut self, index: u8, val: u64) {
        self.vmm[index as usize].set_zmm64u(1, val);
    }

    /// Read low dword of XMM register
    #[inline]
    pub(super) fn xmm_lo_dword(&self, index: u8) -> u32 {
        self.vmm[index as usize].zmm32u(0)
    }

    /// Write low dword of XMM register (preserves rest)
    #[inline]
    pub(super) fn write_xmm_lo_dword(&mut self, index: u8, val: u32) {
        self.vmm[index as usize].set_zmm32u(0, val);
    }

    /// Read YMM register (lower 256 bits of vmm[index])
    #[inline]
    pub(super) fn read_ymm_reg(&self, index: u8) -> BxPackedYmmRegister {
        self.vmm[index as usize].zmm256(0)
    }

    /// Write YMM register (writes lower 256 bits, clears upper 256 bits)
    #[inline]
    pub(super) fn write_ymm_reg(&mut self, index: u8, val: BxPackedYmmRegister) {
        let i = index as usize;
        self.vmm[i].clear();
        self.vmm[i].set_zmm256(0, val);
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
