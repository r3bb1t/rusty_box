//! Virtual-processor state and the exits it produces.
//!
//! Nothing here calls the platform; these are the portable shapes both of this
//! crate's platform implementations speak in, so the decoding of a raw
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
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
    Rip,
    Rflags,
    Cs,
    Ds,
    Es,
    Ss,
    Fs,
    Gs,
    /// Segment-shaped on this platform, like the six above: the local
    /// descriptor table and the task register each carry a base, a limit and
    /// an attribute word, not just a selector.
    Ldtr,
    Tr,
    /// Table-shaped: a base and a 16-bit limit, and nothing else.
    Gdtr,
    Idtr,
    Cr0,
    Cr2,
    Cr3,
    Cr4,
    Cr8,
    Dr0,
    Dr1,
    Dr2,
    Dr3,
    Dr6,
    Dr7,
    Efer,
    KernelGsBase,
    Star,
    Lstar,
    Cstar,
    Sfmask,
    SysenterCs,
    SysenterEsp,
    SysenterEip,
    Pat,
    ApicBase,
    Tsc,
    Xcr0,
    /// `WHvRegisterPendingInterruption` — the event injected into the guest.
    PendingInterruption,
    /// `WHvRegisterInterruptState` — interrupt shadow and NMI mask.
    InterruptState,
    /// `WHvRegisterInternalActivityState` — startup / halt / idle suspend.
    InternalActivityState,
    /// `WHvX64RegisterDeliverabilityNotifications` — ask for an
    /// interrupt-window exit when the guest can next take a vector.
    DeliverabilityNotifications,
    /// `WHvRegisterPendingEvent` (0x80000002) — the 128-bit event slot, and the
    /// only register this port names that does not fit in a word. Read and
    /// written through [`crate::RegisterValue::Words128`]; see
    /// [`PendingExtIntEvent`] for the one event shape this port encodes.
    PendingEvent,
    /// `WHvX64RegisterApicTpr` (0x3008) — the task-priority register under its
    /// own platform name, as opposed to `CR8`. Named so a caller can ask
    /// whether an offloaded APIC accepts a write to one of its registers; the
    /// answer is the platform's, and asking is the only way to have it.
    ApicTpr,
}

impl Reg {
    /// Whether this register carries a base, a limit, a selector and an
    /// attribute word rather than a single value.
    ///
    /// The platform's own split, and the reason a state exchange cannot be one
    /// array of words: `WHV_X64_SEGMENT_REGISTER` and `WHV_X64_TABLE_REGISTER`
    /// are different members of the value union, and reading either as a word
    /// returns its base and silently drops the rest.
    #[must_use]
    pub const fn is_segment(self) -> bool {
        matches!(
            self,
            Self::Cs | Self::Ds | Self::Es | Self::Ss | Self::Fs | Self::Gs | Self::Ldtr | Self::Tr
        )
    }

    /// Whether this register is a descriptor-table base and limit.
    #[must_use]
    pub const fn is_table(self) -> bool {
        matches!(self, Self::Gdtr | Self::Idtr)
    }

    /// Whether this register is 128 bits wide.
    ///
    /// The distinction a word transfer cannot survive: `WHV_REGISTER_VALUE` is
    /// sixteen bytes and reading one of these as a `u64` returns its low half
    /// and drops the rest silently. [`ALL_REGS`] is filtered on this, so the
    /// per-slice exchange can stay a word transfer.
    #[must_use]
    pub const fn is_words128(self) -> bool {
        matches!(self, Self::PendingEvent)
    }
}

/// A descriptor-table register in the shape `WHV_X64_TABLE_REGISTER` wants it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TableRegister {
    pub base: u64,
    pub limit: u16,
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

/// Bit positions and the event-type value within
/// `WHV_X64_PENDING_EXT_INT_EVENT`, the ExtINT member of the 128-bit
/// `WHvRegisterPendingEvent` slot.
mod pending_event_bit {
    pub(super) const PENDING: u32 = 0;
    pub(super) const EVENT_TYPE: u32 = 1;
    /// `EventType` is three bits wide (`EventPending:1; EventType:3;
    /// Reserved0:4`). The mask stops at bit 3 so that a reserved bit the
    /// platform sets above the field cannot be read as part of the type — which
    /// would answer "no ExtINT pending" for a slot that holds one, and have the
    /// caller inject a second on top of it.
    pub(super) const EVENT_TYPE_WIDTH: u64 = 0b111;
    pub(super) const VECTOR: u32 = 8;
    /// `WHvX64PendingEventExtInt`. The one event type this port encodes, and
    /// the reason the register exists for it: an 8259 in host userspace has no
    /// `WHV_INTERRUPT_TYPE` to express itself with, so ExtINT reaches the guest
    /// through this slot or not at all.
    pub(super) const EXT_INT: u64 = 5;
}

