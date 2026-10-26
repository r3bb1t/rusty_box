//! Waiting: `WFI`, `WFE`, and `WFIT` and `WFET` (FEAT_WFxT), the Event
//! Register `WFE` reads, and what ends a wait (Arm pseudocode `Hint_WFI`,
//! `Hint_WFE`, `Hint_WFIT`, `Hint_WFET`, `WaitForInterrupt`, `WaitForEvent`,
//! `LocalTimeoutEvent`).
//!
//! The Event Register is set by `SEV` and `SEVL` (`SendEvent`, which with one
//! processor is `SendEventLocal`), by every exception return
//! (`AArch64.ExceptionReturn`), and by the event stream (`TestEventCNTV`: a
//! transition of the bit of the virtual count `CNTKCTL_EL1` selects). The
//! stream's events reach the register when it is read, from the count it was
//! last brought up to; a write of `CNTKCTL_EL1` brings it up to date first,
//! so each stretch of the count is judged by the controls that held then.
//!
//! A wait ends at a wake-up event. Any pending physical interrupt, masked by
//! `PSTATE` or not, ends every wait: `WFI`'s and `WFIT`'s wake-up events are
//! these, and for `WFE` and `WFET` a masked one is a wake-up the
//! architecture permits, a PE being free to leave its low-power state for
//! any reason (QEMU's `arm_cpu_has_work` wakes both on the same). `WFE` and
//! `WFET` also end at an event, and the timed forms when `CNTVCT_EL0` reaches
//! their timeout. Ending a `WFE` or `WFET` wait clears the Event Register,
//! the IMPLEMENTATION DEFINED choice `WaitForEvent` leaves.
//!
//! `WFE` and `WFET` complete without waiting while the local exclusive
//! monitor is armed: another observer's store to the monitored location
//! clears the global monitor, which is a wake-up event, and a device's DMA
//! is such a store, which this processor does not watch. QEMU's
//! `HELPER(wfe)` makes the same choice.

use rusty_box_core::VmInstant;

use super::exception::{Exception, ExceptionClass};
use super::exec::{ExecResult, Fault, Flow};
use super::timer::Count;
use super::{sctlr, Activity, Cpu, El};

/// What a waiting processor waits for (R2: each wait is a state of its own).
/// A timed wait's deadline is a tick the tick count reaches: a timeout it
/// never reaches makes the untimed wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wait {
    /// `WFI`, or a PSCI `CPU_SUSPEND`: a pending physical interrupt.
    Interrupt,
    /// `WFIT`: a pending physical interrupt, or the tick at which
    /// `CNTVCT_EL0` reaches the timeout.
    InterruptUntil(VmInstant),
    /// `WFE`: a pending physical interrupt, or an event.
    Event,
    /// `WFET`: a pending physical interrupt, an event, or the tick at which
    /// `CNTVCT_EL0` reaches the timeout.
    EventUntil(VmInstant),
}

/// The Event Register (Arm `EventRegister`), with the event stream's events
/// up to the count `folded_to` in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EventRegister {
    set: bool,
    folded_to: Count,
}

impl EventRegister {
    /// The register as it holds at the count `count`: `set`, with the event
    /// stream's later events still to come.
    pub(crate) const fn at(set: bool, count: Count) -> Self {
        Self { set, folded_to: count }
    }
}

/// `CNTKCTL_EL1.EVNTEN`, `EVNTDIR`, `EVNTI` and `EVNTIS` (FEAT_ECV).
const EVNTEN: u64 = 1 << 2;
const EVNTDIR: u64 = 1 << 3;
const EVNTI_SHIFT: u32 = 4;
pub(crate) const EVNTIS: u64 = 1 << 17;

/// A wait instruction, as its trap reports it (Arm `WFxType`): the timed
/// forms with the register that holds their timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WfxType {
    Wfi,
    Wfe,
    Wfit { rd: u32 },
    Wfet { rd: u32 },
}

impl Cpu {
    /// Arm `SendEventLocal`: set the Event Register. With one processor,
    /// `SEV`'s `SendEvent` is this too.
    #[inline]
    pub(crate) fn send_event(&mut self) {
        self.event.set = true;
    }

    /// Arm `IsEventRegisterSet`, the event stream's events up to now
    /// included.
    pub(crate) fn event_register(&self) -> bool {
        self.event.set || self.stream_event_after(self.event.folded_to).is_some_and(|count| count <= self.counter())
    }

