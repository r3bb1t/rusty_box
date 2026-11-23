use std::fmt;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuEmulator;
use rusty_box::events::{App, AppHandlers, ExecutionRequest};

/// Application identity for an emulator instance. The generation travels with
/// the PID so an old request cannot accidentally address a replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProcessId {
    pid: u32,
    generation: u32,
}

impl ProcessId {
    pub(super) const fn new(pid: u32, generation: u32) -> Self {
        Self { pid, generation }
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.pid, self.generation)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Target {
    Selected,
    Exact(ProcessId),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selected => f.write_str("selected process"),
            Self::Exact(id) => write!(f, "process {id}"),
        }
    }
}

pub(super) struct SelectProcess(pub ProcessId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum TargetError {
    #[error("select a process before running the current target")]
    NoSelection,
    #[error("process {0} is missing or has been replaced")]
    Stale(ProcessId),
}

pub(super) struct Process {
    id: ProcessId,
    pub(super) name: &'static str,
    pub(super) cpu: CpuEmulator,
}

impl Process {
    pub(super) fn new(id: ProcessId, name: &'static str, eax: u32) -> rusty_box::Result<Self> {
        let mut cpu = CpuEmulator::builder().build()?;
        let [a, b, c, d] = eax.to_le_bytes();
        // mov eax, imm32; hlt
        cpu.memory()
            .write_phys(BxPhyAddress::new(0x1000), &[0xB8, a, b, c, d, 0xF4])?;
        Ok(Self { id, name, cpu })
    }
}

#[derive(Default)]
pub(super) struct Catalog {
    processes: Vec<Process>,
    selected: Option<ProcessId>,
    resolutions: usize,
}

/// A slot validated for a single delivery. Handlers can borrow its process
/// without access to the catalog's insertion or selection methods.
#[derive(Clone, Copy)]
pub(super) struct ResolvedProcess {
    pub(super) id: ProcessId,
    slot: usize,
}

impl Catalog {
    pub(super) fn insert(&mut self, process: Process) {
        if let Some(previous) = self
            .processes
            .iter_mut()
            .find(|old| old.id.pid == process.id.pid)
        {
            *previous = process;
        } else {
            self.processes.push(process);
        }
    }

    fn find(&self, id: ProcessId) -> Result<ResolvedProcess, TargetError> {
        self.processes
            .iter()
            .position(|process| process.id == id)
            .map(|slot| ResolvedProcess { id, slot })
            .ok_or(TargetError::Stale(id))
    }

    pub(super) fn resolve(&mut self, target: Target) -> Result<ResolvedProcess, TargetError> {
        self.resolutions += 1;
        let id = match target {
            Target::Selected => self.selected.ok_or(TargetError::NoSelection)?,
            Target::Exact(id) => id,
        };
        self.find(id)
    }

    pub(super) fn borrow_target(&mut self, target: ResolvedProcess) -> &mut Process {
        // The delivery holds the catalog exclusively and only lends out the
        // guest in the validated slot. No handler can change its placement.
        &mut self.processes[target.slot]
    }

    pub(super) fn resolutions(&self) -> usize {
        self.resolutions
    }
}

pub(super) fn routes<S, E: From<TargetError>, H: AppHandlers<S, E>>(
    app: App<S, E, H>,
    mut catalog: impl FnMut(&mut S) -> &mut Catalog,
) -> App<S, E, impl AppHandlers<S, E>> {
    app.on(
        move |event: &SelectProcess, state: &mut S| -> Result<ExecutionRequest, E> {
            let catalog = catalog(state);
            catalog.find(event.0)?;
            catalog.selected = Some(event.0);
            Ok(ExecutionRequest::None)
        },
    )
}
