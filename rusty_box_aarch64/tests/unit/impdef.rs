//! The Cortex-A57's IMPLEMENTATION DEFINED registers as a guest sees them
//! (R9): what each `MRS` reads out of reset, what each `MSR` keeps and
//! what it leaves, the exception an access takes at each level and under
//! `HCR_EL2.TIDCP` and `ACTLR_EL2`, what `RAMINDEX` fills, `CPUACTLR_EL1`'s
//! WFI and WFE controls, a snapshot of the registers, and the Armv9.2
//! model, which has none of them. Each expected value is the TRM's (DDI
//! 0488H, 4.2.15 and 4.3), worked out here from its tables, not taken from
//! the model's constants.

use std::vec::Vec;

use rusty_box_core::{ClockHz, SnapError, VmClock};

use super::super::el2::HCR_TIDCP;
use super::super::features::Implementation;
use super::super::{Activity, Cpu, CpuConfig, El, Levels, NoEl2, Pstate, Step, SysReg, WithEl2};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const CODE: u64 = 0x1_0000;
const VBAR_EL1: u64 = 0x2_0000;
const VBAR_EL2: u64 = 0x3_0000;

/// One instruction word wherever it is fetched; every other access a bus
/// fault.
struct OneWord(u32);

impl PhysBus for OneWord {
    fn read(
        &mut self,
        _addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        match width {
            AccessWidth::Word => Ok(u64::from(self.0)),
            AccessWidth::Byte | AccessWidth::Half | AccessWidth::Dword => Err(BusFault::Unmapped),
        }
    }

    fn write(
        &mut self,
        _addr: PhysAddr,
        _width: AccessWidth,
        _value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
        Err(BusFault::Unmapped)
    }
}

/// What one instruction did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Did {
    Completed,
    /// It retired into a wait.
    Waits,
    /// It took an exception to `to`, with the `ESR` there.
    Took {
        to: El,
        esr: u64,
    },
}

const IL: u64 = 1 << 25;
/// The UNDEFINED exception's `ESR`: EC 0, IL.
const UNDEFINED_ESR: u64 = IL;

/// A processor implementing `imp` and the levels `L`, out of reset, at
/// `el`, its vector bases set.
fn processor<L: Levels>(imp: Implementation, el: El) -> Cpu<L> {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut cpu = Cpu::<L>::implementing(
        CpuConfig {
            ips,
            reset_pc: CODE,
        },
        Implementation {
            counter_hz: ips,
            ..imp
        },
    );
    cpu.sys.vbar_el1 = VBAR_EL1;
    if let Some(el2) = cpu.el2_mut() {
        el2.vbar = VBAR_EL2;
    }
    at(&mut cpu, el);
    cpu
}

fn at<L: Levels>(cpu: &mut Cpu<L>, el: El) {
    cpu.pstate = Pstate {
        el,
        sp_sel: el != El::El0,
        ..Pstate::RESET
    };
}

/// Step `insn` with X0 = `x0`: what it did, and X0 after it. The processor
/// is put back at the level it executed at.
fn run<L: Levels>(cpu: &mut Cpu<L>, insn: u32, x0: u64) -> (Did, u64) {
    let pstate = cpu.pstate;
    cpu.pc = CODE;
    cpu.x[0] = x0;
    let step = cpu.step(&mut OneWord(insn), &mut NoInstrumentation);
    let did = match step {
        Step::Waiting => Did::Waits,
        Step::Retired if cpu.pc == CODE + 4 => Did::Completed,
        Step::Retired if cpu.pc & !0x7FF == VBAR_EL2 => Did::Took {
            to: El::El2,
            esr: cpu.el2().esr,
        },
        Step::Retired if cpu.pc & !0x7FF == VBAR_EL1 => Did::Took {
            to: El::El1,
            esr: cpu.sys.esr_el1,
        },
        other => panic!("{insn:#010x}: {other:?} to {:#x}", cpu.pc),
    };
    let x0 = cpu.x[0];
    cpu.pstate = pstate;
    cpu.activity = Activity::Running;
    (did, x0)
}

