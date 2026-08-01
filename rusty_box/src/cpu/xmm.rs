#![allow(dead_code)]
//! XMM/YMM/ZMM register types and MXCSR for SSE/AVX/AVX-512
//!
//! Based on Bochs cpu/simd_int.h and cpu/xmm.h.
//! Safe structs backed by byte arrays with inline accessor methods.
//! On x86 targets LLVM optimises from_le_bytes/to_le_bytes to identical code as union access.

use crate::cpu::{decoder::Instruction, BxCpuC, BxCpuIdTrait};

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
    /// SoftFloat `softfloat_exception_masks` layout (bit0=IE .. bit5=PE).
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

    /// Write XMM register (writes lower 128 bits, clears upper bits for
    /// VEX-encoded SSE). Bochs xmm.h `BX_WRITE_XMM_REGZ` -> `BX_CLEAR_AVX_HIGH128`.
    ///
    /// How much is cleared depends on how much of the register file XCR0 has
    /// made architecturally visible: a guest that never enabled ZMM state does
    /// not have bits 256..511 zeroed by a VEX write, because for that guest
    /// they do not exist.
    #[inline]
    pub(super) fn write_xmm_reg(&mut self, index: u8, val: BxPackedXmmRegister) {
        use super::opcodes_table::BxAvxVectorLength;
        let i = index as usize;
        self.vmm[i].set_zmm128(0, val);
        if self.maxvl > BxAvxVectorLength::Vl128 {
            self.vmm[i].set_zmm128(1, BxPackedXmmRegister::default());
            if self.maxvl > BxAvxVectorLength::Vl256 {
                self.vmm[i].set_zmm256(1, BxPackedYmmRegister::default());
            }
        }
    }

    /// Write XMM register preserving upper bits (for legacy SSE without VEX)
    #[inline]
    pub(super) fn write_xmm_reg_lo128(&mut self, index: u8, val: BxPackedXmmRegister) {
        self.vmm[index as usize].set_zmm128(0, val);
    }

    /// Write an XMM result the way Bochs `BX_WRITE_XMM_REGZ` does: a legacy
    /// SSE encoding preserves the bits above 128, a VEX encoding clears them.
    ///
    /// Handlers shared between the legacy and VEX forms of an instruction
    /// (MOVD/MOVQ, PCMPxSTRM, …) MUST use this rather than
    /// `write_xmm_reg_lo128`, or the VEX form leaks stale YMM data.
    #[inline]
    pub(super) fn write_xmm_regz(
        &mut self,
        instr: &Instruction,
        index: u8,
        val: BxPackedXmmRegister,
    ) {
        if instr.is_vex() {
            self.write_xmm_reg(index, val);
        } else {
            self.write_xmm_reg_lo128(index, val);
        }
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

    /// Write YMM register (writes lower 256 bits, clears the upper 256 only
    /// when XCR0 has made them visible). Bochs xmm.h `BX_WRITE_YMM_REGZ` ->
    /// `BX_CLEAR_AVX_HIGH256`.
    #[inline]
    pub(super) fn write_ymm_reg(&mut self, index: u8, val: BxPackedYmmRegister) {
        use super::opcodes_table::BxAvxVectorLength;
        let i = index as usize;
        self.vmm[i].set_zmm256(0, val);
        if self.maxvl > BxAvxVectorLength::Vl256 {
            self.vmm[i].set_zmm256(1, BxPackedYmmRegister::default());
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

/// Emulator construction needs a bigger stack than the default 2 MiB test
/// thread: `Emulator` is ~4 MiB and the debug build materialises a few
/// copies while boxing it. 64 MiB is ample; the previous 256 MiB made
/// enough concurrent reservations to intermittently exhaust the process
/// and fail unrelated tests with STATUS_STACK_OVERFLOW.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
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
            .stack_size(TEST_STACK_SIZE)
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
            .stack_size(TEST_STACK_SIZE)
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

    // ════════════════════════════════════════════════════════════════════
    // Per-opcode CPUID/ISA gate — Bochs init_FetchDecodeTables parity.
    //
    // These reach into the CPU's own `ia_extensions_bitmask`, which is what
    // makes the gate per-CPU rather than the process-global handler table
    // Bochs patches. Nothing here mutates shared state.
    // ════════════════════════════════════════════════════════════════════

    use crate::cpu::decoder::{Opcode, X86Feature};

    fn set_feature(emu: &mut Emulator<'static, Corei7SkylakeX>, f: X86Feature, on: bool) {
        let index = f as usize;
        let (word, bit) = (index / 32, 1u32 << (index % 32));
        let cpu = emu.cpu_mut();
        if on {
            cpu.ia_extensions_bitmask[word] |= bit;
        } else {
            cpu.ia_extensions_bitmask[word] &= !bit;
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // #AC on misaligned data access — Bochs access.cc access_read_linear /
    // access_write_linear. `alignment_check_mask` was previously stored and
    // snapshotted but never consulted, so #AC was never raised.
    // ════════════════════════════════════════════════════════════════════



    /// Proves the #AC check is *wired into* each of the six scalar linear
    /// accessors with the correct `ac_mask`, not merely that the predicate
    /// works in isolation.
    ///
    /// Leverage: Bochs `access_read_linear` / `access_write_linear` test
    /// alignment *before* the TLB walk, so on a misaligned access #AC must
    /// fire even though translation in this bare harness would fail with #PF.
    /// "Which vector incremented" is therefore a direct probe of the call
    /// site: a missing call or a too-narrow mask shows up as #PF, a too-wide
    /// mask fires #AC on a naturally aligned access.
    #[test]
    fn ac_check_is_wired_into_every_scalar_linear_accessor() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                use crate::cpu::decoder::BxSegregs;
                const BASE: u64 = 0x0060_0000;
                let mut emu = avx_emulator();
                let ac = Exception::Ac as usize;

                // (width, offset, expect #AC). The "false" rows sit exactly on
                // the mask boundary: aligned for that width, so a too-wide
                // mask is caught there and a too-narrow one on the "true" rows.
                let cases: &[(&str, u64, bool)] = &[
                    ("word", 1, true),
                    ("word", 2, false),
                    ("dword", 1, true),
                    ("dword", 2, true),
                    ("dword", 3, true),
                    ("dword", 4, false),
                    ("qword", 1, true),
                    ("qword", 4, true),
                    ("qword", 7, true),
                    ("qword", 8, false),
                ];

                for &(width, off, want_ac) in cases {
                    for is_write in [false, true] {
                        // Re-arm every iteration: delivering the previous
                        // exception runs CPL-0 machinery that may clear
                        // user_pl, and the check requires both conditions.
                        emu.cpu_mut().alignment_check_mask = 0xf;
                        emu.cpu_mut().user_pl = true;

                        let before = emu.cpu().get_exception_diag()[ac];
                        let addr = BASE + off;
                        let _ = match (width, is_write) {
                            ("word", false) => emu
                                .cpu_mut()
                                .read_linear_word(BxSegregs::Ds, addr)
                                .map(|_| ()),
                            ("word", true) => {
                                emu.cpu_mut().write_linear_word(BxSegregs::Ds, addr, 0)
                            }
                            ("dword", false) => emu
                                .cpu_mut()
                                .read_linear_dword(BxSegregs::Ds, addr)
                                .map(|_| ()),
                            ("dword", true) => {
                                emu.cpu_mut().write_linear_dword(BxSegregs::Ds, addr, 0)
                            }
                            (_, false) => emu
                                .cpu_mut()
                                .read_linear_qword(BxSegregs::Ds, addr)
                                .map(|_| ()),
                            (_, true) => {
                                emu.cpu_mut().write_linear_qword(BxSegregs::Ds, addr, 0)
                            }
                        };
                        let raised_ac = emu.cpu().get_exception_diag()[ac] > before;
                        let dir = if is_write { "write" } else { "read" };
                        if want_ac {
                            assert!(
                                raised_ac,
                                "{dir}_linear_{width} at +{off} must raise #AC before the \
                                 TLB walk — missing check_alignment call or too-narrow mask"
                            );
                        } else {
                            assert!(
                                !raised_ac,
                                "{dir}_linear_{width} at +{off} is aligned for this width \
                                 — the accessor's ac_mask is too wide"
                            );
                        }
                    }
                }

                // Byte accessors take no ac_mask in Bochs and must never #AC.
                emu.cpu_mut().alignment_check_mask = 0xf;
                emu.cpu_mut().user_pl = true;
                let before = emu.cpu().get_exception_diag()[ac];
                let _ = emu.cpu_mut().read_linear_byte(BxSegregs::Ds, BASE + 1);
                emu.cpu_mut().alignment_check_mask = 0xf;
                emu.cpu_mut().user_pl = true;
                let _ = emu.cpu_mut().write_linear_byte(BxSegregs::Ds, BASE + 1, 0);
                assert_eq!(
                    emu.cpu().get_exception_diag()[ac],
                    before,
                    "byte accesses are never alignment-checked"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn misaligned_user_access_raises_ac_when_armed() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut emu = avx_emulator();
                // Bochs ac_mask values, straight from access2.cc: word=1,
                // dword=3, qword=7. Byte accesses are never checked and vector
                // accesses take the separate #GP `_aligned` path.
                const WORD: u32 = 1;
                const DWORD: u32 = 3;
                const QWORD: u32 = 7;

                // Disarmed: alignment_check_mask is 0 unless CS.RPL==3 with
                // CR0.AM and EFLAGS.AC, so nothing faults.
                assert_eq!(emu.cpu().alignment_check_mask, 0);
                for (addr, mask) in [(0x1001u64, WORD), (0x1003, DWORD), (0x1007, QWORD)] {
                    assert!(
                        emu.cpu_mut().check_alignment(addr, mask).is_ok(),
                        "no #AC while the mask is disarmed"
                    );
                }

                // Arm exactly what handle_alignment_check() would set.
                emu.cpu_mut().alignment_check_mask = 0xf;
                emu.cpu_mut().user_pl = true;

                // Naturally aligned accesses still pass at every width.
                for (addr, mask) in [(0x1000u64, WORD), (0x1000, DWORD), (0x1000, QWORD),
                                     (0x1002, WORD), (0x1004, DWORD), (0x1008, QWORD)] {
                    assert!(
                        emu.cpu_mut().check_alignment(addr, mask).is_ok(),
                        "aligned access at {addr:#x} mask {mask} must not fault"
                    );
                }

                // Misaligned accesses raise #AC, one width at a time.
                for (addr, mask, name) in [
                    (0x1001u64, WORD, "word at +1"),
                    (0x1001, DWORD, "dword at +1"),
                    (0x1002, DWORD, "dword at +2"),
                    (0x1004, QWORD, "qword at +4"),
                    (0x1001, QWORD, "qword at +1"),
                ] {
                    let before = emu.cpu().get_exception_diag()[Exception::Ac as usize];
                    assert!(
                        emu.cpu_mut().check_alignment(addr, mask).is_err(),
                        "{name} must fault"
                    );
                    assert_eq!(
                        emu.cpu().get_exception_diag()[Exception::Ac as usize],
                        before + 1,
                        "{name} must raise #AC specifically"
                    );
                }

                // Bochs gates the check on `user`: a supervisor access never
                // faults, even with the mask armed. `user_pl` is forced false
                // around descriptor loads and other CPL-0 accesses.
                emu.cpu_mut().user_pl = false;
                for (addr, mask) in [(0x1001u64, WORD), (0x1003, DWORD), (0x1005, QWORD)] {
                    assert!(
                        emu.cpu_mut().check_alignment(addr, mask).is_ok(),
                        "#AC applies to user-privilege accesses only"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn isa_gate_disables_opcodes_whose_feature_is_absent() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
            let mut emu = avx_emulator();

            // Ungated base instructions are never touched.
            assert_eq!(
                emu.cpu().isa_resolve_opcode(Opcode::Nop),
                Opcode::Nop,
                "an opcode with no ISA feature must always be allowed"
            );

            // Skylake-X advertises AVX2, so VPERMD stays enabled...
            assert_eq!(
                emu.cpu().isa_resolve_opcode(Opcode::V256VpermdVdqHdqWdq),
                Opcode::V256VpermdVdqHdqWdq
            );
            // ...and #UDs the moment the feature bit goes away.
            set_feature(&mut emu, X86Feature::IsaAvx2, false);
            assert_eq!(
                emu.cpu().isa_resolve_opcode(Opcode::V256VpermdVdqHdqWdq),
                Opcode::IaError,
                "clearing AVX2 must disable AVX2-only opcodes"
            );
            set_feature(&mut emu, X86Feature::IsaAvx2, true);

            // Features Skylake-X genuinely lacks are gated out of the box. GFNI
            // matters in particular: its VEX form is 3-operand, so executing the
            // legacy 2-operand handler under a VEX prefix would silently compute
            // the wrong thing rather than fault.
            for op in [
                Opcode::Gf2p8affineqbVdqWdqIb,
                Opcode::Sha256rnds2VdqWdq,
                Opcode::Getsec,
                Opcode::V256VaesencVdqHdqWdq,
            ] {
                assert_eq!(
                    emu.cpu().isa_resolve_opcode(op),
                    Opcode::IaError,
                    "{op:?} is not in the Skylake-X feature set and must #UD"
                );
            }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn isa_gate_turns_unadvertised_avx512_into_a_guest_ud() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let emu = avx_emulator();

                // Skylake-X advertises exactly these four, as upstream's
                // corei7_skylake-x.cc does.
                for f in [
                    X86Feature::IsaAvx512,
                    X86Feature::IsaAvx512Bw,
                    X86Feature::IsaAvx512Dq,
                    X86Feature::IsaAvx512Cd,
                ] {
                    assert!(
                        emu.cpu().bx_cpuid_support_isa_extension(f),
                        "{f:?} is part of Skylake-X"
                    );
                }
                // …and not these, which are later parts.
                for f in [
                    X86Feature::IsaAvx512Vbmi,
                    X86Feature::IsaAvx512Ifma52,
                    X86Feature::IsaAvx512Vnni,
                ] {
                    assert!(
                        !emu.cpu().bx_cpuid_support_isa_extension(f),
                        "{f:?} is not part of Skylake-X"
                    );
                }

                // Opcodes of an advertised feature resolve to themselves.
                for op in [
                    Opcode::EvexVpaddbVdqHdqWdq,
                    Opcode::EvexVmovdqu16VdqWdq,
                    Opcode::EvexValigndVdqHdqWdqIbKmask,
                ] {
                    assert_eq!(
                        emu.cpu().isa_resolve_opcode(op),
                        op,
                        "{op:?} belongs to an advertised AVX-512 feature"
                    );
                }

                // An opcode of a feature this model does not have must become a
                // guest #UD. Before the ISA gate existed it reached the
                // dispatcher catch-all and produced CpuError::UnimplementedOpcode
                // — an emulator-level error, i.e. the host stops rather than the
                // guest faulting. Upstream points it at BxError.
                for op in [
                    Opcode::EvexVpermt2bVdqHdqWdqKmask,
                    Opcode::EvexVpermi2bVdqHdqWdqKmask,
                ] {
                    assert_eq!(
                        emu.cpu().isa_resolve_opcode(op),
                        Opcode::IaError,
                        "{op:?} is AVX512_VBMI, which Skylake-X does not have"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn isa_gate_applies_the_bochs_special_cases() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
            let mut emu = avx_emulator();

            // Without LZCNT, F3 0F BD is architecturally BSR — Bochs copies the
            // BSR table entry over LZCNT's rather than raising #UD.
            set_feature(&mut emu, X86Feature::IsaLzcnt, false);
            assert_eq!(
                emu.cpu().isa_resolve_opcode(Opcode::LzcntGdEd),
                Opcode::BsrGdEd
            );
            assert_eq!(
                emu.cpu().isa_resolve_opcode(Opcode::LzcntGqEq),
                Opcode::BsrGqEq
            );
            set_feature(&mut emu, X86Feature::IsaLzcnt, true);
            assert_eq!(
                emu.cpu().isa_resolve_opcode(Opcode::LzcntGdEd),
                Opcode::LzcntGdEd
            );

            // Same shape for TZCNT/BSF under BMI1.
            set_feature(&mut emu, X86Feature::IsaBmi1, false);
            assert_eq!(
                emu.cpu().isa_resolve_opcode(Opcode::TzcntGdEd),
                Opcode::BsfGdEd
            );
            set_feature(&mut emu, X86Feature::IsaBmi1, true);

            // 3DNow! Extensions re-enable a fixed list of MMX-era opcodes even
            // when their declared feature (SSE) is absent.
            set_feature(&mut emu, X86Feature::IsaSse, false);
            assert_eq!(
                emu.cpu().isa_resolve_opcode(Opcode::PavgbPqQq),
                Opcode::IaError,
                "without SSE or 3DNow!-Ext the MMX-era form is unavailable"
            );
            set_feature(&mut emu, X86Feature::Isa3dnowExt, true);
            assert_eq!(
                emu.cpu().isa_resolve_opcode(Opcode::PavgbPqQq),
                Opcode::PavgbPqQq,
                "3DNow!-Ext must re-enable the aliased MMX opcodes"
            );
            set_feature(&mut emu, X86Feature::Isa3dnowExt, false);
            set_feature(&mut emu, X86Feature::IsaSse, true);

            // AVX10.1 subsumes the AVX-512 sub-extensions.
            set_feature(&mut emu, X86Feature::IsaAvx512Bw, false);
            let evex_bw = Opcode::EvexVpaddbVdqHdqWdq;
            assert_eq!(emu.cpu().isa_resolve_opcode(evex_bw), Opcode::IaError);
            set_feature(&mut emu, X86Feature::IsaAvx10_1, true);
            assert_eq!(
                emu.cpu().isa_resolve_opcode(evex_bw),
                evex_bw,
                "AVX10.1 must stand in for the individual AVX-512 sub-features"
            );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn isa_gate_lets_vpclmulqdq_vl256_run_once_its_feature_is_present() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut emu = avx_emulator();
                enable_guest_avx(&mut emu);
                // Corei7SkylakeX lacks VPCLMULQDQ, so the 256-bit form #UDs by
                // default (asserted in the integration suite). Turn the feature
                // on and the per-lane handler must run.
                set_feature(&mut emu, X86Feature::IsaVaesVpclmulqdq, true);

                let mut ymm1 = [0u8; 32];
                let mut ymm2 = [0u8; 32];
                ymm1[0] = 2;
                ymm1[16] = 5;
                ymm2[0] = 3;
                ymm2[16] = 7;
                emu.reg_write_ymm(X86Reg::Ymm1, ymm1);
                emu.reg_write_ymm(X86Reg::Ymm2, ymm2);
                emu.reg_write_ymm(X86Reg::Ymm0, [0xAA; 32]);

                // VPCLMULQDQ ymm0, ymm1, ymm2, 0
                run_one(&mut emu, CODE_BASE, &[0xC4, 0xE3, 0x75, 0x44, 0xC2, 0x00]).unwrap();

                let got = emu.reg_read_ymm(X86Reg::Ymm0);
                let mut want = [0u8; 32];
                want[0] = 6; // carry-less 2 x 3
                want[16] = 27; // carry-less 5 x 7
                assert_eq!(
                    got, want,
                    "VEX.256 VPCLMULQDQ multiplies each 128-bit lane independently"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
