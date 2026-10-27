//! `WFI`, `WFE`, `WFIT`, `WFET` (FEAT_WFxT), `SEV`, `SEVL`, the Event
//! Register and the event stream, as a guest sees them (R9): whether an
//! instruction retires, waits or traps, what ends a wait and at which tick,
//! and what a snapshot keeps.
//!
//! The rules are Arm's pseudocode: `Hint_WFI`, `Hint_WFE`, `Hint_WFIT`,
//! `Hint_WFET`, `AArch64_CheckForWFxTrap`, `AArch64_WFxTrap`,
//! `LocalTimeoutEvent`, `TestEventCNTV`, `SendEventLocal` and
//! `AArch64_ExceptionReturn`. The words are GNU as's
//! (`-march=armv9.2-a+wfxt`, objdump -d).

use std::vec::Vec;

use rusty_box_core::{SnapError, VmClock, VmInstant};

use super::super::features::{Feature, Implementation};
use super::super::step::{processor, Taken, CODE, DATA, UNDEFINED, VBAR};
use super::super::{Activity, Cpu, El, Pstate, Step, Wait};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const WFE: u32 = 0xD503_205F;
const WFI: u32 = 0xD503_207F;
const SEV: u32 = 0xD503_209F;
const SEVL: u32 = 0xD503_20BF;
const WFET_X1: u32 = 0xD503_1001;
const WFIT_X1: u32 = 0xD503_1021;
const WFET_XZR: u32 = 0xD503_101F;
const MSR_CNTKCTL: u32 = 0xD518_E100; // msr cntkctl_el1, x0
const MRS_CNTKCTL: u32 = 0xD538_E100; // mrs x0, cntkctl_el1
const ERET: u32 = 0xD69F_03E0;
const LDXR: u32 = 0xC85F_7C22; // ldxr x2, [x1]
const CLREX: u32 = 0xD503_3F5F;

/// `CNTKCTL_EL1`: `EVNTEN`, `EVNTDIR`, `EVNTI` = 4 (bit 4 of the count,
/// which rises at every count 16 mod 32 and falls at every multiple of 32),
/// and `EVNTIS` (FEAT_ECV), which moves the bit up by 8.
const EVNTEN: u64 = 1 << 2;
const EVNTDIR: u64 = 1 << 3;
const EVNTI_4: u64 = 4 << 4;
const EVNTIS: u64 = 1 << 17;

/// `SCTLR_EL1.nTWI` and `nTWE`.
const NTWI: u64 = 1 << 16;
const NTWE: u64 = 1 << 18;

/// A trapped wait: EC 0x01, CV = 1 and COND = 0xE, then TI, and for the
/// timed forms RV = 1 and RN = 1 (X1).
const WFI_TRAP: Taken = Taken { ec: 0x01, iss: 0x01E0_0000 };
const WFE_TRAP: Taken = Taken { ec: 0x01, iss: 0x01E0_0001 };
const WFIT_TRAP: Taken = Taken { ec: 0x01, iss: 0x01E0_0026 };
const WFET_TRAP: Taken = Taken { ec: 0x01, iss: 0x01E0_0027 };

/// The instruction at `CODE` over a RAM that holds `DATA` too.
struct Ram(Vec<u8>);

impl PhysBus for Ram {
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, _clock: VmClock) -> Result<(), BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get_mut(a..a + n).ok_or(BusFault::Unmapped)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

/// What one instruction did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Retired,
    Waiting(Wait),
    Took(Taken),
}

/// Step `insn` at `CODE` on a running processor. A taken exception leaves
/// EL1; the processor is put back at the level it executed at.
fn run(cpu: &mut Cpu, insn: u32) -> Outcome {
    assert_eq!(cpu.activity, Activity::Running, "a waiting processor executes nothing");
    let pstate = cpu.pstate;
    cpu.pc = CODE;
    cpu.sys.esr_el1 = 0x3F << 26;
    let mut ram = Ram(std::vec![0; (DATA + 0x100) as usize]);
    ram.0[CODE as usize..CODE as usize + 4].copy_from_slice(&insn.to_le_bytes());
    match cpu.step(&mut ram, &mut NoInstrumentation) {
        Step::Waiting => match cpu.activity {
            Activity::Waiting(wait) => {
                assert_eq!(cpu.pc, CODE + 4, "a wait retires");
                Outcome::Waiting(wait)
            }
            Activity::Running => panic!("{insn:#010x} reported a wait and runs"),
        },
        Step::Retired if cpu.sys.esr_el1 >> 26 == 0x3F => Outcome::Retired,
        Step::Retired => {
            assert_eq!(cpu.pc & !0x7FF, VBAR, "{insn:#010x} took an exception to the vector table");
            cpu.pstate = pstate;
            let esr = cpu.sys.esr_el1;
            Outcome::Took(Taken {
                ec: esr >> 26,
                iss: esr & 0x01FF_FFFF,
            })
        }
        Step::Stop(cause) => panic!("{insn:#010x} stopped the machine: {cause:?}"),
    }
}