    /// Bring the Event Register up to the count now: set it if the event
    /// stream has generated an event since it was last brought up to date.
    pub(crate) fn fold_event_stream(&mut self) {
        let now = self.counter();
        if self.stream_event_after(self.event.folded_to).is_some_and(|count| count <= now) {
            self.event.set = true;
        }
        self.event.folded_to = now;
    }

    /// `IsEventRegisterSet`, then `ClearEventRegister`: whether the register
    /// was set, leaving it clear.
    fn take_event(&mut self) -> bool {
        self.fold_event_stream();
        core::mem::replace(&mut self.event.set, false)
    }

    /// The first count after `after` at which the event stream generates an
    /// event (Arm `TestEventCNTV`): where bit `EVNTI` of the count (eight
    /// bits higher with `EVNTIS`, FEAT_ECV) goes from 0 to 1, or from 1 to 0
    /// with `EVNTDIR` set. `None` with `EVNTEN` clear, and past the end of the
    /// count. With no EL2, `CNTVOFF_EL2` is zero and the virtual count is
    /// the physical one. `CNTKCTL_EL1` holds `EVNTIS` only with FEAT_ECV:
    /// a write keeps it only then ([`Cpu::cntkctl_mask`]), and a snapshot
    /// that holds it otherwise is refused.
    fn stream_event_after(&self, after: Count) -> Option<Count> {
        let ctl = self.sys.cntkctl_el1;
        if ctl & EVNTEN == 0 {
            return None;
        }
        let n = ((ctl >> EVNTI_SHIFT) & 0xF) as u32 + if ctl & EVNTIS != 0 { 8 } else { 0 };
        // The bit is 1 on the upper half of each period of 2^(n+1) counts:
        // it rises where a period's upper half starts and falls where the
        // next period starts.
        let half = 1u64 << n;
        let period = half << 1;
        let edge = if ctl & EVNTDIR == 0 { half } else { 0 };
        // The largest multiple of `period` at most `after`, plus less than a
        // period, stays below 2^64.
        let after = after.get();
        let candidate = after - after % period + edge;
        let event = if candidate > after { Some(candidate) } else { candidate.checked_add(period) };
        event.map(Count::new)
    }

    /// Arm `AArch64_InterruptPending` with no EL2 and no SError source: an
    /// asserted physical IRQ or FIQ, masked by `PSTATE` or not.
    #[inline]
    fn interrupt_pending(&self) -> bool {
        self.irq_line || self.fiq_line
    }

    /// Leave the wait state if a wake-up event has come: any pending
    /// physical interrupt, masked by `PSTATE` or not; for `WFE` and `WFET` an
    /// event too, which the ending consumes; for `WFIT` and `WFET` the tick
    /// their timeout names. Returns whether the processor is now running.
    pub fn wake_if_pending(&mut self) -> bool {
        let Activity::Waiting(wait) = self.activity else {
            return true;
        };
        let interrupt = self.interrupt_pending();
        let now = self.icount;
        let wake = match wait {
            Wait::Interrupt => interrupt,
            Wait::InterruptUntil(timeout) => interrupt || now >= timeout.ticks(),
            Wait::Event => self.take_event() || interrupt,
            Wait::EventUntil(timeout) => self.take_event() || interrupt || now >= timeout.ticks(),
        };
        if wake {
            self.activity = Activity::Running;
        }
        wake
    }

    /// The tick at which a waiting processor's wait ends by itself: its
    /// timeout, or for `WFE` and `WFET` the event stream's next event,
    /// whichever comes first. `None` when only an interrupt (or the host)
    /// can end it, and while the processor runs.
    pub fn wake_deadline(&self) -> Option<VmInstant> {
        let Activity::Waiting(wait) = self.activity else {
            return None;
        };
        let stream = || self.stream_event_after(self.event.folded_to).and_then(|count| self.tick_reaching(count));
        match wait {
            Wait::Interrupt => None,
            Wait::InterruptUntil(timeout) => Some(timeout),
            Wait::Event => stream(),
            Wait::EventUntil(timeout) => Some(stream().map_or(timeout, |event| event.min(timeout))),
        }
    }

