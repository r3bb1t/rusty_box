//! What the processor holds out of reset where the architecture leaves the
//! value open.
//!
//! Provenance (R7): Arm's `AArch64_TakeReset` (DDI 0487M.d, J1.2.2.30)
//! sets PSTATE's control fields, resets each System register field whose
//! description gives a reset value, and leaves the rest ARBITRARY: the
//! general-purpose registers (`AArch64_ResetGeneralRegisters`), the SIMD&FP
//! registers (`AArch64_ResetSIMDFPRegisters`, or with FEAT_SME
//! `ResetSVERegisters` and `ResetSMEState('0')`), the stack pointers,
//! `ELR_ELx` and `SPSR_ELx` (`AArch64_ResetSpecialRegisters`), PSTATE's N,
//! Z, C and V, which it does not name, and every field a register
//! description resets to "an architecturally UNKNOWN value". The
//! Cortex-A57 TRM (DDI 0488H) gives the Cortex-A57's value for many of
//! them in its register summaries (4.2, Table 9-2, Table 14-2), and that is
//! the Cortex-A57's answer; the Armv9.2 model, which no document describes,
//! takes the same, so the two models come out of reset alike:
//!
//! - `SCTLR_EL1` 0x00C5_0838 (Table 4-3) and `SCTLR_EL2` 0x30C5_0838
//!   (Tables 4-3 and 4-13): its RES1 bits and `SA`.
//! - Zero: `FPCR` and `FPSR` (Table 14-2), `ACTLR_EL1` and `CPACR_EL1`
//!   (Table 4-4), `HCR_EL2`, `ACTLR_EL2`, `HSTR_EL2`, `HACR_EL2`,
//!   `HPFAR_EL2` and `ESR_EL2` (Table 4-13, where Table 4-2 has `ESR_EL2`
//!   UNKNOWN), and the RES0 `AFSRn_ELx` and `AMAIR_ELx`; `CNTKCTL_EL1` bits
//!   9:8 and 2:0, and each timer's `ENABLE` (Table 9-2).
//! - `CPTR_EL2` its RES1 bits, 0x33FF; `CNTHCTL_EL2` with `EL1PCTEN` and
//!   `EL1PCEN` set (Table 9-2); `VPIDR_EL2` and `VMPIDR_EL2` the processor's
//!   own `MIDR_EL1` and `MPIDR_EL1` (Table 4-13). `MDCR_EL2`'s 0x6 is
//!   `HPMN`, the count of the TRM's six event counters, a field of the PMU
//!   this processor does not have, so the register here is zero.
//! - Bits 63:32 zero in the 64-bit registers of Tables 4-2, 4-3, 4-7
//!   (`PAR_EL1`, footnote t) and 4-13 whose other bits it leaves UNKNOWN:
//!   `FAR_ELx`, `VBAR_ELx`, `TTBRn_ELx`, `VTTBR_EL2` and `PAR_EL1`.
//!
//! What both leave open is the board's own, by what the register holds:
//!
//! - **A control** — a register or field that enables, traps, selects,
//!   offsets or configures — is zero: `TCR_ELx`, `VTCR_EL2` and `MAIR_ELx`
//!   (beside their RES1 bits), `MDSCR_EL1`, the breakpoint and watchpoint
//!   controls, `CNTVOFF_EL2` and `CNTPOFF_EL2`, `SMCR_ELx`, the timers'
//!   `IMASK` and `CNTKCTL_EL1`'s event stream fields, and PSTATE's PAN,
//!   UAO, SSBS and BTYPE. Each of these is either off at zero (no trap, no
//!   breakpoint or watchpoint, no event stream, no offset of a count, no
//!   top byte ignored, no tag) or acts only once a control the TRM resets
//!   to zero turns it on (the translation controls with `SCTLR_ELx.M`
//!   clear, a timer's with `ENABLE` clear), so nothing acts that software
//!   did not ask for; and software that read-modify-writes a control —
//!   Linux its `CNTKCTL_EL1`, EDK2 its `CPACR_EL1` and `CPTR_EL2` — finds
//!   every field it leaves alone off. `MAIR_ELx` zero makes every
//!   attribute index Device-nGnRnE, the type each data access has while
//!   the MMU is off (`AArch64_S1DisabledOutput`).
//! - **A selector** — `CSSELR_EL1` — is zero: Level 1, InD 0, the data
//!   cache `CLIDR_EL1` lists first, so a `CCSIDR_EL1` read before any
//!   selection describes a cache the processor has.
//! - **A status flag** — `DISR_EL1.A`, `VDISR_EL2.A`, the Event Register
//!   (DDI 0487M.d, D1.7.1.1: "The reset value of the Event Register is
//!   UNKNOWN") — is clear: nothing deferred, no event pending, so the first
//!   `ESB` reports nothing it did not see and the first `WFE` waits.
//! - **A value** — data, an address, a return state, a thread pointer, a
//!   syndrome, a comparison value — is the board's fill ([`fill`]):
//!   0xA5A5_A5A5_A5A5_0000 with the register's name in bits 15:0, kept
//!   to the bits the register holds (32 for `ESR_EL1`, `CONTEXTIDR_ELx`,
//!   the DCC's data registers and the Cortex-A57's `IL1DATAn_EL1` and
//!   `DL1DATAn_EL1`, the low word under the TRM's rule above,
//!   and the bits an `MSR` keeps). It is a value no software writes for
//!   itself, so a guest that reads a register it never wrote finds one it
//!   did not put there rather than an empty-looking zero, and the name says
//!   where it came from: `Xn` names itself by `n`, `Vn` by 0x100 + `n` in
//!   each half, and every other register by its `MRS` encoding
//!   (`op0:op1:CRn:CRm:op2`, bit 15 set). Its bits alternate from the top,
//!   so bits 55:52 are 0b1010, neither all ones nor all zeros: a 64-bit
//!   fill used as an address is outside both halves of the address space
//!   for every virtual address size and top-byte-ignore setting (a
//!   translation fault with the MMU on) and above every physical address
//!   any `PARange` allows (an Address size fault with it off), so a guest
//!   that branches through or dereferences an unwritten general-purpose
//!   register or stack pointer faults at that instruction. PSTATE's NZCV
//!   takes the fill's bits 31:28, 0b1010: N and C set.
//! - **State no guest can read before it is zeroed** keeps zero: the bits
//!   of Z0-Z31 above V0-V31, P0-P15, FFR, ZA and ZT0 (FEAT_SME). Without
//!   FEAT_SVE only Streaming SVE mode reaches them, and entering it zeroes
//!   Z, P and FFR (`ResetSVEState`) as enabling ZA zeroes ZA and ZT0
//!   (`ResetSMEState('1')`).
//!
//! The pointer authentication keys are values with a reset of their own
//! ([`PacKeys::RESET`](super::pauth::PacKeys::RESET)). A directly booted
//! kernel's `x0` to `x3` are what Linux's boot protocol sets
//! (`booting.rst` §4); every other register is as reset leaves it.

