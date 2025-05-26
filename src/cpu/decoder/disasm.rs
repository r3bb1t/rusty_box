use crate::config::BxAddress;

use super::instr::BxDisasmStyle;

fn disasm(
    opcode: &[u8; 16],
    is_32: bool,
    is_64: bool,
    disbuf: &mut [u8],
    cs_base: u64,
    rip: BxAddress,
    style: BxDisasmStyle,
) {
    todo!()
}

pub fn bx_disasm_wrapper(
    is_32: bool,
    is_64: bool,
    cs_base: BxAddress,
    ip: BxAddress,
    instr: &[u8; 16],
    disbuf: &mut [u8],
) {
}