/// A register of the Cortex-A57's space: `S3_<op1>_C<crn>_C<crm>_<op2>`.
#[derive(Debug, Clone, Copy)]
struct Reg {
    name: &'static str,
    op1: u32,
    crn: u32,
    crm: u32,
    op2: u32,
}

impl Reg {
    const fn new(name: &'static str, op1: u32, crn: u32, crm: u32, op2: u32) -> Self {
        Self {
            name,
            op1,
            crn,
            crm,
            op2,
        }
    }

    /// `MRS X0, <reg>`.
    const fn mrs(self) -> u32 {
        0xD538_0000 | (self.op1 << 16) | (self.crn << 12) | (self.crm << 8) | (self.op2 << 5)
    }

    /// `MSR <reg>, X0`.
    const fn msr(self) -> u32 {
        self.mrs() & !(1 << 21)
    }

    /// `ESR_ELx` of a trapped `MRS X0` (`read`) or `MSR <reg>, X0`: EC
    /// 0x18, IL, and the ISS naming op0 3, the encoding, Rt 0 and the
    /// direction (Arm `AArch64_SystemAccessTrapSyndrome`).
    const fn trapped(self, read: bool) -> u64 {
        (0x18 << 26)
            | IL
            | (3 << 20)
            | ((self.op2 as u64) << 17)
            | ((self.op1 as u64) << 14)
            | ((self.crn as u64) << 10)
            | ((self.crm as u64) << 1)
            | read as u64
    }

    /// Its encoding as a host names it.
    fn host_name(self) -> SysReg {
        SysReg::new(
            3,
            self.op1 as u8,
            self.crn as u8,
            self.crm as u8,
            self.op2 as u8,
        )
        .unwrap_or_else(|| panic!("{} is in the system register space", self.name))
    }
}

/// What an `MSR` keeps.
#[derive(Debug, Clone, Copy)]
enum Kept {
    /// The bits of these fields; the rest read as they were.
    Fields(u64),
    /// Nothing: the register reads zero after any write.
    Zeroed,
    /// The register is read-only: its `MSR` is UNDEFINED.
    ReadOnly,
}

/// One register's TRM description: its reset value and what a write keeps.
#[derive(Debug, Clone, Copy)]
struct Described {
    reg: Reg,
    reset: u64,
    kept: Kept,
}

const L2CTLR: Reg = Reg::new("L2CTLR_EL1", 1, 11, 0, 2);
const L2ECTLR: Reg = Reg::new("L2ECTLR_EL1", 1, 11, 0, 3);
const L2ACTLR: Reg = Reg::new("L2ACTLR_EL1", 1, 15, 0, 0);
const CPUACTLR: Reg = Reg::new("CPUACTLR_EL1", 1, 15, 2, 0);
const CPUECTLR: Reg = Reg::new("CPUECTLR_EL1", 1, 15, 2, 1);
const CPUMERRSR: Reg = Reg::new("CPUMERRSR_EL1", 1, 15, 2, 2);
const L2MERRSR: Reg = Reg::new("L2MERRSR_EL1", 1, 15, 2, 3);
const CBAR: Reg = Reg::new("CBAR_EL1", 1, 15, 3, 0);
const IL1DATA: [Reg; 4] = [
    Reg::new("IL1DATA0_EL1", 0, 15, 0, 0),
    Reg::new("IL1DATA1_EL1", 0, 15, 0, 1),
    Reg::new("IL1DATA2_EL1", 0, 15, 0, 2),
    Reg::new("IL1DATA3_EL1", 0, 15, 0, 3),
];
const DL1DATA: [Reg; 5] = [
    Reg::new("DL1DATA0_EL1", 0, 15, 1, 0),
    Reg::new("DL1DATA1_EL1", 0, 15, 1, 1),
    Reg::new("DL1DATA2_EL1", 0, 15, 1, 2),
    Reg::new("DL1DATA3_EL1", 0, 15, 1, 3),
    Reg::new("DL1DATA4_EL1", 0, 15, 1, 4),
];

