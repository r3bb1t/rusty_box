//! Stepping a paused processor (spec §7.2): into, over and out, by source
//! line or by instruction, and run to an address. The step machine works
//! over the [`Stepper`] port, which the run thread answers for a real
//! machine and tests answer with a scripted processor.

use std::time::Duration;

pub(crate) use super::unwind::CodeWidth;
pub(crate) use rusty_box::cpu::{BreakpointRefusal, CodeBreakpointHit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepKind {
    Into,
    Over,
    Out,
    RunTo { address: u64 },
}

/// What a step moves by: the source view steps by line, the disassembly by
/// instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepUnit {
    Line,
    Instruction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StepRequest {
    pub(crate) kind: StepKind,
    pub(crate) unit: StepUnit,
    /// The processor stepped.
    pub(crate) cpu: usize,
}

#[cfg(test)]
impl StepRequest {
    /// One instruction of processor `cpu`, into a call.
    pub(crate) const fn instruction_into(cpu: usize) -> Self {
        StepRequest { kind: StepKind::Into, unit: StepUnit::Instruction, cpu }
    }
}

/// The stepped processor's instruction and stack pointers, and the width
/// of the code it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StepRegisters {
    pub(crate) rip: u64,
    pub(crate) rsp: u64,
    pub(crate) width: CodeWidth,
}

/// What an instruction does to the flow of control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flow {
    Call,
    /// `syscall`, `sysenter`, `int n`: it returns to the next instruction.
    Trap,
    Return,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Decoded {
    pub(crate) length: u64,
    pub(crate) flow: Flow,
}

/// A source line: the module (its base), the file (a place in its tables'
/// files) and the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourcePlace {
    pub(crate) module: u64,
    pub(crate) file: usize,
    pub(crate) line: u32,
}

/// A line row in linear addresses: `start..end` is line `place`'s code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RowSpan {
    pub(crate) start: u64,
    pub(crate) end: u64,
    pub(crate) place: SourcePlace,
}

/// Where the current frame returns: its caller's instruction and its stack
/// pointer once the return has run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CallerFrame {
    pub(crate) rip: u64,
    pub(crate) rsp: u64,
}

/// What one instruction came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stepped {
    Retired,
    /// It retired onto an instruction with one of the owner's breakpoints.
    UserBreakpoint(CodeBreakpointHit),
    /// The machine powered off or shut down.
    MachineOver,
}

/// What a run came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunStop {
    /// The stepped processor is at one of the targets.
    AtTarget,
    UserBreakpoint(CodeBreakpointHit),
    MachineOver,
    Cancelled,
    /// The step has run its time.
    Ceiling,
    /// The machine did not take the targets beside the owner's breakpoints.
    BreakpointsRefused(BreakpointRefusal),
}

/// A paused processor the step machine drives.
pub(crate) trait Stepper {
    fn registers(&mut self) -> StepRegisters;
    /// Runs one instruction.
    fn single_step(&mut self) -> Stepped;
    /// Runs until the stepped processor is about to run an instruction at
    /// one of `targets` (temporary breakpoints beside the owner's), or until
    /// the step has run for `until`. A target at the current instruction is
    /// left first; a target another processor reaches is not a stop.
    fn run_until(&mut self, targets: &[u64], until: Duration) -> RunStop;
    fn instruction_at(&mut self, rip: u64) -> Option<Decoded>;
    /// The line row holding `rip`, from the module's tables.
    fn line_row(&mut self, rip: u64) -> Option<RowSpan>;
    /// The current frame's caller, by the unwinder.
    fn caller(&mut self) -> Option<CallerFrame>;
    /// Whether the owner asked the step to stop.
    fn cancelled(&mut self) -> bool;
    /// How long the step has run.
    fn elapsed(&mut self) -> Duration;
}

/// The step's limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StepLimits {
    pub(crate) single_steps: u64,
    pub(crate) time: Duration,
}

/// Spec §7.2: ten million instructions single-stepped or a minute of
/// running, whichever comes first.
pub(crate) const STEP_LIMITS: StepLimits = StepLimits { single_steps: 10_000_000, time: Duration::from_secs(60) };

/// Which limit a step reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepCeiling {
    Instructions(u64),
    Time,
}

/// How a step ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepEnd {
    Done,
    UserBreakpoint(CodeBreakpointHit),
    Ceiling(StepCeiling),
    Cancelled,
    MachineOver,
    /// Step out: the unwinder did not find where the frame returns.
    NoCaller,
    /// The machine did not take the step's breakpoints beside the owner's.
    BreakpointsRefused(BreakpointRefusal),
}

