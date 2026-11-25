//! A single AArch64 processor paired with an external board.

use core::fmt;
use core::sync::atomic::AtomicBool;

use rusty_box_core::{Progress, RunBudget, VmClock, VmInstant};

use super::{BatchOutcome, PowerState, STOP_POLL_TICKS, StopReason, StopSource};
use crate::bus::PhysBus;
use crate::cpu::{Activity, Cpu, Levels, Reg, Step, StopCause, TimerLines, VirtualLines};
use crate::instrumentation::{Instrumentation, NoInstrumentation};

/// Signals the board can route from the processor into its interrupt controller.
pub struct ProcessorOutputs {
    pub timers: TimerLines,
    pub pmu: bool,
    pub dcc: bool,
}

/// The board's state after settling its devices at the current virtual instant.
pub struct BoardInputs {
    pub irq: bool,
    pub fiq: bool,
    pub virtual_lines: VirtualLines,
    /// The next instant strictly after the clock passed to `synchronize`.
    pub deadline: Option<VmInstant>,
    /// Flush cached translations, decoded code and the exclusive monitor
    /// after board-side writes or remapping, for example DMA.
    pub memory_changed: bool,
    /// Yield to the host after the current device effect has committed.
    pub stop: bool,
}

impl BoardInputs {
    pub const IDLE: Self = Self {
        irq: false,
        fiq: false,
        virtual_lines: VirtualLines::NONE,
        deadline: None,
        memory_changed: false,
        stop: false,
    };
}

/// Board policy for physical decoding, interrupt routing and device clocks.
///
/// A bus must leave `devices_touched` true after any guest access that might
/// change these inputs. The runtime settles the board before execution, after
/// such an access, and at deadlines, including whilst the CPU waits.
/// A board with cached fetches must report host/DMA memory changes before
/// the next instruction. Guest RAM stores use the CPU's existing coherence.
pub trait MachineBoard: PhysBus {
    fn synchronize(&mut self, clock: VmClock, outputs: ProcessorOutputs) -> BoardInputs;

    /// Reset board devices at the current clock. The board owns its memory
    /// retention and peripheral reset policy.
    fn reset(&mut self, clock: VmClock);
}

/// A device reported a deadline that was already due after synchronization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidDeadline {
    pub now: VmInstant,
    pub reported: VmInstant,
}

impl fmt::Display for InvalidDeadline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "board reported deadline {} after settling tick {}",
            self.reported.ticks(),
            self.now.ticks()
        )
    }
}

impl core::error::Error for InvalidDeadline {}

/// A processor, board and observer owned by one machine. Execution pairs the
/// processor and bus through the machine's exclusive borrow (R3).
pub struct Aarch64Machine<B, L: Levels, I = NoInstrumentation> {
    cpu: Cpu<L>,
    board: B,
    observer: I,
    reset_pc: u64,
    power: PowerState,
    stop: StopSource,
    next_event: Option<VmInstant>,
    pmu_event: u64,
}

struct RunEnds {
    tick_end: u64,
    instruction_end: u64,
}

impl<B: MachineBoard, L: Levels> Aarch64Machine<B, L> {
    pub fn new(cpu: Cpu<L>, board: B) -> Self {
        Self::with_instrumentation(cpu, board, NoInstrumentation)
    }
}

impl<B: MachineBoard, L: Levels, I: Instrumentation> Aarch64Machine<B, L, I> {
    /// Pair a processor with a board. Its current PC is the reset entry.
    pub fn with_instrumentation(mut cpu: Cpu<L>, board: B, observer: I) -> Self {
        cpu.forget_board_memory();
        Self {
            reset_pc: cpu.pc(),
            cpu,
            board,
            observer,
            power: PowerState::Running,
            stop: StopSource::None,
            next_event: None,
            pmu_event: u64::MAX,
        }
    }

    pub fn clock(&self) -> VmClock {
        self.cpu.clock()
    }

    pub fn instructions(&self) -> u64 {
        self.cpu.instructions()
    }

    pub fn reg_read(&self, reg: Reg) -> u64 {
        self.cpu.reg(reg)
    }

    pub fn reg_write(&mut self, reg: Reg, value: u64) {
        self.cpu.set_reg(reg, value);
    }

    pub fn board(&self) -> &B {
        &self.board
    }

    /// Lend the board while execution is suspended. The next run settles its
    /// IRQs and deadlines. Invalidate before lending so an unwinding edit
    /// cannot leave stale fetches, translations or exclusive reservations.
    pub fn with_board_mut<F, R>(&mut self, edit: F) -> R
    where
        F: FnOnce(&mut B) -> R,
    {
        self.cpu.forget_board_memory();
        edit(&mut self.board)
    }

    pub fn instrumentation(&self) -> &I {
        &self.observer
    }

    pub fn set_stop_flag(&mut self, flag: &'static AtomicBool) {
        self.stop = StopSource::Static(flag);
    }

