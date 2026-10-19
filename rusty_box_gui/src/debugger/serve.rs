//! What a run thread does with a machine that is between runs: the shell's
//! queued work, one task at a time, until the shell lets it run on or ends it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rusty_box::cpu::decoder::Opcode;
use rusty_box::cpu::{BreakpointRefusal, CodeBreakpoint, CodeBreakpointHit, CodeSize, CpuDebugView, Instrumentation};
use rusty_box::emulator::{Emulator, RunBudget, SliceEngine, StopReason};
use rusty_box_symbols::units::Rva;

use super::control::{BetweenRuns, DebugNote, RunControl, StepCut, Task};
use super::dump::dump_ram;
use super::guest::{GuestMemory, GuestRead, Root};
use super::inspect::{read_snapshot, DebugSnapshot, SymModule};
use super::search::find;
use super::stepping::{
    step, CallerFrame, CodeWidth, Decoded, Flow, RowSpan, RunStop, SourcePlace, StepEnd, StepRegisters, StepRequest,
    Stepped, Stepper, STEP_LIMITS,
};
use super::unwind::{Frame, FrameFound, FrameRegisters};
use super::frames::{caller_frame, frames_at, read_frames, CallerOf};
use super::guest::{vmx_guest_view, PausedCpu};
use super::source_breakpoints::binaries_to_find;
use super::windows::{modules_elsewhere, running_space};
use super::xref::image::{copy_image, ImageAt, ImageOffset};
use super::xref::index::ModuleSpan;
use super::xref::service::{CopyRequest, IndexJob, IndexState};
use crate::symbols::hub::SymbolsView;

/// Whether the engine can stop after one instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stepping {
    /// The interpreter retires and counts every instruction.
    Instructions,
    /// The hypervisor retires instructions nothing counts.
    #[cfg_attr(
        not(all(not(feature = "guest-trace"), feature = "hv-whp", windows)),
        allow(dead_code, reason = "only a hypervisor run cannot step")
    )]
    Unavailable,
}

/// Whether the guest is still alive after a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MachineAfter {
    Live,
    /// A step ended in the guest switching itself off or shutting down.
    Over,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Performed {
    /// Instructions the boot processor retired doing the task.
    pub(crate) retired: u64,
    pub(crate) after: MachineAfter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Next {
    Resume,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Served {
    pub(crate) next: Next,
    pub(crate) retired: u64,
}

/// Performs `task` on `emu`, which is between runs.
///
/// A step runs with the stop flag raised: a batch is never refused for the
/// flag, which is read only after it (`run_until_budget_spent`), so exactly
/// one instruction retires.
pub(crate) fn perform<T: Instrumentation, E: SliceEngine<T>>(
    emu: &mut Emulator<T, E>,
    control: &RunControl,
    task: Task,
    stepping: Stepping,
) -> Result<Performed, rusty_box::Error> {
    let live = |retired| Performed {
        retired,
        after: MachineAfter::Live,
    };
    match task {
        Task::Refresh => {
            // A paused machine retires nothing, and the status bar says so.
            if let Some(gui) = emu.gui_mut() {
                gui.show_ips(0);
            }
            let snapshot = read_for(emu, control);
            control.publish_view(snapshot);
            Ok(live(0))
        }
        Task::Step(request) => {
            let _finished = StepUnderWay { control };
            perform_step(emu, control, request, stepping)
        }
        Task::DumpRam(path) => {
            control.record_dump(dump_ram(emu, &path, || control.ending()));
            Ok(live(0))
        }
        Task::Find(request) => {
            let outcome = find(emu, control.view().cpu, &request, || control.ending());
            control.record_find(outcome);
            let snapshot = read_for(emu, control);
            control.publish_view(snapshot);
            Ok(live(0))
        }
        Task::ApplyBreakpoints(set) => {
            apply_breakpoints(emu, control, &set);
            Ok(live(0))
        }
        Task::CopyImages(request) => {
            copy_images(emu, control, request);
            Ok(live(0))
        }
    }
}

/// Copies each module `request` names, through the processor the page shows,
/// and hands it to the cross-reference service with its PDB procedures as
/// sync points; the service indexes it off this thread while the next one is
/// copied. A module that does not copy is recorded as failed, with why.
fn copy_images<T: Instrumentation, E: SliceEngine<T>>(emu: &mut Emulator<T, E>, control: &RunControl, request: CopyRequest) {
    let cpu = control.view().cpu;
    let cpu = if cpu < emu.cpu_count() { cpu } else { 0 };
    let symbols = control.symbols();
    let mut spans = request.spans;
    spans.sort_unstable();
    let spans: Arc<[ModuleSpan]> = Arc::from(spans);
    for module in request.modules {
        // A module cancelled or forgotten since it was asked for is not copied.
        if control.xrefs().state(&module.key) != Some(IndexState::Copying) {
            continue;
        }
        let Some(mut debug) = emu.debug_cpu(cpu) else {
            control.xrefs().fail(module.key, "the processor it is read through is gone".to_owned());
            continue;
        };
        // Paused in VMX root operation, the modules listed are the guest's,
        // read as the walk read them.
        let vmx_guest = debug.vmx_guest();
        let mut paused = PausedCpu { cpu: &mut debug, vmx_guest };
        let at = ImageAt { base: module.key.base, size: module.key.size, root: module.root };
        match copy_image(&mut paused.walked(), at) {
            Ok(image) => {
                let extra_sync = symbols
                    .as_ref()
                    .and_then(|view| view.tables(module.key.identity.known()?).map(Arc::clone))
                    .map(|tables| tables.procedure_starts().into_iter().map(|rva| ImageOffset(rva.0)).collect())
                    .unwrap_or_default();
                control.xrefs().accept(IndexJob {
                    key: module.key,
                    image,
                    extra_sync,
                    spans: Arc::clone(&spans),
                    taken_at: request.taken_at,
                });
            }
            Err(miss) => control.xrefs().fail(module.key, miss.describe()),
        }
    }
}

/// A step under way: the control hears it ended however the step leaves.
struct StepUnderWay<'a> {
    control: &'a RunControl,
}

impl Drop for StepUnderWay<'_> {
    fn drop(&mut self) {
        self.control.step_finished();
    }
}

