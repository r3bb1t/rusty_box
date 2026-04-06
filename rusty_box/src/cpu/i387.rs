#![allow(dead_code)]
use core::fmt::Debug;

use crate::config::BxAddress;

use super::softfloat3e::softfloat_types::floatx80;

// Tag word values (from tag_w.h)
pub const FPU_TAG_VALID: u16 = 0x00;
pub const FPU_TAG_ZERO: u16 = 0x01;
pub const FPU_TAG_SPECIAL: u16 = 0x02;
pub const FPU_TAG_EMPTY: u16 = 0x03;

// Control word bits (from control_w.h)
pub const FPU_CW_RESERVED_BITS: u16 = 0xE0C0;
pub const FPU_CW_INF: u16 = 0x1000;
pub const FPU_CW_RC: u16 = 0x0C00;
pub const FPU_CW_PC: u16 = 0x0300;

pub const FPU_RC_RND: u16 = 0x0000;
pub const FPU_RC_DOWN: u16 = 0x0400;
pub const FPU_RC_UP: u16 = 0x0800;
pub const FPU_RC_CHOP: u16 = 0x0C00;

pub const FPU_CW_PRECISION: u16 = 0x0020;
pub const FPU_CW_UNDERFLOW: u16 = 0x0010;
pub const FPU_CW_OVERFLOW: u16 = 0x0008;
pub const FPU_CW_ZERO_DIV: u16 = 0x0004;
pub const FPU_CW_DENORMAL: u16 = 0x0002;
pub const FPU_CW_INVALID: u16 = 0x0001;
pub const FPU_CW_EXCEPTIONS_MASK: u16 = 0x003F;

pub const FPU_PR_32_BITS: u16 = 0x000;
pub const FPU_PR_RESERVED_BITS: u16 = 0x100;
pub const FPU_PR_64_BITS: u16 = 0x200;
pub const FPU_PR_80_BITS: u16 = 0x300;

// Status word bits (from status_w.h)
pub const FPU_SW_BACKWARD: u16 = 0x8000;
pub const FPU_SW_C3: u16 = 0x4000;
pub const FPU_SW_TOP: u16 = 0x3800;
pub const FPU_SW_C2: u16 = 0x0400;
pub const FPU_SW_C1: u16 = 0x0200;
pub const FPU_SW_C0: u16 = 0x0100;
pub const FPU_SW_SUMMARY: u16 = 0x0080;
pub const FPU_SW_STACK_FAULT: u16 = 0x0040;
pub const FPU_SW_PRECISION: u16 = 0x0020;
pub const FPU_SW_UNDERFLOW: u16 = 0x0010;
pub const FPU_SW_OVERFLOW: u16 = 0x0008;
pub const FPU_SW_ZERO_DIV: u16 = 0x0004;
pub const FPU_SW_DENORMAL_OP: u16 = 0x0002;
pub const FPU_SW_INVALID: u16 = 0x0001;
pub const FPU_SW_CC: u16 = FPU_SW_C0 | FPU_SW_C1 | FPU_SW_C2 | FPU_SW_C3;
pub const FPU_SW_EXCEPTIONS_MASK: u16 = 0x027F;

// Special exception combinations
pub const FPU_EX_STACK_OVERFLOW: u16 = 0x0041 | FPU_SW_C1;
pub const FPU_EX_STACK_UNDERFLOW: u16 = 0x0041;

#[derive(Debug, Default)]
pub struct I387 {
    /// Control word (u16, was incorrectly u64)
    pub(crate) cwd: u16,
    /// Status word
    pub(crate) swd: u16,
    /// Tag word
    pub(crate) twd: u16,
    /// Last instruction opcode
    pub(crate) foo: u16,

    pub(crate) fip: BxAddress,
    pub(crate) fdp: BxAddress,
    pub(crate) fcs: u16,
    pub(crate) fds: u16,

    pub(crate) st_space: [floatx80; 8],

    pub(crate) tos: u8,
    pub(crate) align1: u8,
    pub(crate) align2: u8,
    pub(crate) align3: u8,
}

impl I387 {
    /// FINIT/FNINIT: initialize FPU state
    #[inline]
    pub fn init(&mut self) {
        self.cwd = 0x037F;
        self.swd = 0;
        self.tos = 0;
        self.twd = 0xFFFF;
        self.foo = 0;
        self.fip = 0;
        self.fcs = 0;
        self.fds = 0;
        self.fdp = 0;
    }

    /// CPU reset: different from FINIT
    pub fn reset(&mut self) {
        self.cwd = 0x0040;
        self.swd = 0;
        self.tos = 0;
        self.twd = 0x5555;
        self.foo = 0;
        self.fip = 0;
        self.fcs = 0;
        self.fds = 0;
        self.fdp = 0;

        for reg in &mut self.st_space {
            *reg = floatx80::default();
        }
    }

    // --- Status accessors (matching Bochs i387.h) ---

    #[inline]
    pub fn is_ia_masked(&self) -> bool {
        (self.cwd & FPU_CW_INVALID) != 0
    }

    #[inline]
    pub fn get_control_word(&self) -> u16 {
        self.cwd
    }