/// An ExtINT the guest is to take on its next interruptible instruction
/// boundary, in the shape `WHV_X64_PENDING_EXT_INT_EVENT` wants it.
///
/// `EventPending` is bit 0, `EventType` bits 1..4 (5 for ExtINT) and `Vector`
/// bits 8..16; bits 4..8 are reserved, as is the upper word of the 128-bit
/// register, and both are written as zero.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PendingExtIntEvent {
    pub vector: u8,
}

impl PendingExtIntEvent {
    /// The two words to hand `WHvSetVirtualProcessorRegisters` for
    /// [`Reg::PendingEvent`].
    #[must_use]
    pub const fn as_words(self) -> [u64; 2] {
        [
            1 << pending_event_bit::PENDING
                | pending_event_bit::EXT_INT << pending_event_bit::EVENT_TYPE
                | (self.vector as u64) << pending_event_bit::VECTOR,
            0,
        ]
    }

    /// Read the slot back, or answer `None` when it holds no ExtINT.
    ///
    /// Two different absences answer the same way on purpose: an empty slot,
    /// and a slot holding an event of another type — an exception, say, which
    /// the hypervisor may have put there itself. Neither is an ExtINT this port
    /// placed, and a caller asking whether its own injection is still pending
    /// wants "no" for both.
    #[must_use]
    pub const fn from_words(words: [u64; 2]) -> Option<Self> {
        let low = words[0];
        let pending = low >> pending_event_bit::PENDING & 1 != 0;
        let event_type =
            low >> pending_event_bit::EVENT_TYPE & pending_event_bit::EVENT_TYPE_WIDTH;
        if pending && event_type == pending_event_bit::EXT_INT {
            Some(Self { vector: (low >> pending_event_bit::VECTOR) as u8 })
        } else {
            None
        }
    }
}

/// Which APIC register the guest wrote, for a `WHvRunVpExitReasonX64ApicWriteTrap`.
///
/// The `WHV_X64_APIC_WRITE_TYPE` values are the xAPIC MMIO offsets themselves —
/// LDR at 0xD0, DFR at 0xE0, SVR at 0xF0, LINT0 at 0x350 and LINT1 at 0x360 —
/// which is why [`Self::mmio_offset`] is the same number rather than a lookup.
/// Exhaustive (R5): these are every trap the platform's four
/// `X64ApicWrite*ExitTrap` bits plus its SVR trap can produce.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApicWriteType {
    Ldr,
    Dfr,
    Svr,
    Lint0,
    Lint1,
}

impl ApicWriteType {
    /// Where this register lives in the xAPIC's memory-mapped page. The
    /// platform's own value for the trap IS this number, which is what
    /// `each_apic_write_type_is_its_own_mmio_offset` pins.
    #[must_use]
    pub const fn mmio_offset(self) -> u16 {
        match self {
            Self::Ldr => 0x0D0,
            Self::Dfr => 0x0E0,
            Self::Svr => 0x0F0,
            Self::Lint0 => 0x350,
            Self::Lint1 => 0x360,
        }
    }

    /// The same register's field in the state page.
    ///
    /// A different number from [`Self::mmio_offset`], and deliberately a
    /// different type: the state page is a structure whose field order has
    /// nothing to do with the xAPIC's address map, so mirroring a trapped
    /// write into it goes through here rather than through an offset (R4).
    #[must_use]
    pub const fn register(self) -> ApicRegister {
        match self {
            Self::Ldr => ApicRegister::Ldr,
            Self::Dfr => ApicRegister::Dfr,
            Self::Svr => ApicRegister::Spurious,
            Self::Lint0 => ApicRegister::LvtLint0,
            Self::Lint1 => ApicRegister::LvtLint1,
        }
    }
}