/// Steps the processor `request` names, and publishes where it stopped.
fn perform_step<T: Instrumentation, E: SliceEngine<T>>(
    emu: &mut Emulator<T, E>,
    control: &RunControl,
    request: StepRequest,
    stepping: Stepping,
) -> Result<Performed, rusty_box::Error> {
    let live = |retired| Performed {
        retired,
        after: MachineAfter::Live,
    };
    match stepping {
        Stepping::Instructions => {}
        Stepping::Unavailable => {
            control.note_refusal(DebugNote::StepNeedsInterpreter);
            return Ok(live(0));
        }
    }
    let before = emu.cpu_snapshot().icount;
    // A processor past the last (a view of another machine) steps the boot
    // processor.
    let cpu = if request.cpu < emu.cpu_count() { request.cpu } else { 0 };
    // A machine has its boot processor at least.
    let Some(mut stepper) = MachineStepper::new(emu, control, cpu) else {
        return Ok(live(0));
    };
    let end = step(&mut stepper, &StepRequest { cpu, ..request }, &STEP_LIMITS);
    if let Some(failure) = stepper.failure.take() {
        return Err(failure);
    }
    emu.present_console_output();
    emu.update_gui();
    let snapshot = read_for(emu, control);
    control.publish(snapshot);
    let retired = emu.cpu_snapshot().icount.saturating_sub(before);
    let cut = |cut: StepCut| {
        control.note_refusal(DebugNote::StepCut(cut));
        MachineAfter::Live
    };
    let after = match end {
        StepEnd::Done => MachineAfter::Live,
        StepEnd::UserBreakpoint(hit) => {
            control.stepped_onto_breakpoint(hit);
            MachineAfter::Live
        }
        StepEnd::MachineOver => MachineAfter::Over,
        StepEnd::Ceiling(ceiling) => cut(StepCut::Ceiling(ceiling)),
        StepEnd::Cancelled => cut(StepCut::Cancelled),
        StepEnd::NoCaller => cut(StepCut::NoCaller),
        StepEnd::BreakpointsRefused(refusal) => cut(StepCut::BreakpointsRefused(refusal)),
    };
    Ok(Performed { retired, after })
}

/// Gives `emu` the code breakpoints `set`; a set the machine refuses leaves
/// a note for the page, and the machine keeps the set it had.
pub(crate) fn apply_breakpoints<T: Instrumentation, E: SliceEngine<T>>(
    emu: &mut Emulator<T, E>,
    control: &RunControl,
    set: &[CodeBreakpoint],
) {
    match emu.set_code_breakpoints(set) {
        Ok(()) => control.record_applied_breakpoints(set),
        Err(BreakpointRefusal::EngineCannotStop) => control.note_refusal(DebugNote::BreakpointsNeedInterpreter),
        Err(BreakpointRefusal::TooManyLinear | BreakpointRefusal::TooManyPhysical) => {
            control.note_refusal(DebugNote::TooManyBreakpoints);
        }
    }
}

/// How many instructions a step's run takes at a time between looks at the
/// clock and at the owner's cancel.
const RUN_CHUNK: u64 = 65_536;

/// A paused machine as the step machine drives it: processor `cpu` is the
/// one stepped; its line rows come from the symbol engine's view and the
/// last pause's modules; its caller from the unwinder.
struct MachineStepper<'a, T: Instrumentation, E: SliceEngine<T>> {
    emu: &'a mut Emulator<T, E>,
    control: &'a RunControl,
    cpu: usize,
    /// The owner's breakpoints the machine holds, which a run's targets
    /// join.
    user: Vec<CodeBreakpoint>,
    symbols: Option<Arc<SymbolsView>>,
    modules: Vec<SymModule>,
    started: Instant,
    /// The stepped processor as last read.
    last_view: CpuDebugView,
    /// The machine failed: the step ends, and the run with it.
    failure: Option<rusty_box::Error>,
}

impl<'a, T: Instrumentation, E: SliceEngine<T>> MachineStepper<'a, T, E> {
    /// `None` when the machine has no processor `cpu`.
    fn new(emu: &'a mut Emulator<T, E>, control: &'a RunControl, cpu: usize) -> Option<Self> {
        let last_view = emu.debug_cpu(cpu)?.view();
        let modules = control.snapshot().map(|snapshot| snapshot.symbols.modules.clone()).unwrap_or_default();
        Some(MachineStepper {
            emu,
            control,
            cpu,
            user: control.applied_breakpoints(),
            symbols: control.symbols(),
            modules,
            started: Instant::now(),
            last_view,
            failure: None,
        })
    }

    /// The stepped processor's view.
    fn view(&mut self) -> CpuDebugView {
        if let Some(debug) = self.emu.debug_cpu(self.cpu) {
            self.last_view = debug.view();
        }
        self.last_view
    }

    /// Runs one batch of `budget`; a machine failure ends the step.
    fn run(&mut self, budget: RunBudget) -> Option<StopReason> {
        match self.emu.step(budget) {
            Ok(outcome) => Some(outcome.stop),
            Err(failure) => {
                self.failure = Some(failure);
                None
            }
        }
    }

    /// Runs on, the targets armed, until one of them on the stepped
    /// processor, a breakpoint of the owner's, the clock or a cancel.
    fn run_to_targets(&mut self, targets: &[u64], until: Duration) -> RunStop {
        if targets.contains(&self.registers().rip) {
            match self.single_step() {
                Stepped::Retired if targets.contains(&self.registers().rip) => return RunStop::AtTarget,
                Stepped::Retired => {}
                Stepped::UserBreakpoint(hit) => return RunStop::UserBreakpoint(hit),
                Stepped::MachineOver => return RunStop::MachineOver,
            }
        }
        loop {
            if self.elapsed() >= until {
                return RunStop::Ceiling;
            }
            if self.cancelled() {
                return RunStop::Cancelled;
            }
            match self.run(RunBudget::Instructions(RUN_CHUNK)) {
                Some(StopReason::Breakpoint(hit)) => {
                    if hit.cpu == self.cpu && targets.contains(&hit.linear) {
                        return RunStop::AtTarget;
                    }
                    if self.user.contains(&hit.breakpoint) && owners_stop(self.emu, self.control, &hit) {
                        return RunStop::UserBreakpoint(hit);
                    }
                    // A target another processor reached, or a program's line
                    // in another process: it runs on.
                }
                Some(StopReason::GuestPowerOff | StopReason::CpuShutdown) | None => return RunStop::MachineOver,
                Some(StopReason::BudgetExhausted | StopReason::Halted | StopReason::StopRequested | StopReason::EngineFault) => {}
            }
        }
    }
}

