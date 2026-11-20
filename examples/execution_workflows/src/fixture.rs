//! Deterministic CPU-only execution of the example's benign byte classifier.

use std::time::{Duration, Instant};

use libafl::Error;
use libafl::executors::{Executor, ExitKind, HasObservers};
use libafl::inputs::BytesInput;
use libafl::observers::OwnedMapObserver;
use libafl::state::HasExecutions;
use libafl_bolts::tuples::RefIndexable;
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::instrumentation::{BranchEvent, HookMask, Instrumentation, ResetType};
use rusty_box::cpu::memory_session::{EntryMode, SessionStop};
use rusty_box::cpu::registers::{EAX, RIP};
use rusty_box::cpu::{CpuEmulator, ProcessorActivity};

const RAM_SIZE: usize = 16 * 1024;
const ENTRY: u32 = 0x1000;
const STACK: u32 = 0x3FF0;
const INPUT: BxPhyAddress = BxPhyAddress::new(0x2000);
const TALLY: BxPhyAddress = BxPhyAddress::new(0x2004);
const BUDGET: u64 = 64;
const EDGES: usize = 5;

// x86 Protected32 with flat segments. ASCII A increments a private tally
// and accepts. ASCII T waits for the host's dispatch budget. Everything else
// returns the classifier's ordinary rejection code.
const CODE: &[u8] = &[
    0x80, 0x3D, 0x00, 0x20, 0, 0, b'A', // 1000 cmp byte [2000h], 'A'
    0x74, 0x0F, // 1007 je 1018h
    0x80, 0x3D, 0x00, 0x20, 0, 0, b'T', // 1009 cmp byte [2000h], 'T'
    0x74, 0x12, // 1010 je 1024h
    0xB8, 0, 0, 0, 0,    // 1012 mov eax,0
    0xF4, // 1017 hlt
    0xFF, 0x05, 0x04, 0x20, 0, 0, // 1018 inc dword [2004h]
    0xA1, 0x04, 0x20, 0, 0,    // 101e mov eax,[2004h]
    0xF4, // 1023 hlt
    0xEB, 0xFE, // 1024 jmp 1024h
];

#[derive(Default)]
struct EdgeTrace([u8; EDGES]);

impl Instrumentation for EdgeTrace {
    fn active_hooks(&self) -> HookMask {
        HookMask::BRANCH | HookMask::RESET
    }

    fn reset(&mut self, _: ResetType) {
        self.0.fill(0);
    }

    fn branch(&mut self, event: &BranchEvent) {
        let edge = match *event {
            BranchEvent::CnearTaken {
                src_rip: 0x1007,
                dst_rip: 0x1018,
            } => 0,
            BranchEvent::CnearNotTaken {
                src_rip: 0x1007,
                fallthrough_rip: 0x1009,
            } => 1,
            BranchEvent::CnearTaken {
                src_rip: 0x1010,
                dst_rip: 0x1024,
            } => 2,
            BranchEvent::CnearNotTaken {
                src_rip: 0x1010,
                fallthrough_rip: 0x1012,
            } => 3,
            BranchEvent::Ucnear {
                src_rip: 0x1024,
                dst_rip: 0x1024,
                ..
            } => 4,
            _ => return,
        };
        self.0[edge] = 1;
    }
}

struct Fixture<T: Instrumentation> {
    cpu: CpuEmulator<T>,
    initial_ram: Box<[u8]>,
}

impl<T: Instrumentation> Fixture<T> {
    fn new(tracer: T) -> rusty_box::Result<Self> {
        let mut initial_ram = vec![0; RAM_SIZE].into_boxed_slice();
        initial_ram[ENTRY as usize..ENTRY as usize + CODE.len()].copy_from_slice(CODE);
        let cpu = CpuEmulator::builder()
            .ram(RAM_SIZE)
            .entry(ENTRY)
            .stack(STACK)
            .build_with_tracer(tracer)?;
        Ok(Self { cpu, initial_ram })
    }