/// `RAMINDEX`: `SYS #0, C15, C4, #0, X0`.
const RAMINDEX: u32 = 0xD508_F400;
/// Its trapped `ESR_ELx`: EC 0x18, IL, ISS op0 1, op2 0, op1 0, CRn 15,
/// Rt 0, CRm 4, a write.
const RAMINDEX_TRAPPED: u64 = (0x18 << 26) | IL | (1 << 20) | (15 << 10) | (4 << 1);

/// Where the board's own layout has the GIC-400: PERIPHBASE.
const BOARD_PERIPHBASE: u64 = 0x0800_0000;

/// The board's fill of an UNKNOWN 32-bit value: 0xA5A5 above the
/// register's `op0:op1:CRn:CRm:op2`.
const fn fill(reg: Reg) -> u64 {
    0xA5A5_0000
        | (3 << 14)
        | ((reg.op1 as u64) << 11)
        | ((reg.crn as u64) << 7)
        | ((reg.crm as u64) << 3)
        | reg.op2 as u64
}

/// Every register with its TRM description (Table 4-15 and 4.3.58 to
/// 4.3.70), the board configured as `impdef.rs` says: one processor, ECC
/// in the L2 only, no register slices, an ACE interface.
fn described() -> Vec<Described> {
    let mut all = std::vec![
        // Writable: 21, 20, 9, 8:6, 5, 2:0 (Table 4-69).
        Described {
            reg: L2CTLR,
            reset: 0,
            kept: Kept::Fields(0x0030_03E7),
        },
        // Writable: 2:0 (Table 4-70).
        Described {
            reg: L2ECTLR,
            reset: 0,
            kept: Kept::Fields(0x7),
        },
        // RES0: 31:29, 24, 12, 9, 5 (Table 4-76).
        Described {
            reg: L2ACTLR,
            reset: 0x10,
            kept: Kept::Fields(0x1EFF_EDDF),
        },
        // RES0: 62:60, 46:45, 43:40, 35, 6, 2 (Tables 4-77 and 4-78).
        Described {
            reg: CPUACTLR,
            reset: 0,
            kept: Kept::Fields(0x8FFF_90F7_FFFF_FFBB),
        },
        // Fields: 38, 36:35, 33:32, 6, 2:0 (Table 4-79).
        Described {
            reg: CPUECTLR,
            reset: 0x1B_0000_0000,
            kept: Kept::Fields(0x5B_0000_0047),
        },
        Described {
            reg: CPUMERRSR,
            reset: 0,
            kept: Kept::Zeroed,
        },
        Described {
            reg: L2MERRSR,
            reset: 0,
            kept: Kept::Zeroed,
        },
        Described {
            reg: CBAR,
            reset: BOARD_PERIPHBASE,
            kept: Kept::ReadOnly,
        },
    ];
    for reg in IL1DATA.into_iter().chain(DL1DATA) {
        all.push(Described {
            reg,
            reset: fill(reg),
            kept: Kept::Fields(0xFFFF_FFFF),
        });
    }
    all
}

/// Every register reads its TRM reset value, at EL1 and at EL2; `CBAR_EL1`
/// reads PERIPHBASE, where the board's layout has the GIC-400, and the
/// UNKNOWN data registers the board's fill. A host reads the same.
#[test]
fn each_register_reads_its_reset_value() {
    fn check<L: Levels>(el: El) {
        let mut cpu = processor::<L>(Implementation::CORTEX_A57, el);
        for d in described() {
            let read = run(&mut cpu, d.reg.mrs(), 0xDEAD);
            assert_eq!(read, (Did::Completed, d.reset), "{} at {el:?}", d.reg.name);
            assert_eq!(cpu.sysreg(d.reg.host_name()), Ok(d.reset), "{}", d.reg.name);
        }
    }
    check::<NoEl2>(El::El1);
    check::<WithEl2>(El::El2);
    check::<WithEl2>(El::El1);
}