/// One scalar register of the local APIC as the state page carries it.
///
/// A name rather than an offset because the page is a STRUCTURE, not an address
/// space: its fields sit in a fixed order with no padding, and a register's
/// position in it has nothing to do with that register's memory-mapped offset
/// (R4). See [`ApicStatePage`] for the provenance of the order.
///
/// Exhaustive (R5): these are every scalar field the structure declares. The
/// three 256-bit interrupt bitmaps are [`ApicVector`] instead, because each is
/// eight words rather than one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApicRegister {
    Id,
    Version,
    /// Logical destination.
    Ldr,
    /// Destination format.
    Dfr,
    /// Spurious-interrupt vector, which also holds the APIC software enable.
    Spurious,
    /// Error status as the hypervisor last latched it.
    Esr,
    IcrHigh,
    IcrLow,
    LvtTimer,
    LvtThermal,
    LvtPerfmon,
    LvtLint0,
    LvtLint1,
    LvtError,
    LvtCmci,
    ErrorStatus,
    InitialCount,
    CounterValue,
    DivideConfiguration,
    RemoteRead,
}

impl ApicRegister {
    /// Which 32-bit word of the page this register is.
    ///
    /// The structure's field order, stated once (R5). The three bitmaps
    /// occupy words 5..29 between [`Self::Spurious`] and [`Self::Esr`], which
    /// is what puts the LVT entries as far into the page as they are.
    #[must_use]
    pub const fn word(self) -> usize {
        match self {
            Self::Id => 0,
            Self::Version => 1,
            Self::Ldr => 2,
            Self::Dfr => 3,
            Self::Spurious => 4,
            // Words 5..29 are the three bitmaps.
            Self::Esr => 29,
            Self::IcrHigh => 30,
            Self::IcrLow => 31,
            Self::LvtTimer => 32,
            Self::LvtThermal => 33,
            Self::LvtPerfmon => 34,
            Self::LvtLint0 => 35,
            Self::LvtLint1 => 36,
            Self::LvtError => 37,
            Self::LvtCmci => 38,
            Self::ErrorStatus => 39,
            Self::InitialCount => 40,
            Self::CounterValue => 41,
            Self::DivideConfiguration => 42,
            Self::RemoteRead => 43,
        }
    }
}

/// One of the local APIC's three 256-bit interrupt bitmaps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApicVector {
    /// In-service: the vectors the processor has accepted and not yet
    /// acknowledged.
    InService,
    /// Trigger mode: which of the in-service vectors arrived level-triggered,
    /// and so need an EOI broadcast to their source.
    TriggerMode,
    /// Request: the vectors raised and not yet accepted.
    Request,
}

impl ApicVector {
    /// How many 32-bit words one bitmap is: 256 vectors, one bit each.
    pub const WORDS: usize = 8;

    /// The word this bitmap starts at.
    const fn first_word(self) -> usize {
        match self {
            Self::InService => 5,
            Self::TriggerMode => 5 + Self::WORDS,
            Self::Request => 5 + 2 * Self::WORDS,
        }
    }
}