/// How far a 32-bit return may leave the stack pointer above the caller's:
/// a `stdcall` callee's `ret n` pops up to 65535 bytes of arguments.
const MOST_POPPED: u64 = 0xFFFF;

/// Whether `registers` are the caller's after a return to it: 64-bit code
/// is at the caller's stack pointer exactly (no x64 convention pops its
/// arguments in the callee); 32-bit code is there or up to `ret n`'s
/// arguments above it. A deeper call (recursion) or another stack passing
/// the return address is not.
fn returned(registers: StepRegisters, caller_rsp: u64) -> bool {
    match registers.width {
        CodeWidth::Long64 => registers.rsp == caller_rsp,
        CodeWidth::Protected32 => registers.rsp.checked_sub(caller_rsp).is_some_and(|above| above <= MOST_POPPED),
    }
}

/// The single steps a step has taken, against its limits.
struct Budget {
    limits: StepLimits,
    single_steps: u64,
}

impl Budget {
    /// Whether the step may go on: under its limits and not cancelled.
    fn check<S: Stepper>(&self, stepper: &mut S) -> Result<(), StepEnd> {
        if stepper.cancelled() {
            return Err(StepEnd::Cancelled);
        }
        if self.single_steps >= self.limits.single_steps {
            return Err(StepEnd::Ceiling(StepCeiling::Instructions(self.single_steps)));
        }
        if stepper.elapsed() >= self.limits.time {
            return Err(StepEnd::Ceiling(StepCeiling::Time));
        }
        Ok(())
    }

    fn single_step<S: Stepper>(&mut self, stepper: &mut S) -> Result<(), StepEnd> {
        self.check(stepper)?;
        self.single_steps = self.single_steps.saturating_add(1);
        match stepper.single_step() {
            Stepped::Retired => Ok(()),
            Stepped::UserBreakpoint(hit) => Err(StepEnd::UserBreakpoint(hit)),
            Stepped::MachineOver => Err(StepEnd::MachineOver),
        }
    }

    /// Runs until the processor is at `target` with `caller_rsp`'s stack
    /// (or anywhere at `target`, without one).
    fn run_to<S: Stepper>(&mut self, stepper: &mut S, target: u64, caller_rsp: Option<u64>) -> Result<(), StepEnd> {
        loop {
            self.check(stepper)?;
            match stepper.run_until(&[target], self.limits.time) {
                RunStop::AtTarget => {
                    let registers = stepper.registers();
                    if registers.rip == target && caller_rsp.is_none_or(|caller_rsp| returned(registers, caller_rsp)) {
                        return Ok(());
                    }
                }
                RunStop::UserBreakpoint(hit) => return Err(StepEnd::UserBreakpoint(hit)),
                RunStop::MachineOver => return Err(StepEnd::MachineOver),
                RunStop::Cancelled => return Err(StepEnd::Cancelled),
                RunStop::Ceiling => return Err(StepEnd::Ceiling(StepCeiling::Time)),
                RunStop::BreakpointsRefused(refusal) => return Err(StepEnd::BreakpointsRefused(refusal)),
            }
        }
    }

    /// Runs the current frame to its return.
    fn out<S: Stepper>(&mut self, stepper: &mut S) -> Result<(), StepEnd> {
        let caller = stepper.caller().ok_or(StepEnd::NoCaller)?;
        self.run_to(stepper, caller.rip, Some(caller.rsp))
    }

    /// One instruction, `decoded`; a call or trap is run to its return when
    /// `over`.
    fn instruction<S: Stepper>(&mut self, stepper: &mut S, decoded: Option<Decoded>, over: bool) -> Result<(), StepEnd> {
        match decoded {
            Some(Decoded { length, flow: Flow::Call | Flow::Trap }) if over => {
                let registers = stepper.registers();
                self.run_to(stepper, registers.rip.wrapping_add(length), Some(registers.rsp))
            }
            Some(_) | None => self.single_step(stepper),
        }
    }

    /// The instruction at the processor's RIP.
    fn decode<S: Stepper>(stepper: &mut S) -> Option<Decoded> {
        let rip = stepper.registers().rip;
        stepper.instruction_at(rip)
    }