/// A write keeps the bits of the register's fields and leaves its
/// read-only fields as they are: all ones and then all zeros written, each
/// field reads back as written, each RES0 bit as 0 and `L2CTLR_EL1`'s
/// read-only fields — one processor (25:24), no L1 ECC (22), no slices
/// (13:10), L2RSTDISABLE (31) — as configured; the syndrome registers read
/// zero after any write; `CBAR_EL1`'s `MSR` is UNDEFINED and leaves it.
#[test]
fn each_write_keeps_the_fields_and_leaves_the_rest() {
    fn check<L: Levels>(el: El) {
        let mut cpu = processor::<L>(Implementation::CORTEX_A57, el);
        for d in described() {
            let name = d.reg.name;
            match d.kept {
                Kept::Fields(fields) => {
                    let fixed = d.reset & !fields;
                    for value in [u64::MAX, 0] {
                        assert_eq!(
                            run(&mut cpu, d.reg.msr(), value).0,
                            Did::Completed,
                            "{name}"
                        );
                        let read = run(&mut cpu, d.reg.mrs(), 0).1;
                        assert_eq!(read, fixed | (value & fields), "{name} after {value:#x}");
                    }
                }
                Kept::Zeroed => {
                    assert_eq!(
                        run(&mut cpu, d.reg.msr(), u64::MAX).0,
                        Did::Completed,
                        "{name}"
                    );
                    assert_eq!(run(&mut cpu, d.reg.mrs(), 1).1, 0, "{name}");
                }
                Kept::ReadOnly => {
                    // Taken at the level it was made at, EL1 or EL2.
                    let undefined = Did::Took {
                        to: el,
                        esr: UNDEFINED_ESR,
                    };
                    assert_eq!(run(&mut cpu, d.reg.msr(), 0).0, undefined, "{name}");
                    assert_eq!(run(&mut cpu, d.reg.mrs(), 0).1, d.reset, "{name}");
                }
            }
        }
        let l2ctlr = run(&mut cpu, L2CTLR.msr(), u64::MAX);
        assert_eq!(l2ctlr.0, Did::Completed);
        let read = run(&mut cpu, L2CTLR.mrs(), 0).1;
        assert_eq!(
            read & ((1 << 31) | (0b11 << 24) | (1 << 22) | (0xF << 10)),
            0
        );
    }
    check::<NoEl2>(El::El1);
    check::<WithEl2>(El::El2);
}

/// EL0 has none of them (Table 4-15's "-"): each `MRS`, `MSR` and
/// `RAMINDEX` is UNDEFINED, taken to EL1.
#[test]
fn el0s_accesses_are_undefined() {
    let mut cpu = processor::<NoEl2>(Implementation::CORTEX_A57, El::El0);
    let undefined = Did::Took {
        to: El::El1,
        esr: UNDEFINED_ESR,
    };
    for d in described() {
        assert_eq!(run(&mut cpu, d.reg.mrs(), 0).0, undefined, "{}", d.reg.name);
        assert_eq!(run(&mut cpu, d.reg.msr(), 0).0, undefined, "{}", d.reg.name);
    }
    assert_eq!(run(&mut cpu, RAMINDEX, 0).0, undefined);
}

