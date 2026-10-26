//! Stepping one instruction word on a processor, for the unit tests of an
//! instruction set feature: the processor at EL1 with the MMU off, the word
//! at [`CODE`], a 64-byte data region at [`DATA`], and the exception the
//! step took, as `ESR_EL1` reports it.

use rusty_box_core::{ClockHz, VmClock};

use super::features::{Feature, Implementation};
use super::{Cpu, CpuConfig, Step};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

pub(crate) const CODE: u64 = 0x10000;
pub(crate) const VBAR: u64 = 0x20000;
pub(crate) const DATA: u64 = 0x30000;

/// `CPACR_EL1` with SIMD&FP and SME enabled at EL0 and EL1.
pub(crate) const CPACR_ALL: u64 = (3 << 20) | (3 << 24);

/// The one instruction at `CODE`, a data region, and every other read a
/// bus fault.
struct OneWord {
    insn: u32,
    data: [u8; 64],
}

impl PhysBus for OneWord {
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get();
        let n = width.bytes();
        if a >= CODE && a + n <= CODE + 4 {
            return Ok(u64::from(self.insn >> (8 * (a - CODE))) & width.value_mask());
        }
        if a >= DATA && a + n <= DATA + self.data.len() as u64 {
            let off = (a - DATA) as usize;
            let mut v = 0u64;
            for i in 0..n as usize {
                v |= u64::from(self.data[off + i]) << (8 * i);
            }
            return Ok(v);
        }
        Err(BusFault::Unmapped)
    }

    fn write(&mut self, _addr: PhysAddr, _width: AccessWidth, _value: u64, _clock: VmClock) -> Result<(), BusFault> {
        Err(BusFault::Unmapped)
    }
}

/// A processor implementing `imp`, with SIMD&FP enabled, and SME where
/// there is one.
pub(crate) fn processor(imp: Implementation) -> Cpu {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut cpu = Cpu::implementing(
        CpuConfig {
            ips,
            counter_hz: ips,
            reset_pc: CODE,
        },
        imp,
    );
    cpu.sys.cpacr_el1 = if imp.has(Feature::Sme) { CPACR_ALL } else { 3 << 20 };
    cpu.sys.vbar_el1 = VBAR;
    cpu
}

/// A taken exception, as `ESR_EL1` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Taken {
    pub(crate) ec: u64,
    pub(crate) iss: u64,
}

pub(crate) const UNDEFINED: Taken = Taken { ec: 0x00, iss: 0 };
/// The SIMD&FP access trap (EC 0x07, COND 0xE).
pub(crate) const FP_TRAP: Taken = Taken { ec: 0x07, iss: 0x01E0_0000 };
/// The SME trap with SMTC 0: SME disabled.
pub(crate) const SME_DISABLED: Taken = Taken { ec: 0x1D, iss: 0 };
/// The SME trap with SMTC 1: an instruction illegal in Streaming SVE mode.
pub(crate) const ILLEGAL_WHEN_STREAMING: Taken = Taken { ec: 0x1D, iss: 1 };

/// Step `insn` at `CODE` from the processor's current state (EL1h unless
/// the test moved it), returning the exception it took, if any. A taken
/// exception leaves EL1; the processor is put back at the exception level
/// it executed at, so that tests can carry on from there.
pub(crate) fn exec(cpu: &mut Cpu, insn: u32) -> Option<Taken> {
    let pstate = cpu.pstate;
    cpu.pc = CODE;
    cpu.sys.esr_el1 = 0x3F << 26;
    let mut bus = OneWord { insn, data: [0x5A; 64] };
    assert_eq!(cpu.step(&mut bus, &mut NoInstrumentation), Step::Retired, "{insn:#010x}");
    let esr = cpu.sys.esr_el1;
    if esr >> 26 == 0x3F {
        return None;
    }
    assert_eq!(cpu.pc & !0x7FF, VBAR, "{insn:#010x} took an exception to the vector table");
    cpu.pstate = pstate;
    Some(Taken {
        ec: esr >> 26,
        iss: esr & 0x01FF_FFFF,
    })
}