/// What an instruction does to the flow of control, by its opcode.
fn flow_of(opcode: Opcode) -> Flow {
    match opcode {
        Opcode::CallEd
        | Opcode::CallEw
        | Opcode::CallJd
        | Opcode::CallJw
        | Opcode::CallJq
        | Opcode::CallEq
        | Opcode::CallfOp16Ap
        | Opcode::CallfOp32Ap
        | Opcode::CallfOp16Ep
        | Opcode::CallfOp32Ep
        | Opcode::CallfOp64Ep => Flow::Call,
        Opcode::Syscall
        | Opcode::SyscallLegacy
        | Opcode::Sysenter
        | Opcode::SysenterLongmode
        | Opcode::IntIb
        | Opcode::INT3
        | Opcode::Into => Flow::Trap,
        Opcode::RetOp16
        | Opcode::RetOp16Iw
        | Opcode::RetOp32
        | Opcode::RetOp32Iw
        | Opcode::RetOp64
        | Opcode::RetOp64Iw
        | Opcode::RetfOp16
        | Opcode::RetfOp16Iw
        | Opcode::RetfOp32
        | Opcode::RetfOp32Iw
        | Opcode::RetfOp64
        | Opcode::RetfOp64Iw
        | Opcode::IretOp16
        | Opcode::IretOp32
        | Opcode::IretOp64 => Flow::Return,
        _ => Flow::Other,
    }
}

/// The longest x86 instruction.
const MAX_INSTRUCTION_BYTES: usize = 15;

impl<T: Instrumentation, E: SliceEngine<T>> Stepper for MachineStepper<'_, T, E> {
    fn registers(&mut self) -> StepRegisters {
        let view = self.view();
        match view.code_size {
            CodeSize::Bits64 => StepRegisters { rip: view.registers.rip, rsp: view.registers.rsp, width: CodeWidth::Long64 },
            // Breakpoints and line rows are by linear address: CS's base plus
            // EIP.
            CodeSize::Bits32 | CodeSize::Bits16 => StepRegisters {
                rip: view.segment_bases.cs.wrapping_add(view.registers.rip) & 0xFFFF_FFFF,
                rsp: view.segment_bases.ss.wrapping_add(view.registers.rsp) & 0xFFFF_FFFF,
                width: CodeWidth::Protected32,
            },
        }
    }

    fn single_step(&mut self) -> Stepped {
        match self.run(RunBudget::Instructions(1)) {
            Some(StopReason::Breakpoint(hit)) if owners_stop(self.emu, self.control, &hit) => Stepped::UserBreakpoint(hit),
            // A program's line in another process: the step goes on.
            Some(StopReason::Breakpoint(_)) => Stepped::Retired,
            Some(StopReason::GuestPowerOff | StopReason::CpuShutdown) | None => Stepped::MachineOver,
            Some(StopReason::BudgetExhausted | StopReason::Halted | StopReason::StopRequested | StopReason::EngineFault) => {
                Stepped::Retired
            }
        }
    }

    fn run_until(&mut self, targets: &[u64], until: Duration) -> RunStop {
        let mut armed = self.user.clone();
        armed.extend(targets.iter().map(|target| CodeBreakpoint::Linear(*target)));
        if let Err(refusal) = self.emu.set_code_breakpoints(&armed) {
            return RunStop::BreakpointsRefused(refusal);
        }
        let stop = self.run_to_targets(targets, until);
        match self.emu.set_code_breakpoints(&self.user) {
            Ok(()) => stop,
            Err(refusal) => RunStop::BreakpointsRefused(refusal),
        }
    }

    fn instruction_at(&mut self, rip: u64) -> Option<Decoded> {
        let size = self.view().code_size;
        let mut bytes = [0u8; MAX_INSTRUCTION_BYTES];
        let mut debug = self.emu.debug_cpu(self.cpu)?;
        // An instruction may end before the page that does not map.
        let readable = (1..=MAX_INSTRUCTION_BYTES).rev().find(|length| {
            bytes.get_mut(..*length).is_some_and(|prefix| debug.read_linear(Root::Live, rip, prefix) == GuestRead::Read)
        })?;
        let instruction = size.decode(bytes.get(..readable)?).ok()?;
        Some(Decoded { length: u64::from(instruction.ilen()), flow: flow_of(instruction.get_ia_opcode()) })
    }

    fn line_row(&mut self, rip: u64) -> Option<RowSpan> {
        let module = self.modules.iter().find(|module| rip.checked_sub(module.base).is_some_and(|offset| offset < module.size))?;
        let tables = self.symbols.as_ref()?.tables(module.identity.known()?)?;
        let row = tables.line_at(Rva(rip.checked_sub(module.base)?))?;
        Some(RowSpan {
            start: module.base.wrapping_add(row.start.0),
            end: module.base.wrapping_add(row.end.0),
            place: SourcePlace { module: module.base, file: row.file, line: row.line },
        })
    }

    fn caller(&mut self) -> Option<CallerFrame> {
        let at = self.registers();
        let snapshot = self.view().registers;
        let mut registers = [None; 16];
        for (slot, value) in registers.iter_mut().zip([
            snapshot.rax,
            snapshot.rcx,
            snapshot.rdx,
            snapshot.rbx,
            snapshot.rsp,
            snapshot.rbp,
            snapshot.rsi,
            snapshot.rdi,
            snapshot.r8,
            snapshot.r9,
            snapshot.r10,
            snapshot.r11,
            snapshot.r12,
            snapshot.r13,
            snapshot.r14,
            snapshot.r15,
        ]) {
            *slot = Some(value);
        }
        let of = CallerOf { root: Root::Live, modules: &self.modules, width: at.width };
        let frame = Frame { registers: FrameRegisters { rip: at.rip, rsp: at.rsp, registers }, how: FrameFound::Registers };
        let mut debug = self.emu.debug_cpu(self.cpu)?;
        let caller = caller_frame(&mut debug, &of, self.symbols.as_deref(), &frame)?;
        Some(CallerFrame { rip: caller.registers.rip, rsp: caller.registers.rsp })
    }

    fn cancelled(&mut self) -> bool {
        self.control.step_cancelled()
    }

    fn elapsed(&mut self) -> Duration {
        self.started.elapsed()
    }
}