    #[inline]
    pub fn get_tag_word(&self) -> u16 {
        self.twd
    }

    #[inline]
    pub fn get_status_word(&self) -> u16 {
        (self.swd & !FPU_SW_TOP) | (((self.tos as u16) << 11) & FPU_SW_TOP)
    }

    #[inline]
    pub fn get_partial_status(&self) -> u16 {
        self.swd
    }

    // --- Stack management ---

    #[inline]
    pub fn fpu_push(&mut self) {
        self.tos = (self.tos.wrapping_sub(1)) & 7;
    }

    #[inline]
    pub fn fpu_pop(&mut self) {
        self.twd |= 3 << (self.tos * 2);
        self.tos = (self.tos.wrapping_add(1)) & 7;
    }

    #[inline]
    pub fn fpu_gettagi(&self, stnr: i32) -> i32 {
        ((self.twd >> ((((stnr as u8).wrapping_add(self.tos)) & 7) * 2)) & 3) as i32
    }

    #[inline]
    pub fn fpu_settagi_valid(&mut self, stnr: i32) {
        let regnr = ((stnr as u8).wrapping_add(self.tos) & 7) as u16;
        self.twd &= !(3u16 << (regnr * 2)); // FPU_Tag_Valid == 0b00
    }

    #[inline]
    pub fn fpu_settagi(&mut self, tag: i32, stnr: i32) {
        let regnr = ((stnr as u8).wrapping_add(self.tos) & 7) as u16;
        self.twd &= !(3u16 << (regnr * 2));
        self.twd |= ((tag as u16) & 3) << (regnr * 2);
    }

    #[inline]
    pub fn fpu_read_regi(&self, stnr: i32) -> floatx80 {
        self.st_space[((self.tos as usize) + (stnr as usize)) & 7]
    }

    #[inline]
    pub fn fpu_save_regi(&mut self, reg: floatx80, stnr: i32) {
        self.st_space[((self.tos as usize) + (stnr as usize)) & 7] = reg;
        self.fpu_settagi_valid(stnr);
    }

    #[inline]
    pub fn fpu_save_regi_with_tag(&mut self, reg: floatx80, tag: i32, stnr: i32) {
        self.st_space[((self.tos as usize) + (stnr as usize)) & 7] = reg;
        self.fpu_settagi(tag, stnr);
    }
}

// --- BxPackedRegister (unchanged) ---

pub type BxPackedRegT = BxPackedRegister;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
#[allow(non_snake_case)]
pub struct BxPackedRegister {
    pub(crate) bytes: [u8; 8],
}

#[allow(non_snake_case)]
impl BxPackedRegister {
    #[inline(always)] pub fn U64(&self) -> u64 { u64::from_le_bytes(self.bytes) }
    #[inline(always)] pub fn set_U64(&mut self, v: u64) { self.bytes = v.to_le_bytes(); }

    #[inline(always)] pub fn U32(&self, i: usize) -> u32 {
        let s = i * 4;
        u32::from_le_bytes(self.bytes[s..s+4].try_into().unwrap())
    }
    #[inline(always)] pub fn set_U32(&mut self, i: usize, v: u32) {
        let s = i * 4;
        self.bytes[s..s+4].copy_from_slice(&v.to_le_bytes());
    }

    #[inline(always)] pub fn U16(&self, i: usize) -> u16 {
        let s = i * 2;
        u16::from_le_bytes(self.bytes[s..s+2].try_into().unwrap())
    }
    #[inline(always)] pub fn set_U16(&mut self, i: usize, v: u16) {
        let s = i * 2;
        self.bytes[s..s+2].copy_from_slice(&v.to_le_bytes());
    }

    #[inline(always)] pub fn Ubyte(&self, i: usize) -> u8 { self.bytes[i] }
    #[inline(always)] pub fn set_Ubyte(&mut self, i: usize, v: u8) { self.bytes[i] = v; }

    #[inline(always)] pub fn S64(&self) -> i64 { self.U64() as i64 }
    #[inline(always)] pub fn set_S64(&mut self, v: i64) { self.set_U64(v as u64); }

    #[inline(always)] pub fn S32(&self, i: usize) -> i32 { self.U32(i) as i32 }
    #[inline(always)] pub fn set_S32(&mut self, i: usize, v: i32) { self.set_U32(i, v as u32); }

    #[inline(always)] pub fn S16(&self, i: usize) -> i16 { self.U16(i) as i16 }
    #[inline(always)] pub fn set_S16(&mut self, i: usize, v: i16) { self.set_U16(i, v as u16); }

    #[inline(always)] pub fn Sbyte(&self, i: usize) -> i8 { self.bytes[i] as i8 }
    #[inline(always)] pub fn set_Sbyte(&mut self, i: usize, v: i8) { self.bytes[i] = v as u8; }
}

impl Debug for BxPackedRegister {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "MMX({:#018x})", self.U64())
    }
}

impl Default for BxPackedRegister {
    fn default() -> Self {
        // Bochs FPU default pattern
        let mut r = BxPackedRegister { bytes: [0; 8] };
        r.set_U64(0x0007040600070406);
        r
    }
}
