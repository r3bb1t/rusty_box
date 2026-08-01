//! x86 instruction decoder pipeline — shared types and sub-module declarations.
//!
//! Mirrors Bochs `cpu/decoder/` directory layout. The public entry points are
//! [`decode32::fetch_decode32`] and [`decode64::fetch_decode64`].

pub mod decode32;
pub mod decode64;
pub(crate) mod opmap;
pub(crate) mod opmap_0f38;
pub(crate) mod opmap_0f3a;
pub(crate) mod evex_operands;
pub(crate) mod opmap_evex;
pub mod tables;
mod x87;

use crate::opcode::Opcode;
use tables::OpcodeAttrs;

// Re-export SsePrefix from tables for convenience
pub use tables::SsePrefix;

#[derive(Clone, Copy)]
#[repr(transparent)]
pub(crate) struct OpcodeTableEntry(u64);

impl OpcodeTableEntry {
    #[inline]
    pub(crate) const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[inline]
    pub(crate) const fn bits(self) -> u64 {
        self.0
    }

    #[inline]
    pub(crate) const fn mask_bits(self) -> u32 {
        (self.0 & OPCODE_TABLE_MASK_BITS) as u32
    }

    #[inline]
    pub(crate) const fn value_bits(self) -> u32 {
        (self.0 >> OPCODE_TABLE_VALUE_SHIFT) as u32
    }

    #[inline]
    pub(crate) const fn opcode(self) -> Opcode {
        Opcode::from_u16_const((self.0 >> OPCODE_TABLE_OPCODE_SHIFT) as u16)
    }

    #[inline]
    pub(crate) const fn matches_decmask(self, decmask: u32) -> bool {
        (self.value_bits() & self.mask_bits()) == (decmask & self.mask_bits())
    }
}

impl Opcode {
    #[inline]
    const fn table_entry(self, attrs: OpcodeAttrs, lockable: bool) -> OpcodeTableEntry {
        let attrs = if lockable {
            attrs
        } else {
            attrs.union(OpcodeAttrs::LOCK_PREFIX_NOT_ALLOWED)
        };

        OpcodeTableEntry::new((self as u64) << OPCODE_TABLE_OPCODE_SHIFT | attrs.bits())
    }
}

/// Build an opcode table entry (non-lockable).
///
/// Bochs: `#define form_opcode(attr, ia_opcode)` (fetchdecode.h line 490).
pub(crate) const fn form_opcode(attrs: OpcodeAttrs, ia_opcode: Opcode) -> u64 {
    ia_opcode.table_entry(attrs, false).bits()
}

/// Build an opcode table entry (lockable — no LOCK_PREFIX_NOT_ALLOWED bit).
///
/// Bochs: `#define form_opcode_lockable(attr, ia_opcode)` (fetchdecode.h line 493).
pub(crate) const fn form_opcode_lockable(attrs: OpcodeAttrs, ia_opcode: Opcode) -> u64 {
    ia_opcode.table_entry(attrs, true).bits()
}

const OPCODE_TABLE_MASK_BITS: u64 = 0x00FF_FFFF;
const OPCODE_TABLE_VALUE_SHIFT: u32 = 24;
const OPCODE_TABLE_OPCODE_SHIFT: u32 = 48;

#[inline]
pub(crate) const fn find_opcode_in_table(table: &[u64], decmask: u32) -> Opcode {
    let mut i = 0;
    while i < table.len() {
        let entry = OpcodeTableEntry(table[i]);
        if entry.matches_decmask(decmask) {
            return entry.opcode();
        }

        i += 1;
    }

    Opcode::IaError
}

#[inline]
pub(crate) const fn read_u16_le(bytes: &[u8], pos: usize) -> u16 {
    (bytes[pos] as u16) | ((bytes[pos + 1] as u16) << 8)
}

#[inline]
pub(crate) const fn read_u32_le(bytes: &[u8], pos: usize) -> u32 {
    (bytes[pos] as u32)
        | ((bytes[pos + 1] as u32) << 8)
        | ((bytes[pos + 2] as u32) << 16)
        | ((bytes[pos + 3] as u32) << 24)
}
