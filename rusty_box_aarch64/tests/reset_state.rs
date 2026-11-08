//! What a guest finds out of reset where the architecture leaves the value
//! open (R9), on both processors, and on the Cortex-A57 with EL2 (`cpu::reset` has
//! the rule and its reasons). A guest program at the reset level reads
//! each register with `MRS` (`MOV` for the stack pointer in use) and stores
//! what it read; the registers it cannot read without changing one are
//! read as the host sees them before it runs.
//!
//! - The Cortex-A57 TRM's values (DDI 0488H): `SCTLR_EL1` 0x00C5_0838,
//!   `SCTLR_EL2` 0x30C5_0838, `CPTR_EL2` its RES1 bits, `HCR_EL2`,
//!   `ESR_EL2`, `HPFAR_EL2`, `CPACR_EL1`, `FPCR` and `FPSR` zero, and bits
//!   63:32 of `FAR_ELx`, `VBAR_ELx`, `TTBRn_ELx`, `VTTBR_EL2` and
//!   `PAR_EL1` zero.
//! - A control zero, a selector the L1 data cache, a status flag clear.
//! - A value the board's fill: 0xA5A5_A5A5_A5A5_0000 with the register's
//!   name in bits 15:0 — `n` for `Xn`, 0x100 + `n` in each half of `Vn`, the
//!   `MRS` encoding for the rest — kept to the bits the register holds;
//!   NZCV the fill's bits 31:28.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    CpuModel, GuestStorage, Instrumentation, Levels, MachineBuilder, MemoryLoan, NoEl2, RamSize,
    Reg, StopReason, SysReg, VReg, VirtConfig, VirtMachine, WithEl2,
};

const RAM_LEN: usize = 1 << 20;
/// Where the program stores what it reads.
const BUF: u64 = RAM_BASE + 0x1000;

/// `adr x30, BUF` at `RAM_BASE`, `mov x0, sp`, `str x0, [x30], #8` and
/// `b .`.
const ADR_X30_BUF: u32 = 0x1000_801E;
const MOV_X0_SP: u32 = 0x9100_03E0;
const STR_X0_POST: u32 = 0xF800_87C0;
const SPIN: u32 = 0x1400_0000;

/// `mrs x0, reg`.
fn mrs_x0(reg: SysReg) -> u32 {
    0xD520_0000 | (u32::from(reg.encoding()) << 5)
}

fn sysreg(op0: u8, op1: u8, crn: u8, crm: u8, op2: u8) -> SysReg {
    SysReg::new(op0, op1, crn, crm, op2).expect("a system register encoding")
}

/// The board's fill naming `name`.
const fn fill(name: u16) -> u64 {
    0xA5A5_A5A5_A5A5_0000 | name as u64
}

/// The fill naming the system register `reg`.
fn value(reg: SysReg) -> u64 {
    fill(reg.encoding())
}

/// [`value`] in its low word alone.
fn low_word(reg: SysReg) -> u64 {
    value(reg) & 0xFFFF_FFFF
}

/// What the guest reads.
enum Read {
    /// `mrs x0, reg`.
    Mrs(SysReg),
    /// `mov x0, sp`.
    Sp,
}

/// Run a program that reads each of `reads` in turn at the reset level,
/// and return what it stored.
fn guest_reads<S: GuestStorage, L: Levels, I: Instrumentation>(
    m: &mut VirtMachine<S, L, I>,
    reads: &[(&str, Read, u64)],
) -> Vec<u64> {
    let mut words = vec![ADR_X30_BUF];
    for (_, read, _) in reads {
        words.push(match read {
            Read::Mrs(reg) => mrs_x0(*reg),
            Read::Sp => MOV_X0_SP,
        });
        words.push(STR_X0_POST);
    }
    let end = RAM_BASE + 4 * words.len() as u64;
    words.push(SPIN);
    for (i, word) in words.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    let out = m.emu_start(RAM_BASE, Some(end), Some(words.len() as u64));
    assert_eq!(
        out.stop,
        StopReason::ExitReached,
        "every read completed; stopped at {:#x} after {} reads",
        m.reg_read(Reg::Pc),
        (m.reg_read(Reg::X30).wrapping_sub(BUF)) / 8
    );
    let mut stored = vec![0u8; 8 * reads.len()];
    m.mem_read(BUF, &mut stored).expect("the buffer is in RAM");
    stored
        .chunks_exact(8)
        .map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")))
        .collect()
}