use super::debug::{DebugPoints, MAX_BREAKPOINTS, MAX_WATCHPOINTS};
use super::regs::SysReg;

/// The board's fill, with no register's name in it: bits that alternate
/// from the top, 1010 0101 in each byte above the name.
const FILL: u64 = 0xA5A5_A5A5_A5A5_0000;

/// The fill naming `name` in its bits 15:0.
const fn fill(name: u16) -> u64 {
    FILL | name as u64
}

/// The board's reset value of the system register `reg`, which holds a
/// value: the fill naming `reg` by its encoding. The caller keeps the
/// bits the register holds.
pub(crate) const fn value_of(reg: SysReg) -> u64 {
    fill(reg.encoding())
}

/// [`value_of`] in bits 31:0 alone: for a 32-bit register, and for a
/// 64-bit one whose bits 63:32 the Cortex-A57 TRM resets to zero.
pub(crate) const fn low_word_of(reg: SysReg) -> u64 {
    value_of(reg) & 0xFFFF_FFFF
}

/// X0-X30, each naming itself by its number.
pub(crate) const GENERAL: [u64; 31] = {
    let mut x = [0; 31];
    let mut n = 0;
    while n < x.len() {
        x[n] = fill(n as u16);
        n += 1;
    }
    x
};

/// V0-V31, each half naming the register by 0x100 + its number.
pub(crate) const SIMD_FP: [u128; 32] = {
    let mut v = [0; 32];
    let mut n = 0;
    while n < v.len() {
        let half = fill(0x100 | n as u16) as u128;
        v[n] = (half << 64) | half;
        n += 1;
    }
    v
};

