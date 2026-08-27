//! Virtual-processor state and the exits it produces.
//!
//! Nothing here calls the platform; these are the portable shapes both
//! [`crate::sys`] implementations speak in, so the decoding of a raw
//! `WHV_RUN_VP_EXIT_CONTEXT` happens once, behind the seam, and no caller ever
//! matches on a `cfg`.

/// A virtual-processor register this port reads or writes.
///
/// Deliberately a closed set rather than a pass-through of
/// `WHV_REGISTER_NAME`: the mapping to platform values is one exhaustive match
/// (R5), so a register added here cannot be forgotten there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reg {
    Rax,
    Rbx,
    Rcx,
    Rdx,
    Rsi,
    Rdi,
    Rsp,
    Rbp,
    Rip,
    Rflags,
    Cs,
    Ds,
    Es,
    Ss,
    Cr0,
    Cr3,
    Cr4,
    /// `WHvRegisterPendingInterruption` — the event injected into the guest.
    PendingInterruption,
    /// `WHvRegisterInterruptState` — interrupt shadow and NMI mask.
    InterruptState,
    /// `WHvRegisterInternalActivityState` — startup / halt / idle suspend.
    InternalActivityState,
    /// `WHvX64RegisterDeliverabilityNotifications` — ask for an
    /// interrupt-window exit when the guest can next take a vector.
    DeliverabilityNotifications,
}

/// A segment register in the shape `WHV_X64_SEGMENT_REGISTER` wants it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SegmentRegister {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    /// The packed attribute word: segment type in bits 0..4, non-system in bit
    /// 4, DPL in 5..7, present in bit 7, then available / long / default /
    /// granularity in 12..16.
    pub attributes: u16,
}

impl SegmentRegister {
    /// A real-mode code segment: base is `selector << 4`, limit 0xFFFF, and
    /// the attribute word an executable, readable, accessed, present segment.
    #[must_use]
    pub const fn real_mode_code(selector: u16) -> Self {
        Self {
            base: (selector as u64) << 4,
            limit: 0xFFFF,
            selector,
            attributes: 0x9B,
        }
    }

    /// A real-mode data segment: as above, but writable rather than
    /// executable.
    #[must_use]
    pub const fn real_mode_data(selector: u16) -> Self {
        Self {
            base: (selector as u64) << 4,
            limit: 0xFFFF,
            selector,
            attributes: 0x93,
        }
    }
}

/// The `WHV_X64_PENDING_INTERRUPTION_TYPE` values, transcribed from the SDK's
/// `WinHvPlatformDefs.h`. The gaps are the SDK's own — it defines 0, 2 and 3
/// and nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InterruptionType {
    /// A maskable external interrupt, as an 8259 or an I/O APIC would deliver.
    Interrupt,
    /// A non-maskable interrupt.
    Nmi,
    /// An exception, optionally with an error code.
    Exception,
}

impl InterruptionType {
    const fn as_field(self) -> u64 {
        match self {
            Self::Interrupt => 0,
            Self::Nmi => 2,
            Self::Exception => 3,
        }
    }
}

/// An event to hand the guest through `WHvRegisterPendingInterruption`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PendingInterruption {
    pub kind: InterruptionType,
    pub vector: u16,
    pub error_code: Option<u32>,
}

impl PendingInterruption {
    /// The register word, laid out as `WHV_X64_PENDING_INTERRUPTION_REGISTER`:
    /// pending in bit 0, type in 1..4, deliver-error-code in bit 4,
    /// instruction length in 5..9, nested in bit 9, vector in 16..32 and the
    /// error code in the high word.
    #[must_use]
    pub const fn as_word(self) -> u64 {
        let (deliver, code) = match self.error_code {
            Some(code) => (1, code as u64),
            None => (0, 0),
        };
        1 | self.kind.as_field() << 1 | deliver << 4 | (self.vector as u64) << 16 | code << 32
    }
}

/// The `WHV_INTERRUPT_TYPE` values, transcribed from the SDK's
/// `WinHvPlatformDefs.h`.
///
/// Note what is absent: there is no ExtINT. An 8259 living in host userspace
/// therefore has no way to express "the PIC is asserting INTR" through
/// [`InterruptRequest`]; its vectors must reach the guest another way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InterruptKind {
    Fixed,
    LowestPriority,
    Nmi,
    Init,
    Sipi,
    LocalInt1,
}

impl InterruptKind {
    const fn as_field(self) -> u64 {
        match self {
            Self::Fixed => 0,
            Self::LowestPriority => 1,
            Self::Nmi => 4,
            Self::Init => 5,
            Self::Sipi => 6,
            Self::LocalInt1 => 9,
        }
    }
}

/// How the destination field names its target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DestinationMode {
    Physical,
    Logical,
}

/// Edge or level, as the interrupt's source drives it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TriggerMode {
    Edge,
    Level,
}

