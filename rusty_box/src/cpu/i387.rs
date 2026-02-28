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
        (self.swd & !FPU_SW_TOP & 0xFFFF) | (((self.tos as u16) << 11) & FPU_SW_TOP)
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

#[derive(Clone, Copy)]
pub union BxPackedRegister {
    pub Sbyte: [i8; 8],
    pub S16: [i16; 4],
    pub S32: [i32; 2],
    pub S64: i64,
    pub Ubyte: [u8; 8],
    pub U16: [u16; 4],
    pub U32: [u32; 2],
    pub U64: u64,
}

impl Debug for BxPackedRegister {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BxPackedRegister")
            .field("Sbyte", unsafe { &self.Sbyte })
            .field("S16", unsafe { &self.S16 })
            .field("S32", unsafe { &self.S32 })
            .field("S64", unsafe { &self.S64 })
            .field("Ubyte", unsafe { &self.Ubyte })
            .field("U16", unsafe { &self.U16 })
            .field("U32", unsafe { &self.U32 })
            .field("U64", unsafe { &self.U64 })
            .finish()
    }
}

impl Default for BxPackedRegister {
    fn default() -> Self {
        BxPackedRegister {
            U64: 0x0007040600070406,
        }
    }
}