    fn reset(&mut self) -> rusty_box::Result<()> {
        self.cpu
            .memory()
            .write_phys(BxPhyAddress::new(0), &self.initial_ram)?;
        self.cpu.reset_at(EntryMode::Protected32, ENTRY, STACK)
    }

    fn set_input(&mut self, byte: u8) -> rusty_box::Result<()> {
        self.cpu.memory().write_phys(INPUT, &[byte])
    }

    fn run(&mut self, byte: u8) -> Result<Run, Error> {
        let result = self.cpu.run(BUDGET);
        let eip = self.cpu.regs().read(RIP);
        let status = match result.stop {
            SessionStop::Budget => Outcome::DispatchBudget,
            SessionStop::Inactive(ProcessorActivity::Halted) => match self.cpu.regs().read(EAX) {
                0 => Outcome::Rejected,
                1 => Outcome::Accepted,
                value => {
                    return Err(Error::illegal_state(format!(
                        "fixture input {byte:02x} returned {value} at {eip:#x}"
                    )));
                }
            },
            stop => {
                return Err(Error::illegal_state(format!(
                    "fixture input {byte:02x} stopped at {eip:#x} after {} dispatches: {stop:?}",
                    result.instructions
                )));
            }
        };
        let mut tally = [0; 4];
        self.cpu
            .memory()
            .read_phys(TALLY, &mut tally)
            .map_err(host_error)?;
        Ok(Run {
            status,
            eip,
            dispatches: result.instructions,
            tally: u32::from_le_bytes(tally),
        })
    }
}

fn host_error(error: rusty_box::Error) -> Error {
    Error::illegal_state(error.to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Accepted,
    Rejected,
    DispatchBudget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Run {
    pub status: Outcome,
    pub eip: u64,
    pub dispatches: u64,
    pub tally: u32,
}

/// Evidence for exactly one input and the fixed fixture image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Replay {
    pub first_byte: u8,
    pub run: Run,
    pub edges: [u8; EDGES],
}

/// LibAFL executor with a private reset image and owned coverage observer.
/// Inputs are the one-byte classifier's first byte, with empty input as zero.
pub struct FixtureExecutor {
    fixture: Fixture<EdgeTrace>,
    observers: (OwnedMapObserver<u8>, ()),
    last: Option<Replay>,
}

impl FixtureExecutor {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            fixture: Fixture::new(EdgeTrace::default()).map_err(host_error)?,
            observers: (
                OwnedMapObserver::new("x86_fixture_edges", vec![0; EDGES]),
                (),
            ),
            last: None,
        })
    }

    pub fn coverage(&self) -> &OwnedMapObserver<u8> {
        &self.observers.0
    }

    pub fn last(&self) -> Option<Replay> {
        self.last
    }

    pub fn replay(&mut self, input: &[u8]) -> Result<Replay, Error> {
        self.last = None;
        self.observers.0.fill(0);
        let first_byte = input.first().copied().unwrap_or(0);
        self.fixture.reset().map_err(host_error)?;
        self.fixture.set_input(first_byte).map_err(host_error)?;
        let run = self.fixture.run(first_byte)?;
        let edges = self.fixture.cpu.instrumentation().0;
        self.observers.0.copy_from_slice(&edges);
        let replay = Replay {
            first_byte,
            run,
            edges,
        };
        self.last = Some(replay);
        Ok(replay)
    }
}

impl HasObservers for FixtureExecutor {
    type Observers = (OwnedMapObserver<u8>, ());

    fn observers(&self) -> RefIndexable<&Self::Observers, Self::Observers> {
        RefIndexable::from(&self.observers)
    }

    fn observers_mut(&mut self) -> RefIndexable<&mut Self::Observers, Self::Observers> {
        RefIndexable::from(&mut self.observers)
    }
}

