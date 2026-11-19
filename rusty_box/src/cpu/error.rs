use thiserror::Error;

use crate::cpu::cpu::Exception;
use crate::cpu::decoder::Opcode;
pub type Result<T> = core::result::Result<T, CpuError>;

#[derive(Error, Debug)]
pub enum CpuError {
    #[error("exception({vector:?}): bad vector, error_code={error_code}")]
    BadVector { vector: Exception, error_code: u16 },

    #[error("CPU/Emulator not initialized - call initialize() first")]
    CpuNotInitialized,

    #[error("prefetch: running in bogus memory, pAddr={p_addr:#x}")]
    PrefetchBogusMemory { p_addr: u64 },

    #[error("prefetch: getHostMemAddr vetoed direct read, pAddr={p_addr:#x}")]
    VetoedDirectRead { p_addr: u64 },

    /// The scalar instruction has not retired. The device reported no effect.
    #[error("physical access needs host service: {0:?}")]
    PhysicalAccessPending(super::memory_session::PhysicalRequest),

    /// The physical device refused the operation without committing any effect.
    #[error("physical bus failure {code}: {request:?}")]
    PhysicalBusFailure {
        request: super::memory_session::PhysicalRequest,
        code: u32,
    },

    /// The guarded scalar step encountered an architectural fault before delivery.
    #[error("guarded scalar step stopped before delivering {vector:?}({error_code:#x})")]
    GuestFaultBeforeDelivery { vector: Exception, error_code: u16 },

    // smm
    #[error("smram map[{index}] = {value}")]
    SmramMap { index: usize, value: u32 },

    #[error(transparent)]
    CpuId(#[from] super::cpuid::CpuIdError),

    #[error("Decoder error")]
    Decoder(#[from] super::decoder::DecodeError),

    #[error(transparent)]
    TryFromIntError(#[from] core::num::TryFromIntError),

    #[error(transparent)]
    Memory(#[from] crate::memory::MemoryError),

    /// No port provider was supplied. The instruction has not retired and
    /// RIP still names it. String forms are refused before any iteration;
    /// this milestone does not implement partial REP I/O host completion.
    #[error(
        "port I/O unavailable: port={port:?}, bytes={len}, input={direction_in}, string={string}"
    )]
    IoUnavailable {
        port: crate::iodev::IoPort,
        len: u32,
        direction_in: bool,
        string: bool,
    },

    #[error("Unimplemented instruction or feature")]
    UnimplementedInstruction,

    #[error("Unimplemented opcode: {opcode:?}")]
    UnimplementedOpcode { opcode: Opcode },

    #[error("Invalid boot image: {reason}")]
    InvalidBootImage { reason: &'static str },

    #[error("Unsupported CPU operation: {operation}")]
    UnsupportedCpuOperation { operation: &'static str },

    /// A host read or write of a model-specific register was refused, and why.
    #[error("MSR {msr:#010x}: {reason}")]
    MsrRefused { msr: u32, reason: MsrRefusal },

    #[error("machine boundary effect failed")]
    MachineBoundaryFailed,

    /// A host callback failed. The instruction's committed effects remain.
    #[error(transparent)]
    MmioSubscriber(crate::emulator::hardware::MmioSubscriberError),

    /// CPU-only execution reached deferred machine work (LAPIC bus/timer,
    /// CPU-control or EOI). The issuing instruction may have retired already:
    /// RIP and counts remain committed, and the request stays latched.
    #[error("CPU-only execution needs a machine boundary; issuing instruction may have retired")]
    MachineBoundaryUnavailable,

    /// The machine's engine refused work the machine cannot do without it: an
    /// I/O APIC message its backend would not take, an interrupt edge it could
    /// not be told about.
    ///
    /// Carried whole rather than reduced to the operation that failed. The
    /// fault's kind and the backend's own error number are what separate a
    /// mapping that could not be applied from a processor that would not run,
    /// and a refused interrupt leaves no other trace to read.
    #[error("engine fault: {0}")]
    EngineFault(rusty_box_core::EngineFault),

    /// Bochs-style control flow: exceptions/interrupt delivery longjmp back to the
    /// main decode loop. We model that by unwinding the current instruction/trace
    /// and restarting decode.
    #[error("cpu loop restart (bochs longjmp)")]
    CpuLoopRestart,

    /// x86 exception generated during execution (e.g., #UD, #GP, #PF)
    /// The exception has been delivered via IVT/IDT, but execution cannot continue
    /// (e.g., unhandled exception handler address is 0000:0000)
    #[error("x86 exception #{vector} delivered but unhandled")]
    Exception { vector: u8 },

    /// Bochs `BX_PANIC` equivalents reachable from the VMX path —
    /// "impossible" CPU states (e.g., VMEXIT requested while not in
    /// VMX-guest mode without the bit-31 vmentry-failure flag). A host-side
    /// implementation bug, not a guest-induced architectural fault: a VMX
    /// abort the guest causes shuts the processor down instead, as Bochs
    /// `VMabort` does.
    #[error("VMX internal error: {reason:?}")]
    VmxInternalError {
        reason: super::vmx::VmxInternalReason,
    },
}

/// Why a host read or write of an MSR was refused (R0, R5).
///
/// A closed set: a host that wants to tell "this processor has no such
/// register" from "this API does not carry it" matches on these, and a new
/// reason has to be handled everywhere one is matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsrRefusal {
    /// This processor's CPU model does not have the register: the feature
    /// that defines it is off, so a guest's RDMSR or WRMSR of it would #GP.
    Absent,
    /// The register exists, but the host API does not read or write it.
    NotCarried,
    /// The register exists and is read-only, so there is nothing to write.
    ReadOnly,
    /// The processor rejects the value: a reserved bit is set, or an address
    /// is not canonical. A guest's WRMSR of it would #GP.
    InvalidValue,
    /// The processor would take the write and then ignore it. Answered as a
    /// refusal because a host has no other way to see "ignored".
    WriteIgnored,
}

impl core::fmt::Display for MsrRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Absent => "this processor's CPU model does not have it",
            Self::NotCarried => "the host API does not read or write it",
            Self::ReadOnly => "it is read-only",
            Self::InvalidValue => "the processor rejects that value",
            Self::WriteIgnored => "the processor would ignore the write",
        })
    }
}