/// End a wait the way an interrupt does: assert IRQ, wake, deassert.
fn wake_by_interrupt(cpu: &mut Cpu) {
    cpu.set_interrupt_lines(true, false);
    assert!(cpu.wake_if_pending(), "an interrupt ends every wait");
    cpu.set_interrupt_lines(false, false);
}

fn at_el0(cpu: &mut Cpu) {
    cpu.pstate = Pstate {
        el: El::El0,
        sp_sel: false,
        ..Pstate::RESET
    };
}

fn tick(t: u64) -> VmInstant {
    VmInstant::from_ticks(t)
}

/// The test processors count one tick per counter value (`step::processor`
/// runs both at 1 MHz), so a count is its tick.
fn cpu(imp: Implementation) -> Cpu {
    let cpu = processor(imp);
    assert_eq!(cpu.ips, cpu.imp.counter_hz);
    cpu
}

/// `SEV`, `SEVL` and an exception return set the Event Register, and `WFE`
/// consumes it: a `WFE` that finds it set retires and leaves it clear, so
/// the next one waits; a `WFE` that finds it clear waits for an event.
#[test]
fn wfe_consumes_the_event_that_sev_sevl_and_eret_send() {
    let mut cpu = cpu(Implementation::CORTEX_A57);
    assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event));
    wake_by_interrupt(&mut cpu);
    for send in [SEV, SEVL] {
        assert_eq!(run(&mut cpu, send), Outcome::Retired);
        assert_eq!(run(&mut cpu, WFE), Outcome::Retired, "{send:#010x}");
        assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event), "{send:#010x}: consumed");
        wake_by_interrupt(&mut cpu);
    }
    // ERET to EL1h, interrupts masked.
    cpu.sys.spsr_el1 = 0x3C5;
    cpu.sys.elr_el1 = CODE + 0x100;
    assert_eq!(run(&mut cpu, ERET), Outcome::Retired);
    assert_eq!(cpu.pc, CODE + 0x100);
    assert_eq!(run(&mut cpu, WFE), Outcome::Retired, "an exception return sends an event");
    // An illegal return sends one too (`SendEventLocal` follows
    // `SetPSTATEFromPSR` whatever the return).
    cpu.sys.spsr_el1 = 0x3C8; // EL2h: not implemented
    assert_eq!(run(&mut cpu, ERET), Outcome::Retired);
    assert!(cpu.pstate.il);
    cpu.pstate.il = false;
    assert_eq!(run(&mut cpu, WFE), Outcome::Retired);
}

/// At EL0, `SCTLR_EL1.nTWE` clear traps a `WFE` that would wait, and only
/// that one: a set Event Register completes it first (`Hint_WFE`). The
/// syndrome is EC 0x01 with CV, COND 0xE and TI = WFE.
#[test]
fn el0_wfe_traps_only_when_it_would_wait() {
    let mut cpu = cpu(Implementation::CORTEX_A57);
    cpu.sys.sctlr_el1 &= !(NTWE | NTWI);
    at_el0(&mut cpu);
    assert_eq!(run(&mut cpu, WFE), Outcome::Took(WFE_TRAP));
    assert_eq!(run(&mut cpu, SEVL), Outcome::Retired);
    assert_eq!(run(&mut cpu, WFE), Outcome::Retired, "the event completes it before the trap check");
    assert_eq!(run(&mut cpu, WFE), Outcome::Took(WFE_TRAP), "and was consumed");
    cpu.sys.sctlr_el1 |= NTWE;
    assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event));
}

