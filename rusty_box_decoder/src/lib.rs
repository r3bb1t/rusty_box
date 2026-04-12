#![forbid(unsafe_code)]

pub mod error;
pub use error::{DecodeError, DecodeResult};

pub mod features;

/// x86 instruction decoder pipeline — mirrors Bochs `cpu/decoder/` layout.
///
/// Internal modules:
/// - `decode32` / `decode64` — 32-bit and 64-bit fetch-decode implementations
/// - `tables` — generated constants, attributes, and decoding masks
/// - `opmap` / `opmap_0f38` / `opmap_0f3a` — opcode lookup tables
/// - `x87` — x87 FPU opcode tables
pub mod decoder;

/// Core instruction representation — flattened struct with named fields.
pub mod instruction;

/// x86 opcode enumeration — one variant per distinct instruction form.
pub mod opcode;

/// Type-safe instruction enum — each opcode variant carries exactly its operands.
pub mod typed;

// Re-export key public types and functions at crate root for convenience.
pub use decoder::{decode32, decode64};
pub use decoder::decode32::fetch_decode32;
pub use decoder::decode64::fetch_decode64;
pub use decoder::tables::{BxDecodeError, SsePrefix};

#[cfg(test)]
mod tests;

pub const BX_ISA_EXTENSIONS_ARRAY_SIZE: usize = 5;

// Re-export X86Feature from features.rs as the canonical ISA feature enum.
pub use features::X86Feature;

/// Segment register encoding
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BxSegregs {
    Es = 0,
    Cs = 1,
    Ss = 2,
    Ds = 3,
    Fs = 4,
    Gs = 5,
    // NULL now has to fit in 3 bits.
    Null = 7,
}

/// Returns `true` if `seg` encodes the null segment register (value 7).
pub fn is_null_seg_reg(seg: u8) -> bool {
    seg == BxSegregs::Null as _
}

impl BxSegregs {
    /// Convert from raw u8 to BxSegregs (const-compatible).
    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => BxSegregs::Es,
            1 => BxSegregs::Cs,
            2 => BxSegregs::Ss,
            3 => BxSegregs::Ds,
            4 => BxSegregs::Fs,
            5 => BxSegregs::Gs,
            7 => BxSegregs::Null,
            _ => BxSegregs::Ds,
        }
    }
}

impl From<u8> for BxSegregs {
    fn from(val: u8) -> Self {
        BxSegregs::from_u8(val)
    }
}

pub const BX_GENERAL_REGISTERS: usize = 16;

pub const BX_16BIT_REG_IP: usize = BX_GENERAL_REGISTERS;
pub const BX_32BIT_REG_EIP: usize = BX_GENERAL_REGISTERS;
pub const BX_64BIT_REG_RIP: usize = BX_GENERAL_REGISTERS;

pub const BX_32BIT_REG_SSP: usize = BX_GENERAL_REGISTERS + 1;
pub const BX_64BIT_REG_SSP: usize = BX_GENERAL_REGISTERS + 1;

pub const BX_TMP_REGISTER: usize = BX_GENERAL_REGISTERS + 2;
pub const BX_NIL_REGISTER: usize = BX_GENERAL_REGISTERS + 3;

pub const BX_XMM_REGISTERS: usize = 32;

#[cfg(test)]
mod test_call_decode;