    /// Steps until the processor is at the start of a row of another line
    /// than `from` (any row, without one). A function entered without rows
    /// is stepped out of again; code without rows reached otherwise ends the
    /// step, the source left.
    fn lines<S: Stepper>(&mut self, stepper: &mut S, from: Option<SourcePlace>, over: bool) -> Result<(), StepEnd> {
        loop {
            let decoded = Self::decode(stepper);
            self.instruction(stepper, decoded, over)?;
            let flow = decoded.map_or(Flow::Other, |decoded| decoded.flow);
            if self.settled(stepper, from, flow)? {
                return Ok(());
            }
        }
    }

    /// Whether the step ends where the processor is, having got there by an
    /// instruction of `flow`.
    fn settled<S: Stepper>(&mut self, stepper: &mut S, from: Option<SourcePlace>, flow: Flow) -> Result<bool, StepEnd> {
        let rip = stepper.registers().rip;
        match stepper.line_row(rip) {
            Some(row) => Ok(rip == row.start && from != Some(row.place)),
            None if matches!(flow, Flow::Call | Flow::Trap) => {
                self.out(stepper)?;
                let rip = stepper.registers().rip;
                Ok(stepper.line_row(rip).is_some_and(|row| rip == row.start && from != Some(row.place)))
            }
            None => Ok(true),
        }
    }
}

