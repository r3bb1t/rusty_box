// BCD (Binary Coded Decimal) instructions: DAA, DAS, AAA, AAS, AAM, AAD
// Mirrors Bochs cpp/cpu/bcd.cc

use crate::cpu::decoder::Instruction;
use crate::cpu::eflags::EFlags;
use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// Set AF flag (bit 4)
    fn set_af(&mut self, val: bool) {
        if val {
            self.eflags.insert(EFlags::AF);
        } else {
            self.eflags.remove(EFlags::AF);
        }
    }

    /// Set CF flag (bit 0)
    fn set_cf(&mut self, val: bool) {
        if val {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
    }
}

/// DAS: Decimal Adjust AL after Subtraction
/// Opcode: 0x2F
/// Matches BX_CPU_C::DAS
/// The algorithm for DAS is fashioned after the pseudo code in the
/// Pentium Processor Family Developer's Manual, volume 3.
pub fn DAS<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, _instr: &Instruction) -> Result<(), crate::cpu::CpuError> {
    /* DAS effect the following flags: A,C,S,Z,P */

    let tmp_al = cpu.al();
    let original_cf = cpu.get_cf();
    let mut tmp_cf = false;
    let mut tmp_af = false;

    if ((tmp_al & 0x0F) > 0x09) || cpu.get_af() {
        tmp_cf = (cpu.al() < 0x06) || original_cf;
        cpu.set_al(cpu.al().wrapping_sub(0x06));
        tmp_af = true;
    }

    if (tmp_al > 0x99) || original_cf {
        cpu.set_al(cpu.al().wrapping_sub(0x60));
        tmp_cf = true;
    }

    cpu.update_flags_logic8(cpu.al());
    cpu.set_cf(tmp_cf);
    cpu.set_af(tmp_af);

    Ok(())
}