/// `SP_EL0`.
pub(crate) const SP_EL0: u64 = value_of(SysReg::SP_EL0);
/// `SP_EL1`.
pub(crate) const SP_EL1: u64 = value_of(SysReg::SP_EL1);
/// `SP_EL2`, named by its encoding `S3_6_C4_C1_0`, which only EL3 reaches.
pub(crate) const SP_EL2: u64 = value_of(SysReg::encode(3, 6, 4, 1, 0));
/// `TPIDR2_EL0` (FEAT_SME).
pub(crate) const TPIDR2_EL0: SysReg = SysReg::encode(3, 3, 13, 0, 5);
/// `CONTEXTIDR_EL2`.
pub(crate) const CONTEXTIDR_EL2: SysReg = SysReg::encode(3, 4, 13, 0, 1);
/// `DBGDTRTX_EL0`, whose encoding a read names `DBGDTRRX_EL0`: the name
/// of both Debug Communications Channel data registers.
pub(crate) const DBGDTR_RXTX: SysReg = SysReg::encode(2, 3, 0, 5, 0);

/// `DBGBVR<n>_EL1`.
pub(crate) const fn dbgbvr(n: u8) -> SysReg {
    SysReg::encode(2, 0, 0, n, 4)
}

/// `DBGWVR<n>_EL1`.
pub(crate) const fn dbgwvr(n: u8) -> SysReg {
    SysReg::encode(2, 0, 0, n, 6)
}

/// `DBGBVR<n>_EL1` of each breakpoint of `points` out of reset, and zero in
/// the registers of the breakpoints beyond them, which the processor does
/// not have.
pub(crate) const fn breakpoint_values(points: DebugPoints) -> [u64; MAX_BREAKPOINTS] {
    let mut values = [0; MAX_BREAKPOINTS];
    let mut n = 0;
    while n < points.breakpoints() {
        values[n] = value_of(dbgbvr(n as u8));
        n += 1;
    }
    values
}

/// `DBGWVR<n>_EL1` of each watchpoint of `points` out of reset, bits 1:0
/// RES0, and zero in the registers of the watchpoints beyond them.
pub(crate) const fn watchpoint_values(points: DebugPoints) -> [u64; MAX_WATCHPOINTS] {
    let mut values = [0; MAX_WATCHPOINTS];
    let mut n = 0;
    while n < points.watchpoints() {
        values[n] = value_of(dbgwvr(n as u8)) & !3;
        n += 1;
    }
    values
}

/// PSTATE's N, Z, C and V in their `NZCV` layout: the fill's bits 31:28.
pub(crate) const NZCV: u64 = FILL & 0xF000_0000;

#[cfg(test)]
use super::el2::El2Storage;