/// Whether a breakpoint stop at `hit` is one of the owner's: a user
/// program's line stops only in the processes running it, which the address
/// space processor `hit.cpu` runs in tells (read as the pause read reads it:
/// the guest a VMCS holds when paused in VMX root, KVA-shadow tables seen
/// through). A stop on a machine without that processor is the owner's.
pub(crate) fn owners_stop<T: Instrumentation, E: SliceEngine<T>>(
    emu: &mut Emulator<T, E>,
    control: &RunControl,
    hit: &CodeBreakpointHit,
) -> bool {
    if !control.has_scopes() {
        return true;
    }
    let Some(mut debug) = emu.debug_cpu(hit.cpu) else {
        return true;
    };
    let processor = debug.view();
    let vmx_guest = debug.vmx_guest();
    let view = match &vmx_guest {
        Some(guest) => vmx_guest_view(guest, &processor),
        None => processor,
    };
    let mut paused = PausedCpu { cpu: &mut debug, vmx_guest };
    let space = running_space(&mut paused.walked(), &view, &mut control.discovery());
    control.admits(hit, space)
}

/// Reads the snapshot the shell's view asks for, under the kernel layouts the
/// shell handed over. What the guest's own structures gave the last snapshot
/// is carried over when the guest has not moved and the view still asks for
/// the same processor and listing under the same layouts; a view that only
/// changed where memory is shown does not walk them again.
fn read_for<T: Instrumentation, E: SliceEngine<T>>(
    emu: &mut Emulator<T, E>,
    control: &RunControl,
) -> DebugSnapshot {
    let view = control.view();
    let unmoved = control.unmoved_snapshot();
    let layout = control.kernel_layout();
    let mut snapshot = {
        let mut discovery = control.discovery();
        read_snapshot(emu, &view, &mut discovery, unmoved.as_deref(), layout.as_ref())
    };
    // The call stack from where the panes stand, through the processor they
    // were read through (as the guest's own structures were, in VMX root).
    let at = frames_at(&snapshot);
    let request = control.inspect();
    let symbols = control.symbols();
    if let Some(mut debug) = emu.debug_cpu(snapshot.cpu) {
        let processor = debug.view();
        let vmx_guest = debug.vmx_guest();
        let view = match &vmx_guest {
            Some(guest) => vmx_guest_view(guest, &processor),
            None => processor,
        };
        let mut paused = PausedCpu { cpu: &mut debug, vmx_guest };
        snapshot.frames = read_frames(&mut paused.walked(), &at, &snapshot.symbols, symbols.as_deref(), &request);
        // The owner's programs in the processes the panes do not show, which
        // a source line binds in as well.
        let names = symbols
            .as_deref()
            .map(|view| binaries_to_find(&control.targets(), &view.projects))
            .unwrap_or_default();
        snapshot.elsewhere = modules_elsewhere(&mut paused.walked(), &view, &mut control.discovery(), layout.as_ref(), &names);
    }
    snapshot
}

/// Serves the shell until it lets the machine run on or ends the run.
pub(crate) fn serve<T: Instrumentation, E: SliceEngine<T>>(
    emu: &mut Emulator<T, E>,
    control: &RunControl,
    stepping: Stepping,
) -> Result<Served, rusty_box::Error> {
    let mut retired = 0u64;
    loop {
        match control.next_between_runs() {
            BetweenRuns::Perform(task) => {
                let performed = perform(emu, control, task, stepping)?;
                retired = retired.saturating_add(performed.retired);
                match performed.after {
                    MachineAfter::Live => {}
                    MachineAfter::Over => {
                        return Ok(Served {
                            next: Next::End,
                            retired,
                        })
                    }
                }
            }
            BetweenRuns::Resume => {
                return Ok(Served {
                    next: Next::Resume,
                    retired,
                })
            }
            BetweenRuns::End => {
                return Ok(Served {
                    next: Next::End,
                    retired,
                })
            }
        }
    }
}

#[cfg(all(test, feature = "gui-egui"))]
mod tests {
    use super::*;
    use crate::debugger::control::{DumpOutcome, StepCut};
    use crate::debugger::inspect::{ModuleSpace, SymModule, SymbolIndex};
    use crate::debugger::module_identity::GuestIdentity;
    use crate::debugger::stepping::{StepKind, StepRequest, StepUnit};
    use crate::debugger::guest::Root;
    use crate::debugger::xref::decode::XrefKind;
    use crate::debugger::xref::service::{
        AddressRange, CopyModule, FoundSite, IndexSpace, IndexState, ModuleKey, XrefQuery, XrefScope, XrefTarget,
    };
    use crate::debugger::xref::test_image::{module_image, Function, ImageSpec};
    use crate::symbols::hub::{ModuleEntry, ModuleSymbols, SymbolsView};
    use rusty_box::cpu::{CodeBreakpointHit, CpuSetupMode, X86Reg};
    use rusty_box::emulator::EmulatorConfig;
    use rusty_box_symbols::identity::{Guid, ModuleIdentity, PdbId, PeImageId};
    use rusty_box_symbols::service::SymbolSource;
    use rusty_box_symbols::tables::SymbolTables;
    use rusty_box_symbols_fixtures::pdb::{
        build_pdb, FixtureLine, FixtureLocal, FixturePlace, FixtureProcedure, FixtureScope, FixtureType, PdbFixture,
    };
    use crate::debugger::frames::{LocalsRead, TipRead};
    use crate::debugger::source_breakpoints::{
        applied_set, AppliedBreakpoints, BreakpointModules, BreakpointTarget, ModulesElsewhere, ScopedAddress, SourceTarget,
    };
    use crate::debugger::variables::{ShownValue, Signedness};
    use std::sync::{atomic::AtomicBool, Arc};

    const ONE: StepRequest = StepRequest::instruction_into(0);

