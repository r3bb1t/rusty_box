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
#[derive(Debug)]
pub enum BxPackedRegister {
    Sbyte([i8; 8]),
    S16([i16; 4]),
    S32([i32; 2]),
    S64(i64),
    Ubyte([u8; 8]),
    U16([u16; 4]),
    U32([u32; 2]),
    U64(u64),
}

impl Default for BxPackedRegister {
    fn default() -> Self {
        Self::U64(0)
    }
}