/// What a guest finds in the general-purpose and SIMD&FP registers and
/// PSTATE before its first instruction changes any.
fn check_register_file<S: GuestStorage, L: Levels, I: Instrumentation>(
    model: &str,
    m: &VirtMachine<S, L, I>,
) {
    for n in 0..=30u8 {
        let x = Reg::x(n).expect("a general-purpose register");
        assert_eq!(m.reg_read(x), fill(u16::from(n)), "{model}: x{n}");
    }
    for n in 0..32u8 {
        let half = u128::from(fill(0x100 | u16::from(n)));
        assert_eq!(
            m.vreg_read(VReg::new(n).expect("a SIMD&FP register")),
            (half << 64) | half,
            "{model}: v{n}"
        );
    }
    let pstate = m.pstate();
    assert_eq!(
        (pstate.n, pstate.z, pstate.c, pstate.v),
        (true, false, true, false),
        "{model}: NZCV 0b1010"
    );
    assert_eq!(m.sysreg_read(SysReg::FPCR), Ok(0), "{model}: FPCR (TRM)");
    assert_eq!(m.sysreg_read(SysReg::FPSR), Ok(0), "{model}: FPSR (TRM)");
}

/// The EL1 and EL0 registers every processor has, as a guest at the reset
/// level reads them; `spsr_mask` is the `SPSR_ELx` bits the model holds.
fn el1_reads(spsr_mask: u64) -> Vec<(&'static str, Read, u64)> {
    let dbgbvr0 = sysreg(2, 0, 0, 0, 4);
    let dbgwvr0 = sysreg(2, 0, 0, 0, 6);
    vec![
        ("NZCV", Read::Mrs(SysReg::NZCV), 0xA000_0000),
        ("SP_EL0", Read::Mrs(SysReg::SP_EL0), value(SysReg::SP_EL0)),
        (
            "SPSR_EL1",
            Read::Mrs(SysReg::SPSR_EL1),
            value(SysReg::SPSR_EL1) & spsr_mask,
        ),
        ("ELR_EL1", Read::Mrs(SysReg::ELR_EL1), value(SysReg::ELR_EL1)),
        ("ESR_EL1", Read::Mrs(SysReg::ESR_EL1), low_word(SysReg::ESR_EL1)),
        ("FAR_EL1", Read::Mrs(SysReg::FAR_EL1), low_word(SysReg::FAR_EL1)),
        ("PAR_EL1", Read::Mrs(SysReg::PAR_EL1), low_word(SysReg::PAR_EL1)),
        (
            "VBAR_EL1",
            Read::Mrs(SysReg::VBAR_EL1),
            low_word(SysReg::VBAR_EL1) & !0x7FF,
        ),
        ("TTBR0_EL1", Read::Mrs(SysReg::TTBR0_EL1), low_word(SysReg::TTBR0_EL1)),
        ("TTBR1_EL1", Read::Mrs(SysReg::TTBR1_EL1), low_word(SysReg::TTBR1_EL1)),
        (
            "CONTEXTIDR_EL1",
            Read::Mrs(SysReg::CONTEXTIDR_EL1),
            low_word(SysReg::CONTEXTIDR_EL1),
        ),
        ("TPIDR_EL0", Read::Mrs(SysReg::TPIDR_EL0), value(SysReg::TPIDR_EL0)),
        ("TPIDRRO_EL0", Read::Mrs(SysReg::TPIDRRO_EL0), value(SysReg::TPIDRRO_EL0)),
        ("TPIDR_EL1", Read::Mrs(SysReg::TPIDR_EL1), value(SysReg::TPIDR_EL1)),
        (
            "CNTP_CVAL_EL0",
            Read::Mrs(SysReg::CNTP_CVAL_EL0),
            value(SysReg::CNTP_CVAL_EL0),
        ),
        (
            "CNTV_CVAL_EL0",
            Read::Mrs(SysReg::CNTV_CVAL_EL0),
            value(SysReg::CNTV_CVAL_EL0),
        ),
        ("DBGBVR0_EL1", Read::Mrs(dbgbvr0), value(dbgbvr0)),
        ("DBGWVR0_EL1", Read::Mrs(dbgwvr0), value(dbgwvr0) & !3),
        ("SCTLR_EL1", Read::Mrs(SysReg::SCTLR_EL1), 0x00C5_0838),
        ("CPACR_EL1", Read::Mrs(SysReg::CPACR_EL1), 0),
        ("TCR_EL1", Read::Mrs(SysReg::TCR_EL1), 0),
        ("MAIR_EL1", Read::Mrs(SysReg::MAIR_EL1), 0),
        ("CSSELR_EL1", Read::Mrs(sysreg(3, 2, 0, 0, 0)), 0),
        ("MDSCR_EL1", Read::Mrs(sysreg(2, 0, 0, 2, 2)), 0),
        ("DBGBCR0_EL1", Read::Mrs(sysreg(2, 0, 0, 0, 5)), 0),
        ("CNTKCTL_EL1", Read::Mrs(SysReg::CNTKCTL_EL1), 0),
        ("CNTP_CTL_EL0", Read::Mrs(SysReg::CNTP_CTL_EL0), 0),
        ("CNTV_CTL_EL0", Read::Mrs(SysReg::CNTV_CTL_EL0), 0),
    ]
}