/// The Armv9.2 model has none of the Cortex-A57's registers: every `MRS`,
/// `MSR` and `RAMINDEX` of them is UNDEFINED at EL1, and its `ACTLR_EL1`
/// keeps all 64 bits, as the space's accessors leave it with nothing
/// implemented.
#[test]
fn the_armv9p2_model_has_none_of_them() {
    let mut cpu = processor::<NoEl2>(Implementation::ARMV9P2, El::El1);
    let undefined = Did::Took {
        to: El::El1,
        esr: UNDEFINED_ESR,
    };
    for d in described() {
        assert_eq!(run(&mut cpu, d.reg.mrs(), 0).0, undefined, "{}", d.reg.name);
        assert_eq!(run(&mut cpu, d.reg.msr(), 0).0, undefined, "{}", d.reg.name);
    }
    assert_eq!(run(&mut cpu, RAMINDEX, 0).0, undefined);
    const MSR_ACTLR_EL1: u32 = 0xD518_1020; // msr actlr_el1, x0
    const MRS_ACTLR_EL1: u32 = 0xD538_1020; // mrs x0, actlr_el1
    assert_eq!(run(&mut cpu, MSR_ACTLR_EL1, u64::MAX).0, Did::Completed);
    assert_eq!(run(&mut cpu, MRS_ACTLR_EL1, 0).1, u64::MAX);
}

/// `HCR_EL2.TIDCP` traps each EL1 access to EL2, EC 0x18 with the ISS
/// naming the encoding and direction, `RAMINDEX` and an encoding the
/// processor gives nothing included; EL2's are not trapped, and EL0's stay
/// UNDEFINED at EL1.
#[test]
fn tidcp_traps_el1s_accesses_to_el2() {
    let mut cpu = processor::<WithEl2>(Implementation::CORTEX_A57, El::El1);
    cpu.el2_state.regs.hcr |= HCR_TIDCP;
    let unimplemented = Reg::new("S3_1_C15_C7_7", 1, 15, 7, 7);
    let mut regs: Vec<Reg> = described().into_iter().map(|d| d.reg).collect();
    regs.push(unimplemented);
    for reg in &regs {
        for (insn, read) in [(reg.mrs(), true), (reg.msr(), false)] {
            let want = Did::Took {
                to: El::El2,
                esr: reg.trapped(read),
            };
            assert_eq!(run(&mut cpu, insn, 0).0, want, "{} read {read}", reg.name);
        }
    }
    let ramindex = Did::Took {
        to: El::El2,
        esr: RAMINDEX_TRAPPED,
    };
    assert_eq!(run(&mut cpu, RAMINDEX, 0).0, ramindex);

    at(&mut cpu, El::El2);
    for d in described() {
        assert_eq!(
            run(&mut cpu, d.reg.mrs(), 0),
            (Did::Completed, d.reset),
            "{}",
            d.reg.name
        );
    }
    assert_eq!(run(&mut cpu, RAMINDEX, 0).0, Did::Completed);
    let undefined = Did::Took {
        to: El::El2,
        esr: UNDEFINED_ESR,
    };
    assert_eq!(run(&mut cpu, unimplemented.mrs(), 0).0, undefined);

    at(&mut cpu, El::El0);
    let undefined = Did::Took {
        to: El::El1,
        esr: UNDEFINED_ESR,
    };
    for reg in &regs {
        assert_eq!(run(&mut cpu, reg.mrs(), 0).0, undefined, "{}", reg.name);
    }
}