/// Runs `request` on `stepper` within `limits`.
pub(crate) fn step<S: Stepper>(stepper: &mut S, request: &StepRequest, limits: &StepLimits) -> StepEnd {
    let mut budget = Budget { limits: *limits, single_steps: 0 };
    let start = stepper.registers().rip;
    let by_line = request.unit == StepUnit::Line;
    // Code without rows is stepped by instruction, whatever the unit.
    let from = stepper.line_row(start).map(|row| row.place).filter(|_| by_line);
    let ended = match request.kind {
        StepKind::Into if from.is_some() => budget.lines(stepper, from, false),
        StepKind::Over if from.is_some() => budget.lines(stepper, from, true),
        StepKind::Into => {
            let decoded = Budget::decode(stepper);
            budget.instruction(stepper, decoded, false)
        }
        StepKind::Over => {
            let decoded = Budget::decode(stepper);
            budget.instruction(stepper, decoded, true)
        }
        // Back in the caller, a step by line goes on to a row start.
        StepKind::Out if by_line => budget.out(stepper).and_then(|()| match budget.settled(stepper, None, Flow::Return)? {
            true => Ok(()),
            false => budget.lines(stepper, None, true),
        }),
        StepKind::Out => budget.out(stepper),
        StepKind::RunTo { address } => budget.run_to(stepper, address, None),
    };
    match ended {
        Ok(()) => StepEnd::Done,
        Err(end) => end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box::cpu::CodeBreakpoint;
    use std::collections::{BTreeMap, BTreeSet};

    /// What one scripted instruction does.
    #[derive(Clone, Copy)]
    enum Op {
        Next,
        Jump(u64),
        Call(u64),
        /// Calls `target` while the call depth is below `depth`, else falls
        /// through.
        CallWhile { target: u64, depth: usize },
        Ret,
        /// `ret n`: returns and pops `n` bytes of arguments.
        RetPopping(u64),
        PowerOff,
    }

    #[derive(Clone, Copy)]
    struct Instruction {
        length: u64,
        op: Op,
    }

    const TOP: u64 = 0x8000;

    /// A processor running a scripted program; its stack holds return
    /// addresses only.
    struct FakeCpu {
        program: BTreeMap<u64, Instruction>,
        rows: Vec<RowSpan>,
        rip: u64,
        stack: Vec<u64>,
        /// Argument bytes `ret n`s popped, which lift the stack pointer.
        popped: u64,
        width: CodeWidth,
        user_breakpoints: BTreeSet<u64>,
        executed: u64,
        /// The next run stops at its target with this stack pointer, as
        /// another thread reaching the return address would; the processor
        /// itself has not moved.
        ghost: Option<u64>,
        /// The registers a ghost stop shows until the next instruction.
        shown: Option<StepRegisters>,
        cancel_after: Option<u64>,
    }

    impl FakeCpu {
        fn new(rip: u64) -> Self {
            FakeCpu {
                program: BTreeMap::new(),
                rows: Vec::new(),
                rip,
                stack: Vec::new(),
                popped: 0,
                width: CodeWidth::Long64,
                user_breakpoints: BTreeSet::new(),
                executed: 0,
                ghost: None,
                shown: None,
                cancel_after: None,
            }
        }

        fn at(&mut self, address: u64, length: u64, op: Op) {
            self.program.insert(address, Instruction { length, op });
        }

        fn row(&mut self, start: u64, end: u64, line: u32) {
            self.rows.push(RowSpan { start, end, place: SourcePlace { module: 0, file: 0, line } });
        }

        fn rsp(&self) -> u64 {
            TOP - 8 * self.stack.len() as u64 + self.popped
        }

        fn execute(&mut self) -> Stepped {
            self.shown = None;
            self.executed += 1;
            let Some(instruction) = self.program.get(&self.rip).copied() else {
                panic!("no instruction at {:#x}", self.rip);
            };
            let next = self.rip + instruction.length;
            match instruction.op {
                Op::Next => self.rip = next,
                Op::Jump(target) => self.rip = target,
                Op::Call(target) => {
                    self.stack.push(next);
                    self.rip = target;
                }
                Op::CallWhile { target, depth } if self.stack.len() < depth => {
                    self.stack.push(next);
                    self.rip = target;
                }
                Op::CallWhile { .. } => self.rip = next,
                Op::Ret => self.rip = self.stack.pop().expect("a return address"),
                Op::RetPopping(bytes) => {
                    self.rip = self.stack.pop().expect("a return address");
                    self.popped += bytes;
                }
                Op::PowerOff => return Stepped::MachineOver,
            }
            if self.user_breakpoints.contains(&self.rip) {
                return Stepped::UserBreakpoint(hit(self.rip));
            }
            Stepped::Retired
        }
    }

    fn hit(linear: u64) -> CodeBreakpointHit {
        CodeBreakpointHit { cpu: 0, breakpoint: CodeBreakpoint::Linear(linear), linear }
    }

    impl Stepper for FakeCpu {
        fn registers(&mut self) -> StepRegisters {
            self.shown.unwrap_or(StepRegisters { rip: self.rip, rsp: self.rsp(), width: self.width })
        }

        fn single_step(&mut self) -> Stepped {
            self.execute()
        }

        fn run_until(&mut self, targets: &[u64], until: Duration) -> RunStop {
            if let Some(rsp) = self.ghost.take() {
                let target = *targets.first().expect("a target");
                self.shown = Some(StepRegisters { rip: target, rsp, width: self.width });
                return RunStop::AtTarget;
            }
            loop {
                if self.elapsed() >= until {
                    return RunStop::Ceiling;
                }
                if self.cancelled() {
                    return RunStop::Cancelled;
                }
                match self.execute() {
                    Stepped::Retired if targets.contains(&self.rip) => return RunStop::AtTarget,
                    Stepped::Retired => {}
                    Stepped::UserBreakpoint(hit) => return RunStop::UserBreakpoint(hit),
                    Stepped::MachineOver => return RunStop::MachineOver,
                }
            }
        }

        fn instruction_at(&mut self, rip: u64) -> Option<Decoded> {
            let instruction = self.program.get(&rip)?;
            let flow = match instruction.op {
                Op::Call(_) | Op::CallWhile { .. } => Flow::Call,
                Op::Ret | Op::RetPopping(_) => Flow::Return,
                Op::Next | Op::Jump(_) | Op::PowerOff => Flow::Other,
            };
            Some(Decoded { length: instruction.length, flow })
        }

        fn line_row(&mut self, rip: u64) -> Option<RowSpan> {
            self.rows.iter().copied().find(|row| row.start <= rip && rip < row.end)
        }

        fn caller(&mut self) -> Option<CallerFrame> {
            let rip = *self.stack.last()?;
            Some(CallerFrame { rip, rsp: self.rsp() + 8 })
        }

        fn cancelled(&mut self) -> bool {
            self.cancel_after.is_some_and(|after| self.executed >= after)
        }

        fn elapsed(&mut self) -> Duration {
            Duration::from_micros(self.executed)
        }
    }

    const LIMITS: StepLimits = StepLimits { single_steps: 1000, time: Duration::from_secs(1) };

    /// `main`: line 1 at 0x100; line 2 (`x = f();`) is 0x104..0x10D, its
    /// `call` at 0x104 returning to 0x109 inside it; line 3 at 0x10D, line 4
    /// at 0x111, then a `ret`. `f` at 0x200 (lines 10–12) calls itself at
    /// 0x204 while the call depth is below three. `g` at 0x400 has no rows.
    fn program(rip: u64) -> FakeCpu {
        let mut cpu = FakeCpu::new(rip);
        cpu.at(0x100, 4, Op::Next);
        cpu.at(0x104, 5, Op::Call(0x200));
        cpu.at(0x109, 4, Op::Next);
        cpu.at(0x10D, 4, Op::Call(0x400));
        cpu.at(0x111, 1, Op::Ret);
        cpu.row(0x100, 0x104, 1);
        cpu.row(0x104, 0x10D, 2);
        cpu.row(0x10D, 0x111, 3);
        cpu.row(0x111, 0x112, 4);
        cpu.at(0x200, 4, Op::Next);
        cpu.at(0x204, 5, Op::CallWhile { target: 0x200, depth: 3 });
        cpu.at(0x209, 1, Op::Ret);
        cpu.row(0x200, 0x204, 10);
        cpu.row(0x204, 0x209, 11);
        cpu.row(0x209, 0x20A, 12);
        cpu.at(0x400, 4, Op::Next);
        cpu.at(0x404, 1, Op::Ret);
        // `main`'s own caller.
        cpu.stack.push(0x900);
        cpu.at(0x900, 1, Op::PowerOff);
        cpu
    }

    fn request(kind: StepKind, unit: StepUnit) -> StepRequest {
        StepRequest { kind, unit, cpu: 0 }
    }

    #[test]
    fn into_by_instruction_runs_one_instruction() {
        let mut cpu = program(0x104);
        assert_eq!(step(&mut cpu, &request(StepKind::Into, StepUnit::Instruction), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x200);
    }

    #[test]
    fn into_by_line_stops_at_the_next_row_start() {
        let mut cpu = program(0x100);
        assert_eq!(step(&mut cpu, &request(StepKind::Into, StepUnit::Line), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x104);
    }

    #[test]
    fn into_by_line_enters_a_callee_with_rows() {
        let mut cpu = program(0x104);
        assert_eq!(step(&mut cpu, &request(StepKind::Into, StepUnit::Line), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x200);
        assert_eq!(cpu.stack.len(), 2);
    }

    /// `g` has no rows: it is stepped out of again, and the step goes on
    /// to the next row start of `main`.
    #[test]
    fn into_by_line_steps_back_out_of_a_callee_without_rows() {
        let mut cpu = program(0x10D);
        assert_eq!(step(&mut cpu, &request(StepKind::Into, StepUnit::Line), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x111);
        assert_eq!(cpu.stack.len(), 1);
    }

    /// Review focus 1: the call returns into the middle of line 2's row;
    /// the step goes on to line 3's start.
    #[test]
    fn over_by_line_runs_a_call_and_stops_at_a_row_start() {
        let mut cpu = program(0x104);
        assert_eq!(step(&mut cpu, &request(StepKind::Over, StepUnit::Line), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x10D);
        assert_eq!(cpu.stack.len(), 1);
    }

    #[test]
    fn over_by_instruction_runs_a_call_to_its_return() {
        let mut cpu = program(0x104);
        assert_eq!(step(&mut cpu, &request(StepKind::Over, StepUnit::Instruction), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x109);
        let mut plain = program(0x100);
        assert_eq!(step(&mut plain, &request(StepKind::Over, StepUnit::Instruction), &LIMITS), StepEnd::Done);
        assert_eq!(plain.rip, 0x104, "an instruction that is no call is one step");
    }

    /// Review focus 2: inside `f`, stepping over its recursive call: the
    /// deeper calls pass the return address first and are let through.
    #[test]
    fn over_a_recursive_call_stops_at_its_own_return() {
        let mut cpu = program(0x204);
        cpu.stack.push(0x109);
        let depth = cpu.stack.len();
        assert_eq!(step(&mut cpu, &request(StepKind::Over, StepUnit::Instruction), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x209);
        assert_eq!(cpu.stack.len(), depth, "the outer return, not a deeper one");
    }

    /// Review focus 2: another thread reaches the return address first.
    #[test]
    fn over_goes_on_when_another_stack_reaches_the_return() {
        let mut cpu = program(0x104);
        // A kernel thread's stack a few pages above.
        cpu.ghost = Some(TOP - 8 + 0x3000);
        assert_eq!(step(&mut cpu, &request(StepKind::Over, StepUnit::Instruction), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.registers(), StepRegisters { rip: 0x109, rsp: TOP - 8, width: CodeWidth::Long64 });
    }

    /// A 32-bit `stdcall` callee pops its arguments: the return still
    /// counts, its stack pointer above the caller's by them.
    #[test]
    fn over_a_32_bit_call_whose_callee_pops_its_arguments() {
        let mut cpu = program(0x104);
        cpu.width = CodeWidth::Protected32;
        cpu.at(0x209, 3, Op::RetPopping(8));
        cpu.at(0x204, 5, Op::Next);
        assert_eq!(step(&mut cpu, &request(StepKind::Over, StepUnit::Instruction), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.registers(), StepRegisters { rip: 0x109, rsp: TOP - 8 + 8, width: CodeWidth::Protected32 });
    }

    #[test]
    fn out_by_instruction_returns_to_the_caller() {
        let mut cpu = program(0x200);
        cpu.stack.push(0x109);
        assert_eq!(step(&mut cpu, &request(StepKind::Out, StepUnit::Instruction), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x109);
        assert_eq!(cpu.stack.len(), 1);
    }

    #[test]
    fn out_by_line_goes_on_to_a_row_start() {
        let mut cpu = program(0x200);
        cpu.stack.push(0x109);
        assert_eq!(step(&mut cpu, &request(StepKind::Out, StepUnit::Line), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x10D);
    }

    #[test]
    fn out_without_a_known_caller_says_so() {
        let mut cpu = program(0x100);
        cpu.stack.clear();
        assert_eq!(step(&mut cpu, &request(StepKind::Out, StepUnit::Instruction), &LIMITS), StepEnd::NoCaller);
    }

    #[test]
    fn run_to_stops_at_the_address() {
        let mut cpu = program(0x100);
        let end = step(&mut cpu, &request(StepKind::RunTo { address: 0x10D }, StepUnit::Line), &LIMITS);
        assert_eq!(end, StepEnd::Done);
        assert_eq!(cpu.rip, 0x10D);
    }

    #[test]
    fn a_user_breakpoint_ends_a_step() {
        let mut cpu = program(0x104);
        cpu.user_breakpoints.insert(0x204);
        assert_eq!(step(&mut cpu, &request(StepKind::Over, StepUnit::Line), &LIMITS), StepEnd::UserBreakpoint(hit(0x204)));
        let mut cpu = program(0x100);
        cpu.user_breakpoints.insert(0x104);
        assert_eq!(
            step(&mut cpu, &request(StepKind::Into, StepUnit::Instruction), &LIMITS),
            StepEnd::UserBreakpoint(hit(0x104))
        );
    }

    #[test]
    fn a_line_that_never_ends_stops_at_the_instruction_ceiling() {
        let mut cpu = FakeCpu::new(0x300);
        cpu.at(0x300, 2, Op::Jump(0x300));
        cpu.row(0x300, 0x302, 20);
        let end = step(&mut cpu, &request(StepKind::Into, StepUnit::Line), &LIMITS);
        assert_eq!(end, StepEnd::Ceiling(StepCeiling::Instructions(LIMITS.single_steps)));
    }

    #[test]
    fn a_run_that_never_returns_stops_at_the_time_ceiling() {
        let mut cpu = program(0x300);
        cpu.at(0x300, 5, Op::Call(0x305));
        cpu.at(0x305, 2, Op::Jump(0x305));
        let end = step(&mut cpu, &request(StepKind::Over, StepUnit::Instruction), &LIMITS);
        assert_eq!(end, StepEnd::Ceiling(StepCeiling::Time));
    }

    #[test]
    fn a_step_is_cancelled() {
        let mut cpu = FakeCpu::new(0x300);
        cpu.at(0x300, 2, Op::Jump(0x300));
        cpu.row(0x300, 0x302, 20);
        cpu.cancel_after = Some(10);
        assert_eq!(step(&mut cpu, &request(StepKind::Into, StepUnit::Line), &LIMITS), StepEnd::Cancelled);
        assert!(cpu.executed <= 11);
    }

    #[test]
    fn a_machine_that_powers_off_ends_the_step() {
        let mut cpu = program(0x111);
        assert_eq!(step(&mut cpu, &request(StepKind::Over, StepUnit::Line), &LIMITS), StepEnd::Done, "returns to 0x900");
        assert_eq!(step(&mut cpu, &request(StepKind::Into, StepUnit::Instruction), &LIMITS), StepEnd::MachineOver);
    }

    /// Code with no rows is stepped by instruction, whatever the unit.
    #[test]
    fn by_line_without_rows_steps_an_instruction() {
        let mut cpu = program(0x400);
        cpu.stack.push(0x111);
        assert_eq!(step(&mut cpu, &request(StepKind::Into, StepUnit::Line), &LIMITS), StepEnd::Done);
        assert_eq!(cpu.rip, 0x404);
    }
}
