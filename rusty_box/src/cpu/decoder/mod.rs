// Re-export the entire decoder crate's public API.
pub use rusty_box_decoder::*;

// Flatten key types so callers can write `decoder::Instruction` etc.
pub use rusty_box_decoder::instruction::{
    AddressSize, GprIndex, Instruction, InstructionFlags, Operands, OperandSize, RepPrefix,
};
pub use rusty_box_decoder::opcode::Opcode;
pub use rusty_box_decoder::features::X86Feature;

// Keep the impl BxCpuC block in the main crate
use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(in crate::cpu) fn init_fetch_decode_tables(&mut self) -> crate::cpu::Result<()> {
        // TODO: implement this in future
        Ok(())
    }
}