/// The `WHvVirtualProcessorStateTypeInterruptControllerState2` page.
///
/// A packed run of 32-bit registers in the field order of Hyper-V's
/// `HV_X64_INTERRUPT_CONTROLLER_STATE` — id, version, LDR, DFR, SVR, then the
/// in-service, trigger-mode and request bitmaps at eight words each, then ESR,
/// the two halves of the ICR, and the LVT entries. There is no padding and no
/// relation to the xAPIC's memory-mapped offsets; the page is 4096 bytes only
/// because the platform demands that size, and everything past word 43 is
/// unused.
///
/// # Provenance
///
/// `WinHvPlatformDefs.h` declares no x64 structure for this state type at all —
/// only the ARM64 ones — so the field order above is Hyper-V's published
/// interrupt-controller state, held against this platform by measurement (R7).
/// Two tests here pin the structure's shape and the request bitmap's position;
/// what they leave open, the platform probe closes. What each settles, and what
/// none of them does:
///
/// - `the_apic_state_page_round_trips_through_the_platform` reads a fresh
///   processor's page. It pins words 0, 1, 3 and 4 BY VALUE — 0x00050014 at
///   word 1 is an APIC version reporting six LVT entries, 0xFFFFFFFF at word 3
///   and 0x000000FF at word 4 are the architectural reset values of DFR and
///   SVR — and pins the six masked LVT words at 32..38. Six masked entries can
///   only land there with 27 words in front of them, so it also pins the
///   EXTENT of words 5..32. It does not pin what is inside that run: at reset
///   every word of it reads zero.
/// - `a_requested_vector_appears_in_the_pages_request_bitmap` makes the
///   platform set one bit inside that run and finds it where
///   [`ApicVector::Request`] predicts, which pins the request bitmap's position
///   and word order by observation. The in-service and trigger-mode bitmaps
///   then have only the two remaining eight-word slots between them, and
///   `Esr`, `IcrHigh` and `IcrLow` only the three words after those.
///
/// - The order of the two remaining bitmaps is **measured**, by the platform
///   probe's P9 (`whp_probe.rs`, `q18_in_service_versus_trigger_mode`;
///   recorded in `docs/whp-platform-probe-2026-09-03.md`). A symmetric test
///   cannot settle it — a conversion that reads and writes through one wrong
///   mapping agrees with itself — so the probe took two observations in which
///   the platform treats the two fields differently, and both select this
///   order. A level-triggered request into a processor that has never run sets
///   [`ApicVector::TriggerMode`] and not [`ApicVector::InService`], while the
///   same request edge-triggered sets neither; and a vector the guest accepts
///   without acknowledging leaves the request bitmap for
///   [`ApicVector::InService`], with trigger-mode clear for that edge delivery.
///   Each is the opposite of what the swapped arrangement predicts. Do not
///   "correct" this order back.
///
/// The order of the ICR's two halves (the xAPIC memory map has the low half
/// first, this structure the high) remains transcribed rather than measured:
/// both words read zero in every page the probe took, so nothing distinguishes
/// them. Everything from [`ApicRegister::LvtCmci`] on is likewise transcribed
/// and reads as zero at reset, so no measurement bears on it either — including
/// the duplication of [`ApicRegister::Esr`] at word 29 with
/// [`ApicRegister::ErrorStatus`] at word 39, which must be settled against the
/// SDK's own field order before either is converted in anger.
///
/// Boxed because it is a whole page: a `Partition` verb takes it by reference,
/// and a page-sized value passed by move would land on the stack of whatever
/// built it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ApicStatePage(pub Box<[u8; ApicStatePage::BYTES]>);

impl ApicStatePage {
    /// How many bytes `WHvGetVirtualProcessorState` fills for this state type,
    /// and the exact size its setter demands.
    pub const BYTES: usize = 4096;

    /// How many words of the page the structure actually occupies. The rest is
    /// zero on every read this port has taken.
    pub const WORDS: usize = 44;

    /// A page of zeroes — what a read fills in, and the only starting point a
    /// write should be built from.
    #[must_use]
    pub fn zeroed() -> Self {
        Self(Box::new([0; Self::BYTES]))
    }

    /// One scalar register.
    #[must_use]
    pub fn register(&self, reg: ApicRegister) -> u32 {
        self.word(reg.word())
    }

    /// Write one scalar register.
    pub fn set_register(&mut self, reg: ApicRegister, value: u32) {
        self.set_word(reg.word(), value);
    }

    /// One interrupt bitmap, low word first — bit `n` of word `w` is vector
    /// `32 * w + n`.
    #[must_use]
    pub fn vector(&self, which: ApicVector) -> [u32; ApicVector::WORDS] {
        let mut out = [0; ApicVector::WORDS];
        for (index, slot) in out.iter_mut().enumerate() {
            *slot = self.word(which.first_word() + index);
        }
        out
    }

    /// Write one interrupt bitmap, in the order [`Self::vector`] returns it.
    pub fn set_vector(&mut self, which: ApicVector, bits: [u32; ApicVector::WORDS]) {
        for (index, value) in bits.into_iter().enumerate() {
            self.set_word(which.first_word() + index, value);
        }
    }

    /// Every word either accessor can name is inside the page, so neither
    /// needs a bounds check: [`ApicRegister::word`] and
    /// [`ApicVector::first_word`] are closed sets and [`Self::WORDS`] is their
    /// upper bound. Checked rather than asserted in prose, because a field
    /// appended to the structure would otherwise index past the page.
    const _IN_PAGE: () = assert!(Self::WORDS * 4 <= Self::BYTES);

    /// The 32-bit word at `index`. The one place the page's byte order is
    /// decided (R5); the platform writes little-endian, as the host is.
    fn word(&self, index: usize) -> u32 {
        let at = index * 4;
        u32::from_le_bytes([self.0[at], self.0[at + 1], self.0[at + 2], self.0[at + 3]])
    }