/// `ACTLR_EL2` decides EL1's writes of `CPUACTLR_EL1` (bit 0),
/// `CPUECTLR_EL1` (1), `L2CTLR_EL1` (4), `L2ECTLR_EL1` (5) and
/// `L2ACTLR_EL1` (6) (Table 4-46): with its bit clear, as out of reset, a
/// write traps to EL2 with EC 0x18 and changes nothing; with it set, the
/// write is kept. Reads, and writes of the registers it does not control,
/// go through either way; EL2's writes need no bit (with no EL3, each
/// `ACTLR_EL3` bit is 1).
#[test]
fn actlr_el2_decides_el1s_writes() {
    let controlled = [
        (CPUACTLR, 1 << 0, 1 << 8),
        (CPUECTLR, 1 << 1, 1 << 6),
        (L2CTLR, 1 << 4, 1 << 21),
        (L2ECTLR, 1 << 5, 0b101),
        (L2ACTLR, 1 << 6, 1 << 28),
    ];
    let mut cpu = processor::<WithEl2>(Implementation::CORTEX_A57, El::El1);
    assert_eq!(cpu.el2_state.regs.actlr, 0);
    for (reg, _, field) in controlled {
        let before = run(&mut cpu, reg.mrs(), 0).1;
        let want = Did::Took {
            to: El::El2,
            esr: reg.trapped(false),
        };
        assert_eq!(
            run(&mut cpu, reg.msr(), before ^ field).0,
            want,
            "{}",
            reg.name
        );
        assert_eq!(run(&mut cpu, reg.mrs(), 0).1, before, "{}", reg.name);
    }
    for reg in [CPUMERRSR, L2MERRSR, IL1DATA[0], DL1DATA[4]] {
        assert_eq!(
            run(&mut cpu, reg.msr(), 0).0,
            Did::Completed,
            "{}",
            reg.name
        );
    }
    for (reg, bit, field) in controlled {
        cpu.el2_state.regs.actlr = bit;
        let before = run(&mut cpu, reg.mrs(), 0).1;
        assert_eq!(
            run(&mut cpu, reg.msr(), before ^ field).0,
            Did::Completed,
            "{}",
            reg.name
        );
        assert_eq!(
            run(&mut cpu, reg.mrs(), 0).1,
            before ^ field,
            "{}",
            reg.name
        );
    }
    cpu.el2_state.regs.actlr = 0;
    at(&mut cpu, El::El2);
    for (reg, _, field) in controlled {
        let before = run(&mut cpu, reg.mrs(), 0).1;
        assert_eq!(
            run(&mut cpu, reg.msr(), before ^ field).0,
            Did::Completed,
            "{}",
            reg.name
        );
        assert_eq!(
            run(&mut cpu, reg.mrs(), 0).1,
            before ^ field,
            "{}",
            reg.name
        );
    }
}

/// The architectural registers whose contents Arm leaves to the
/// implementation, as the Cortex-A57's TRM gives them: `ACTLR_EL2` keeps
/// its five access controls, bits 6:4 and 1:0 (Table 4-46); `ACTLR_EL1`,
/// `AFSR0_EL1`, `AFSR1_EL1`, `AMAIR_EL1`, `HACR_EL2`, `AFSR0_EL2`,
/// `AFSR1_EL2` and `AMAIR_EL2` are RES0 and read zero after all ones are
/// written. The Armv9.2 model's keep all 64 bits.
#[test]
fn the_auxiliary_registers_are_the_trms() {
    let actlr_el2 = Reg::new("ACTLR_EL2", 4, 1, 0, 1);
    let res0 = [
        Reg::new("ACTLR_EL1", 0, 1, 0, 1),
        Reg::new("AFSR0_EL1", 0, 5, 1, 0),
        Reg::new("AFSR1_EL1", 0, 5, 1, 1),
        Reg::new("AMAIR_EL1", 0, 10, 3, 0),
        Reg::new("HACR_EL2", 4, 1, 1, 7),
        Reg::new("AFSR0_EL2", 4, 5, 1, 0),
        Reg::new("AFSR1_EL2", 4, 5, 1, 1),
        Reg::new("AMAIR_EL2", 4, 10, 3, 0),
    ];
    for (imp, actlr_kept, res0_kept) in [
        (Implementation::CORTEX_A57, 0x73, 0),
        (Implementation::ARMV9P2, u64::MAX, u64::MAX),
    ] {
        let mut cpu = processor::<WithEl2>(imp, El::El2);
        assert_eq!(run(&mut cpu, actlr_el2.msr(), u64::MAX).0, Did::Completed);
        assert_eq!(run(&mut cpu, actlr_el2.mrs(), 0).1, actlr_kept);
        for reg in res0 {
            assert_eq!(
                run(&mut cpu, reg.msr(), u64::MAX).0,
                Did::Completed,
                "{}",
                reg.name
            );
            assert_eq!(run(&mut cpu, reg.mrs(), 0).1, res0_kept, "{}", reg.name);
        }
    }
}

