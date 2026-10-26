//! System register accesses as a guest sees them (R9): what an `MRS`
//! reads, what an `MSR` keeps, and the exception an access takes, as
//! `ESR_EL1` reports it.

use super::super::features::Implementation;
use super::super::step::{exec, processor, Taken, UNDEFINED};
use super::super::{Cpu, El, Pstate};

/// `MRS X0, MDCCSR_EL0`, `MRS X0, DBGDTR_EL0`, `MRS X0, DBGDTRRX_EL0`.
const MRS_MDCCSR: u32 = 0xD533_0100;
const MRS_DBGDTR: u32 = 0xD533_0400;
const MRS_DBGDTRRX: u32 = 0xD533_0500;
/// `MSR DBGDTR_EL0, X0`, `MSR DBGDTRTX_EL0, X0`.
const MSR_DBGDTR: u32 = 0xD513_0400;
const MSR_DBGDTRTX: u32 = 0xD513_0500;
/// The `MSR` form of `MDCCSR_EL0`'s encoding, which is read-only.
const MSR_MDCCSR: u32 = 0xD513_0100;

fn at_el0(cpu: &mut Cpu) {
    cpu.pstate = Pstate {
        el: El::El0,
        sp_sel: false,
        ..Pstate::RESET
    };
}

/// The Debug Communications Channel with no debugger attached, as QEMU's
/// `virt` has it: `MDCCSR_EL0` reads 0 (RXfull and TXfull clear),
/// `DBGDTR_EL0` and `DBGDTRRX_EL0` read 0, and a word written to
/// `DBGDTR_EL0` or `DBGDTRTX_EL0` is gone. EL0 reaches all four unless
/// `MDSCR_EL1.TDCC` is set, which traps EL0's accesses to EL1 (EC 0x18,
/// the ISS naming the register); EL1's go through. `MDCCSR_EL0` cannot be
/// written.
#[test]
fn the_debug_communications_channel_is_idle_and_tdcc_traps_el0() {
    // ISS: op0 2, op2 0, op1 3, CRn 0, Rt 0, CRm and the direction.
    let reads = [(MRS_MDCCSR, 0x0020_C003), (MRS_DBGDTR, 0x0020_C009), (MRS_DBGDTRRX, 0x0020_C00B)];
    let writes = [(MSR_DBGDTR, 0x0020_C008), (MSR_DBGDTRTX, 0x0020_C00A)];
    let mut cpu = processor(Implementation::CORTEX_A57);
    for el0 in [false, true] {
        if el0 {
            at_el0(&mut cpu);
        }
        for (insn, _) in writes {
            cpu.x[0] = 0x1234_5678;
            assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}, EL0 {el0}");
        }
        for (insn, _) in reads {
            cpu.x[0] = 0xDEAD;
            assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}, EL0 {el0}");
            assert_eq!(cpu.x[0], 0, "{insn:#010x}, EL0 {el0}");
        }
        assert_eq!(exec(&mut cpu, MSR_MDCCSR), Some(UNDEFINED), "EL0 {el0}");
    }

    cpu.sys.mdscr_el1 = 1 << 12;
    for (insn, iss) in reads.into_iter().chain(writes) {
        assert_eq!(exec(&mut cpu, insn), Some(Taken { ec: 0x18, iss }), "{insn:#010x} at EL0");
    }
    cpu.pstate = Pstate::RESET;
    for (insn, _) in reads.into_iter().chain(writes) {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL1");
    }
}
