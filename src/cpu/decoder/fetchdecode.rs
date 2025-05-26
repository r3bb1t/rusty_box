use super::ia_opcodes::Opcode;

pub enum SsePrefix {
    PrefixNone = 0,
    Prefix66 = 1,
    PrefixF3 = 2,
    PrefixF2 = 3,
}

pub(super) const ATTR_LAST_OPCODE: u64 = 0x8000000000000000;

pub(super) const ATTR_LOCK_PREFIX_NOT_ALLOWED: u64 = 0x0000000000000001; // Example value, adjust as needed

//pub(super) const fn form_opcode(attr: u64, ia_opcode: u64) -> u64 {
//    attr | (ia_opcode << 48) | ATTR_LOCK_PREFIX_NOT_ALLOWED
//}
//
//pub(super) const fn last_opcode(attr: u64, ia_opcode: u64) -> u64 {
//    attr | (ia_opcode << 48) | ATTR_LOCK_PREFIX_NOT_ALLOWED | ATTR_LAST_OPCODE
//}
//
//pub(super) const fn form_opcode_lockable(attr: u64, ia_opcode: u64) -> u64 {
//    attr | (ia_opcode << 48)
//}
//
//pub(super) const fn last_opcode_lockable(attr: u64, ia_opcode: u64) -> u64 {
//    attr | (ia_opcode << 48) | ATTR_LAST_OPCODE
//}

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
