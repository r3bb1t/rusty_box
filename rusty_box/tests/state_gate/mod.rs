//! The long-mode fault harness the CPU-state gate tests share
//! (`vex_avx_state_gate.rs`, `fpu_mmx_sse_state_gate.rs`).
//!
//! One instruction runs on a flat 64-bit machine whose #UD, #NM and #MF gates
//! point at one-byte `HLT` handlers of their own. Where the processor stops
//! says what the guest saw, and the exception frame it left says which
//! instruction raised it — properties of the guest, not of the emulator's
//! bookkeeping (doctrine R9), so they hold under `--release`. The runners take
//! a machine with any tracer, so a test can watch what instrumentation sees
//! while the guest faults.

use rusty_box::cpu::{CpuSetupMode, Instrumentation, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

/// Emulator construction needs more than the 2 MiB a test thread starts with.
pub const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
/// Where the instruction under test is written and run from.
pub const CODE: u64 = 0x0020_0000;
const IDT: u64 = 0x0028_0000;
const STACK: u64 = 0x0030_0000;

/// An IDT gate, and the one-instruction handler (`HLT`) it points at.
pub struct Gate {
    pub vector: u64,
    pub handler: u64,
}

pub const UD_GATE: Gate = Gate {
    vector: 6,
    handler: 0x0029_0000,
};
pub const NM_GATE: Gate = Gate {
    vector: 7,
    handler: 0x0029_0010,
};
pub const MF_GATE: Gate = Gate {
    vector: 16,
    handler: 0x0029_0020,
};

/// What one instruction did, as the guest sees it.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Outcome {
    /// It retired, and the processor went on to the next instruction.
    Retired,
    /// It raised #UD: the processor entered the #UD handler with the
    /// instruction's own address on its stack.
    InvalidOpcode,
    /// It raised #NM, the same way.
    DeviceNotAvailable,
    /// It raised #MF, the same way.
    FloatingPointError,
}

/// Flat 64-bit long mode, as `config` builds it. The control registers are
/// the reset values plus what long mode needs: CR0.EM, CR0.TS and CR0.MP
/// clear, CR4.OSFXSR and CR4.OSXSAVE clear, XCR0 = 1.
pub fn long64_emulator(config: EmulatorConfig) -> Box<Emulator> {
    Emulator::new_with_mode(config, CpuSetupMode::FlatLong64).expect("emulator")
}

/// Point the #UD, #NM and #MF gates at their `HLT` handlers.
pub fn install_fault_handlers<T: Instrumentation>(emu: &mut Emulator<T>) {
    emu.reg_write(X86Reg::IdtrBase, IDT);
    emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
    for gate in [&UD_GATE, &NM_GATE, &MF_GATE] {
        let mut entry = [0u8; 16];
        entry[0..2].copy_from_slice(&(gate.handler as u16).to_le_bytes());
        entry[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
        entry[5] = 0x8E; // present, DPL 0, 64-bit interrupt gate
        entry[6..8].copy_from_slice(&((gate.handler >> 16) as u16).to_le_bytes());
        entry[8..12].copy_from_slice(&((gate.handler >> 32) as u32).to_le_bytes());
        emu.mem_write(IDT + gate.vector * 16, &entry)
            .expect("write the gate");
        emu.mem_write(gate.handler, &[0xF4]).expect("write the handler");
    }
}

/// Install the fault handlers, then run one encoding at [`CODE`].
pub fn run_one<T: Instrumentation>(emu: &mut Emulator<T>, code: &[u8]) -> Outcome {
    run_one_at(emu, CODE, code)
}

/// Install the fault handlers, then run one encoding at `at`:
/// [`run_installed_at`].
pub fn run_one_at<T: Instrumentation>(emu: &mut Emulator<T>, at: u64, code: &[u8]) -> Outcome {
    install_fault_handlers(emu);
    run_installed_at(emu, at, code)
}

/// Run one encoding, written at `at`, against the handlers already in place
/// and report what the guest saw. A fault leaves exactly one long-mode
/// exception frame — SS, RSP, RFLAGS, CS and RIP, since none of #UD, #NM and
/// #MF pushes an error code — whose RIP is the instruction itself. A handler
/// that returns (rather than halting) has popped its frame, and the
/// instruction it returned to retires.
pub fn run_installed_at<T: Instrumentation>(
    emu: &mut Emulator<T>,
    at: u64,
    code: &[u8],
) -> Outcome {
    let mut image = code.to_vec();
    image.extend_from_slice(&[0xEB, 0xFE]); // jmp $
    emu.mem_write(at, &image).expect("write code");
    emu.reg_write(X86Reg::Rsp, STACK);
    let stop = emu.emu_start(at, None, None, Some(8)).expect("emu_start");
    let rip = emu.cpu().rip();
    let outcome = if rip == at + code.len() as u64 {
        Outcome::Retired
    } else if rip == UD_GATE.handler + 1 {
        Outcome::InvalidOpcode
    } else if rip == NM_GATE.handler + 1 {
        Outcome::DeviceNotAvailable
    } else if rip == MF_GATE.handler + 1 {
        Outcome::FloatingPointError
    } else {
        panic!("the instruction neither retired nor took #UD, #NM or #MF: rip={rip:#x}, stop={stop:?}");
    };
    match outcome {
        Outcome::Retired => assert_eq!(
            emu.reg_read(X86Reg::Rsp),
            STACK,
            "a retired instruction leaves the stack as it found it"
        ),
        Outcome::InvalidOpcode | Outcome::DeviceNotAvailable | Outcome::FloatingPointError => {
            assert_eq!(
                emu.reg_read(X86Reg::Rsp),
                STACK - 40,
                "a fault pushes exactly one exception frame"
            );
            let mut pushed_rip = [0u8; 8];
            emu.mem_read(STACK - 40, &mut pushed_rip)
                .expect("read the frame");
            assert_eq!(
                u64::from_le_bytes(pushed_rip),
                at,
                "the frame's RIP must be the faulting instruction"
            );
        }
    }
    outcome
}