/// The registers only a processor with EL2 has, read at EL2 on the
/// Cortex-A57, the model a machine gives EL2: `SP_EL1` by `MRS`, `SP_EL2`
/// as the stack pointer in use.
fn el2_reads(spsr_mask: u64) -> Vec<(&'static str, Read, u64)> {
    vec![
        ("SP_EL2", Read::Sp, value(sysreg(3, 6, 4, 1, 0))),
        ("SP_EL1", Read::Mrs(SysReg::SP_EL1), value(SysReg::SP_EL1)),
        (
            "SPSR_EL2",
            Read::Mrs(SysReg::SPSR_EL2),
            value(SysReg::SPSR_EL2) & spsr_mask,
        ),
        ("ELR_EL2", Read::Mrs(SysReg::ELR_EL2), value(SysReg::ELR_EL2)),
        ("FAR_EL2", Read::Mrs(SysReg::FAR_EL2), low_word(SysReg::FAR_EL2)),
        (
            "VBAR_EL2",
            Read::Mrs(SysReg::VBAR_EL2),
            low_word(SysReg::VBAR_EL2) & !0x7FF,
        ),
        ("TTBR0_EL2", Read::Mrs(SysReg::TTBR0_EL2), low_word(SysReg::TTBR0_EL2)),
        ("VTTBR_EL2", Read::Mrs(SysReg::VTTBR_EL2), low_word(SysReg::VTTBR_EL2)),
        ("TPIDR_EL2", Read::Mrs(SysReg::TPIDR_EL2), value(SysReg::TPIDR_EL2)),
        (
            "CNTHP_CVAL_EL2",
            Read::Mrs(SysReg::CNTHP_CVAL_EL2),
            value(SysReg::CNTHP_CVAL_EL2),
        ),
        ("SCTLR_EL2", Read::Mrs(SysReg::SCTLR_EL2), 0x30C5_0838),
        ("HCR_EL2", Read::Mrs(SysReg::HCR_EL2), 1 << 31),
        ("CPTR_EL2", Read::Mrs(SysReg::CPTR_EL2), 0x33FF),
        ("MDCR_EL2", Read::Mrs(SysReg::MDCR_EL2), 0),
        ("ESR_EL2", Read::Mrs(SysReg::ESR_EL2), 0),
        ("HPFAR_EL2", Read::Mrs(SysReg::HPFAR_EL2), 0),
        ("TCR_EL2", Read::Mrs(SysReg::TCR_EL2), 0x8080_0000),
        ("VTCR_EL2", Read::Mrs(SysReg::VTCR_EL2), 0x8000_0000),
        ("MAIR_EL2", Read::Mrs(SysReg::MAIR_EL2), 0),
        ("CNTVOFF_EL2", Read::Mrs(SysReg::CNTVOFF_EL2), 0),
        ("CNTHCTL_EL2", Read::Mrs(SysReg::CNTHCTL_EL2), 0b11),
    ]
}

