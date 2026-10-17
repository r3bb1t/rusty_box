//! Debugger code breakpoints, as a guest and its host see them: the
//! processor stops before the instruction, the guest's memory is untouched,
//! and the run after the stop executes the instruction.

use core::sync::atomic::Ordering;

use super::*;
use crate::cpu::instrumentation::{CpuSetupMode, X86Reg};
use crate::cpu::{BreakpointRefusal, CodeBreakpoint, CodeBreakpointHit};

/// See `tests.rs`: building an `Emulator` wants more than the default test
/// stack in a debug build.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

const CODE: u64 = 0x10_0000;

fn on_test_stack<F: FnOnce() + Send + 'static>(test: F) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(test)
        .expect("spawn")
        .join()
        .expect("the test thread");
}

/// A long-mode machine with `code` at [`CODE`] and RIP on it.
fn machine_with(code: &[u8]) -> Box<Emulator<()>> {
    let mut emu = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
        .expect("machine");
    emu.mem_write(CODE, code).expect("write");
    emu.reg_write(X86Reg::Rip, CODE);
    emu
}

/// `nop ; inc rax ; nop ; hlt`.
const STRAIGHT: [u8; 6] = [0x90, 0x48, 0xFF, 0xC0, 0x90, 0xF4];

fn hit_at(breakpoint: CodeBreakpoint, linear: u64) -> StopReason {
    StopReason::Breakpoint(CodeBreakpointHit {
        cpu: 0,
        breakpoint,
        linear,
    })
}

fn run_on(emu: &mut Emulator<()>) {
    emu.stop_flag().store(false, Ordering::Relaxed);
}

#[test]
fn a_linear_breakpoint_stops_before_its_instruction_and_the_next_run_executes_it() {
    on_test_stack(|| {
        let mut emu = machine_with(&STRAIGHT);
        let at = CodeBreakpoint::Linear(CODE + 1);
        emu.set_code_breakpoints(&[at]).expect("set");
        let outcome = emu.step(RunBudget::Instructions(100)).expect("step");
        assert_eq!(outcome.stop, hit_at(at, CODE + 1));
        assert!(outcome.is_terminal(), "the host lowers the flag to run on");
        assert_eq!(outcome.progress, Progress::Instructions(1), "only the nop retired");
        assert_eq!(emu.reg_read(X86Reg::Rip), CODE + 1);
        assert_eq!(emu.reg_read(X86Reg::Rax), 0, "the instruction did not execute");
        let mut code = [0u8; STRAIGHT.len()];
        emu.mem_read(CODE, &mut code).expect("read");
        assert_eq!(code, STRAIGHT, "nothing was written into the guest");
        assert_eq!(emu.stopped_at_breakpoint(), None, "a hit is reported once");
        // With the flag still up, a step stops as any raised flag stops it.
        let outcome = emu.step(RunBudget::Instructions(1)).expect("step");
        assert_eq!(outcome.stop, StopReason::StopRequested, "the hit is not reported twice");
        assert_eq!(emu.reg_read(X86Reg::Rax), 1);

        run_on(&mut emu);
        let outcome = emu.step(RunBudget::Instructions(100)).expect("step");
        assert_eq!(emu.reg_read(X86Reg::Rax), 1, "the instruction executed once");
        assert_eq!(outcome.stop, StopReason::Halted);
    });
}

/// A driver that reads no stop reason (`run_interactive` returns a count)
/// asks the machine instead, once.
#[test]
fn a_driver_that_reads_no_stop_reason_asks_for_the_hit_once() {
    on_test_stack(|| {
        // inc rax ; inc rbx ; jmp $
        let mut emu = machine_with(&[0x48, 0xFF, 0xC0, 0x48, 0xFF, 0xC3, 0xEB, 0xFE]);
        let at = CodeBreakpoint::Linear(CODE + 3);
        emu.set_code_breakpoints(&[at]).expect("set");
        let retired = emu.run_interactive(1_000_000).expect("run");
        assert_eq!(retired, 1, "the run ended at the breakpoint");
        assert_eq!(
            emu.stopped_at_breakpoint(),
            Some(CodeBreakpointHit { cpu: 0, breakpoint: at, linear: CODE + 3 })
        );
        assert_eq!(emu.stopped_at_breakpoint(), None, "reported once");
        assert_eq!(emu.reg_read(X86Reg::Rbx), 0);
    });
}