    /// Restore the processor and the board's devices, keeping virtual time.
    pub fn reset(&mut self) {
        self.cpu.reset_keeping_time(self.reset_pc);
        self.board.reset(self.clock());
        self.power = PowerState::Running;
    }

    fn synchronize(&mut self) -> Result<bool, InvalidDeadline> {
        self.cpu.take_outputs_dirty();
        let pmu = self.cpu.pmu_outputs();
        self.pmu_event = pmu.next_overflow.map_or(u64::MAX, |at| at.get());
        let clock = self.clock();
        let inputs = self.board.synchronize(
            clock,
            ProcessorOutputs {
                timers: self.cpu.timer_lines(),
                pmu: pmu.interrupt,
                dcc: self.cpu.dcc_interrupt(),
            },
        );
        if inputs.memory_changed {
            self.cpu.forget_board_memory();
        }
        if let Some(reported) = inputs.deadline.filter(|&at| at <= clock.now()) {
            return Err(InvalidDeadline {
                now: clock.now(),
                reported,
            });
        }
        self.next_event = inputs
            .deadline
            .into_iter()
            .chain(
                self.cpu
                    .next_timer_deadline()
                    .filter(|&at| at > clock.now()),
            )
            .min();
        self.cpu.set_interrupt_lines(inputs.irq, inputs.fiq);
        self.cpu.set_virtual_interrupt_lines(inputs.virtual_lines);
        Ok(inputs.stop)
    }

    /// Run an exact tick budget, or bound retired instructions. Use a tick
    /// budget when a waiting CPU can have recurring device deadlines.
    pub fn step(&mut self, budget: RunBudget) -> Result<BatchOutcome, InvalidDeadline> {
        let started = self.cpu.retired;
        let (tick_end, instruction_end) = run_ends!(self.cpu, budget);
        let stop = self.run(RunEnds {
            tick_end,
            instruction_end,
        })?;
        Ok(BatchOutcome {
            progress: Progress::Instructions(self.cpu.retired - started),
            stop,
        })
    }

    fn run(&mut self, ends: RunEnds) -> Result<StopReason, InvalidDeadline> {
        match self.power {
            PowerState::Running => {}
            PowerState::PoweredOff => return Ok(StopReason::PoweredOff),
            PowerState::ProcessorOff => return Ok(StopReason::ProcessorOff),
        }
        let mut settle = true;
        loop {
            if self.stop.raised() {
                return Ok(StopReason::StopRequested);
            }
            if settle
                || self.cpu.retired >= self.pmu_event
                || self.next_event.is_some_and(|at| at <= self.cpu.now())
            {
                if self.synchronize()? {
                    return Ok(StopReason::StopRequested);
                }
            }
            let now = self.cpu.icount;
            if now >= ends.tick_end || self.cpu.retired >= ends.instruction_end {
                return Ok(StopReason::BudgetExhausted);
            }
            let deadline = self.next_event.map(VmInstant::ticks);
            let slice_end = ends.tick_end.min(now.saturating_add(STOP_POLL_TICKS));
            if matches!(self.cpu.activity, Activity::Waiting(_)) && !self.cpu.wake_if_pending() {
                let Some(wake) = wait_target!(self.cpu, deadline) else {
                    if ends.tick_end != u64::MAX {
                        self.cpu.wait_until(VmInstant::from_ticks(ends.tick_end));
                    }
                    return Ok(StopReason::Halted);
                };
                self.cpu
                    .wait_until(VmInstant::from_ticks(wake.min(slice_end)));
                settle = false;
                continue;
            }
            if let Some(taken) = self.cpu.deliver_interrupt() {
                self.observer.exception(taken);
            }
            let burst = slice_end
                .min(deadline.unwrap_or(u64::MAX))
                .saturating_sub(now)
                .min(ends.instruction_end - self.cpu.retired)
                .min(self.pmu_event - self.cpu.retired);
            self.cpu.bound_run(burst);
            match self
                .cpu
                .run_trace(&mut self.board, &mut self.observer, burst, &[])
            {
                Step::Retired | Step::Waiting => {}
                Step::Stop(cause) => return Ok(self.stopped(cause)),
            }
            settle = self.cpu.outputs_dirty || self.board.devices_touched();
        }
    }

    fn stopped(&mut self, cause: StopCause) -> StopReason {
        match cause {
            StopCause::PowerOff => {
                self.power = PowerState::PoweredOff;
                StopReason::PoweredOff
            }
            StopCause::CpuOff => {
                self.power = PowerState::ProcessorOff;
                StopReason::ProcessorOff
            }
            StopCause::Reset => StopReason::ResetRequested,
            StopCause::Undefined(i) => StopReason::Undefined(i),
            StopCause::BusFault { pc, addr } => StopReason::BusFault { pc, addr },
        }
    }
}