/// A pending interrupt, masked by PSTATE or not, completes `WFI` before its
/// trap check (`Hint_WFI`: `AArch64_InterruptPending`); without one, EL0's
/// `WFI` traps with `nTWI` clear and waits with it set.
#[test]
fn wfi_with_an_interrupt_pending_completes_before_the_trap_check() {
    let mut cpu = cpu(Implementation::CORTEX_A57);
    assert!(cpu.pstate.i && cpu.pstate.f, "interrupts are masked at reset");
    cpu.set_interrupt_lines(true, false);
    assert_eq!(run(&mut cpu, WFI), Outcome::Retired, "a masked IRQ is pending");
    cpu.set_interrupt_lines(false, true);
    assert_eq!(run(&mut cpu, WFI), Outcome::Retired, "a masked FIQ is pending");
    cpu.set_interrupt_lines(false, false);
    assert_eq!(run(&mut cpu, WFI), Outcome::Waiting(Wait::Interrupt));
    wake_by_interrupt(&mut cpu);

    cpu.sys.sctlr_el1 &= !NTWI;
    at_el0(&mut cpu);
    cpu.set_interrupt_lines(true, false);
    assert_eq!(run(&mut cpu, WFI), Outcome::Retired, "no trap: the interrupt completes it");
    cpu.set_interrupt_lines(false, false);
    assert_eq!(run(&mut cpu, WFI), Outcome::Took(WFI_TRAP));
}

/// `WFIT` waits for an interrupt or for `CNTVCT_EL0` to reach X1, and
/// retires at once, trap or no trap, when either already holds; `WFET`
/// waits for an event, an interrupt or the timeout, and retires at once
/// when the Event Register is set (consuming it) or the timeout has passed.
/// Trapped at EL0, each reports TI, RV and RN.
#[test]
fn wfit_and_wfet_wait_until_their_timeout() {
    let mut cpu = cpu(Implementation::EVERY_FEATURE);
    cpu.wait_until(tick(1000));

    cpu.x[1] = 1100;
    assert_eq!(run(&mut cpu, WFIT_X1), Outcome::Waiting(Wait::InterruptUntil(tick(1100))));
    assert_eq!(cpu.wake_deadline(), Some(tick(1100)));
    cpu.wait_until(tick(1099));
    assert!(!cpu.wake_if_pending(), "one tick short of the timeout");
    cpu.wait_until(tick(1100));
    assert!(cpu.wake_if_pending(), "CNTVCT_EL0 reached X1");

    cpu.x[1] = 5000;
    assert_eq!(run(&mut cpu, WFIT_X1), Outcome::Waiting(Wait::InterruptUntil(tick(5000))));
    wake_by_interrupt(&mut cpu);
    cpu.set_interrupt_lines(true, false);
    assert_eq!(run(&mut cpu, WFIT_X1), Outcome::Retired, "an interrupt is pending");
    cpu.set_interrupt_lines(false, false);
    cpu.x[1] = cpu.counter().get();
    assert_eq!(run(&mut cpu, WFIT_X1), Outcome::Retired, "the timeout has been reached");

    cpu.x[1] = cpu.counter().get() + 50;
    let deadline = tick(cpu.x[1]);
    assert_eq!(run(&mut cpu, WFET_X1), Outcome::Waiting(Wait::EventUntil(deadline)));
    assert_eq!(cpu.wake_deadline(), Some(deadline));
    cpu.wait_until(deadline);
    assert!(cpu.wake_if_pending());
    assert_eq!(run(&mut cpu, SEVL), Outcome::Retired);
    cpu.x[1] = u64::MAX;
    assert_eq!(run(&mut cpu, WFET_X1), Outcome::Retired, "the event completes it");
    assert_eq!(
        run(&mut cpu, WFET_X1),
        Outcome::Waiting(Wait::Event),
        "and was consumed; a timeout the tick count never reaches is none"
    );
    assert_eq!(cpu.wake_deadline(), None);
    wake_by_interrupt(&mut cpu);
    assert_eq!(run(&mut cpu, WFIT_X1), Outcome::Waiting(Wait::Interrupt));
    wake_by_interrupt(&mut cpu);
    assert_eq!(run(&mut cpu, WFET_XZR), Outcome::Retired, "XZR is a timeout of 0, long passed");

    cpu.sys.sctlr_el1 &= !(NTWE | NTWI);
    at_el0(&mut cpu);
    cpu.x[1] = cpu.counter().get() + 10;
    assert_eq!(run(&mut cpu, WFIT_X1), Outcome::Took(WFIT_TRAP));
    assert_eq!(run(&mut cpu, WFET_X1), Outcome::Took(WFET_TRAP));
    cpu.x[1] = 0;
    assert_eq!(run(&mut cpu, WFIT_X1), Outcome::Retired, "a passed timeout completes it before the trap check");
    assert_eq!(run(&mut cpu, WFET_X1), Outcome::Retired);

    let mut v80 = self::cpu(Implementation::CORTEX_A57);
    for insn in [WFET_X1, WFIT_X1] {
        assert_eq!(run(&mut v80, insn), Outcome::Took(UNDEFINED), "{insn:#010x} without FEAT_WFxT");
    }
}