#[test]
fn a_physical_breakpoint_stops_before_the_instruction_at_that_address() {
    on_test_stack(|| {
        let mut emu = machine_with(&STRAIGHT);
        // The machine maps linear to physical one to one.
        let at = CodeBreakpoint::Physical(CODE + 4);
        emu.set_code_breakpoints(&[at]).expect("set");
        let outcome = emu.step(RunBudget::Instructions(100)).expect("step");
        assert_eq!(outcome.stop, hit_at(at, CODE + 4));
        assert_eq!(emu.reg_read(X86Reg::Rip), CODE + 4);
        assert_eq!(emu.reg_read(X86Reg::Rax), 1);
    });
}

#[test]
fn a_breakpoint_in_a_loop_already_cached_hits_on_every_pass() {
    on_test_stack(|| {
        // inc rax ; inc rbx ; jmp back to the inc rax
        let mut emu = machine_with(&[0x48, 0xFF, 0xC0, 0x48, 0xFF, 0xC3, 0xEB, 0xF8]);
        // Run first, so the loop's traces are cached and linked.
        let outcome = emu.step(RunBudget::Instructions(999)).expect("warm up");
        assert_eq!(outcome.stop, StopReason::BudgetExhausted);
        let at = CodeBreakpoint::Linear(CODE + 3);
        emu.set_code_breakpoints(&[at]).expect("set");
        let mut rbx = None;
        for pass in 0..3 {
            let outcome = emu.step(RunBudget::Instructions(1_000)).expect("step");
            assert_eq!(outcome.stop, hit_at(at, CODE + 3), "pass {pass}");
            assert_eq!(emu.reg_read(X86Reg::Rip), CODE + 3);
            let now = emu.reg_read(X86Reg::Rbx);
            if let Some(before) = rbx {
                assert_eq!(now, before + 1, "one loop between two hits");
            }
            rbx = Some(now);
            run_on(&mut emu);
        }
    });
}

#[test]
fn a_breakpoint_at_a_branch_target_hits_on_every_pass() {
    on_test_stack(|| {
        // inc rax ; inc rbx ; jmp back to the inc rax
        let mut emu = machine_with(&[0x48, 0xFF, 0xC0, 0x48, 0xFF, 0xC3, 0xEB, 0xF8]);
        let at = CodeBreakpoint::Linear(CODE);
        emu.set_code_breakpoints(&[at]).expect("set");
        for pass in 0..4u64 {
            let outcome = emu.step(RunBudget::Instructions(1_000)).expect("step");
            assert_eq!(outcome.stop, hit_at(at, CODE), "pass {pass}");
            assert_eq!(emu.reg_read(X86Reg::Rax), pass, "the jmp back is followed into the check");
            run_on(&mut emu);
        }
    });
}

#[test]
fn a_step_onto_a_breakpoint_reports_it_and_the_next_step_executes_it() {
    on_test_stack(|| {
        let mut emu = machine_with(&STRAIGHT);
        let at = CodeBreakpoint::Linear(CODE + 1);
        emu.set_code_breakpoints(&[at]).expect("set");
        let outcome = emu.step(RunBudget::Instructions(1)).expect("step");
        assert_eq!(outcome.progress, Progress::Instructions(1));
        assert_eq!(outcome.stop, hit_at(at, CODE + 1), "the boundary the step ends on is checked");
        run_on(&mut emu);
        let outcome = emu.step(RunBudget::Instructions(1)).expect("step");
        assert_eq!(outcome.progress, Progress::Instructions(1));
        assert_eq!(emu.reg_read(X86Reg::Rax), 1);
        assert_eq!(outcome.stop, StopReason::BudgetExhausted);
    });
}