    fn set_word(&mut self, index: usize, value: u32) {
        let at = index * 4;
        self.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
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
/// local APIC; where the partition's local-APIC emulation mode is `None` there
/// is no APIC to accept it.
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
    /// The guest wrote one of the APIC registers the partition asked to trap.
    /// The payload is `WHV_X64_APIC_WRITE_CONTEXT`: which register, and the
    /// value written — without both, the exit says only that something
    /// happened and the host cannot mirror the write into its own model.
    ApicWriteTrap { register: ApicWriteType, value: u64 },
    /// The host called `WHvCancelRunVirtualProcessor`.
    Canceled { reason: i32 },
    /// An exit reason the SDK has grown since this port was written.
    Unrecognized(i32),
}

impl ExitReason {
    /// A short, stable name for this reason.
    ///
    /// What a host puts in a message when it has no service for an exit. The
    /// name has to outlive the exit — a fault carries a `&'static str` — so it
    /// is a constant here rather than a formatted payload, and the payload each
    /// variant carries is deliberately left out: the reason is what the reader
    /// needs to know which arm was missing.
    ///
    /// Exhaustive (R5), so a variant added above cannot reach a caller
    /// unnamed.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "X64None",
            Self::MemoryAccess(_) => "X64MemoryAccess",
            Self::IoPortAccess(_) => "X64IoPortAccess",
            Self::UnrecoverableException => "X64UnrecoverableException",
            Self::InvalidVpRegisterValue => "X64InvalidVpRegisterValue",
            Self::UnsupportedFeature { .. } => "X64UnsupportedFeature",
            Self::InterruptWindow => "X64InterruptWindow",
            Self::Halt => "X64Halt",
            Self::ApicEoi { .. } => "X64ApicEoi",
            Self::SynicSintDeliverable => "SynicSintDeliverable",
            Self::MsrAccess(_) => "X64MsrAccess",
            Self::Cpuid(_) => "X64Cpuid",
            Self::Exception => "X64Exception",
            Self::Rdtsc => "X64Rdtsc",
            Self::ApicSmiTrap => "X64ApicSmiTrap",
            Self::Hypercall => "Hypercall",
            Self::ApicInitSipiTrap => "X64ApicInitSipiTrap",
            Self::ApicWriteTrap { .. } => "X64ApicWriteTrap",
            Self::Canceled { .. } => "Canceled",
            // A platform identifier like every other arm, not a sentence: the
            // caller puts this in an `EngineFault`'s `at`, beside names it can
            // look up in the SDK, and a phrase there reads as prose in a field
            // of identifiers. The code the platform gave is carried separately.
            Self::Unrecognized(_) => "WHvRunVpExitReasonUnrecognized",
        }
    }
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

/// The registers this port names that a whole-state exchange does NOT carry.
///
/// [`Reg::PendingEvent`] is 128 bits wide, and a word transfer would keep its
/// low half and drop the rest; [`Reg::ApicTpr`] belongs to an APIC the
/// hypervisor may be emulating, so whether it is even writable is the
/// platform's answer rather than this port's assumption. Both are reached one
/// at a time, deliberately.
pub const UNEXCHANGED_REGS: &[Reg] = &[Reg::PendingEvent, Reg::ApicTpr];