/// An interrupt handed to the partition's emulated APIC, rather than injected
/// straight into a processor.
///
/// This is the delivery path that exists only when the hypervisor emulates a
/// local APIC; with [`crate::LocalApicMode::None`] there is no APIC to accept
/// it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InterruptRequest {
    pub kind: InterruptKind,
    pub destination_mode: DestinationMode,
    pub trigger_mode: TriggerMode,
    /// The APIC ID, or the logical destination, depending on
    /// `destination_mode`.
    pub destination: u32,
    pub vector: u32,
}

impl InterruptRequest {
    /// The packed control word, laid out as `WHV_INTERRUPT_CONTROL`: type in
    /// bits 0..8, destination mode in 8..12, trigger mode in 12..16 and the
    /// target VTL — always zero here — in 16..24.
    #[must_use]
    pub const fn control_word(self) -> u64 {
        let destination_mode = match self.destination_mode {
            DestinationMode::Physical => 0,
            DestinationMode::Logical => 1,
        };
        let trigger_mode = match self.trigger_mode {
            TriggerMode::Edge => 0,
            TriggerMode::Level => 1,
        };
        self.kind.as_field() | destination_mode << 8 | trigger_mode << 12
    }
}

/// Bit positions within `WHV_INTERNAL_ACTIVITY_REGISTER`.
mod activity_bit {
    pub(super) const STARTUP_SUSPEND: u32 = 0;
    pub(super) const HALT_SUSPEND: u32 = 1;
    pub(super) const IDLE_SUSPEND: u32 = 2;
}

/// Why a virtual processor is not executing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InternalActivity {
    /// Parked awaiting a SIPI.
    pub startup_suspend: bool,
    /// Parked in `HLT`.
    pub halt_suspend: bool,
    /// Parked by the hypervisor's own idle handling.
    pub idle_suspend: bool,
}

impl InternalActivity {
    #[must_use]
    pub const fn from_word(word: u64) -> Self {
        const fn bit(word: u64, at: u32) -> bool {
            word >> at & 1 != 0
        }
        Self {
            startup_suspend: bit(word, activity_bit::STARTUP_SUSPEND),
            halt_suspend: bit(word, activity_bit::HALT_SUSPEND),
            idle_suspend: bit(word, activity_bit::IDLE_SUSPEND),
        }
    }

    #[must_use]
    pub const fn as_word(self) -> u64 {
        (self.startup_suspend as u64) << activity_bit::STARTUP_SUSPEND
            | (self.halt_suspend as u64) << activity_bit::HALT_SUSPEND
            | (self.idle_suspend as u64) << activity_bit::IDLE_SUSPEND
    }
}

/// What the guest was doing to memory when it exited.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccessType {
    Read,
    Write,
    Execute,
    /// A value the SDK does not define today, preserved rather than lost.
    Unknown(u32),
}

/// The context of a `WHvRunVpExitReasonMemoryAccess`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MemoryAccess {
    pub gpa: u64,
    /// The linear address, meaningful only when `gva_valid`.
    pub gva: u64,
    pub gva_valid: bool,
    pub access: AccessType,
    /// True when nothing is mapped at `gpa` at all; false when something is
    /// mapped but refused the access. This is the bit that separates ordinary
    /// MMIO from a write to a read-only shadowed window.
    pub gpa_unmapped: bool,
    /// How many of `instruction_bytes` the hypervisor filled in. Zero means it
    /// could not fetch the instruction, and the host must fetch it itself.
    pub instruction_byte_count: u8,
    pub instruction_bytes: [u8; 16],
}

/// The context of a `WHvRunVpExitReasonX64IoPortAccess`. Pre-decoded by the
/// hypervisor, which is why non-string port I/O needs no instruction decoder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IoPortAccess {
    pub port: u16,
    pub is_write: bool,
    /// 1, 2 or 4 bytes.
    pub access_size: u8,
    pub string_op: bool,
    pub rep_prefix: bool,
    pub rax: u64,
    pub rcx: u64,
    pub rsi: u64,
    pub rdi: u64,
}

/// The context of a `WHvRunVpExitReasonX64Cpuid`, carrying both what the guest
/// asked and what the hypervisor would have answered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CpuidAccess {
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rbx: u64,
    pub default_rax: u64,
    pub default_rcx: u64,
    pub default_rdx: u64,
    pub default_rbx: u64,
}

/// The context of a `WHvRunVpExitReasonX64MsrAccess`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MsrAccess {
    pub msr: u32,
    pub is_write: bool,
    pub rax: u64,
    pub rdx: u64,
}