#[cfg(test)]
impl<L: super::Levels> super::Cpu<L> {
    /// Every register reset fills, zero instead: the state a reference that
    /// zeroes them out of reset (QEMU) runs from, for replaying what it
    /// recorded. `the_fill_is_every_unwritten_value` checks that nothing
    /// is left.
    pub(crate) fn zero_the_fill(&mut self) {
        self.x = [0; 31];
        self.v = [0; 32];
        self.sp_el0 = 0;
        self.sp_el1 = 0;
        self.pstate.set_nzcv(0);
        let s = &mut self.sys;
        for value in [
            &mut s.ttbr0_el1,
            &mut s.ttbr1_el1,
            &mut s.spsr_el1,
            &mut s.elr_el1,
            &mut s.esr_el1,
            &mut s.far_el1,
            &mut s.par_el1,
            &mut s.vbar_el1,
            &mut s.contextidr_el1,
            &mut s.tpidr_el0,
            &mut s.tpidrro_el0,
            &mut s.tpidr_el1,
            &mut s.tpidr2_el0,
            &mut s.cntp_cval_el0,
            &mut s.cntv_cval_el0,
        ] {
            *value = 0;
        }
        s.dbgbvr = [0; MAX_BREAKPOINTS];
        s.dbgwvr = [0; MAX_WATCHPOINTS];
        s.dtrrx = 0;
        s.dtrtx = 0;
        if let super::impdef::ImpdefRegs::CortexA57(regs) = &mut self.impdef {
            regs.il1data = [0; super::impdef::IL1DATA_COUNT];
            regs.dl1data = [0; super::impdef::DL1DATA_COUNT];
        }
        if let Some(el2) = self.el2_state.get_mut() {
            el2.sp_el2 = 0;
            let r = &mut el2.regs;
            for value in [
                &mut r.ttbr0,
                &mut r.vttbr,
                &mut r.vbar,
                &mut r.far,
                &mut r.elr,
                &mut r.spsr,
                &mut r.tpidr,
                &mut r.cnthp_cval,
                &mut r.contextidr,
                &mut r.ttbr1,
                &mut r.cnthv_cval,
            ] {
                *value = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rusty_box_core::ClockHz;

    use super::super::el2::{El2Reg, El2Storage};
    use super::super::features::Implementation;
    use super::super::impdef::ImpdefRegs;
    use super::super::{Cpu, CpuConfig, Levels, NoEl2, WithEl2};

    fn processor<L: Levels>(imp: Implementation) -> Cpu<L> {
        Cpu::implementing(
            CpuConfig {
                ips: ClockHz::BOCHS_DEFAULT,
                reset_pc: 0x4000_0000,
            },
            imp,
        )
    }

    /// Whether `value` carries the fill: bits 31:16 are 0xA5A5 in every
    /// register it is kept to, whole or in its low word.
    fn filled(value: u64) -> bool {
        (value >> 16) & 0xFFFF == 0xA5A5
    }

    /// Every register whose value carries the fill out of reset is one
    /// [`Cpu::zero_the_fill`] zeroes, on both models with and without EL2.
    #[test]
    fn the_fill_is_every_unwritten_value() {
        fn check<L: Levels>(model: &str, imp: Implementation) {
            let mut cpu = processor::<L>(imp);
            assert!(cpu.x.iter().all(|&x| filled(x)));
            cpu.zero_the_fill();
            assert!(cpu.x.iter().chain([&cpu.sp_el0, &cpu.sp_el1]).all(|&x| x == 0));
            assert!(cpu.v.iter().all(|&v| v == 0));
            assert_eq!(cpu.pstate.nzcv(), 0);
            for (i, value) in cpu.sys.to_array().into_iter().enumerate() {
                assert!(!filled(value), "{model}: system register {i}: {value:#x}");
            }
            if let ImpdefRegs::CortexA57(regs) = cpu.impdef {
                let words = regs.il1data.iter().chain(regs.dl1data.iter());
                assert!(words.clone().all(|&w| w == 0), "{model}: {regs:x?}");
                for value in [
                    regs.l2ctlr,
                    regs.l2ectlr,
                    regs.l2actlr,
                    regs.cpuactlr,
                    regs.cpuectlr,
                    regs.cbar,
                ] {
                    assert!(!filled(value), "{model}: {regs:x?}");
                }
            }
            if let Some(el2) = cpu.el2_state.get() {
                assert_eq!(el2.sp_el2, 0);
                for reg in El2Reg::ALL {
                    let value = el2.regs.get(reg);
                    assert!(!filled(value), "{model}: {reg:?}: {value:#x}");
                }
            }
        }
        for (model, imp) in [
            ("Cortex-A57", Implementation::CORTEX_A57),
            ("Armv9.2", Implementation::ARMV9P2),
        ] {
            check::<NoEl2>(model, imp);
            check::<WithEl2>(model, imp);
        }
    }
}