/// Each model's `SPSR_ELx` bits: NZCV, SS, IL, DAIF and M, and on the
/// Armv9.2 model BTYPE, SSBS, PAN, UAO and DIT too.
const A57_SPSR: u64 = 0xF030_03DF;
const ARMV9P2_SPSR: u64 = A57_SPSR | (3 << 10) | (1 << 12) | (1 << 22) | (1 << 23) | (1 << 24);

fn check_reads(model: &str, reads: &[(&str, Read, u64)], got: &[u64]) {
    for ((name, _, want), got) in reads.iter().zip(got) {
        assert_eq!(*got, *want, "{model}: {name}: {got:#x}, not {want:#x}");
    }
}

/// Every model, and the Cortex-A57 with EL2 too: the register file, then
/// what a guest's `MRS` reads.
#[test]
fn a_guest_finds_the_trm_s_values_zero_controls_and_the_fill_elsewhere() {
    for (model, cpu, spsr) in [
        ("Cortex-A57", CpuModel::CortexA57, A57_SPSR),
        ("Armv9.2", CpuModel::Armv9p2, ARMV9P2_SPSR),
    ] {
        let config = VirtConfig {
            cpu,
            ..VirtConfig::DEFAULT
        };
        let v9 = cpu == CpuModel::Armv9p2;

        let mut ram = vec![0u8; RAM_LEN];
        let mut m: VirtMachine<_, NoEl2> = MachineBuilder::new(config)
            .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
            .bare()
            .build_in(MemoryLoan {
                ram: &mut ram,
                vars: &mut [],
            })
            .expect("a bare machine builds");
        check_register_file(model, &m);
        let mut reads = el1_reads(spsr);
        reads.push(("SP_EL1", Read::Sp, value(SysReg::SP_EL1)));
        if v9 {
            let tpidr2 = sysreg(3, 3, 13, 0, 5);
            reads.push(("TPIDR2_EL0", Read::Mrs(tpidr2), value(tpidr2)));
            reads.push(("DISR_EL1", Read::Mrs(sysreg(3, 0, 12, 1, 1)), 0));
        }
        let got = guest_reads(&mut m, &reads);
        check_reads(model, &reads, &got);
    }

    let mut ram = vec![0u8; RAM_LEN];
    let mut m: VirtMachine<_, WithEl2> = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .with_el2()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("a bare machine with EL2 builds");
    check_register_file("Cortex-A57 with EL2", &m);
    let mut reads = el1_reads(A57_SPSR);
    reads.extend(el2_reads(A57_SPSR));
    let got = guest_reads(&mut m, &reads);
    check_reads("Cortex-A57 with EL2", &reads, &got);
}

/// A guest that branches through a register it never wrote takes an
/// Instruction Abort at the address it reached: the fill is above every
/// physical address, so with the MMU off it is an Address size fault
/// (`IFSC` 0b000000), with `ELR_EL1` and `FAR_EL1` the fill. (`x8`'s fill
/// is word-aligned; an odd register's would take the PC alignment fault
/// first.)
#[test]
fn a_branch_through_an_unwritten_register_faults_at_once() {
    const VBAR: u64 = RAM_BASE + 0x800;
    // `br x8` at `RAM_BASE`; the synchronous exception handler spins.
    const BR_X8: u32 = 0xD61F_0100;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m: VirtMachine<_, NoEl2> = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("a bare machine builds");
    for (at, word) in [(RAM_BASE, BR_X8), (VBAR + 0x200, SPIN)] {
        m.mem_write(at, &word.to_le_bytes()).expect("in RAM");
    }
    m.sysreg_write(SysReg::VBAR_EL1, VBAR).expect("VBAR_EL1");
    let out = m.emu_start(RAM_BASE, Some(VBAR + 0x200), Some(10));
    assert_eq!(out.stop, StopReason::ExitReached);
    let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1");
    assert_eq!(esr >> 26, 0x21, "an Instruction Abort from the same level");
    assert_eq!(esr & 0x3F, 0b00_0000, "an Address size fault at level 0");
    assert_eq!(m.sysreg_read(SysReg::ELR_EL1), Ok(fill(8)));
    assert_eq!(m.sysreg_read(SysReg::FAR_EL1), Ok(fill(8)));
}
