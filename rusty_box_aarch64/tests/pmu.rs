//! What a guest sees of the PMU (FEAT_PMUv3), which V-BSA requires of a
//! virtual environment's processing element (DEN0150 RV_L1PE_02), on both
//! processors (R9):
//!
//! - `PMCR_EL0`, `PMCEID0_EL0`, `PMCEID1_EL0` and `ID_AA64DFR0_EL1.PMUVer`
//!   say which PMU it is.
//! - The cycle counter and the event counters count the instructions the
//!   processor runs, one cycle each: `INST_RETIRED` leaves out an
//!   instruction that took an exception in its place, `CPU_CYCLES` does
//!   not, and `EXC_TAKEN` and `EXC_RETURN` count the exception and its
//!   return.
//! - The filters of `PMEVTYPER<n>_EL0` and `PMCCFILTR_EL0` hold: a counter
//!   counts at the levels it is set to, and an exception counts at the
//!   level it was taken from.
//! - An overflow with its interrupt enabled is PMUIRQ, PPI 23, pending at
//!   the distributor and acknowledged as INTID 23, until the guest clears
//!   the overflow flag.
//! - EL0 reaches the PMU only as `PMUSERENR_EL0` allows: without it, an
//!   access traps to EL1.
//!
//! Each guest is a few instructions at the base of RAM, where a bare
//! machine starts at EL1, built here from the encodings of `MRS`, `MSR` and
//! a few fixed words; the comments give their assembly.