/// `RAMINDEX` reads RAM entries the model holds none of, every one invalid,
/// which in Non-secure EL1 and EL2 fills the data registers with zeros
/// (4.3.64): an L1-I RAM (`RAMID` 0x00 to 0x05) zeroes `IL1DATAn_EL1` and
/// leaves `DL1DATAn_EL1`, an L1-D or L2 RAM (0x08 to 0x0A, 0x10 to 0x14,
/// 0x18) the reverse, and a reserved `RAMID` zeroes both.
#[test]
fn ramindex_reads_every_entry_invalid() {
    let mut cpu = processor::<NoEl2>(Implementation::CORTEX_A57, El::El1);
    let fill_all = |cpu: &mut Cpu<NoEl2>| {
        for (i, reg) in IL1DATA.into_iter().chain(DL1DATA).enumerate() {
            assert_eq!(run(cpu, reg.msr(), 0x100 + i as u64).0, Did::Completed);
        }
    };
    let reads = |cpu: &mut Cpu<NoEl2>, regs: &[Reg]| -> Vec<u64> {
        regs.iter().map(|reg| run(cpu, reg.mrs(), 0).1).collect()
    };
    let filled_il1: Vec<u64> = (0x100..0x104).collect();
    let filled_dl1: Vec<u64> = (0x104..0x109).collect();
    for ramid in [0x00u64, 0x01, 0x02, 0x03, 0x04, 0x05] {
        fill_all(&mut cpu);
        let operand = (ramid << 24) | 0x0D80;
        assert_eq!(run(&mut cpu, RAMINDEX, operand).0, Did::Completed);
        assert_eq!(reads(&mut cpu, &IL1DATA), [0; 4], "RAMID {ramid:#x}");
        assert_eq!(reads(&mut cpu, &DL1DATA), filled_dl1, "RAMID {ramid:#x}");
    }
    for ramid in [0x08u64, 0x09, 0x0A, 0x10, 0x11, 0x12, 0x13, 0x14, 0x18] {
        fill_all(&mut cpu);
        assert_eq!(run(&mut cpu, RAMINDEX, ramid << 24).0, Did::Completed);
        assert_eq!(reads(&mut cpu, &IL1DATA), filled_il1, "RAMID {ramid:#x}");
        assert_eq!(reads(&mut cpu, &DL1DATA), [0; 5], "RAMID {ramid:#x}");
    }
    for ramid in [0x06u64, 0x07, 0x0B, 0x0F, 0x15, 0x17, 0x19, 0xFF] {
        fill_all(&mut cpu);
        assert_eq!(run(&mut cpu, RAMINDEX, ramid << 24).0, Did::Completed);
        assert_eq!(reads(&mut cpu, &IL1DATA), [0; 4], "RAMID {ramid:#x}");
        assert_eq!(reads(&mut cpu, &DL1DATA), [0; 5], "RAMID {ramid:#x}");
    }
}

