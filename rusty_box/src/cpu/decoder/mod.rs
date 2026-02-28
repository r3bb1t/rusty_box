// Re-export everything from rusty_box_decoder
pub use rusty_box_decoder::*;

pub use rusty_box_decoder::instr_generated::Instruction;

// Re-export commonly used items with their original names
pub use rusty_box_decoder::{
    BxSegregs, X86FeatureName, BX_16BIT_REG_IP, BX_32BIT_REG_EIP, BX_64BIT_REG_RIP,
    BX_64BIT_REG_SSP, BX_GENERAL_REGISTERS, BX_ISA_EXTENSIONS_ARRAY_SIZE, BX_NIL_REGISTER,
    BX_TMP_REGISTER, BX_XMM_REGISTERS,
};

// Re-export modules for direct access
pub use rusty_box_decoder::{features, fetchdecode32, fetchdecode64, ia_opcodes, simple_decoder};

// Re-export commonly used functions and types
pub use rusty_box_decoder::{
    features::X86Feature, fetchdecode32::fetch_decode32, fetchdecode64::fetch_decode64,
    ia_opcodes::Opcode, simple_decoder::decode_simple_32,
};

// Keep the impl BxCpuC block in the main crate
use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(in crate::cpu) fn init_fetch_decode_tables(&mut self) -> crate::cpu::Result<()> {
        // TODO: implement this in future
        Ok(())
    }
}