    /// Arm `AArch64_CheckForWFxTrap` with no EL2 and no EL3: at EL0,
    /// `SCTLR_EL1.nTWE` (`WFE`, `WFET`) or `nTWI` (`WFI`, `WFIT`) clear traps
    /// the wait to EL1, with `AArch64_WFxTrap`'s syndrome: CV = 1 and COND =
    /// 0xE (`ConditionSyndrome`), TI the form, and for the timed forms RV = 1
    /// and RN their register.
    fn wfx_trap_check(&self, form: WfxType) -> Result<(), Fault> {
        let allowed = match form {
            WfxType::Wfe | WfxType::Wfet { .. } => sctlr::NTWE,
            WfxType::Wfi | WfxType::Wfit { .. } => sctlr::NTWI,
        };
        if self.pstate.el != El::El0 || self.sys.sctlr_el1 & allowed != 0 {
            return Ok(());
        }
        let form_iss = match form {
            WfxType::Wfi => 0b00,
            WfxType::Wfe => 0b01,
            WfxType::Wfit { rd } => (1 << 2) | ((rd & 31) << 5) | 0b10,
            WfxType::Wfet { rd } => (1 << 2) | ((rd & 31) << 5) | 0b11,
        };
        Err(Exception::new(ExceptionClass::WfxTrap, (1 << 24) | (0xE << 20) | form_iss).into())
    }

    /// Start waiting for what `wait` names: the one place a processor's
    /// activity becomes a wait, which [`Flow::Wait`] then reports.
    pub(crate) fn begin_wait(&mut self, wait: Wait) -> Flow {
        self.activity = Activity::Waiting(wait);
        Flow::Wait
    }

    /// `WaitForEvent`: wait for what `wait` names, unless the exclusive
    /// monitor is armed.
    fn wait_for_event(&mut self, wait: Wait) -> Flow {
        if self.monitor.armed {
            Flow::Next
        } else {
            self.begin_wait(wait)
        }
    }

    /// Arm `Hint_WFE`.
    pub(crate) fn exec_wfe(&mut self) -> ExecResult {
        if self.take_event() {
            return Ok(Flow::Next);
        }
        self.wfx_trap_check(WfxType::Wfe)?;
        Ok(self.wait_for_event(Wait::Event))
    }

    /// Arm `Hint_WFI`: a pending interrupt completes it before the trap
    /// check.
    pub(crate) fn exec_wfi(&mut self) -> ExecResult {
        if self.interrupt_pending() {
            return Ok(Flow::Next);
        }
        self.wfx_trap_check(WfxType::Wfi)?;
        Ok(self.begin_wait(Wait::Interrupt))
    }

    /// `WFET <Xt>` and `WFIT <Xt>` (FEAT_WFxT): `1101 0101 0000 0011 0001
    /// 0000 op2 Rd`, op2 0 for `WFET` and 1 for `WFIT`.
    pub(crate) fn exec_wfxt(&mut self, insn: u32) -> ExecResult {
        let rd = insn & 31;
        if insn & (1 << 5) == 0 {
            self.exec_wfet(rd)
        } else {
            self.exec_wfit(rd)
        }
    }

    /// Arm `Hint_WFET(UInt(X[d]))`: `LocalTimeoutEvent` is the count
    /// reaching X[d].
    fn exec_wfet(&mut self, rd: u32) -> ExecResult {
        if self.take_event() {
            return Ok(Flow::Next);
        }
        let timeout = Count::new(self.x_or_zr(rd));
        if self.counter() >= timeout {
            return Ok(Flow::Next);
        }
        self.wfx_trap_check(WfxType::Wfet { rd })?;
        let wait = self.tick_reaching(timeout).map_or(Wait::Event, Wait::EventUntil);
        Ok(self.wait_for_event(wait))
    }

    /// Arm `Hint_WFIT(UInt(X[d]))`.
    fn exec_wfit(&mut self, rd: u32) -> ExecResult {
        let timeout = Count::new(self.x_or_zr(rd));
        if self.interrupt_pending() || self.counter() >= timeout {
            return Ok(Flow::Next);
        }
        self.wfx_trap_check(WfxType::Wfit { rd })?;
        let wait = self.tick_reaching(timeout).map_or(Wait::Interrupt, Wait::InterruptUntil);
        Ok(self.begin_wait(wait))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/wait.rs"]
mod tests;