use rusty_box_aarch64::board::{GIC_CPU_BASE, GIC_DIST_BASE, PPI_PMU, RAM_BASE};
use rusty_box_aarch64::{
    CpuModel, LentStorage, MachineBuilder, MemoryLoan, NoEl2, RamSize, Reg, StopReason, SysReg,
    VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 1 << 20;
/// The vector table, and the code EL0 runs.
const VBAR: u64 = RAM_BASE + 0x800;
const EL0_CODE: u64 = RAM_BASE + 0x2000;
/// The vectors of an exception from the current level with `SP_ELx`, and
/// from a lower level using AArch64.
const SAME_LEVEL: u64 = VBAR + 0x200;
const LOWER_LEVEL: u64 = VBAR + 0x400;

const NOP: u32 = 0xD503_201F;
const UDF: u32 = 0x0000_0000;
const ERET: u32 = 0xD69F_03E0;
const SVC: u32 = 0xD400_0001;
/// `b .`
const SPIN: u32 = 0x1400_0000;
/// The handler that returns past the instruction that raised its
/// exception: `mrs x20, elr_el1`, `add x20, x20, #4`, `msr elr_el1, x20`,
/// `eret`.
const RETURN_PAST: [u32; 4] = [0xD538_4034, 0x9100_1294, 0xD518_4034, ERET];

/// The event numbers the tests count.
const INST_RETIRED: u64 = 0x08;
const EXC_TAKEN: u64 = 0x09;
const EXC_RETURN: u64 = 0x0A;
const CPU_CYCLES: u64 = 0x11;
/// The filter bits: `P` keeps a counter from counting at EL1, `U` at EL0.
const FILTER_P: u64 = 1 << 31;
const FILTER_U: u64 = 1 << 30;
/// `PMCNTENSET_EL0.C` and its siblings' cycle counter bit; `PMCR_EL0.E`.
const CYCLES: u64 = 1 << 31;
const PMCR_E: u64 = 1;

fn sysreg(op0: u8, op1: u8, crn: u8, crm: u8, op2: u8) -> SysReg {
    SysReg::new(op0, op1, crn, crm, op2).expect("a system register encoding")
}

fn pmevcntr(n: u8) -> SysReg {
    sysreg(3, 3, 14, 8 | (n >> 3), n & 7)
}

fn pmevtyper(n: u8) -> SysReg {
    sysreg(3, 3, 14, 12 | (n >> 3), n & 7)
}

fn pmcntenclr() -> SysReg {
    sysreg(3, 3, 9, 12, 2)
}

fn pmovsclr() -> SysReg {
    sysreg(3, 3, 9, 12, 3)
}

fn pmceid(n: u8) -> SysReg {
    sysreg(3, 3, 9, 12, 6 + n)
}

/// `mrs x<rt>, reg`.
fn mrs(rt: u32, reg: SysReg) -> u32 {
    0xD520_0000 | (u32::from(reg.encoding()) << 5) | rt
}

/// `msr reg, x<rt>`; `rt` 31 writes zero.
fn msr(reg: SysReg, rt: u32) -> u32 {
    0xD500_0000 | (u32::from(reg.encoding()) << 5) | rt
}

const XZR: u32 = 31;

/// A bare machine with `cpu`, the code `blocks` placed (each at its
/// address), the registers `regs` set, run from the base of RAM until it
/// reaches `until`; then `check` the machine.
fn run<F>(cpu: CpuModel, blocks: &[(u64, &[u32])], regs: &[(Reg, u64)], until: u64, check: F)
where
    F: FnOnce(&mut VirtMachine<LentStorage<'_>, NoEl2>),
{
    let mut ram = vec![0u8; RAM_LEN];
    let mut machine = MachineBuilder::new(VirtConfig {
        cpu,
        ..VirtConfig::DEFAULT
    })
    .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
    .bare()
    .build_in(MemoryLoan {
        ram: &mut ram,
        vars: &mut [],
    })
    .expect("the test board builds");
    for &(at, words) in blocks {
        for (i, word) in words.iter().enumerate() {
            machine
                .mem_write(at + 4 * i as u64, &word.to_le_bytes())
                .expect("the program is in RAM");
        }
    }
    for &(reg, value) in regs {
        machine.reg_write(reg, value);
    }
    let out = machine.emu_start(RAM_BASE, Some(until), Some(10_000));
    assert_eq!(
        out.stop,
        StopReason::ExitReached,
        "{cpu}: the guest reached {until:#x}; it stopped at {:#x}",
        machine.reg_read(Reg::Pc)
    );
    check(&mut machine);
}

/// `ID_AA64DFR0_EL1.PMUVer` names the PMU's revision, `PMCR_EL0` its six
/// event counters (`N`) and, on the Cortex-A57, its maker and part (`IMP`
/// 0x41, `IDCODE` 0x01, DDI 0488H 11.4.1), which FEAT_PMUv3p7 makes read
/// as zero on the Armv9.2 model; `LC` reads one, there being no AArch32.
/// `PMCEID0_EL0` and `PMCEID1_EL0` name the events counted: `SW_INCR`,
/// `INST_RETIRED`, `EXC_TAKEN`, `EXC_RETURN`, `CID_WRITE_RETIRED`,
/// `CPU_CYCLES`, `TTBR_WRITE_RETIRED` and `CHAIN`, and on the Armv9.2 model
/// the stall events FEAT_PMUv3p1 and FEAT_PMUv3p4 require.
#[test]
fn each_processor_names_its_pmu() {
    let program = [
        mrs(9, SysReg::PMCR_EL0),
        mrs(10, pmceid(0)),
        mrs(11, pmceid(1)),
        mrs(12, sysreg(3, 0, 0, 5, 0)),
        SPIN,
    ];
    let spin = RAM_BASE + 16;
    for (cpu, pmcr, pmuver, ceid1) in [
        (CpuModel::CortexA57, 0x4101_3040, 1, 0),
        (CpuModel::Armv9p2, 0x0000_3040, 7, 0x1000_0018),
    ] {
        run(cpu, &[(RAM_BASE, &program)], &[], spin, |m| {
            let pmcr_read = m.reg_read(Reg::X9);
            assert_eq!(pmcr_read, pmcr, "{cpu}: PMCR_EL0");
            assert_eq!((pmcr_read >> 11) & 0x1F, 6, "{cpu}: six event counters");
            assert_eq!(m.reg_read(Reg::X10), 0x5002_0F01, "{cpu}: PMCEID0_EL0");
            assert_eq!(m.reg_read(Reg::X11), ceid1, "{cpu}: PMCEID1_EL0");
            assert_eq!((m.reg_read(Reg::X12) >> 8) & 0xF, pmuver, "{cpu}: PMUVer");
        });
    }
}

/// From the `MSR` that sets `PMCR_EL0.E` to the one that clears the
/// counters' enables, the cycle counter and `CPU_CYCLES` count every
/// instruction run: the `MSR` that started them, ten `NOP`s, a `UDF`, the
/// four instructions of the handler that returns past it, and two
/// `NOP`s, 18. `INST_RETIRED` leaves out the `UDF`, which took an exception
/// in its place, 17; `EXC_TAKEN` and `EXC_RETURN` count its exception and
/// the handler's `ERET`.
#[test]
fn the_counters_count_the_instructions_run() {
    let mut program = vec![
        msr(sysreg(3, 0, 12, 0, 0), 5), // msr vbar_el1, x5
        msr(pmevtyper(0), 1),
        msr(pmevtyper(1), 2),
        msr(pmevtyper(2), 3),
        msr(pmevtyper(3), 4),
        msr(SysReg::PMCCNTR_EL0, XZR),
        msr(pmevcntr(0), XZR),
        msr(pmevcntr(1), XZR),
        msr(pmevcntr(2), XZR),
        msr(pmevcntr(3), XZR),
        msr(SysReg::PMCNTENSET_EL0, 6),
        msr(SysReg::PMCR_EL0, 7),
    ];
    program.extend([NOP; 10]);
    program.push(UDF);
    program.extend([NOP; 2]);
    program.extend([
        msr(pmcntenclr(), 6),
        mrs(9, SysReg::PMCCNTR_EL0),
        mrs(10, pmevcntr(0)),
        mrs(11, pmevcntr(1)),
        mrs(12, pmevcntr(2)),
        mrs(13, pmevcntr(3)),
        SPIN,
    ]);
    let spin = RAM_BASE + 4 * (program.len() as u64 - 1);
    for cpu in CpuModel::ALL {
        run(
            cpu,
            &[(RAM_BASE, &program), (SAME_LEVEL, &RETURN_PAST)],
            &[
                (Reg::X1, INST_RETIRED),
                (Reg::X2, CPU_CYCLES),
                (Reg::X3, EXC_TAKEN),
                (Reg::X4, EXC_RETURN),
                (Reg::X5, VBAR),
                (Reg::X6, CYCLES | 0xF),
                (Reg::X7, PMCR_E),
            ],
            spin,
            |m| {
                assert_eq!(m.reg_read(Reg::X9), 18, "{cpu}: PMCCNTR_EL0");
                assert_eq!(m.reg_read(Reg::X10), 17, "{cpu}: INST_RETIRED");
                assert_eq!(m.reg_read(Reg::X11), 18, "{cpu}: CPU_CYCLES");
                assert_eq!(m.reg_read(Reg::X12), 1, "{cpu}: EXC_TAKEN");
                assert_eq!(m.reg_read(Reg::X13), 1, "{cpu}: EXC_RETURN");
            },
        );
    }
}

/// A counter counts only at the levels its filter leaves it: EL1 runs four
/// instructions with the counters on (the `MSR` that starts them, two
/// `MSR`s and an `ERET` to EL0), EL0 six (five `NOP`s and an `SVC`), and
/// the `SVC`'s exception is taken from EL0 — counted by the `EXC_TAKEN`
/// counter that counts at EL0, not by the one that counts at EL1.
#[test]
fn the_filters_count_only_the_levels_they_leave() {
    let program = [
        msr(sysreg(3, 0, 12, 0, 0), 5), // msr vbar_el1, x5
        msr(pmevtyper(0), 1),           // INST_RETIRED, not at EL0
        msr(pmevtyper(1), 2),           // INST_RETIRED, not at EL1
        msr(pmevtyper(2), 3),           // EXC_TAKEN, not at EL0
        msr(pmevtyper(3), 4),           // EXC_TAKEN, not at EL1
        msr(SysReg::PMCCFILTR_EL0, 8),  // cycles, not at EL1
        msr(SysReg::PMCCNTR_EL0, XZR),
        msr(pmevcntr(0), XZR),
        msr(pmevcntr(1), XZR),
        msr(pmevcntr(2), XZR),
        msr(pmevcntr(3), XZR),
        msr(SysReg::PMCNTENSET_EL0, 6),
        msr(SysReg::PMCR_EL0, 7),
        msr(SysReg::SPSR_EL1, XZR), // EL0t
        msr(SysReg::ELR_EL1, 9),    // EL0's code
        ERET,
    ];
    let el0 = [NOP, NOP, NOP, NOP, NOP, SVC];
    let handler = [
        msr(pmcntenclr(), 6),
        mrs(10, pmevcntr(0)),
        mrs(11, pmevcntr(1)),
        mrs(12, pmevcntr(2)),
        mrs(13, pmevcntr(3)),
        mrs(14, SysReg::PMCCNTR_EL0),
        SPIN,
    ];
    let spin = LOWER_LEVEL + 4 * (handler.len() as u64 - 1);
    for cpu in CpuModel::ALL {
        run(
            cpu,
            &[
                (RAM_BASE, &program),
                (EL0_CODE, &el0),
                (LOWER_LEVEL, &handler),
            ],
            &[
                (Reg::X1, INST_RETIRED | FILTER_U),
                (Reg::X2, INST_RETIRED | FILTER_P),
                (Reg::X3, EXC_TAKEN | FILTER_U),
                (Reg::X4, EXC_TAKEN | FILTER_P),
                (Reg::X5, VBAR),
                (Reg::X6, CYCLES | 0xF),
                (Reg::X7, PMCR_E),
                (Reg::X8, FILTER_P),
                (Reg::X9, EL0_CODE),
            ],
            spin,
            |m| {
                assert_eq!(m.reg_read(Reg::X10), 4, "{cpu}: EL1's instructions");
                assert_eq!(m.reg_read(Reg::X11), 6, "{cpu}: EL0's instructions");
                assert_eq!(m.reg_read(Reg::X12), 0, "{cpu}: exceptions from EL1");
                assert_eq!(m.reg_read(Reg::X13), 1, "{cpu}: exceptions from EL0");
                assert_eq!(m.reg_read(Reg::X14), 6, "{cpu}: EL0's cycles");
            },
        );
    }
}

/// An event counter that overflows with its interrupt enabled raises
/// PMUIRQ: PPI 23 is pending at the distributor, the CPU interface signals
/// it as INTID 23, `PMOVSSET_EL0` names the counter, and clearing the
/// overflow flag withdraws it. The counter, counting `INST_RETIRED` from
/// 0xFFFF_FFFC, overflows bits 31:0 at the fourth instruction it counts,
/// the third `NOP` after the `MSR` that starts it.
#[test]
fn an_overflow_raises_ppi_23() {
    let mut program = vec![
        0xB900_0022, // str w2, [x1]           GICD_CTLR: enabled
        0xB901_0023, // str w3, [x1, #0x100]   GICD_ISENABLER0: PPI 23
        0xB900_0082, // str w2, [x4]           GICC_CTLR: enabled
        0xB900_0485, // str w5, [x4, #4]       GICC_PMR: every priority
        msr(pmevtyper(0), 9),
        msr(pmevcntr(0), 6),
        msr(SysReg::PMINTENSET_EL1, 7),
        msr(SysReg::PMCNTENSET_EL0, 7),
        0xB942_002A, // ldr w10, [x1, #0x200]  GICD_ISPENDR0, before
        msr(SysReg::PMCR_EL0, 8),
    ];
    program.extend([NOP; 6]);
    program.extend([
        0xB942_0029, // ldr w9, [x1, #0x200]   GICD_ISPENDR0
        0xB940_0C8B, // ldr w11, [x4, #0xc]    GICC_IAR
        mrs(12, SysReg::PMOVSSET_EL0),
        msr(pmovsclr(), 7),
        0xB900_108B, // str w11, [x4, #0x10]   GICC_EOIR
        0xB942_002D, // ldr w13, [x1, #0x200]  GICD_ISPENDR0, after
        SPIN,
    ]);
    let spin = RAM_BASE + 4 * (program.len() as u64 - 1);
    let ppi = 1u64 << PPI_PMU;
    assert_eq!(PPI_PMU, 23, "BSA's recommended INTID");
    for cpu in CpuModel::ALL {
        run(
            cpu,
            &[(RAM_BASE, &program)],
            &[
                (Reg::X1, GIC_DIST_BASE),
                (Reg::X2, 1),
                (Reg::X3, ppi),
                (Reg::X4, GIC_CPU_BASE),
                (Reg::X5, 0xFF),
                (Reg::X6, 0xFFFF_FFFC),
                (Reg::X7, 1),
                (Reg::X8, PMCR_E),
                (Reg::X9, INST_RETIRED),
            ],
            spin,
            |m| {
                assert_eq!(m.reg_read(Reg::X10) & ppi, 0, "{cpu}: nothing before");
                assert_eq!(m.reg_read(Reg::X9) & ppi, ppi, "{cpu}: pending");
                assert_eq!(m.reg_read(Reg::X11), 23, "{cpu}: acknowledged as INTID 23");
                assert_eq!(m.reg_read(Reg::X12), 1, "{cpu}: counter 0 overflowed");
                assert_eq!(
                    m.reg_read(Reg::X13) & ppi,
                    0,
                    "{cpu}: withdrawn with the overflow flag"
                );
            },
        );
    }
}

/// The processor's clock stops while it waits: across a `WFI` that sleeps
/// until the virtual timer fires 100,000 counts later, the cycle counter
/// counts the three instructions run (the `MSR` that starts it, the `WFI`
/// and the read of `CNTVCT_EL0`), while the generic timer's count goes on
/// through the wait.
#[test]
fn the_cycle_counter_stops_while_the_processor_waits() {
    let program = [
        0xB900_0022, // str w2, [x1]           GICD_CTLR: enabled
        0xB901_0023, // str w3, [x1, #0x100]   GICD_ISENABLER0: PPI 27
        0xB900_0082, // str w2, [x4]           GICC_CTLR: enabled
        0xB900_0485, // str w5, [x4, #4]       GICC_PMR: every priority
        msr(SysReg::PMCCNTR_EL0, XZR),
        msr(SysReg::PMCNTENSET_EL0, 6),
        mrs(9, SysReg::CNTVCT_EL0),
        msr(sysreg(3, 3, 14, 3, 0), 7), // msr cntv_tval_el0, x7
        msr(SysReg::CNTV_CTL_EL0, 2),   // ENABLE
        msr(SysReg::PMCR_EL0, 8),
        0xD503_207F, // wfi
        mrs(10, SysReg::CNTVCT_EL0),
        mrs(11, SysReg::PMCCNTR_EL0),
        SPIN,
    ];
    let spin = RAM_BASE + 4 * (program.len() as u64 - 1);
    for cpu in CpuModel::ALL {
        run(
            cpu,
            &[(RAM_BASE, &program)],
            &[
                (Reg::X1, GIC_DIST_BASE),
                (Reg::X2, 1),
                (Reg::X3, 1 << 27),
                (Reg::X4, GIC_CPU_BASE),
                (Reg::X5, 0xFF),
                (Reg::X6, CYCLES),
                (Reg::X7, 100_000),
                (Reg::X8, PMCR_E),
            ],
            spin,
            |m| {
                let waited = m.reg_read(Reg::X10) - m.reg_read(Reg::X9);
                assert!(waited >= 100_000, "{cpu}: the counter ran on: {waited}");
                assert_eq!(m.reg_read(Reg::X11), 3, "{cpu}: three cycles");
            },
        );
    }
}

/// EL0 reaches the cycle counter only as `PMUSERENR_EL0` lets it: with no
/// enable set, its `MRS` traps to EL1 (EC 0x18, the ISS naming the read
/// of `PMCCNTR_EL0` into X9); with `EN`, or with `CR` alone, the read
/// completes and the `SVC` after it is the exception EL1 sees (EC 0x15).
#[test]
fn el0_reaches_the_pmu_only_as_pmuserenr_allows() {
    let program = [
        msr(sysreg(3, 0, 12, 0, 0), 5), // msr vbar_el1, x5
        msr(SysReg::PMUSERENR_EL0, 8),
        msr(SysReg::SPSR_EL1, XZR), // EL0t
        msr(SysReg::ELR_EL1, 9),    // EL0's code
        ERET,
    ];
    let el0 = [mrs(9, SysReg::PMCCNTR_EL0), SVC];
    let handler = [mrs(10, SysReg::ESR_EL1), SPIN];
    let spin = LOWER_LEVEL + 4;
    // op0 3, op2 0, op1 3, CRn 9, Rt 9, CRm 13, a read.
    let pmccntr_read = (3 << 20) | (3 << 14) | (9 << 10) | (9 << 5) | (13 << 1) | 1;
    for cpu in CpuModel::ALL {
        for (userenr, esr) in [
            (0, (0x18 << 26) | (1 << 25) | pmccntr_read),
            (1, (0x15 << 26) | (1 << 25)),
            (1 << 2, (0x15 << 26) | (1 << 25)),
            (1 << 3, (0x18 << 26) | (1 << 25) | pmccntr_read),
        ] {
            run(
                cpu,
                &[
                    (RAM_BASE, &program),
                    (EL0_CODE, &el0),
                    (LOWER_LEVEL, &handler),
                ],
                &[(Reg::X5, VBAR), (Reg::X8, userenr), (Reg::X9, EL0_CODE)],
                spin,
                |m| {
                    assert_eq!(
                        m.reg_read(Reg::X10),
                        esr,
                        "{cpu}: PMUSERENR_EL0 {userenr:#x}"
                    );
                },
            );
        }
    }
}
