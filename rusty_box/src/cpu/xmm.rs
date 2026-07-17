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
        pub fn $name(&self, i: usize) -> $sty {
            self.$uname(i) as $sty
        }
        #[inline(always)]
        pub fn $setter(&mut self, i: usize, v: $sty) {
            self.$usetter(i, v as $uty)
        }
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
        pub fn $name(&self, i: usize) -> u8 {
            self.bytes[i]
        }
        #[inline(always)]
        pub fn $setter(&mut self, i: usize, v: u8) {
            self.bytes[i] = v;
        }
        #[inline(always)]
        pub fn $sname(&self, i: usize) -> i8 {
            self.bytes[i] as i8
        }
        #[inline(always)]
        pub fn $ssetter(&mut self, i: usize, v: i8) {
            self.bytes[i] = v as u8;
        }
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
    pub fn raw(&self) -> &[u8; 16] {
        &self.bytes
    }
    #[inline(always)]
    pub fn raw_mut(&mut self) -> &mut [u8; 16] {
        &mut self.bytes
    }
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
    pub fn raw(&self) -> &[u8; 32] {
        &self.bytes
    }
    #[inline(always)]
    pub fn raw_mut(&mut self) -> &mut [u8; 32] {
        &mut self.bytes
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

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct BxPackedZmmRegister {
    pub(crate) bytes: [u8; 64],
}

impl Default for BxPackedZmmRegister {
    fn default() -> Self {
        Self { bytes: [0; 64] }
    }
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
    pub fn raw(&self) -> &[u8; 64] {
        &self.bytes
    }
    #[inline(always)]
    pub fn raw_mut(&mut self) -> &mut [u8; 64] {
        &mut self.bytes
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

    /// The 6 exception-mask bits (IM..PM) shifted down to bit 0, matching the
    /// SoftFloat `softfloat_exceptionMasks` layout (bit0=IE .. bit5=PE).
    /// Bochs `bx_mxcsr_t::get_exceptions_masks()` (xmm.h).
    #[inline]
    pub fn exceptions_masks(&self) -> i32 {
        ((self.mxcsr & Mxcsr::ALL_MASKS.bits()) >> 7) as i32
    }

    /// OR the low-6 exception status bits (IE..PE) into MXCSR.
    /// Bochs `bx_mxcsr_t::set_exceptions()` (xmm.h).
    #[inline]
    pub fn set_exceptions(&mut self, flags: i32) {
        self.mxcsr |= (flags as u32) & MXCSR_EXCEPTIONS;
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
/// All 6 sticky exception status flags (IE|DE|ZE|OE|UE|PE = 0x3F).
pub(super) const MXCSR_EXCEPTIONS: u32 = Mxcsr::IE
    .union(Mxcsr::DE)
    .union(Mxcsr::ZE)
    .union(Mxcsr::OE)
    .union(Mxcsr::UE)
    .union(Mxcsr::PE)
    .bits();

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
    /// Prepare for AVX instruction — check protected mode, CR4.OSXSAVE,
    /// XCR0.SSE+YMM, then CR0.TS.
    /// Returns Ok(()) if AVX is available, or raises #UD/#NM exception.
    /// Bochs: BX_CPU_C::BxNoAVX().
    #[inline]
    pub(super) fn prepare_avx(&mut self) -> super::Result<()> {
        if !self.protected_mode() || !self.cr4.osxsave() || (self.xcr0.get32() & 0x6) != 0x6 {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        if self.cr0.ts() {
            return self.exception(super::cpu::Exception::Nm, 0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cpu::{
            core_i7_skylake::Corei7SkylakeX,
            cpu::Exception,
            CpuSetupMode, X86Reg,
        },
        emulator::{Emulator, EmulatorConfig},
    };


    const CODE_BASE: u64 = 0x20_0000;
    const XSETBV_BASE: u64 = CODE_BASE + 0x100;
    const IDT_BASE: u64 = 0x28_0000;
    const UD_HANDLER: u64 = 0x29_0000;
    const NM_HANDLER: u64 = 0x29_0010;
    const STACK_TOP: u64 = 0x30_0000;


    fn avx_emulator() -> alloc::boxed::Box<Emulator<'static, Corei7SkylakeX>> {
        Emulator::<Corei7SkylakeX>::new_with_mode(
            EmulatorConfig::default(),
            CpuSetupMode::FlatLong64,
        )
        .unwrap()
    }

    fn install_exception_gate(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        vector: u8,
        handler: u64,
    ) {
        let mut gate = [0u8; 16];
        gate[0..2].copy_from_slice(&(handler as u16).to_le_bytes());
        gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
        gate[5] = 0x8e;
        gate[6..8].copy_from_slice(&((handler >> 16) as u16).to_le_bytes());
        gate[8..12].copy_from_slice(&((handler >> 32) as u32).to_le_bytes());
        emu.mem_write(IDT_BASE + u64::from(vector) * 16, &gate)
            .unwrap();
        emu.mem_write(handler, &[0xf4]).unwrap();
    }

    fn install_avx_exception_handlers(emu: &mut Emulator<'static, Corei7SkylakeX>) {
        emu.reg_write(X86Reg::IdtrBase, IDT_BASE);
        emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
        emu.reg_write(X86Reg::Rsp, STACK_TOP);
        install_exception_gate(emu, Exception::Ud as u8, UD_HANDLER);
        install_exception_gate(emu, Exception::Nm as u8, NM_HANDLER);
        // FlatLong64 seeds the live CS cache as 64-bit but deliberately leaves
        // its minimal GDT's code descriptor 32-bit; exception delivery reloads
        // CS through the GDT, so make the test gate's target a valid long code
        // segment.
        emu.mem_write(0x808, &0x00AF_9A00_0000_FFFFu64.to_le_bytes())
            .unwrap();
    }

    fn enable_guest_avx(emu: &mut Emulator<'static, Corei7SkylakeX>) {
        emu.reg_write(
            X86Reg::Cr4,
            emu.reg_read(X86Reg::Cr4) | (1 << 9) | (1 << 18),
        );
        emu.reg_write(X86Reg::Rax, 0x7);
        emu.reg_write(X86Reg::Rcx, 0);
        emu.reg_write(X86Reg::Rdx, 0);
        emu.mem_write(XSETBV_BASE, &[0x0f, 0x01, 0xd1]).unwrap();
        emu.emu_start(XSETBV_BASE, Some(XSETBV_BASE + 3), None, Some(1))
            .unwrap();
    }

    fn run_one(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        address: u64,
        code: &[u8],
    ) -> crate::error::Result<()> {
        emu.mem_write(address, code).unwrap();
        emu.emu_start(address, Some(address + code.len() as u64), None, Some(1))
            .map(|_| ())
    }

    fn assert_fault_at_original_rip(
        emu: &mut Emulator<'static, Corei7SkylakeX>,
        vector: Exception,
        handler: u64,
        rip: u64,
    ) {
        assert_eq!(emu.cpu().get_exception_diag()[vector as usize], 1);
        assert_eq!(emu.cpu().rip(), handler + 1);
        assert_eq!(emu.reg_read(X86Reg::Rsp), STACK_TOP - 40);
        let mut pushed_rip = [0u8; 8];
        emu.mem_read(STACK_TOP - 40, &mut pushed_rip).unwrap();
        assert_eq!(u64::from_le_bytes(pushed_rip), rip);
    }

    #[test]
    fn vpinsr_and_vextract_require_avx_state() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
        let forms: [(&str, &[u8]); 6] = [
            ("VPINSRB", &[0xC4, 0xE3, 0x71, 0x20, 0xC0, 0x05]),
            ("VPINSRW", &[0xC5, 0xF1, 0xC4, 0xC1, 0x03]),
            ("VPINSRD", &[0xC4, 0xE3, 0x69, 0x22, 0xC1, 0x02]),
            ("VPINSRQ", &[0xC4, 0xE3, 0xE9, 0x22, 0xC1, 0x01]),
            ("VEXTRACTF128", &[0xC4, 0xE3, 0x7D, 0x19, 0xD8, 0x01]),
            ("VEXTRACTI128", &[0xC4, 0xE3, 0x7D, 0x39, 0xD8, 0x01]),
        ];

        for (name, code) in forms {
            let mut emu = avx_emulator();
            install_avx_exception_handlers(&mut emu);
            emu.reg_write(X86Reg::Cr0, emu.reg_read(X86Reg::Cr0) | (1 << 3));
            run_one(&mut emu, CODE_BASE, code).unwrap();
            assert_fault_at_original_rip(&mut emu, Exception::Ud, UD_HANDLER, CODE_BASE);
            assert_eq!(
                emu.cpu().get_exception_diag()[Exception::Nm as usize],
                0,
                "{name} must raise #UD before #NM when OSXSAVE is clear"
            );
        }

        for (name, code) in forms {
            let mut emu = avx_emulator();
            install_avx_exception_handlers(&mut emu);
            enable_guest_avx(&mut emu);
            emu.cpu_mut().xcr0.set32(0x1);
            emu.cpu_mut().handle_avx_mode_change();
            emu.reg_write(X86Reg::Cr0, emu.reg_read(X86Reg::Cr0) | (1 << 3));
            run_one(&mut emu, CODE_BASE, code).unwrap();
            assert_fault_at_original_rip(&mut emu, Exception::Ud, UD_HANDLER, CODE_BASE);
            assert_eq!(
                emu.cpu().get_exception_diag()[Exception::Nm as usize],
                0,
                "{name} must raise #UD before #NM when XCR0 lacks SSE/YMM"
            );
        }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn vextractf128_enabled_extracts_selected_lane() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
        let mut emu = avx_emulator();
        enable_guest_avx(&mut emu);
        let source: [u8; 32] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
            0x0C, 0x0D, 0x0E, 0x0F, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
            0x88, 0x89, 0x8A, 0x8B, 0x8C, 0x8D, 0x8E, 0x8F,
        ];
        emu.reg_write_ymm(X86Reg::Ymm3, source);

        for (offset, name, code) in [
            (
                0,
                "VEXTRACTF128",
                &[0xC4, 0xE3, 0x7D, 0x19, 0xD8, 0x01][..],
            ),
            (
                0x40,
                "VEXTRACTI128",
                &[0xC4, 0xE3, 0x7D, 0x39, 0xD8, 0x01][..],
            ),
        ] {
            emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);
            let address = CODE_BASE + offset;
            run_one(&mut emu, address, code).unwrap();
            let result = emu.reg_read_ymm(X86Reg::Ymm0);
            assert_eq!(&result[..16], &source[16..], "{name} selected the wrong lane");
            assert_eq!(&result[16..], &[0; 16], "{name} must clear YMM upper bits");
        }
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
