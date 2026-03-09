//! x86 instruction decoder pipeline — shared types and sub-module declarations.
//!
//! Mirrors Bochs `cpu/decoder/` directory layout. The public entry points are
//! [`decode32::fetch_decode32`] and [`decode64::fetch_decode64`].

pub mod decode32;
pub mod decode64;
pub mod tables;
pub(crate) mod opmap;
pub(crate) mod opmap_0f38;
pub(crate) mod opmap_0f3a;
mod x87;

use crate::opcode::Opcode;
use tables::BX_PREPARE_EVEX;

use bitflags::bitflags;

bitflags! {
    /// Opcode table attribute flags — matching Bochs `decoder.h` BX_PREPARE_* constants.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct OpFlags: u16 {
        const PrepareAmx = 0x800;
        const EvexVlIgnore = 0x400 | BX_PREPARE_EVEX as u16;
        const PrepareEvexNoBroadcast = 0x200 | BX_PREPARE_EVEX as u16;
        const PrepareEvexNoSae = 0x100 | BX_PREPARE_EVEX as u16;
        const PrepareEvex = 0x80;
        const PrepareOpmask = 0x40;
        const PrepareAvx = 0x20;
        const PrepareSse = 0x10;
        const PrepareMmx = 0x08;
        const PrepareFpu = 0x04;
        const Lockable = 0x02;
        const TraceEnd = 0x01;
    }
}

// Re-export SsePrefix from tables for convenience
pub use tables::SsePrefix;

pub(crate) const ATTR_LAST_OPCODE: u64 = 0x8000000000000000;

pub(crate) const ATTR_LOCK_PREFIX_NOT_ALLOWED: u64 = 131072; // (1 << LOCK_PREFIX_OFFSET) = (1 << 17)

pub(crate) const fn form_opcode(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48) | ATTR_LOCK_PREFIX_NOT_ALLOWED
}

pub(crate) const fn last_opcode(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48) | ATTR_LOCK_PREFIX_NOT_ALLOWED | ATTR_LAST_OPCODE
}

pub(crate) const fn form_opcode_lockable(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48)
}

pub(crate) const fn last_opcode_lockable(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48) | ATTR_LAST_OPCODE
}
