//! Replay of the EL2 vectors QEMU produced (`tests/vectors/qemu_el2.txt`,
//! made by `tests/vectors/gen_qemu_el2.py`): each case's controls written at
//! EL2 as the runner writes them, its word run at EL0, EL1 or EL2 with the
//! runner's sentinel after it, and the outcome compared with QEMU's: a word
//! that completed and X0 after it, or the exception it took, with its
//! level, ESR, FAR (where the exception reports one), ELR and SPSR.
//!
//! QEMU is a comparison, not the reference: what this core does is checked
//! against Arm's accessors in `sysreg_spec.rs`, and the generator leaves out
//! the cases where QEMU departs from Arm (docs/qemu-upstream-bugs.md has
//! the evidence) or where the two may legitimately differ, each with its
//! reason.

use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::features::Implementation;
use super::sysreg::SysRegKey;
use super::{Cpu, CpuConfig, El, Step, WithEl2};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const VECTORS: &str = include_str!("../vectors/qemu_el2.txt");

/// qemu_el2.ld's layout, and where the runner's ERET is.
const RAM_BASE: u64 = 0x4000_0000;
const RAM_BYTES: usize = 0x80_0000;
const RUNNER: u64 = 0x4010_0000;
const EL2_VECTORS: u64 = 0x4018_0000;
const EL1_VECTORS: u64 = 0x4018_1000;
const SLOT: u64 = 0x4020_0000;
const COUNTER_HZ: u64 = 62_500_000;

const ERET: u32 = 0xD69F_03E0;
/// The sentinels after the word, and the syndromes they raise.
const SVC_SENTINEL: u32 = 0xD41F_FFE1;
const HVC_SENTINEL: u32 = 0xD41F_FFE2;
const SVC_FFFF_ESR: u64 = 0x5600_FFFF;
const HVC_FFFF_ESR: u64 = 0x5A00_FFFF;

struct Ram(Vec<u8>);

impl Ram {
    fn offset(addr: u64, n: u64) -> Option<usize> {
        let end = addr.checked_add(n)?;
        (addr >= RAM_BASE && end <= RAM_BASE + RAM_BYTES as u64).then(|| (addr - RAM_BASE) as usize)
    }

    fn put32(&mut self, at: u64, word: u32) {
        let a = (at - RAM_BASE) as usize;
        self.0[a..a + 4].copy_from_slice(&word.to_le_bytes());
    }
}

impl PhysBus for Ram {
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        let n = width.bytes();
        let a = Self::offset(addr.get(), n).ok_or(BusFault::Unmapped)?;
        Ok(self.0[a..a + n as usize]
            .iter()
            .rev()
            .fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
        let n = width.bytes();
        let a = Self::offset(addr.get(), n).ok_or(BusFault::Unmapped)?;
        for (i, b) in self.0[a..a + n as usize].iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

/// One vector: the case and QEMU's outcome.
#[derive(Debug)]
struct Vector<'a> {
    family: &'a str,
    word: u32,
    target: u64,
    /// The values of [`CONTROL_WRITES`], in its order.
    controls: [u64; 12],
    /// The SPSR_EL2 the runner returns to EL0 or EL1 with.
    entry: u64,
    x0: u64,
    compare_value: bool,
    outcome: Outcome,
}

/// What the runner reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Outcome {
    kind: u64,
    x0: u64,
    esr: u64,
    far: u64,
    elr: u64,
    spsr: u64,
}

/// The `MSR` words, naming X0, of the controls in the runner's order:
/// HCR_EL2, CPTR_EL2, MDCR_EL2, CNTHCTL_EL2, HSTR_EL2, CPACR_EL1,
/// SCTLR_EL1, CNTKCTL_EL1, MDSCR_EL1, OSLAR_EL1, SPSR_EL1, ELR_EL1.
const CONTROL_WRITES: [u32; 12] = [
    0xD51C_1100,
    0xD51C_1140,
    0xD51C_1120,
    0xD51C_E100,
    0xD51C_1160,
    0xD518_1040,
    0xD518_1000,
    0xD518_E100,
    0xD510_0240,
    0xD510_1080,
    0xD518_4000,
    0xD518_4020,
];

fn hex(s: &str) -> u64 {
    u64::from_str_radix(s, 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

fn field<'a>(part: Option<&'a str>, key: &str) -> &'a str {
    let part = part.unwrap_or_else(|| panic!("no {key}="));
    part.strip_prefix(key)
        .unwrap_or_else(|| panic!("{part} is not {key}"))
}

fn parse(line: &str) -> Vector<'_> {
    let mut it = line.split_whitespace();
    let family = it.next().unwrap_or("");
    let nums: Vec<u64> = it.by_ref().take(16).map(hex).collect();
    assert_eq!(nums.len(), 16, "{line}");
    let compare_value = field(it.next(), "v=") == "1";
    let kind = hex(field(it.next(), "o="));
    let x0 = hex(field(it.next(), "x="));
    let esr = hex(field(it.next(), "c="));
    let far = hex(field(it.next(), "f="));
    let elr = hex(field(it.next(), "e="));
    let spsr = hex(field(it.next(), "p="));
    // word target hcr cptr mdcr cnthctl hstr cpacr sctlr cntkctl mdscr
    // oslar spsr1 elr1 entry x0
    let mut controls = [0u64; 12];
    controls.copy_from_slice(&nums[2..14]);
    Vector {
        family,
        word: nums[0] as u32,
        target: nums[1],
        controls,
        entry: nums[14],
        x0: nums[15],
        compare_value,
        outcome: Outcome {
            kind,
            x0,
            esr,
            far,
            elr,
            spsr,
        },
    }
}