#[test]
fn a_linear_breakpoint_is_met_through_another_mapping_of_its_page() {
    on_test_stack(|| {
        // inc rax ; inc rbx ; jmp $
        let mut emu = machine_with(&[0x48, 0xFF, 0xC0, 0x48, 0xFF, 0xC3, 0xEB, 0xFE]);
        // The second 2 MiB page maps physical 0 too, so the code is also at
        // CODE + 2 MiB (the machine's page directory sits at 0x3000).
        const SECOND_LARGE_PAGE_PDE: u64 = 0x3008;
        emu.mem_write(SECOND_LARGE_PAGE_PDE, &0x83u64.to_le_bytes()).expect("alias");
        let alias = CODE + 0x20_0000;
        let at = CodeBreakpoint::Linear(CODE + 3);
        emu.set_code_breakpoints(&[at]).expect("set");
        // Through the alias the breakpoint's address is never reached, and
        // the trace built for its physical page is cached.
        emu.reg_write(X86Reg::Rip, alias);
        let outcome = emu.step(RunBudget::Instructions(10)).expect("alias run");
        assert_eq!(outcome.stop, StopReason::BudgetExhausted);
        assert_eq!(emu.reg_read(X86Reg::Rbx), 1);
        // The same trace run at the breakpoint's own mapping stops there.
        emu.reg_write(X86Reg::Rip, CODE);
        let outcome = emu.step(RunBudget::Instructions(10)).expect("run");
        assert_eq!(outcome.stop, hit_at(at, CODE + 3));
        assert_eq!(emu.reg_read(X86Reg::Rbx), 1, "stopped before the second inc rbx");
        assert_eq!(emu.reg_read(X86Reg::Rax), 2);
    });
}

#[test]
fn breakpoints_past_their_slots_are_refused_and_the_set_before_stays() {
    on_test_stack(|| {
        let mut emu = machine_with(&STRAIGHT);
        let kept = CodeBreakpoint::Linear(CODE + 1);
        emu.set_code_breakpoints(&[kept]).expect("set");
        let linear: Vec<CodeBreakpoint> =
            (0..=crate::cpu::MAX_LINEAR_BREAKPOINTS as u64).map(|n| CodeBreakpoint::Linear(0x5000 + n)).collect();
        assert_eq!(emu.set_code_breakpoints(&linear), Err(BreakpointRefusal::TooManyLinear));
        let physical: Vec<CodeBreakpoint> =
            (0..=crate::cpu::MAX_PHYSICAL_BREAKPOINTS as u64).map(|n| CodeBreakpoint::Physical(0x5000 + n)).collect();
        assert_eq!(emu.set_code_breakpoints(&physical), Err(BreakpointRefusal::TooManyPhysical));
        assert_eq!(
            emu.set_code_breakpoints(&linear[..crate::cpu::MAX_LINEAR_BREAKPOINTS]),
            Ok(()),
            "every slot may be filled"
        );
        emu.set_code_breakpoints(&[kept]).expect("set");
        assert_eq!(emu.set_code_breakpoints(&linear), Err(BreakpointRefusal::TooManyLinear));
        let outcome = emu.step(RunBudget::Instructions(100)).expect("step");
        assert_eq!(outcome.stop, hit_at(kept, CODE + 1), "the refused set left the one before");
    });
}

#[test]
fn an_empty_set_clears_the_breakpoints() {
    on_test_stack(|| {
        let mut emu = machine_with(&STRAIGHT);
        emu.set_code_breakpoints(&[CodeBreakpoint::Linear(CODE + 1)]).expect("set");
        emu.set_code_breakpoints(&[]).expect("clear");
        let outcome = emu.step(RunBudget::Instructions(100)).expect("step");
        assert_eq!(outcome.stop, StopReason::Halted);
        assert_eq!(emu.reg_read(X86Reg::Rax), 1);
    });
}

/// An engine that runs the interpreter but says nothing of breakpoints, as
/// an engine written outside this crate may.
#[derive(Default)]
struct SilentEngine;

impl<T: Instrumentation> SliceEngine<T> for SilentEngine {
    const PROGRESS_UNIT: ProgressUnit = ProgressUnit::Instructions;

    fn run_slice(
        &mut self,
        cpu: &mut crate::cpu::BxCpuC<T>,
        io: PcIo<'_>,
        request: SliceRequest,
    ) -> crate::cpu::Result<Progress> {
        SoftwareEngine.run_slice(cpu, io, request)
    }
}

#[test]
fn an_engine_that_does_not_say_it_stops_refuses_breakpoints() {
    on_test_stack(|| {
        let mut emu = Emulator::<(), SilentEngine>::with_engine(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
            .expect("machine");
        assert_eq!(
            emu.set_code_breakpoints(&[CodeBreakpoint::Linear(CODE)]),
            Err(BreakpointRefusal::EngineCannotStop)
        );
        assert_eq!(emu.set_code_breakpoints(&[]), Ok(()), "clearing asks nothing of the engine");
    });
}
