use core::fmt::Debug;

use crate::config::BxAddress;

use super::softfloat3e::softfolat_types::floatx80;

#[derive(Debug, Default)]
pub struct I387 {
    /// Control word
    pub cwd: u64,
    /// status word
    pub swd: u16,
    /// tag word
    pub twd: u16,
    /// last instruction opcode
    pub foo: u16,

    pub fip: BxAddress,
    pub fdp: BxAddress,
    pub fcs: u16,
    pub fds: u16,

    pub st_space: [floatx80; 8],

    pub tos: u8,
    pub align1: u8,
    pub align2: u8,
    pub align3: u8,
}

pub type BxPackedRegT = BxPackedRegister;
// #[derive(Debug)]
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
impl I387 {
    /// Resets the i387 FPU state to initial values (called on CPU reset)
    pub fn reset(&mut self) {
        self.cwd = 0x0040;    // Control word reset value
        self.swd = 0;          // Status word reset
        self.tos = 0;          // Top of stack
        self.twd = 0x5555;     // Tag word: all registers tagged as empty
        self.foo = 0;          // Last instruction opcode
        self.fip = 0;          // FPU instruction pointer
        self.fcs = 0;          // FPU code segment
        self.fds = 0;          // FPU data segment
        self.fdp = 0;          // FPU data pointer
        
        // Clear all ST register space (8 x 10-byte values)
        for reg in &mut self.st_space {
            *reg = floatx80::default();
        }
    }
}