/// The event stream sets the Event Register where the count's bit `EVNTI`
/// rises (or falls, with `EVNTDIR`), and that event ends a `WFE`: its tick
/// is the wait's deadline, a tick earlier the wait goes on, and ending the
/// wait consumes the event. An event that comes while the processor runs
/// completes the next `WFE`.
#[test]
fn the_event_stream_sets_the_event_register_and_ends_a_wfe() {
    for (evntdir, first, second) in [(0, 16, 48), (EVNTDIR, 32, 64)] {
        let mut cpu = cpu(Implementation::CORTEX_A57);
        cpu.x[0] = EVNTEN | EVNTI_4 | evntdir;
        assert_eq!(run(&mut cpu, MSR_CNTKCTL), Outcome::Retired);
        assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event));
        assert_eq!(cpu.wake_deadline(), Some(tick(first)), "EVNTDIR {evntdir}");
        cpu.wait_until(tick(first - 1));
        assert!(!cpu.wake_if_pending());
        cpu.wait_until(tick(first));
        assert!(cpu.wake_if_pending(), "EVNTDIR {evntdir}: the stream's event");
        assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event), "the wake consumed it");
        assert_eq!(cpu.wake_deadline(), Some(tick(second)));
        wake_by_interrupt(&mut cpu);
        cpu.wait_until(tick(second));
        assert_eq!(run(&mut cpu, WFE), Outcome::Retired, "an event while running");
        assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event));
    }
}

/// A write of `CNTKCTL_EL1` judges the count before it by the controls that
/// held then: an event the stream generated before the stream is turned off
/// stays in the Event Register, and the count a stream turned on has
/// already passed generates none.
#[test]
fn cntkctl_writes_keep_past_events_and_make_none() {
    let mut cpu = cpu(Implementation::CORTEX_A57);
    cpu.x[0] = EVNTEN | EVNTI_4;
    assert_eq!(run(&mut cpu, MSR_CNTKCTL), Outcome::Retired);
    cpu.wait_until(tick(20));
    cpu.x[0] = 0;
    assert_eq!(run(&mut cpu, MSR_CNTKCTL), Outcome::Retired);
    assert_eq!(run(&mut cpu, WFE), Outcome::Retired, "the event at 16");
    assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event));
    assert_eq!(cpu.wake_deadline(), None, "no stream, no event to wait for");
    wake_by_interrupt(&mut cpu);

    cpu.wait_until(tick(100));
    cpu.x[0] = EVNTEN | EVNTI_4;
    assert_eq!(run(&mut cpu, MSR_CNTKCTL), Outcome::Retired);
    assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event), "16 + 32n before 100 made no event");
    assert_eq!(cpu.wake_deadline(), Some(tick(112)));
}

/// `EVNTIS` (FEAT_ECV) moves the stream's bit up by 8; without FEAT_ECV
/// `CNTKCTL_EL1` does not keep it.
#[test]
fn evntis_scales_the_stream_with_feat_ecv_only() {
    for (imp, kept, first) in [(Implementation::EVERY_FEATURE, EVNTIS, 4096), (Implementation::CORTEX_A57, 0, 16)] {
        let mut cpu = cpu(imp);
        cpu.x[0] = EVNTEN | EVNTI_4 | EVNTIS;
        assert_eq!(run(&mut cpu, MSR_CNTKCTL), Outcome::Retired);
        assert_eq!(run(&mut cpu, MRS_CNTKCTL), Outcome::Retired);
        assert_eq!(cpu.x[0], EVNTEN | EVNTI_4 | kept);
        assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event));
        assert_eq!(cpu.wake_deadline(), Some(tick(first)), "FEAT_ECV {}", imp.has(Feature::Ecv));
    }
}