/// Why the virtual processor stopped.
///
/// Exhaustive (R5): a platform exit this port has not thought about must break
/// every dispatch site rather than fall into a default arm.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExitReason {
    /// `WHvRunVpExitReasonNone` — the SDK's zero, which a successful run does
    /// not produce.
    None,
    MemoryAccess(MemoryAccess),
    IoPortAccess(IoPortAccess),
    UnrecoverableException,
    InvalidVpRegisterValue,
    UnsupportedFeature { code: i32, parameter: u64 },
    InterruptWindow,
    Halt,
    ApicEoi { vector: u32 },
    SynicSintDeliverable,
    MsrAccess(MsrAccess),
    Cpuid(CpuidAccess),
    Exception,
    Rdtsc,
    ApicSmiTrap,
    Hypercall,
    ApicInitSipiTrap,
    ApicWriteTrap,
    /// The host called `WHvCancelRunVirtualProcessor`.
    Canceled { reason: i32 },
    /// An exit reason the SDK has grown since this port was written.
    Unrecognized(i32),
}

/// The processor state every exit carries, whatever its reason.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VpContext {
    pub rip: u64,
    pub rflags: u64,
    pub cs: SegmentRegister,
    /// The length in bytes of the instruction that caused the exit. Advancing
    /// `rip` by this is how a host finishes a trapped access without owning a
    /// decoder.
    pub instruction_length: u8,
    pub cr8: u8,
    /// The raw `WHV_X64_VP_EXECUTION_STATE` word.
    pub execution_state: u16,
}

/// One return from `WHvRunVirtualProcessor`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Exit {
    pub vp: VpContext,
    pub reason: ExitReason,
}

impl Exit {
    /// Where the guest would resume if the host finishes the trapped
    /// instruction itself.
    #[must_use]
    pub const fn rip_after_instruction(&self) -> u64 {
        self.vp.rip.wrapping_add(self.vp.instruction_length as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_mode_segment_puts_the_selector_in_the_base_and_says_present() {
        let cs = SegmentRegister::real_mode_code(0x1000);
        assert_eq!(cs.base, 0x1_0000);
        assert_eq!(cs.limit, 0xFFFF);
        // Present, non-system, executable/readable/accessed.
        assert_eq!(cs.attributes, 0x9B);
        assert_eq!(SegmentRegister::real_mode_data(0).base, 0);
        assert_eq!(SegmentRegister::real_mode_data(0).attributes, 0x93);
    }

    /// The injection word is the one place a mis-transcribed field silently
    /// delivers the wrong vector, so pin every field against the SDK's layout.
    #[test]
    fn an_injected_vector_lands_in_the_high_half_of_the_word() {
        let injected = PendingInterruption {
            kind: InterruptionType::Interrupt,
            vector: 0x40,
            error_code: None,
        };
        let word = injected.as_word();
        assert_eq!(word & 1, 1, "pending");
        assert_eq!(word >> 1 & 0b111, 0, "type Interrupt is the SDK's zero");
        assert_eq!(word >> 4 & 1, 0, "no error code");
        assert_eq!(word >> 16 & 0xFFFF, 0x40, "vector");
    }

    #[test]
    fn an_exception_with_an_error_code_sets_both_the_flag_and_the_high_word() {
        let injected = PendingInterruption {
            kind: InterruptionType::Exception,
            vector: 14,
            error_code: Some(0xDEAD_BEEF),
        };
        let word = injected.as_word();
        assert_eq!(word >> 1 & 0b111, 3, "type Exception");
        assert_eq!(word >> 4 & 1, 1, "deliver error code");
        assert_eq!(word >> 32, 0xDEAD_BEEF);
    }

    #[test]
    fn an_interrupt_request_packs_its_three_fields_where_the_platform_reads_them() {
        let request = InterruptRequest {
            kind: InterruptKind::Fixed,
            destination_mode: DestinationMode::Physical,
            trigger_mode: TriggerMode::Edge,
            destination: 0,
            vector: 0x40,
        };
        assert_eq!(request.control_word(), 0, "every field's zero is the SDK's own");

        let nmi_logical_level = InterruptRequest {
            kind: InterruptKind::Nmi,
            destination_mode: DestinationMode::Logical,
            trigger_mode: TriggerMode::Level,
            ..request
        };
        assert_eq!(nmi_logical_level.control_word(), 4 | 1 << 8 | 1 << 12);
    }

    #[test]
    fn a_halted_processor_is_distinguishable_from_one_awaiting_a_sipi() {
        let halted = InternalActivity::from_word(1 << 1);
        assert!(halted.halt_suspend && !halted.startup_suspend);
        let parked = InternalActivity::from_word(1 << 0);
        assert!(parked.startup_suspend && !parked.halt_suspend);
        assert_eq!(InternalActivity::from_word(halted.as_word()), halted);
    }

    #[test]
    fn resuming_past_a_trapped_instruction_uses_the_length_the_exit_reported() {
        let exit = Exit {
            vp: VpContext {
                rip: 0x1000,
                rflags: 2,
                cs: SegmentRegister::real_mode_code(0),
                instruction_length: 5,
                cr8: 0,
                execution_state: 0,
            },
            reason: ExitReason::Halt,
        };
        assert_eq!(exit.rip_after_instruction(), 0x1005);
    }
}