/// `CPUACTLR_EL1` bit 8 makes `WFI` a NOP and bit 7 `WFE` (Table 4-78):
/// neither waits, `WFE` leaves the Event Register as it is, and neither
/// traps at EL0 where `SCTLR_EL1.nTWI` or `nTWE` clear would trap a wait.
/// With the bits clear, as out of reset, both wait as Arm's `Hint_WFI`
/// and `Hint_WFE` do.
#[test]
fn cpuactlr_makes_wfi_and_wfe_nops() {
    const WFI: u32 = 0xD503_207F;
    const WFE: u32 = 0xD503_205F;
    const NTWI: u64 = 1 << 16;
    const NTWE: u64 = 1 << 18;
    let mut cpu = processor::<NoEl2>(Implementation::CORTEX_A57, El::El1);
    assert_eq!(run(&mut cpu, WFI, 0).0, Did::Waits);
    assert_eq!(run(&mut cpu, WFE, 0).0, Did::Waits);

    assert_eq!(
        run(&mut cpu, CPUACTLR.msr(), (1 << 8) | (1 << 7)).0,
        Did::Completed
    );
    assert_eq!(run(&mut cpu, WFI, 0).0, Did::Completed);
    assert_eq!(run(&mut cpu, WFE, 0).0, Did::Completed);
    cpu.send_event();
    assert_eq!(run(&mut cpu, WFE, 0).0, Did::Completed);
    assert!(cpu.event_register(), "a NOP leaves the Event Register set");

    cpu.sys.sctlr_el1 &= !(NTWI | NTWE);
    at(&mut cpu, El::El0);
    assert_eq!(run(&mut cpu, WFI, 0).0, Did::Completed);
    assert_eq!(run(&mut cpu, WFE, 0).0, Did::Completed);
}

/// A snapshot carries the registers' values, and a restore brings them
/// back; an image whose `CPUACTLR_EL1` holds a RES0 bit is refused, and
/// the registers stay as they were.
#[test]
fn a_snapshot_keeps_the_registers() {
    let mut cpu = processor::<NoEl2>(Implementation::CORTEX_A57, El::El1);
    let written = [
        (L2CTLR, 0x0030_0007),
        (L2ECTLR, 0x5),
        (L2ACTLR, 0x1000_0011),
        (CPUACTLR, 0x8000_0000_0000_0180),
        (CPUECTLR, 0x40_0000_0041),
        (IL1DATA[2], 0x1234_5678),
        (DL1DATA[3], 0x9ABC_DEF0),
    ];
    for (reg, value) in written {
        assert_eq!(
            run(&mut cpu, reg.msr(), value).0,
            Did::Completed,
            "{}",
            reg.name
        );
    }
    let mut image = Vec::new();
    assert_eq!(cpu.save(&mut image), Ok(()));

    let mut restored = processor::<NoEl2>(Implementation::CORTEX_A57, El::El1);
    assert_eq!(restored.restore(&mut &image[..]), Ok(()));
    for (reg, value) in written {
        assert_eq!(run(&mut restored, reg.mrs(), 0).1, value, "{}", reg.name);
    }
    assert_eq!(run(&mut restored, CBAR.mrs(), 0).1, BOARD_PERIPHBASE);

    // The registers end the image: five doublewords, then nine words;
    // `CPUACTLR_EL1` is the fourth doubleword.
    let at = image.len() - (5 * 8 + 9 * 4) + 3 * 8;
    image[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut refused = processor::<NoEl2>(Implementation::CORTEX_A57, El::El1);
    assert!(matches!(
        refused.restore(&mut &image[..]),
        Err(SnapError::Invalid(_))
    ));
    assert_eq!(run(&mut refused, CPUACTLR.mrs(), 0).1, 0);
}

/// `CBAR_EL1` reads the PERIPHBASE input the processor is built with, its
/// bits 43:18 (Table 4-82).
#[test]
fn cbar_reads_the_periphbase_input() {
    for (periphbase, cbar) in [
        (0x0800_0000u64, 0x0800_0000u64),
        (0x2C00_0000, 0x2C00_0000),
        (0x0F00_3FFF_FFFF, 0x0F00_3FFC_0000),
        (0xFFFF_F000_0000_0000, 0),
    ] {
        let imp = Implementation::CORTEX_A57.with_periphbase(PhysAddr::new(periphbase));
        let mut cpu = processor::<NoEl2>(imp, El::El1);
        assert_eq!(run(&mut cpu, CBAR.mrs(), 0).1, cbar, "{periphbase:#x}");
    }
}