/// Every register this port exchanges, in one list.
///
/// The list a state exchange iterates, and the one the tests below check the
/// shape classification against. A variant added to [`Reg`] and forgotten here
/// is caught by `every_register_is_in_the_exchange_list`, which matches on each
/// variant exhaustively so the compiler refuses to let the two drift; what does
/// not belong in a word transfer goes in [`UNEXCHANGED_REGS`] instead.
pub const ALL_REGS: &[Reg] = &[
    Reg::Rax, Reg::Rbx, Reg::Rcx, Reg::Rdx, Reg::Rsi, Reg::Rdi, Reg::Rsp, Reg::Rbp,
    Reg::R8, Reg::R9, Reg::R10, Reg::R11, Reg::R12, Reg::R13, Reg::R14, Reg::R15,
    Reg::Rip, Reg::Rflags,
    Reg::Cs, Reg::Ds, Reg::Es, Reg::Ss, Reg::Fs, Reg::Gs, Reg::Ldtr, Reg::Tr,
    Reg::Gdtr, Reg::Idtr,
    Reg::Cr0, Reg::Cr2, Reg::Cr3, Reg::Cr4, Reg::Cr8,
    Reg::Dr0, Reg::Dr1, Reg::Dr2, Reg::Dr3, Reg::Dr6, Reg::Dr7,
    Reg::Efer, Reg::KernelGsBase, Reg::Star, Reg::Lstar, Reg::Cstar, Reg::Sfmask,
    Reg::SysenterCs, Reg::SysenterEsp, Reg::SysenterEip,
    Reg::Pat, Reg::ApicBase, Reg::Tsc, Reg::Xcr0,
    Reg::PendingInterruption, Reg::InterruptState, Reg::InternalActivityState,
    Reg::DeliverabilityNotifications,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// [`ALL_REGS`] and [`UNEXCHANGED_REGS`] between them hold every variant,
    /// enforced by a match the compiler makes exhaustive rather than by a count
    /// someone has to remember to bump.
    #[test]
    fn every_register_is_in_the_exchange_list() {
        for reg in ALL_REGS.iter().chain(UNEXCHANGED_REGS) {
            // Naming each variant is the assertion: a new one fails to compile
            // here, and the fix is to add it to the list above as well.
            match reg {
                Reg::Rax | Reg::Rbx | Reg::Rcx | Reg::Rdx | Reg::Rsi | Reg::Rdi | Reg::Rsp
                | Reg::Rbp | Reg::R8 | Reg::R9 | Reg::R10 | Reg::R11 | Reg::R12 | Reg::R13
                | Reg::R14 | Reg::R15 | Reg::Rip | Reg::Rflags => {}
                Reg::Cs | Reg::Ds | Reg::Es | Reg::Ss | Reg::Fs | Reg::Gs | Reg::Ldtr
                | Reg::Tr => assert!(reg.is_segment(), "{reg:?} is segment-shaped"),
                Reg::Gdtr | Reg::Idtr => assert!(reg.is_table(), "{reg:?} is table-shaped"),
                Reg::Cr0 | Reg::Cr2 | Reg::Cr3 | Reg::Cr4 | Reg::Cr8 => {}
                Reg::Dr0 | Reg::Dr1 | Reg::Dr2 | Reg::Dr3 | Reg::Dr6 | Reg::Dr7 => {}
                Reg::Efer | Reg::KernelGsBase | Reg::Star | Reg::Lstar | Reg::Cstar
                | Reg::Sfmask | Reg::SysenterCs | Reg::SysenterEsp | Reg::SysenterEip
                | Reg::Pat | Reg::ApicBase | Reg::Tsc | Reg::Xcr0 => {}
                Reg::PendingInterruption
                | Reg::InterruptState
                | Reg::InternalActivityState
                | Reg::DeliverabilityNotifications => {}
                Reg::PendingEvent | Reg::ApicTpr => assert!(
                    !ALL_REGS.contains(reg),
                    "{reg:?} is reached one at a time, not through the whole-state exchange"
                ),
            }
        }
    }

    /// The invariant that keeps the per-slice exchange a word transfer: a
    /// 128-bit register in [`ALL_REGS`] would be read as its low half and
    /// written back with its high half zeroed, losing state silently rather
    /// than failing.
    #[test]
    fn the_exchange_list_holds_no_register_a_word_transfer_would_truncate() {
        for reg in ALL_REGS {
            assert!(
                !reg.is_words128(),
                "{reg:?} is 128 bits wide; a word exchange would drop half of it"
            );
        }
    }

    /// A register is word-shaped, segment-shaped or table-shaped, and never two
    /// of those — reading one as the wrong member of the platform's value union
    /// returns a base and silently drops everything else.
    #[test]
    fn a_register_has_exactly_one_shape() {
        for reg in ALL_REGS {
            assert!(
                !(reg.is_segment() && reg.is_table()),
                "{reg:?} claims two shapes"
            );
        }
        assert_eq!(
            ALL_REGS.iter().filter(|reg| reg.is_segment()).count(),
            8,
            "six segments plus LDTR and TR"
        );
        assert_eq!(ALL_REGS.iter().filter(|reg| reg.is_table()).count(), 2);
    }

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
    fn a_pending_ext_int_event_encodes_as_the_header_lays_it_out() {
        let event = PendingExtIntEvent { vector: 0x20 };
        assert_eq!(event.as_words(), [0x0000_0000_0000_200B, 0]); // pending | type 5 << 1 | 0x20 << 8
        assert_eq!(PendingExtIntEvent::from_words([0x200B, 0]), Some(event));
        assert_eq!(PendingExtIntEvent::from_words([0, 0]), None, "an empty slot is no event");
        assert_eq!(
            PendingExtIntEvent::from_words([0x2001, 0]),
            None,
            "an exception event is not an ExtInt"
        );
        // `EventType` is three bits, so bit 4 belongs to `Reserved0`. A slot
        // that carries it still holds this port's ExtINT, and reading the
        // reserved bit as part of the type would report the injection gone.
        assert_eq!(
            PendingExtIntEvent::from_words([0x200B | 1 << 4, 0]),
            Some(event),
            "a reserved bit above the type field does not hide the event"
        );
    }

    /// The page is a PACKED structure, and the byte offsets asserted here are
    /// the ones the platform itself produced — see [`ApicStatePage`]'s
    /// provenance note. Written as byte offsets rather than as a round trip
    /// through the accessors so that a field order shifted by one is caught:
    /// a round trip would agree with itself whatever the order was.
    #[test]
    fn the_apic_state_page_packs_its_registers_where_the_platform_puts_them() {
        let mut page = ApicStatePage::zeroed();
        page.set_register(ApicRegister::Version, 0x0005_0014);
        assert_eq!(
            &page.0[0x04..0x08],
            &0x0005_0014u32.to_le_bytes(),
            "the version is the second word, at byte 4 — no padding precedes it"
        );
        page.set_register(ApicRegister::LvtLint0, 0x0001_0000);
        assert_eq!(
            &page.0[0x8C..0x90],
            &0x0001_0000u32.to_le_bytes(),
            "LINT0 is word 35: five scalars and three eight-word bitmaps and the ICR precede it"
        );
        assert_eq!(
            page.register(ApicRegister::Version),
            0x0005_0014,
            "a neighbouring field does not disturb it"
        );
        assert_eq!(page.register(ApicRegister::LvtTimer), 0, "nor does it bleed backwards");
        assert_eq!(page.register(ApicRegister::LvtLint1), 0, "nor forwards");
    }

    /// The three bitmaps are what push the LVT entries as far into the page as
    /// they are, so their extent is the load-bearing part of the layout.
    #[test]
    fn the_three_interrupt_bitmaps_fill_the_words_between_the_svr_and_the_esr() {
        let mut page = ApicStatePage::zeroed();
        let bits = [1, 2, 3, 4, 5, 6, 7, 8];
        page.set_vector(ApicVector::Request, bits);
        assert_eq!(page.vector(ApicVector::Request), bits);
        assert_eq!(&page.0[0x54..0x58], &1u32.to_le_bytes(), "the request bitmap starts at word 21");
        assert_eq!(page.vector(ApicVector::InService), [0; 8], "the other two are untouched");
        assert_eq!(page.vector(ApicVector::TriggerMode), [0; 8]);
        assert_eq!(page.register(ApicRegister::Esr), 0, "and so is the field after them");

        // The three run back to back, so the word after the last one is the ESR
        // and the word before the first is the SVR.
        page.set_register(ApicRegister::Spurious, 0xFF);
        page.set_register(ApicRegister::Esr, 0xDEAD);
        page.set_vector(ApicVector::InService, [u32::MAX; 8]);
        page.set_vector(ApicVector::TriggerMode, [u32::MAX; 8]);
        page.set_vector(ApicVector::Request, [u32::MAX; 8]);
        assert_eq!(page.register(ApicRegister::Spurious), 0xFF);
        assert_eq!(page.register(ApicRegister::Esr), 0xDEAD);
    }

    /// A trapped APIC write names its register two ways, and only one of them
    /// indexes the state page.
    #[test]
    fn an_apic_write_traps_offset_and_its_state_page_field_are_different_numbers() {
        assert_eq!(ApicWriteType::Lint0.mmio_offset(), 0x350);
        assert_eq!(ApicWriteType::Lint0.register(), ApicRegister::LvtLint0);
        assert_eq!(ApicWriteType::Lint0.register().word(), 35);
        assert_eq!(ApicWriteType::Svr.mmio_offset(), 0x0F0);
        assert_eq!(
            ApicWriteType::Svr.register(),
            ApicRegister::Spurious,
            "the platform calls it SVR and the state structure calls it the spurious vector"
        );
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
