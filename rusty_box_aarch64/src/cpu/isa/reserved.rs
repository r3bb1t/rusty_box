//! Arm group `reserved` (Arm ARM C4.1, "Reserved"): the A64 space with
//! `op0 = 0, op1 = 0000`.

use super::Encoding;
use crate::cpu::exec::bits;

/// Arm group `perm_undef`: the permanently undefined space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PermUndef {
    Udf,
}

pub(super) fn reserved(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 30, 29);
    let op1 = bits(word, 24, 16);
    match (op0, op1) {
        (0b00, 0b000000000) => perm_undef(word).map(Encoding::PermUndef),
        _ => None,
    }
}

fn perm_undef(_word: u32) -> Option<PermUndef> {
    Some(PermUndef::Udf)
}