impl<EM, S: HasExecutions, Z> Executor<EM, BytesInput, S, Z> for FixtureExecutor {
    fn run_target(
        &mut self,
        _: &mut Z,
        state: &mut S,
        _: &mut EM,
        input: &BytesInput,
    ) -> Result<ExitKind, Error> {
        *state.executions_mut() += 1;
        match self.replay(input.as_ref())?.run.status {
            Outcome::Accepted | Outcome::Rejected => Ok(ExitKind::Ok),
            Outcome::DispatchBudget => Ok(ExitKind::Timeout),
        }
    }
}

fn measure<T: Instrumentation>(
    fixture: &mut Fixture<T>,
    cases: usize,
) -> Result<Measurement, Error> {
    let mut setup = Duration::ZERO;
    let mut execute = Duration::ZERO;
    let all = Instant::now();
    for i in 0..cases {
        let byte = b"A?T"[i % 3];
        let start = Instant::now();
        fixture.reset().map_err(host_error)?;
        fixture.set_input(byte).map_err(host_error)?;
        setup += start.elapsed();
        let start = Instant::now();
        std::hint::black_box(fixture.run(byte)?);
        execute += start.elapsed();
        std::hint::black_box(fixture.cpu.instrumentation());
    }
    Ok(Measurement {
        setup,
        execute,
        total: all.elapsed(),
    })
}

struct Measurement {
    setup: Duration,
    execute: Duration,
    total: Duration,
}

pub fn benchmark() -> Result<(), Error> {
    const CASES: usize = 20_000;
    let mut plain = Fixture::new(()).map_err(host_error)?;
    let mut traced = Fixture::new(EdgeTrace::default()).map_err(host_error)?;
    for round in 0..6 {
        for coverage in [round % 2 != 0, round % 2 == 0] {
            let m = if coverage {
                measure(&mut traced, CASES)?
            } else {
                measure(&mut plain, CASES)?
            };
            let ns = |duration: Duration| duration.as_nanos() as f64 / CASES as f64;
            println!(
                "round={} coverage={} reset_and_input_ns={:.1} execute_and_read_ns={:.1} cases_per_sec={:.0}",
                round + 1,
                coverage,
                ns(m.setup),
                ns(m.execute),
                CASES as f64 / m.total.as_secs_f64()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_restores_guest_writes_and_decoded_code() {
        let mut executor = FixtureExecutor::new().unwrap();
        let accepted = executor.replay(b"A").unwrap();
        assert_eq!(accepted.run.status, Outcome::Accepted);
        assert_eq!(accepted.run.tally, 1);
        assert_eq!(accepted.edges, [1, 0, 0, 0, 0]);

        // Change the cached comparison's literal. Execute once without reset
        // to observe the changed branch, then run the ordinary replay path.
        executor.fixture.reset().unwrap();
        executor
            .fixture
            .cpu
            .memory()
            .write_phys(BxPhyAddress::new(0x1006), b"?")
            .unwrap();
        executor.fixture.set_input(b'?').unwrap();
        assert_eq!(
            executor.fixture.run(b'?').unwrap().status,
            Outcome::Accepted
        );
        let timeout = executor.replay(b"T").unwrap();
        assert_eq!(timeout.run.status, Outcome::DispatchBudget);
        assert_eq!(timeout.run.dispatches, BUDGET);
        assert_eq!(timeout.run.tally, 0);
        assert_eq!(timeout.edges, [0, 1, 1, 0, 1]);
        let rejected = executor.replay(b"?").unwrap();
        assert_eq!(rejected.run.status, Outcome::Rejected);
        assert_eq!(rejected.edges, [0, 1, 0, 1, 0]);
        for _ in 0..4 {
            assert_eq!(executor.replay(b"A").unwrap(), accepted);
            assert_eq!(executor.replay(b"T").unwrap(), timeout);
            assert_eq!(executor.replay(b"?").unwrap(), rejected);
        }
    }
}