/// Whether an exception class reports an address in FAR_ELx: the
/// instruction and data aborts, PC alignment, and watchpoints.
fn reports_far(esr: u64) -> bool {
    matches!(esr >> 26, 0x20 | 0x21 | 0x22 | 0x24 | 0x25 | 0x34 | 0x35)
}

/// Run `v` the way the runner does and report what it did.
fn replay(v: &Vector<'_>, ram: &mut Ram) -> Outcome {
    let hz = ClockHz::new(COUNTER_HZ).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut cpu = Cpu::<WithEl2>::implementing(
        CpuConfig {
            ips: hz,
            reset_pc: RUNNER,
        },
        Implementation {
            counter_hz: hz,
            ..Implementation::CORTEX_A57
        },
    );
    cpu.el2_state.regs.vbar = EL2_VECTORS;
    cpu.sys.vbar_el1 = EL1_VECTORS;
    for (word, value) in CONTROL_WRITES.iter().zip(v.controls) {
        match cpu.sysreg_write(SysRegKey::from_insn(*word), value) {
            Ok(effect) => cpu.apply_write_effect(effect),
            Err(e) => panic!("{}: writing {word:#010x}: {e:?}", v.family),
        }
    }
    ram.put32(SLOT, v.word);
    ram.put32(
        SLOT + 4,
        if v.target == 0 {
            SVC_SENTINEL
        } else {
            HVC_SENTINEL
        },
    );
    cpu.x[0] = v.x0;
    if v.target == 2 {
        // The runner branches to the slot after `cmp x3, #2` found the
        // target EL2: Z and C set.
        cpu.pstate.z = true;
        cpu.pstate.c = true;
        cpu.pc = SLOT;
    } else {
        cpu.el2_state.regs.spsr = v.entry;
        cpu.el2_state.regs.elr = SLOT;
        ram.put32(RUNNER, ERET);
        cpu.pc = RUNNER;
        assert_eq!(cpu.step(ram, &mut NoInstrumentation), Step::Retired);
        assert_eq!(cpu.pc, SLOT, "{}: the runner's ERET", v.family);
    }
    for _ in 0..4 {
        assert_eq!(
            cpu.step(ram, &mut NoInstrumentation),
            Step::Retired,
            "{} {:#010x}",
            v.family,
            v.word
        );
        let at_vector = |base: u64| cpu.pc & !0x7FF == base;
        if at_vector(EL2_VECTORS) {
            assert_eq!(cpu.pstate.el, El::El2);
            let el2 = &cpu.el2_state.regs;
            let (esr, elr) = (el2.esr, el2.elr.wrapping_sub(SLOT));
            if (esr == HVC_FFFF_ESR || esr == SVC_FFFF_ESR) && elr == 8 {
                return completed(&cpu);
            }
            return Outcome {
                kind: 2,
                x0: cpu.x[0],
                esr,
                far: el2.far,
                elr,
                spsr: el2.spsr,
            };
        }
        if at_vector(EL1_VECTORS) {
            assert_eq!(cpu.pstate.el, El::El1);
            let (esr, elr) = (cpu.sys.esr_el1, cpu.sys.elr_el1.wrapping_sub(SLOT));
            if esr == SVC_FFFF_ESR && elr == 8 {
                return completed(&cpu);
            }
            return Outcome {
                kind: 1,
                x0: cpu.x[0],
                esr,
                far: cpu.sys.far_el1,
                elr,
                spsr: cpu.sys.spsr_el1,
            };
        }
    }
    panic!(
        "{} {:#010x}: no exception after four steps, PC {:#x}",
        v.family, v.word, cpu.pc
    )
}

fn completed(cpu: &Cpu<WithEl2>) -> Outcome {
    Outcome {
        kind: 0,
        x0: cpu.x[0],
        esr: 0,
        far: 0,
        elr: 0,
        spsr: 0,
    }
}

/// Whether `word` reads a register into X0 (`MRS`, `SYSL`): its value is
/// compared only where the vector says both must agree on it.
fn reads(word: u32) -> bool {
    word & 0xFFE0_0000 == 0xD520_0000 || word & 0xFFF0_0000 == 0xD530_0000
}

/// Every case of every family takes the outcome QEMU's Cortex-A57 with EL2
/// took: the same exception at the same level with the same ESR, ELR and
/// SPSR (and FAR where the class reports one), or completion with the same
/// X0.
#[test]
fn the_el2_cases_end_as_on_qemu() {
    let mut ram = Ram(std::vec![0; RAM_BYTES]);
    let mut failures = Vec::new();
    let mut count = 0;
    for line in VECTORS.lines().filter(|l| !l.is_empty()) {
        let v = parse(line);
        count += 1;
        let got = replay(&v, &mut ram);
        let want = v.outcome;
        let value_matters = !reads(v.word) || v.compare_value || want.kind != 0;
        let same = got.kind == want.kind
            && (got.x0 == want.x0 || !value_matters)
            && got.esr == want.esr
            && got.elr == want.elr
            && got.spsr == want.spsr
            && (got.far == want.far || !reports_far(want.esr));
        if !same {
            failures.push(std::format!("{line}\n    replay: {got:x?}"));
        }
    }
    assert!(count > 0);
    std::eprintln!("{count} EL2 vectors, {} disagreements", failures.len());
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