    /// A long-mode machine at `nop ; nop ; hlt`, paused under a control that
    /// shares its stop flag, with the pause's first snapshot published.
    fn paused() -> (Box<Emulator>, RunControl, u64) {
        let mut machine =
            Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
                .expect("machine");
        let code = CpuSetupMode::FlatLong64.first_free_physical_address();
        machine.mem_write(code, &[0x90, 0x90, 0xF4]).expect("write");
        machine.reg_write(X86Reg::Rip, code);
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        machine.set_stop_flag(Arc::clone(control.stop_flag()));
        control.begin_run(crate::debugger::control::RunStart::PowerOn);
        control.pause();
        let BetweenRuns::Perform(task) = control.next_between_runs() else {
            panic!("a pause publishes first")
        };
        perform(&mut machine, &control, task, Stepping::Instructions).expect("refresh");
        (machine, control, code)
    }

    #[test]
    fn a_step_retires_one_instruction_and_publishes_where_it_stopped() {
        let (mut machine, control, code) = paused();
        assert_eq!(
            control.snapshot().expect("published").cpus[0].registers.rip,
            code
        );
        control.step(ONE);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step(ONE)));
        let performed =
            perform(&mut machine, &control, Task::Step(ONE), Stepping::Instructions).expect("step");
        assert_eq!(performed.retired, 1);
        assert_eq!(performed.after, MachineAfter::Live);
        assert_eq!(
            control.snapshot().expect("published").cpus[0].registers.rip,
            code + 1
        );
    }

    /// A set given between runs reaches the machine: a step that lands on a
    /// breakpoint says so, and the next step executes its instruction.
    #[test]
    fn a_step_onto_a_breakpoint_shows_it_and_the_next_step_executes_it() {
        use rusty_box::cpu::CodeBreakpointHit;
        let (mut machine, control, code) = paused();
        let at = CodeBreakpoint::Linear(code + 1);
        perform(&mut machine, &control, Task::ApplyBreakpoints(vec![at]), Stepping::Instructions).expect("apply");
        assert_eq!(control.note(), None, "the interpreter takes breakpoints");
        control.step(ONE);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step(ONE)));
        let performed = perform(&mut machine, &control, Task::Step(ONE), Stepping::Instructions).expect("step");
        assert_eq!(performed.retired, 1);
        assert_eq!(
            control.stopped_at(),
            Some(CodeBreakpointHit { cpu: 0, breakpoint: at, linear: code + 1 })
        );
        control.step(ONE);
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step(ONE)));
        let performed = perform(&mut machine, &control, Task::Step(ONE), Stepping::Instructions).expect("step");
        assert_eq!(performed.retired, 1, "the breakpoint's own instruction executes");
        assert_eq!(machine.cpu_snapshot().rip, code + 2);
        assert_eq!(control.stopped_at(), None);
    }

    /// An engine that runs the interpreter but says nothing of breakpoints.
    #[derive(Default)]
    struct SilentEngine;

    impl<T: Instrumentation> SliceEngine<T> for SilentEngine {
        const PROGRESS_UNIT: rusty_box::emulator::ProgressUnit = rusty_box::emulator::ProgressUnit::Instructions;

        fn run_slice(
            &mut self,
            cpu: &mut rusty_box::cpu::BxCpuC<T>,
            io: rusty_box::emulator::PcIo<'_>,
            request: rusty_box::emulator::SliceRequest,
        ) -> rusty_box::cpu::Result<rusty_box::emulator::Progress> {
            rusty_box::emulator::SoftwareEngine.run_slice(cpu, io, request)
        }
    }

    /// A machine whose engine refuses breakpoints leaves the page a note.
    #[test]
    fn breakpoints_an_engine_refuses_leave_a_note() {
        let mut machine = Emulator::<(), SilentEngine>::with_engine(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
            .expect("machine");
        let control = RunControl::new(Arc::new(AtomicBool::new(false)));
        apply_breakpoints(&mut machine, &control, &[CodeBreakpoint::Linear(0x1000)]);
        assert_eq!(control.note(), Some(DebugNote::BreakpointsNeedInterpreter));
    }

    #[test]
    fn a_step_on_the_hypervisor_is_refused_with_a_note() {
        let (mut machine, control, code) = paused();
        let performed =
            perform(&mut machine, &control, Task::Step(ONE), Stepping::Unavailable).expect("refusal");
        assert_eq!(performed.retired, 0);
        assert_eq!(control.note(), Some(DebugNote::StepNeedsInterpreter));
        assert_eq!(machine.cpu_snapshot().rip, code, "the guest did not move");
    }

    /// A long-mode machine paused at `code`: `call f ; nop ; hlt`, with `f:
    /// nop ; ret` at `code + 0x10` and the stack at `code + 0x1000`.
    struct Calling {
        machine: Box<Emulator>,
        control: RunControl,
        code: u64,
    }

    const CALL_LENGTH: u64 = 5;
    const F: u64 = 0x10;
    const STACK: u64 = 0x1000;

    fn calling() -> Calling {
        let (mut machine, control, code) = paused();
        let program = [0xE8, 0x0B, 0, 0, 0, 0x90, 0xF4, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0xC3];
        machine.mem_write(code, &program).expect("write");
        machine.reg_write(X86Reg::Rsp, code + STACK);
        Calling { machine, control, code }
    }

    fn request(kind: StepKind, unit: StepUnit) -> StepRequest {
        StepRequest { kind, unit, cpu: 0 }
    }

    /// Where the boot processor stands.
    #[derive(Debug, PartialEq, Eq)]
    struct Landing {
        rip: u64,
        rsp: u64,
    }

    fn landing(machine: &Emulator) -> Landing {
        let snapshot = machine.cpu_snapshot();
        Landing { rip: snapshot.rip, rsp: snapshot.rsp }
    }

    /// The pause read with one module, `identity`, holding the code (as a
    /// Windows guest's discovery would list it), and its frames read under
    /// that module; with no unwind data in it, the unwinder takes its
    /// functions for leaves.
    fn publish_module_over(calling: &mut Calling, identity: GuestIdentity, base: u64) {
        let mut snapshot = read_for(&mut calling.machine, &calling.control);
        snapshot.symbols = SymbolIndex {
            modules: vec![SymModule { base, size: 0x10_0000, name: "flat.exe".into(), space: ModuleSpace::User, identity }],
            kernel: None,
        };
        let at = frames_at(&snapshot);
        let symbols = calling.control.symbols();
        let mut debug = calling.machine.debug_cpu(snapshot.cpu).expect("the processor");
        let vmx_guest = debug.vmx_guest();
        let mut paused = PausedCpu { cpu: &mut debug, vmx_guest };
        snapshot.frames =
            read_frames(&mut paused.walked(), &at, &snapshot.symbols, symbols.as_deref(), &calling.control.inspect());
        calling.control.publish(snapshot);
    }

    fn flat_identity() -> ModuleIdentity {
        ModuleIdentity::Pe {
            file_name: "flat.exe".into(),
            image: PeImageId { time_date_stamp: 1, size_of_image: 0x10_0000 },
            pdb: Some(PdbId { name: "flat.pdb".into(), guid: Guid::from_le_bytes([5; 16]), age: 1 }),
        }
    }

    fn main_c(name: &str, offset: u32, lines: Vec<FixtureLine>) -> FixtureProcedure {
        FixtureProcedure { name: name.into(), section: 1, offset, len: 0x10, file: r"C:\src\main.c".into(), checksum: None, lines }
    }

    /// Spec §11's last bullet: `main: call f ; nop ; hlt` (lines 1–2) and
    /// `f: mov dword [rsp-8], 42 ; nop ; ret` (lines 10–11, its local `x` at
    /// `[rsp-8]`), run to a `main.c:10` breakpoint, stepped over a line, and
    /// read as the panes read it: the call stack, `x`, a tip on `x`, the
    /// caller picked; then stepped out.
    #[test]
    fn the_call_stack_and_a_local_read_on_a_running_machine() {
        let (mut machine, control, code) = paused();
        let mut program = vec![0xE8, 0x0B, 0, 0, 0, 0x90, 0xF4];
        program.resize(0x10, 0x90);
        program.extend_from_slice(&[0xC7, 0x44, 0x24, 0xF8, 0x2A, 0, 0, 0, 0x90, 0xC3]);
        machine.mem_write(code, &program).expect("write");
        machine.reg_write(X86Reg::Rsp, code + STACK);
        let mut calling = Calling { machine, control, code };
        let mut fixture = PdbFixture::new([5; 16], 1);
        fixture.procedures.push(main_c("main", 0x100, vec![FixtureLine { offset: 0, line: 1 }, FixtureLine { offset: 6, line: 2 }]));
        fixture.procedures.push(main_c("f", 0x110, vec![FixtureLine { offset: 0, line: 10 }, FixtureLine { offset: 8, line: 11 }]));
        fixture.scopes.push(FixtureScope {
            procedure: "f".into(),
            debug_start: 0,
            debug_end: 0x10,
            parameters: None,
            frame: None,
            locals: vec![FixtureLocal { name: "x".into(), ty: FixtureType::I32, place: FixturePlace::RegRel { register: 335, offset: -8 } }],
            inline_locals: Vec::new(),
            blocks: Vec::new(),
        });
        let tables = SymbolTables::parse(&build_pdb(&fixture)).expect("the fixture parses");
        let mut view = SymbolsView::default();
        view.modules.insert(
            flat_identity(),
            ModuleEntry {
                state: ModuleSymbols::Loaded { from: SymbolSource::Store { path: "flat.pdb".into() } },
                tables: Some(Arc::new(tables)),
            },
        );
        let view = Arc::new(view);
        calling.control.set_symbols(Arc::clone(&view));
        // `.text` is at RVA 0x1000, `main` 0x100 into it and `f` 0x110.
        let base = code - 0x1100;
        publish_module_over(&mut calling, GuestIdentity::Known(flat_identity()), base);
        let index = calling.control.snapshot().expect("published").symbols.clone();
        let line_10 = BreakpointTarget::Source(SourceTarget { file: "main.c".into(), line: 10 });
        let elsewhere = ModulesElsewhere::default();
        let set = applied_set(&[line_10], Some(&BreakpointModules { index: &index, elsewhere: &elsewhere }), &view).set;
        assert_eq!(set, [CodeBreakpoint::Linear(code + F)]);
        perform(&mut calling.machine, &calling.control, Task::ApplyBreakpoints(set), Stepping::Instructions).expect("apply");
        let stop = calling.machine.step(RunBudget::Instructions(1_000)).expect("run").stop;
        assert!(matches!(stop, StopReason::Breakpoint(hit) if hit.linear == code + F), "{stop:?}");
        let over = request(StepKind::Over, StepUnit::Line);
        perform(&mut calling.machine, &calling.control, Task::Step(over), Stepping::Instructions).expect("step");
        assert_eq!(calling.machine.cpu_snapshot().rip, code + 0x18, "line 11");
        publish_module_over(&mut calling, GuestIdentity::Known(flat_identity()), base);
        let shown = calling.control.snapshot().expect("published");
        assert!(shown.frames.frames.len() >= 2, "{:?}", shown.frames.frames);
        assert_eq!(shown.frames.frames[1].registers.rip, code + CALL_LENGTH);
        let forty_two = ShownValue::Integer { raw: 42, size: 4, signedness: Signedness::Signed };
        let LocalsRead::Rows(rows) = &shown.frames.locals else { panic!("locals: {:?}", shown.frames.locals) };
        assert_eq!(rows.iter().map(|row| row.name.as_str()).collect::<Vec<&str>>(), ["x"]);
        assert_eq!(rows[0].value, forty_two);
        calling.control.show_tip(Some("x".into()));
        publish_module_over(&mut calling, GuestIdentity::Known(flat_identity()), base);
        let TipRead::Rows(tip) = &calling.control.snapshot().expect("published").frames.tip else { panic!("a tip") };
        assert_eq!(tip.rows[0].value, forty_two);
        calling.control.select_frame(1);
        publish_module_over(&mut calling, GuestIdentity::Known(flat_identity()), base);
        assert_eq!(calling.control.snapshot().expect("published").frames.selected, 1);
        // Out by line returns past the call and runs to the end of its line:
        // line 2 of `main`.
        let out = request(StepKind::Out, StepUnit::Line);
        perform(&mut calling.machine, &calling.control, Task::Step(out), Stepping::Instructions).expect("out");
        assert_eq!(calling.machine.cpu_snapshot().rip, code + 6, "back in main, at line 2");
    }

    #[test]
    fn stepping_over_a_call_stops_after_it() {
        let mut calling = calling();
        let over = request(StepKind::Over, StepUnit::Instruction);
        perform(&mut calling.machine, &calling.control, Task::Step(over), Stepping::Instructions).expect("step");
        assert_eq!(landing(&calling.machine), Landing { rip: calling.code + CALL_LENGTH, rsp: calling.code + STACK });
        assert_eq!(calling.control.note(), None);
    }

    #[test]
    fn stepping_out_returns_to_the_caller() {
        let mut calling = calling();
        perform(&mut calling.machine, &calling.control, Task::Step(ONE), Stepping::Instructions).expect("into");
        assert_eq!(landing(&calling.machine), Landing { rip: calling.code + F, rsp: calling.code + STACK - 8 });
        let base = calling.code & !0xFFFF;
        publish_module_over(&mut calling, GuestIdentity::Unnamed, base);
        let out = request(StepKind::Out, StepUnit::Instruction);
        perform(&mut calling.machine, &calling.control, Task::Step(out), Stepping::Instructions).expect("out");
        assert_eq!(landing(&calling.machine), Landing { rip: calling.code + CALL_LENGTH, rsp: calling.code + STACK });
    }

    #[test]
    fn running_to_an_address_stops_there() {
        let mut calling = calling();
        let to = request(StepKind::RunTo { address: calling.code + 6 }, StepUnit::Line);
        perform(&mut calling.machine, &calling.control, Task::Step(to), Stepping::Instructions).expect("run to");
        assert_eq!(landing(&calling.machine), Landing { rip: calling.code + 6, rsp: calling.code + STACK });
    }

    #[test]
    fn a_breakpoint_inside_a_call_stepped_over_stops_the_step_there() {
        let mut calling = calling();
        let at = CodeBreakpoint::Linear(calling.code + F);
        perform(&mut calling.machine, &calling.control, Task::ApplyBreakpoints(vec![at]), Stepping::Instructions)
            .expect("apply");
        let over = request(StepKind::Over, StepUnit::Instruction);
        perform(&mut calling.machine, &calling.control, Task::Step(over), Stepping::Instructions).expect("step");
        assert_eq!(calling.machine.cpu_snapshot().rip, calling.code + F);
        assert_eq!(
            calling.control.stopped_at(),
            Some(CodeBreakpointHit { cpu: 0, breakpoint: at, linear: calling.code + F })
        );
    }

    /// A program's line in another process at the callee's address is not
    /// the owner's here: the step runs over the call to its end.
    #[test]
    fn a_breakpoint_of_another_address_space_does_not_stop_a_step() {
        let mut calling = calling();
        let at = CodeBreakpoint::Linear(calling.code + F);
        let elsewhere = ScopedAddress { linear: calling.code + F, spaces: vec![0xDEAD_B000] };
        calling.control.set_breakpoints(AppliedBreakpoints { set: vec![at], scopes: vec![elsewhere] });
        perform(&mut calling.machine, &calling.control, Task::ApplyBreakpoints(vec![at]), Stepping::Instructions)
            .expect("apply");
        let over = request(StepKind::Over, StepUnit::Instruction);
        perform(&mut calling.machine, &calling.control, Task::Step(over), Stepping::Instructions).expect("step");
        assert_eq!(landing(&calling.machine), Landing { rip: calling.code + CALL_LENGTH, rsp: calling.code + STACK });
        assert_eq!(calling.control.stopped_at(), None);
        assert_eq!(calling.control.note(), None, "the step was not cut");
    }

    /// The owner stops a step that would never end.
    #[test]
    fn a_step_is_stopped_on_the_owners_word() {
        let mut calling = calling();
        calling.machine.mem_write(calling.code, &[0xEB, 0xFE]).expect("jmp $");
        let Calling { mut machine, control, code } = calling;
        let control = Arc::new(control);
        let stepping = Arc::clone(&control);
        let to = request(StepKind::RunTo { address: code + 0x100 }, StepUnit::Instruction);
        let runner = std::thread::spawn(move || {
            perform(&mut machine, &stepping, Task::Step(to), Stepping::Instructions).expect("step");
            machine
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        control.cancel_step();
        let machine = runner.join().expect("the step ends");
        assert_eq!(control.note(), Some(DebugNote::StepCut(StepCut::Cancelled)));
        assert_eq!(machine.cpu_snapshot().rip, code);
    }

    /// Line 1 is `call f ; nop`, line 2 the `hlt`: stepping over line 1
    /// runs the call and the `nop` after it.
    #[test]
    fn stepping_over_by_line_runs_to_the_next_line() {
        let mut calling = calling();
        let identity = ModuleIdentity::Pe {
            file_name: "flat.exe".into(),
            image: PeImageId { time_date_stamp: 1, size_of_image: 0x10_0000 },
            pdb: Some(PdbId { name: "flat.pdb".into(), guid: Guid::from_le_bytes([5; 16]), age: 1 }),
        };
        let mut fixture = PdbFixture::new([5; 16], 1);
        fixture.procedures.push(FixtureProcedure {
            name: "main".into(),
            section: 1,
            offset: 0x100,
            len: 0x10,
            file: r"C:\src\main.c".into(),
            checksum: None,
            lines: vec![FixtureLine { offset: 0, line: 1 }, FixtureLine { offset: 6, line: 2 }],
        });
        let tables = SymbolTables::parse(&build_pdb(&fixture)).expect("the fixture parses");
        let mut view = SymbolsView::default();
        view.modules.insert(
            identity.clone(),
            ModuleEntry {
                state: ModuleSymbols::Loaded { from: SymbolSource::Store { path: "flat.pdb".into() } },
                tables: Some(Arc::new(tables)),
            },
        );
        calling.control.set_symbols(Arc::new(view));
        // `.text` is at RVA 0x1000 and `main` 0x100 into it.
        let base = calling.code - 0x1100;
        publish_module_over(&mut calling, GuestIdentity::Known(identity), base);
        let over = request(StepKind::Over, StepUnit::Line);
        perform(&mut calling.machine, &calling.control, Task::Step(over), Stepping::Instructions).expect("step");
        assert_eq!(landing(&calling.machine), Landing { rip: calling.code + 6, rsp: calling.code + STACK });
    }

    /// A PE image written into a long-mode machine's memory is copied by the
    /// run thread and indexed by the service.
    #[test]
    fn the_copy_task_hands_each_image_to_the_service() {
        let (mut machine, control, code) = paused();
        let base = (code + 0x1_0000) & !0xFFF;
        let spec = ImageSpec::bits64(0x1000);
        let mut text = vec![0xE8];
        text.extend_from_slice(&0xFBu32.to_le_bytes());
        let image = module_image(base, &spec, &text, &[], &[]);
        machine.mem_write(base, &image.bytes).expect("write");
        let size = image.bytes.len() as u64;
        let key = ModuleKey { identity: GuestIdentity::Unnamed, name: "flat.sys".into(), base, size, space: IndexSpace::Kernel };
        control.find_references(CopyRequest {
            modules: vec![CopyModule { key: key.clone(), root: Root::Live }],
            spans: vec![ModuleSpan { base, size }],
            taken_at: 5,
        });
        let BetweenRuns::Perform(task) = control.next_between_runs() else {
            panic!("the copy is queued")
        };
        assert!(matches!(task, Task::CopyImages(_)));
        perform(&mut machine, &control, task, Stepping::Instructions).expect("copy");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let index = loop {
            match control.xrefs().state(&key) {
                Some(IndexState::Ready(index)) => break index,
                Some(IndexState::Failed(reason)) => panic!("indexing failed: {reason}"),
                Some(_) | None if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(5)),
                other => panic!("not indexed: {other:?}"),
            }
        };
        assert_eq!(index.taken_at, 5);
        assert!(index.sites.iter().any(|site| site.site == base + 0x1000 && site.target == base + 0x1100));
    }

    /// Spec §11-style end to end: a 64-bit image in a long-mode machine,
    /// `main: call f ; call [rip+slot] ; hlt` and `f: ret`, its slot holding
    /// `f`, found through the control as the shell finds it.
    #[test]
    fn references_are_found_in_a_running_machine() {
        let (mut machine, control, code) = paused();
        let base = (code + 0x1_0000) & !0xFFF;
        let mut spec = ImageSpec::bits64(0x1000);
        let main = 0x1000u64;
        let f = 0x1100u64;
        spec.functions = vec![Function { begin: 0x1000, end: 0x1010 }, Function { begin: 0x1100, end: 0x1101 }];
        let slot = u64::from(spec.data_free());
        let mut text = vec![0xE8];
        text.extend_from_slice(&((f - (main + 5)) as u32).to_le_bytes());
        text.extend_from_slice(&[0xFF, 0x15]);
        text.extend_from_slice(&((slot - (main + 11)) as u32).to_le_bytes());
        text.push(0xF4);
        text.resize(0x100, 0xCC);
        text.push(0xC3);
        let image = module_image(base, &spec, &text, &(base + f).to_le_bytes(), &[]);
        machine.mem_write(base, &image.bytes).expect("write");
        let size = image.bytes.len() as u64;
        let key = ModuleKey { identity: GuestIdentity::Unnamed, name: "flat.exe".into(), base, size, space: IndexSpace::Kernel };
        control.find_references(CopyRequest {
            modules: vec![CopyModule { key: key.clone(), root: Root::Live }],
            spans: vec![ModuleSpan { base, size }],
            taken_at: 77,
        });
        let BetweenRuns::Perform(task) = control.next_between_runs() else {
            panic!("the copy is queued")
        };
        perform(&mut machine, &control, task, Stepping::Instructions).expect("copy");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !matches!(control.xrefs().state(&key), Some(IndexState::Ready(_))) {
            assert!(std::time::Instant::now() < deadline, "not indexed: {:?}", control.xrefs().state(&key));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let query = XrefQuery {
            target: XrefTarget { ranges: vec![AddressRange { start: base + f, end: base + f + 1 }], label: "f".into() },
            scope: XrefScope::TargetModule,
        };
        let answer = control.xrefs().answer(&query, std::slice::from_ref(&key));
        assert_eq!(
            answer.found,
            [
                FoundSite { site: base + main, kind: XrefKind::Call, module: "flat.exe".into(), through: Vec::new() },
                FoundSite {
                    site: base + slot,
                    kind: XrefKind::Pointer,
                    module: "flat.exe".into(),
                    through: vec![FoundSite {
                        site: base + main + 5,
                        kind: XrefKind::CallThrough,
                        module: "flat.exe".into(),
                        through: Vec::new(),
                    }],
                },
            ]
        );
        assert_eq!(answer.taken_at, Some(77));
        assert!(answer.waiting.is_empty() && answer.failed.is_empty());
    }

    /// Cancel while the run thread holds the copy task: the modules it has not
    /// copied yet are not copied, so nothing of them is indexed.
    #[test]
    fn a_copy_cancelled_after_its_task_was_taken_is_skipped() {
        let (mut machine, control, code) = paused();
        let base = (code + 0x1_0000) & !0xFFF;
        let image = module_image(base, &ImageSpec::bits64(0x1000), &[0xC3], &[], &[]);
        machine.mem_write(base, &image.bytes).expect("write");
        let size = image.bytes.len() as u64;
        let key = ModuleKey { identity: GuestIdentity::Unnamed, name: "flat.exe".into(), base, size, space: IndexSpace::Kernel };
        control.find_references(CopyRequest {
            modules: vec![CopyModule { key: key.clone(), root: Root::Live }],
            spans: vec![ModuleSpan { base, size }],
            taken_at: 1,
        });
        let BetweenRuns::Perform(task) = control.next_between_runs() else {
            panic!("the copy is queued")
        };
        control.cancel_references();
        perform(&mut machine, &control, task, Stepping::Instructions).expect("copy");
        assert_eq!(control.xrefs().state(&key), None);
    }

    #[test]
    fn a_dump_task_records_its_outcome() {
        let (mut machine, control, _) = paused();
        let path = std::env::temp_dir().join(format!(
            "rusty-box-gui-serve-{}.bin",
            std::process::id()
        ));
        perform(
            &mut machine,
            &control,
            Task::DumpRam(path.clone()),
            Stepping::Instructions,
        )
        .expect("dump");
        assert_eq!(
            control.take_dump_outcomes(),
            [DumpOutcome::Saved {
                path: path.clone(),
                bytes: machine.mem_size() as u64
            }]
        );
        std::fs::remove_file(&path).expect("clean up");
    }
}