/// With the exclusive monitor armed, `WFE` and `WFET` complete without
/// waiting: the global monitor's clearing, a wake-up event, is not watched.
#[test]
fn wfe_does_not_wait_while_the_exclusive_monitor_is_armed() {
    let mut cpu = cpu(Implementation::EVERY_FEATURE);
    cpu.x[1] = DATA;
    assert_eq!(run(&mut cpu, LDXR), Outcome::Retired);
    assert_eq!(run(&mut cpu, WFE), Outcome::Retired);
    cpu.x[1] = u64::MAX;
    assert_eq!(run(&mut cpu, WFET_X1), Outcome::Retired);
    assert_eq!(run(&mut cpu, CLREX), Outcome::Retired);
    assert_eq!(run(&mut cpu, WFE), Outcome::Waiting(Wait::Event));
}

/// A snapshot keeps the wait and its timeout, and the Event Register with
/// the stream's events in it; one that holds a timed wait or `EVNTIS` a
/// processor cannot reach is refused.
#[test]
fn snapshots_keep_waits_and_the_event_register() {
    let mut cpu = cpu(Implementation::EVERY_FEATURE);
    cpu.x[1] = 777;
    assert_eq!(run(&mut cpu, WFET_X1), Outcome::Waiting(Wait::EventUntil(tick(777))));
    let mut image = Vec::new();
    cpu.save(&mut image).expect("save");
    let mut restored = self::cpu(Implementation::EVERY_FEATURE);
    restored.restore(&mut image.as_slice()).expect("restore");
    assert_eq!(restored.activity, Activity::Waiting(Wait::EventUntil(tick(777))));
    assert_eq!(restored.wake_deadline(), Some(tick(777)));

    let mut cpu = self::cpu(Implementation::CORTEX_A57);
    cpu.x[0] = EVNTEN | EVNTI_4;
    assert_eq!(run(&mut cpu, MSR_CNTKCTL), Outcome::Retired);
    cpu.wait_until(tick(20));
    let mut image = Vec::new();
    cpu.save(&mut image).expect("save");
    let mut restored = self::cpu(Implementation::CORTEX_A57);
    restored.restore(&mut image.as_slice()).expect("restore");
    assert_eq!(run(&mut restored, WFE), Outcome::Retired, "the stream's event at 16 was saved");
    assert_eq!(run(&mut restored, WFE), Outcome::Waiting(Wait::Event));

    for activity in [
        Activity::Running,
        Activity::Waiting(Wait::Interrupt),
        Activity::Waiting(Wait::InterruptUntil(tick(5))),
        Activity::Waiting(Wait::Event),
        Activity::Waiting(Wait::EventUntil(tick(6))),
    ] {
        let mut cpu = self::cpu(Implementation::EVERY_FEATURE);
        cpu.activity = activity;
        let mut image = Vec::new();
        cpu.save(&mut image).expect("save");
        let mut restored = self::cpu(Implementation::EVERY_FEATURE);
        restored.restore(&mut image.as_slice()).expect("restore");
        assert_eq!(restored.activity, activity);
    }

    /// A processor, a state it cannot reach, and why its restore refuses it.
    type Unreachable = (Implementation, fn(&mut Cpu), &'static str);
    let unreachable: [Unreachable; 3] = [
        (
            Implementation::CORTEX_A57,
            |cpu| cpu.activity = Activity::Waiting(Wait::InterruptUntil(tick(9))),
            "processor waits with a timeout on a processor without FEAT_WFxT",
        ),
        (
            Implementation::CORTEX_A57,
            |cpu| cpu.sys.cntkctl_el1 = EVNTIS,
            "snapshot's CNTKCTL_EL1 holds bits this processor cannot set",
        ),
        (
            Implementation::EVERY_FEATURE,
            |cpu| cpu.activity = Activity::Waiting(Wait::EventUntil(tick(u64::MAX))),
            "processor's timed wait ends at no tick the count reaches",
        ),
    ];
    for (imp, make, refusal) in unreachable {
        let mut cpu = self::cpu(imp);
        make(&mut cpu);
        let mut image = Vec::new();
        cpu.save(&mut image).expect("save");
        assert_eq!(self::cpu(imp).restore(&mut image.as_slice()), Err(SnapError::Invalid(refusal)));
    }
}
