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
use tables::{BX_PREPARE_EVEX, ATTR_LOCK_PREFIX_NOT_ALLOWED};

use bitflags::bitflags;

bitflags! {
    /// Opcode table attribute flags — matching Bochs `fetchdecode.h` BX_PREPARE_* constants
    /// (lines 69-80).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct OpFlags: u16 {
        /// AMX instruction (BX_PREPARE_AMX)
        const PrepareAmx = 0x800;
        /// EVEX with VL ignore (BX_EVEX_VL_IGNORE)
        const EvexVlIgnore = 0x400 | BX_PREPARE_EVEX as u16;
        /// EVEX without broadcast (BX_PREPARE_EVEX_NO_BROADCAST)
        const PrepareEvexNoBroadcast = 0x200 | BX_PREPARE_EVEX as u16;
        /// EVEX without SAE (BX_PREPARE_EVEX_NO_SAE)
        const PrepareEvexNoSae = 0x100 | BX_PREPARE_EVEX as u16;
        /// EVEX instruction (BX_PREPARE_EVEX)
        const PrepareEvex = 0x80;
        /// Opmask instruction (BX_PREPARE_OPMASK)
        const PrepareOpmask = 0x40;
        /// AVX instruction (BX_PREPARE_AVX)
        const PrepareAvx = 0x20;
        /// SSE instruction (BX_PREPARE_SSE)
        const PrepareSse = 0x10;
        /// MMX instruction (BX_PREPARE_MMX)
        const PrepareMmx = 0x08;
        /// FPU instruction (BX_PREPARE_FPU)
        const PrepareFpu = 0x04;
        /// Lockable instruction (BX_LOCKABLE)
        const Lockable = 0x02;
        /// Trace end — instruction terminates trace (BX_TRACE_END)
        const TraceEnd = 0x01;
    }
}

// Re-export SsePrefix from tables for convenience
pub use tables::SsePrefix;

/// Marks the last entry in a multi-entry opcode table.
/// Bochs: `ATTR_LAST_OPCODE` (fetchdecode.h line 488).
pub(crate) const ATTR_LAST_OPCODE: u64 = 0x8000000000000000;

/// Build an opcode table entry (non-lockable).
/// Bochs: `#define form_opcode(attr, ia_opcode)` (fetchdecode.h line 490).
pub(crate) const fn form_opcode(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48) | ATTR_LOCK_PREFIX_NOT_ALLOWED
}

/// Build the last opcode table entry (non-lockable).
/// Bochs: `#define last_opcode(attr, ia_opcode)` (fetchdecode.h line 491).
pub(crate) const fn last_opcode(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48) | ATTR_LOCK_PREFIX_NOT_ALLOWED | ATTR_LAST_OPCODE
}

/// Build an opcode table entry (lockable — no LOCK_PREFIX_NOT_ALLOWED bit).
/// Bochs: `#define form_opcode_lockable(attr, ia_opcode)` (fetchdecode.h line 493).
pub(crate) const fn form_opcode_lockable(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48)
}

/// Build the last opcode table entry (lockable).
/// Bochs: `#define last_opcode_lockable(attr, ia_opcode)` (fetchdecode.h line 494).
pub(crate) const fn last_opcode_lockable(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48) | ATTR_LAST_OPCODE
}
