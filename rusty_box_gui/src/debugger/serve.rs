//! What a run thread does with a machine that is between runs: the shell's
//! queued work, one task at a time, until the shell lets it run on or ends it.

use rusty_box::cpu::{BreakpointRefusal, CodeBreakpoint, Instrumentation};
use rusty_box::emulator::{Emulator, RunBudget, SliceEngine, StopReason};

use super::control::{BetweenRuns, DebugNote, RunControl, Task};
use super::dump::dump_ram;
use super::inspect::{read_snapshot, DebugSnapshot};
use super::search::find;

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
        Task::Step => match stepping {
            Stepping::Instructions => {
                let before = emu.cpu_snapshot().icount;
                let outcome = emu.step(RunBudget::Instructions(1))?;
                emu.present_console_output();
                emu.update_gui();
                let snapshot = read_for(emu, control);
                control.publish(snapshot);
                let retired = emu.cpu_snapshot().icount.saturating_sub(before);
                if let StopReason::Breakpoint(hit) = outcome.stop {
                    control.stepped_onto_breakpoint(hit);
                }
                let after = match outcome.stop {
                    StopReason::GuestPowerOff | StopReason::CpuShutdown => MachineAfter::Over,
                    StopReason::BudgetExhausted
                    | StopReason::Halted
                    | StopReason::StopRequested
                    | StopReason::EngineFault
                    | StopReason::Breakpoint(_) => MachineAfter::Live,
                };
                Ok(Performed { retired, after })
            }
            Stepping::Unavailable => {
                control.note_refusal(DebugNote::StepNeedsInterpreter);
                Ok(live(0))
            }
        },
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
    }
}

/// Gives `emu` the code breakpoints `set`; a set the machine refuses leaves
/// a note for the page, and the machine keeps the set it had.
pub(crate) fn apply_breakpoints<T: Instrumentation, E: SliceEngine<T>>(
    emu: &mut Emulator<T, E>,
    control: &RunControl,
    set: &[CodeBreakpoint],
) {
    match emu.set_code_breakpoints(set) {
        Ok(()) => {}
        Err(BreakpointRefusal::EngineCannotStop) => control.note_refusal(DebugNote::BreakpointsNeedInterpreter),
        Err(BreakpointRefusal::TooManyLinear | BreakpointRefusal::TooManyPhysical) => {
            control.note_refusal(DebugNote::TooManyBreakpoints);
        }
    }
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
    let mut discovery = control.discovery();
    read_snapshot(emu, &view, &mut discovery, unmoved.as_deref(), layout.as_ref())
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
    use crate::debugger::control::DumpOutcome;
    use rusty_box::cpu::{CpuSetupMode, X86Reg};
    use rusty_box::emulator::EmulatorConfig;
    use std::sync::{atomic::AtomicBool, Arc};

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
        control.step();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step));
        let performed =
            perform(&mut machine, &control, Task::Step, Stepping::Instructions).expect("step");
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
        control.step();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step));
        let performed = perform(&mut machine, &control, Task::Step, Stepping::Instructions).expect("step");
        assert_eq!(performed.retired, 1);
        assert_eq!(
            control.stopped_at(),
            Some(CodeBreakpointHit { cpu: 0, breakpoint: at, linear: code + 1 })
        );
        control.step();
        assert_eq!(control.next_between_runs(), BetweenRuns::Perform(Task::Step));
        let performed = perform(&mut machine, &control, Task::Step, Stepping::Instructions).expect("step");
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
            perform(&mut machine, &control, Task::Step, Stepping::Unavailable).expect("refusal");
        assert_eq!(performed.retired, 0);
        assert_eq!(control.note(), Some(DebugNote::StepNeedsInterpreter));
        assert_eq!(machine.cpu_snapshot().rip, code, "the guest did not move");
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
