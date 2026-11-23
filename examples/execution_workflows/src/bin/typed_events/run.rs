use std::cell::OnceCell;
use std::fmt;

use rusty_box::cpu::ProcessorActivity;
use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::cpu::registers::{EAX, EIP};
use rusty_box::events::{Emit, ExecutionRequest};

use crate::process::{Process, ProcessId, ResolvedProcess, Target, TargetError};
use crate::{State, WorkflowError};

pub(super) struct RunRequested {
    pub(super) target: Target,
    pub(super) budget: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Delivered(ExecutionRequest),
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct GuestRun {
    pub(super) name: &'static str,
    pub(super) ip_before: u32,
    pub(super) eax: u32,
    pub(super) instructions: u64,
}

pub(super) struct RunRecord {
    requested: Target,
    pub(super) target: Option<ProcessId>,
    pub(super) guest: Option<GuestRun>,
    pub(super) outcome: Outcome,
}

impl fmt::Display for RunRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} → {:?}", self.requested, self.outcome)?;
        if let Some(target) = self.target {
            write!(f, "  {target}")?;
        }
        if let Some(guest) = self.guest {
            write!(
                f,
                " {}, eip_before={:#x}, eax={}, dispatches={}",
                guest.name, guest.ip_before, guest.eax, guest.instructions
            )?;
        }
        Ok(())
    }
}

/// Fresh delivery parts. Cache the resolved identity or the lookup error for
/// this request. The selected guest itself is borrowed only by each handler.
pub(super) struct RunParts<'a> {
    state: &'a mut State,
    requested: Target,
    resolved: OnceCell<Result<ResolvedProcess, TargetError>>,
    progress: RunProgress,
}

#[derive(Default)]
struct RunProgress {
    ip_before: u32,
    instructions: u64,
    guest: Option<GuestRun>,
}

impl<'a> RunParts<'a> {
    pub(super) fn new(state: &'a mut State, event: &RunRequested) -> Self {
        Self {
            state,
            requested: event.target,
            resolved: OnceCell::new(),
            progress: RunProgress::default(),
        }
    }
}

/// The concrete dependency this route's handlers ask for. This is an
/// example-local counterpart to an Axum extractor.
pub(super) struct TargetProcess<'a> {
    process: &'a mut Process,
    progress: &'a mut RunProgress,
}

impl<'a> TargetProcess<'a> {
    fn from_parts(parts: &'a mut RunParts<'_>) -> Result<Self, TargetError> {
        let resolved = *parts
            .resolved
            .get_or_init(|| parts.state.catalog.resolve(parts.requested));
        Ok(Self {
            process: parts.state.catalog.borrow_target(resolved?),
            progress: &mut parts.progress,
        })
    }
}

pub(super) fn with_target<F>(
    mut handler: F,
) -> impl FnMut(&RunRequested, &mut RunParts<'_>) -> Result<ExecutionRequest, WorkflowError>
where
    F: FnMut(&RunRequested, TargetProcess<'_>) -> Result<ExecutionRequest, WorkflowError>,
{
    move |event, parts| handler(event, TargetProcess::from_parts(parts)?)
}

pub(super) fn observe_before(
    _: &RunRequested,
    target: TargetProcess<'_>,
) -> Result<ExecutionRequest, WorkflowError> {
    target.progress.ip_before = target.process.cpu.regs().read(EIP);
    Ok(ExecutionRequest::None)
}

pub(super) fn execute(
    event: &RunRequested,
    target: TargetProcess<'_>,
) -> Result<ExecutionRequest, WorkflowError> {
    let run = target.process.cpu.run(event.budget);
    target.progress.instructions = run.instructions;
    match run.stop {
        SessionStop::Budget => Ok(ExecutionRequest::None),
        SessionStop::Inactive(ProcessorActivity::Halted) => Ok(ExecutionRequest::Stop),
        stop => Err(WorkflowError::UnexpectedStop(stop)),
    }
}

pub(super) fn observe_after(
    _: &RunRequested,
    target: TargetProcess<'_>,
) -> Result<ExecutionRequest, WorkflowError> {
    target.progress.guest = Some(GuestRun {
        name: target.process.name,
        ip_before: target.progress.ip_before,
        eax: target.process.cpu.regs().read(EAX),
        instructions: target.progress.instructions,
    });
    Ok(ExecutionRequest::None)
}

/// Reject invalid budgets before invoking any inner handler or extractor.
pub(super) struct BudgetLimit<S> {
    pub(super) maximum: u64,
    pub(super) inner: S,
}

impl<'a, S: Emit<RunRequested, RunParts<'a>, Error = WorkflowError>>
    Emit<RunRequested, RunParts<'a>> for BudgetLimit<S>
{
    type Error = WorkflowError;

    fn emit(
        &mut self,
        event: &RunRequested,
        parts: &mut RunParts<'a>,
    ) -> Result<ExecutionRequest, Self::Error> {
        if !(1..=self.maximum).contains(&event.budget) {
            return Err(WorkflowError::Budget {
                budget: event.budget,
                maximum: self.maximum,
            });
        }
        self.inner.emit(event, parts)
    }
}

/// Record the request at the outer boundary, including guard failures.
pub(super) struct Audit<S>(pub S);

impl<'a, S: Emit<RunRequested, RunParts<'a>, Error = WorkflowError>>
    Emit<RunRequested, RunParts<'a>> for Audit<S>
{
    type Error = WorkflowError;

    fn emit(
        &mut self,
        event: &RunRequested,
        parts: &mut RunParts<'a>,
    ) -> Result<ExecutionRequest, Self::Error> {
        let result = self.0.emit(event, parts);
        parts.state.runs.push(RunRecord {
            requested: parts.requested,
            target: parts
                .resolved
                .get()
                .and_then(|result| result.ok())
                .map(|target| target.id),
            guest: parts.progress.guest,
            outcome: match result.as_ref() {
                Ok(request) => Outcome::Delivered(*request),
                Err(_) => Outcome::Failed,
            },
        });
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_extraction_is_cached_for_this_delivery() {
        let mut state = crate::demo_state().unwrap();
        let request = RunRequested {
            target: Target::Selected,
            budget: 1,
        };
        let mut parts = RunParts::new(&mut state, &request);
        for _ in 0..2 {
            assert_eq!(
                TargetProcess::from_parts(&mut parts).err(),
                Some(TargetError::NoSelection)
            );
        }
        assert_eq!(state.catalog.resolutions(), 1);
    }
}
