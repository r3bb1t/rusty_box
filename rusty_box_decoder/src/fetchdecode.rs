use crate::bswap::{
    read_host_dword_to_little_endian, read_host_qword_to_little_endian,
    read_host_word_to_little_endian,
};

use super::{fetchdecode_generated::BX_PREPARE_EVEX, ia_opcodes::Opcode};

use bitflags::bitflags;

bitflags! {
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

// SsePrefix enum is defined in fetchdecode_generated.rs
pub use super::fetchdecode_generated::SsePrefix;

pub(super) const ATTR_LAST_OPCODE: u64 = 0x8000000000000000;

pub(super) const ATTR_LOCK_PREFIX_NOT_ALLOWED: u64 = 131072; // (1 << LOCK_PREFIX_OFFSET) = (1 << 17)

pub(super) fn fetch_dword(iptr: &[u8]) -> u32 {
    read_host_dword_to_little_endian(iptr)
}

pub(super) fn fetch_word(iptr: &[u8]) -> u16 {
    read_host_word_to_little_endian(iptr)
}

pub(super) fn fetch_qword(iptr: &[u8]) -> u64 {
    read_host_qword_to_little_endian(iptr)
}

pub(super) const fn form_opcode(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48) | ATTR_LOCK_PREFIX_NOT_ALLOWED
}

pub(super) const fn last_opcode(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48) | ATTR_LOCK_PREFIX_NOT_ALLOWED | ATTR_LAST_OPCODE
}

pub(super) const fn form_opcode_lockable(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48)
}

pub(super) const fn last_opcode_lockable(attr: u64, ia_opcode: Opcode) -> u64 {
    let ia_opcode = ia_opcode as u64;
    attr | (ia_opcode << 48) | ATTR_LAST_OPCODE
}
