//! Run the CPU once. The execution hook delivers events while the guest runs.

use std::cell::OnceCell;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::instrumentation::HookMask;
use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::cpu::{CpuEmulator, Instruction, Instrumentation, ProcessorActivity};
use rusty_box::events::{
    App, AppHandlers, Emit, ExecutionRequest, ReadyApp, Resolve, Resolved, event,
};

fn main() -> Result<(), DemoError> {
    let app = App::<Catalog, UnknownProcess>::new()
        .route(event::<InstructionCompleted>(), print_instruction);
    let catalog = Catalog::new();

    if std::env::args().any(|arg| arg == "--instructions-only") {
        run(app.with_state(catalog))
    } else {
        let app = app
            .route(event::<InstructionCompleted>(), print_process)
            .route(event::<InstructionCompleted>(), print_entry);
        run(app.with_state(catalog))
    }
}

fn run<H: AppHandlers<Catalog, UnknownProcess> + 'static>(
    app: ReadyApp<Catalog, UnknownProcess, H>,
) -> Result<(), DemoError> {
    let mut cpu = load_cpu(app)?;
    let run = cpu.run(16);

    let observer = cpu.instrumentation();
    let request = observer.delivery?;
    match run.stop {
        SessionStop::Inactive(ProcessorActivity::Halted) => println!("CPU halted ({request:?})"),
        stop => return Err(DemoError::UnexpectedStop(stop)),
    }
    if let Some(app) = &observer.app {
        println!("{} process lookups", app.state().lookups);
    }
    Ok(())
}

struct InstructionCompleted {
    ip: u64,
    process_slot: OnceCell<Option<usize>>,
}

fn print_instruction(&InstructionCompleted { ip, .. }: &InstructionCompleted) {
    println!("{ip:#06x} executed");
}

fn print_process(_: &InstructionCompleted, Resolved(process): Resolved<'_, LoadedProcess>) {
    println!("  process {} ({})", process.id, process.name);
}

fn print_entry(
    _: &InstructionCompleted,
    Resolved(LoadedProcess { entry, .. }): Resolved<'_, LoadedProcess>,
) {
    println!("  loaded at {entry:#06x}");
}

struct Catalog {
    processes: &'static [LoadedProcess],
    lookups: usize,
}

impl Catalog {
    fn new() -> Self {
        Self {
            processes: &PROGRAMS,
            lookups: 0,
        }
    }
}

impl Resolve<InstructionCompleted, LoadedProcess> for Catalog {
    type Error = UnknownProcess;

    fn resolve(&mut self, event: &InstructionCompleted) -> Result<&LoadedProcess, Self::Error> {
        // The loader's catalog is fixed for this run. Both handlers share the
        // result on this event and each gets its own short borrow.
        let slot = event.process_slot.get_or_init(|| {
            self.lookups += 1;
            self.processes.iter().position(|process| {
                event
                    .ip
                    .checked_sub(process.entry)
                    .is_some_and(|offset| offset < process.code.len() as u64)
            })
        });
        slot.map(|index| &self.processes[index])
            .ok_or(UnknownProcess(event.ip))
    }
}

struct ExecutionEvents<A> {
    app: Option<A>,
    delivery: Result<ExecutionRequest, UnknownProcess>,
}

impl<A> Default for ExecutionEvents<A> {
    fn default() -> Self {
        Self {
            app: None,
            delivery: Ok(ExecutionRequest::None),
        }
    }
}

impl<A: Emit<InstructionCompleted, Error = UnknownProcess> + 'static> Instrumentation
    for ExecutionEvents<A>
{
    fn active_hooks(&self) -> HookMask {
        HookMask::EXEC
    }

    fn after_execution(&mut self, ip: u64, _: &Instruction) {
        // This is an observation hook. Keep the first error or stop request
        // for the caller and end publication without interrupting the guest.
        if self.delivery != Ok(ExecutionRequest::None) {
            return;
        }
        if let Some(app) = &mut self.app {
            let event = InstructionCompleted {
                ip,
                process_slot: OnceCell::new(),
            };
            self.delivery = app.emit(&event, &mut ());
        }
    }
}

fn load_cpu<A: Emit<InstructionCompleted, Error = UnknownProcess> + 'static>(
    app: A,
) -> rusty_box::Result<CpuEmulator<ExecutionEvents<A>>> {
    let mut cpu = CpuEmulator::builder().build_with_tracer(ExecutionEvents {
        app: Some(app),
        ..ExecutionEvents::default()
    })?;
    for process in &PROGRAMS {
        cpu.memory()
            .write_phys(BxPhyAddress::new(process.entry), process.code)?;
    }
    Ok(cpu)
}

struct LoadedProcess {
    id: u32,
    name: &'static str,
    entry: u64,
    code: &'static [u8],
}

// The host loader supplies these process records. The first code region jumps
// into the second during the same flat, firmware-free x86 run.
const PROGRAMS: [LoadedProcess; 2] = [
    LoadedProcess {
        id: 12,
        name: "counter",
        entry: 0x1000,
        code: &[0xB8, 41, 0, 0, 0, 0xE9, 0xF6, 0x0F, 0, 0], // mov eax,41; jmp 0x2000
    },
    LoadedProcess {
        id: 21,
        name: "worker",
        entry: 0x2000,
        code: &[0x40, 0xF4], // inc eax; hlt
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("no executable loaded at {0:#x}")]
struct UnknownProcess(u64);

#[derive(Debug, thiserror::Error)]
enum DemoError {
    #[error(transparent)]
    Cpu(#[from] rusty_box::Error),
    #[error(transparent)]
    Resolve(#[from] UnknownProcess),
    #[error("unexpected CPU stop: {0:?}")]
    UnexpectedStop(SessionStop),
}

#[cfg(test)]
#[path = "../tests/examples/typed_event_workflow.rs"]
mod tests;
