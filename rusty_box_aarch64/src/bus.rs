//! The guest-physical address space a processor reaches, and the units that
//! name it.
//!
//! A physical address is not a virtual address (R4): the MMU is the only thing
//! that turns one into the other, and these are distinct types so that a
//! device can never be handed an address the MMU has not translated.

use rusty_box_core::VmClock;

/// A guest-physical address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysAddr(u64);

impl PhysAddr {
    #[inline]
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The offset of this address into a window starting at `base`, if it is
    /// inside a window of `len` bytes, the window ending at or below 2^64.
    ///
    /// One unsigned compare decides it: an address below `base` wraps to an
    /// offset of more than 2^64 − `base`, which is at least `len`. Kept to
    /// one compare, a chain of windows stays a chain of single tests in
    /// each, whatever the windows' order.
    #[inline]
    pub const fn offset_in(self, base: u64, len: u64) -> Option<RegOffset> {
        let offset = self.0.wrapping_sub(base);
        if offset < len {
            Some(RegOffset(offset))
        } else {
            None
        }
    }
}

/// How far into a device's register window an access landed.
///
/// Distinct from [`PhysAddr`] (R4): a device decodes offsets, never absolute
/// addresses, so the same model works wherever a board places it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegOffset(u64);

impl RegOffset {
    #[inline]
    pub const fn new(offset: u64) -> Self {
        Self(offset)
    }

    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The byte width of one bus access.
///
/// Wider architectural accesses (a 128-bit Q register, a pair) are split by
/// the processor into accesses of these widths before they reach the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessWidth {
    Byte,
    Half,
    Word,
    Dword,
}

impl AccessWidth {
    /// The width whose size is `1 << log2` bytes, for `log2` in `0..=3`.
    #[inline]
    pub const fn from_log2(log2: u32) -> Option<Self> {
        match log2 {
            0 => Some(Self::Byte),
            1 => Some(Self::Half),
            2 => Some(Self::Word),
            3 => Some(Self::Dword),
            _ => None,
        }
    }

    /// The width an instruction's 2-bit `size` field names; the rest of
    /// `size` is not read.
    #[inline(always)]
    pub(crate) const fn from_size_field(size: u32) -> Self {
        match size & 3 {
            0 => Self::Byte,
            1 => Self::Half,
            2 => Self::Word,
            _ => Self::Dword,
        }
    }

    #[inline]
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
            Self::Dword => 8,
        }
    }

    #[inline]
    pub const fn log2(self) -> u32 {
        match self {
            Self::Byte => 0,
            Self::Half => 1,
            Self::Word => 2,
            Self::Dword => 3,
        }
    }

    /// The mask selecting the low `bytes()` bytes of a 64-bit value.
    #[inline]
    pub const fn value_mask(self) -> u64 {
        match self {
            Self::Byte => 0xFF,
            Self::Half => 0xFFFF,
            Self::Word => 0xFFFF_FFFF,
            Self::Dword => u64::MAX,
        }
    }
}

/// The little-endian value of a 1-, 2-, 4- or 8-byte slice.
#[inline(always)]
pub(crate) fn le_value(bytes: &[u8]) -> u64 {
    match bytes.len() {
        1 => u64::from(bytes[0]),
        2 => bytes
            .try_into()
            .map_or(0, |b| u64::from(u16::from_le_bytes(b))),
        4 => bytes
            .try_into()
            .map_or(0, |b| u64::from(u32::from_le_bytes(b))),
        _ => bytes.try_into().map_or(0, u64::from_le_bytes),
    }
}

/// The `width` bytes at `offset` in `bytes`, little-endian and zero-extended,
/// each width a fixed-size load. `None` when they do not all lie in `bytes`.
#[inline(always)]
pub(crate) fn load_le(bytes: &[u8], offset: usize, width: AccessWidth) -> Option<u64> {
    match width {
        AccessWidth::Byte => load_array(bytes, offset).map(|b| u64::from(u8::from_le_bytes(b))),
        AccessWidth::Half => load_array(bytes, offset).map(|b| u64::from(u16::from_le_bytes(b))),
        AccessWidth::Word => load_array(bytes, offset).map(|b| u64::from(u32::from_le_bytes(b))),
        AccessWidth::Dword => load_array(bytes, offset).map(u64::from_le_bytes),
    }
}

/// The `N` bytes at `offset` in `bytes`, or `None` when they do not all lie
/// there.
#[inline(always)]
fn load_array<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    let end = offset.checked_add(N)?;
    bytes.get(offset..end)?.try_into().ok()
}

/// A read of `width` at `offset` of a device with no register wider than a
/// word, `read` reading the register at an offset. An 8-byte access is two
/// word reads, the low one first: the PrimeCell TRMs leave it to the bus
/// bridge, and QEMU's `access_with_adjusted_size` makes it of an access to
/// a device that declares no access size.
#[inline]
pub(crate) fn read_by_words<F: FnMut(RegOffset) -> u64>(
    offset: RegOffset,
    width: AccessWidth,
    mut read: F,
) -> u64 {
    if width != AccessWidth::Dword {
        return read(offset);
    }
    let low = read(offset);
    let high = read(RegOffset::new(offset.get() + 4));
    low | (high << 32)
}

/// A write of `value`, as [`read_by_words`] reads: `write` writes the
/// register at an offset.
#[inline]
pub(crate) fn write_by_words<F: FnMut(RegOffset, u64)>(
    offset: RegOffset,
    width: AccessWidth,
    value: u64,
    mut write: F,
) {
    if width != AccessWidth::Dword {
        return write(offset, value);
    }
    write(offset, value & 0xFFFF_FFFF);
    write(RegOffset::new(offset.get() + 4), value >> 32);
}

/// Store the low `width` bytes of `value` little-endian at `offset` in
/// `bytes`, each width a fixed-size store. `None`, with nothing written,
/// when they do not all lie in `bytes`.
#[inline(always)]
pub(crate) fn store_le(
    bytes: &mut [u8],
    offset: usize,
    width: AccessWidth,
    value: u64,
) -> Option<()> {
    match width {
        AccessWidth::Byte => store_array(bytes, offset, (value as u8).to_le_bytes()),
        AccessWidth::Half => store_array(bytes, offset, (value as u16).to_le_bytes()),
        AccessWidth::Word => store_array(bytes, offset, (value as u32).to_le_bytes()),
        AccessWidth::Dword => store_array(bytes, offset, value.to_le_bytes()),
    }
}

/// Store `value` at `offset` in `bytes`, or `None`, with nothing written,
/// when it does not lie entirely in `bytes`.
#[inline(always)]
fn store_array<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) -> Option<()> {
    let end = offset.checked_add(N)?;
    let to: &mut [u8; N] = bytes.get_mut(offset..end)?.try_into().ok()?;
    *to = value;
    Some(())
}

/// Why the bus could not complete an access.
///
/// Closed on purpose (R5): the processor turns every variant into a
/// synchronous external abort, and a new variant must be decided there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusFault {
    /// Nothing decodes this address. QEMU's `virt` raises a synchronous
    /// external abort for an access to unassigned memory
    /// (`arm_cpu_do_transaction_failed`); so does this board.
    Unmapped,
}

/// Guest-physical bytes `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhysSpan {
    pub(crate) start: PhysAddr,
    pub(crate) end: PhysAddr,
}

impl PhysSpan {
    /// The `len` bytes at `start`; `None` for none, or for a span past the
    /// end of the address space.
    #[inline]
    pub(crate) fn new(start: PhysAddr, len: u64) -> Option<Self> {
        let end = start
            .get()
            .checked_add(len)
            .filter(|&end| end > start.get())?;
        Some(Self {
            start,
            end: PhysAddr::new(end),
        })
    }

    /// Its length in bytes.
    #[inline]
    pub(crate) const fn len(self) -> u64 {
        self.end.get() - self.start.get()
    }
}

/// How many spans [`Writes`] keeps apart.
const WRITE_SPANS: usize = 4;

/// Memory written that the processor has yet to account for: what
/// something other than the processor wrote — a device, by DMA — for the
/// processor to forget the code it decoded there (`Cpu::forget_code`), and
/// the descriptors the processor's own walks updated, which it notes as its
/// stores once the walks return. Spans beyond what it keeps apart merge
/// into their hull, which only ever covers more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Writes {
    spans: [PhysSpan; WRITE_SPANS],
    len: usize,
}

impl Writes {
    pub(crate) const NONE: Self = Self {
        spans: [PhysSpan {
            start: PhysAddr::new(0),
            end: PhysAddr::new(0),
        }; WRITE_SPANS],
        len: 0,
    };

    /// Note `len` bytes written at `start`.
    pub(crate) fn add(&mut self, start: PhysAddr, len: u64) {
        if let Some(written) = PhysSpan::new(start, len) {
            self.add_span(written);
        }
    }

    fn add_span(&mut self, written: PhysSpan) {
        for span in self.spans[..self.len].iter_mut() {
            if written.start <= span.end && span.start <= written.end {
                span.start = span.start.min(written.start);
                span.end = span.end.max(written.end);
                return;
            }
        }
        if self.len < WRITE_SPANS {
            self.spans[self.len] = written;
            self.len += 1;
        } else {
            let last = &mut self.spans[WRITE_SPANS - 1];
            last.start = last.start.min(written.start);
            last.end = last.end.max(written.end);
        }
    }

    /// Note everything `other` noted.
    pub(crate) fn merge(&mut self, other: &Self) {
        for &span in other.spans() {
            self.add_span(span);
        }
    }

    pub(crate) fn spans(&self) -> &[PhysSpan] {
        &self.spans[..self.len]
    }

    /// The spans noted, leaving none.
    pub(crate) fn take(&mut self) -> Self {
        core::mem::replace(self, Self::NONE)
    }
}

/// A count of the changes to what fetches from some memory read, other than
/// the writes a processor sees as stores: a processor keeps what it decoded
/// from that memory only while its epoch stays the same. 64 bits, so that a
/// count never comes round again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CodeEpoch(u64);

impl CodeEpoch {
    #[inline]
    pub const fn new(count: u64) -> Self {
        Self(count)
    }

    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Whether a processor may keep the instructions it decoded from an
/// address, or must fetch them again each time it executes them.
///
/// Closed (R5): a processor either caches or it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FetchSource {
    /// A fetch has no side effect, and the word it reads changes only by a
    /// store the processor makes itself, by a write its owner reports to it
    /// (a device's DMA, a host's), or with the epoch: RAM, and ROM in its
    /// read mode.
    Stable(CodeEpoch),
    /// Anything else: a fetch is a device access like any other.
    Volatile,
}

/// What a swap of a translation table descriptor found
/// ([`PhysBus::swap_descriptor`]).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorSwap {
    /// The descriptor was the one expected, and the new one replaced it.
    Swapped,
    /// The descriptor was another, and was left as it was.
    Changed,
}

/// Which of a group's active priority registers an `ICC_AP0R<n>_EL1` or
/// `ICC_AP1R<n>_EL1` names: `n` is 0 to 3 (IHI 0069G 12.2.1, 12.2.2).
///
/// Closed (R5): the four the encoding's `op2<1:0>` can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApIndex {
    Zero,
    One,
    Two,
    Three,
}

impl ApIndex {
    /// Every index, in order.
    pub const ALL: [Self; 4] = [Self::Zero, Self::One, Self::Two, Self::Three];

    /// `n`.
    pub const fn get(self) -> usize {
        match self {
            Self::Zero => 0,
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
        }
    }

    /// The index the two bits `n` name.
    pub const fn of(n: u32) -> Self {
        match n & 3 {
            0 => Self::Zero,
            1 => Self::One,
            2 => Self::Two,
            _ => Self::Three,
        }
    }
}

/// One of the GIC CPU interface's System registers, which the board's GIC
/// holds: what an `MRS` or `MSR` of a processor with FEAT_GICv3 reaches
/// through its bus ([`PhysBus::gic_register_read`],
/// [`PhysBus::gic_register_write`]), once the processor's checks of the
/// access let it through.
///
/// Closed (R5): the four register files of a GICv3 CPU interface with no
/// EL3 (IHI 0069G 12.2 to 12.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GicRegister {
    /// A register of the physical CPU interface, `ICC_*_EL1`.
    Physical(IccRegister),
    /// A register of the virtual CPU interface, `ICV_*_EL1`: what an
    /// `ICC_*_EL1` encoding reaches at EL1 while `HCR_EL2.FMO` (a Group 0
    /// register's) or `HCR_EL2.IMO` (a Group 1 register's; either, a common
    /// one's) virtualizes it.
    Virtual(IcvRegister),
    /// A virtual interface control register, `ICH_*_EL2`.
    Control(IchRegister),
    /// `ICC_SRE_EL2`, EL2's System register enable.
    SreEl2,
}

/// One of the GIC CPU interface's System registers of EL1 (`ICC_*_EL1`,
/// IHI 0069G 12.2).
///
/// Closed (R5): the registers a processor with no EL3 has, of a GICv3
/// without FEAT_GICv3_NMI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IccRegister {
    /// `ICC_PMR_EL1`, the priority mask.
    Pmr,
    /// `ICC_IAR0_EL1`, which acknowledges a Group 0 interrupt.
    Iar0,
    /// `ICC_EOIR0_EL1`, a Group 0 interrupt's end.
    Eoir0,
    /// `ICC_HPPIR0_EL1`, the highest pending Group 0 interrupt.
    Hppir0,
    /// `ICC_BPR0_EL1`, Group 0's binary point.
    Bpr0,
    /// `ICC_AP0R<n>_EL1`, Group 0's active priorities.
    Ap0r(ApIndex),
    /// `ICC_AP1R<n>_EL1`, Group 1's active priorities.
    Ap1r(ApIndex),
    /// `ICC_DIR_EL1`, which deactivates an interrupt.
    Dir,
    /// `ICC_RPR_EL1`, the running priority.
    Rpr,
    /// `ICC_SGI1R_EL1`, which generates an SGI.
    Sgi1r,
    /// `ICC_ASGI1R_EL1`, which generates an SGI of the other group.
    Asgi1r,
    /// `ICC_SGI0R_EL1`, which generates a Group 0 SGI.
    Sgi0r,
    /// `ICC_IAR1_EL1`, which acknowledges a Group 1 interrupt.
    Iar1,
    /// `ICC_EOIR1_EL1`, a Group 1 interrupt's end.
    Eoir1,
    /// `ICC_HPPIR1_EL1`, the highest pending Group 1 interrupt.
    Hppir1,
    /// `ICC_BPR1_EL1`, Group 1's binary point.
    Bpr1,
    /// `ICC_CTLR_EL1`, the CPU interface's control.
    Ctlr,
    /// `ICC_SRE_EL1`, the System register enable.
    Sre,
    /// `ICC_IGRPEN0_EL1`, Group 0's enable.
    Igrpen0,
    /// `ICC_IGRPEN1_EL1`, Group 1's enable.
    Igrpen1,
}

/// One of the virtual CPU interface's registers (`ICV_*_EL1`, IHI 0069G
/// 12.3), which share their encodings with the `ICC_*_EL1` registers of the
/// same name.
///
/// Closed (R5): the `ICC_*_EL1` registers that have a virtual form, which
/// are all but `ICC_SRE_EL1` and the SGI generation registers, of a GICv3
/// without FEAT_GICv3_NMI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IcvRegister {
    /// `ICV_PMR_EL1`, the virtual priority mask (`ICH_VMCR_EL2.VPMR`).
    Pmr,
    /// `ICV_IAR0_EL1`, which acknowledges a virtual Group 0 interrupt.
    Iar0,
    /// `ICV_EOIR0_EL1`, a virtual Group 0 interrupt's end.
    Eoir0,
    /// `ICV_HPPIR0_EL1`, the highest pending virtual Group 0 interrupt.
    Hppir0,
    /// `ICV_BPR0_EL1`, virtual Group 0's binary point.
    Bpr0,
    /// `ICV_AP0R<n>_EL1`, virtual Group 0's active priorities
    /// (`ICH_AP0R<n>_EL2`).
    Ap0r(ApIndex),
    /// `ICV_AP1R<n>_EL1`, virtual Group 1's active priorities
    /// (`ICH_AP1R<n>_EL2`).
    Ap1r(ApIndex),
    /// `ICV_DIR_EL1`, which deactivates a virtual interrupt.
    Dir,
    /// `ICV_RPR_EL1`, the virtual running priority.
    Rpr,
    /// `ICV_IAR1_EL1`, which acknowledges a virtual Group 1 interrupt.
    Iar1,
    /// `ICV_EOIR1_EL1`, a virtual Group 1 interrupt's end.
    Eoir1,
    /// `ICV_HPPIR1_EL1`, the highest pending virtual Group 1 interrupt.
    Hppir1,
    /// `ICV_BPR1_EL1`, virtual Group 1's binary point.
    Bpr1,
    /// `ICV_CTLR_EL1`, the virtual CPU interface's control.
    Ctlr,
    /// `ICV_IGRPEN0_EL1`, virtual Group 0's enable.
    Igrpen0,
    /// `ICV_IGRPEN1_EL1`, virtual Group 1's enable.
    Igrpen1,
}

/// Which List register an `ICH_LR<n>_EL2` names: `n` is 0 to 15 (IHI
/// 0069G 12.4.6), from the encoding's `CRm<0>:op2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LrIndex(u8);

impl LrIndex {
    /// The most List registers a CPU interface has (`ICH_VTR_EL2.ListRegs`
    /// 0b01111).
    pub const COUNT: usize = 16;

    /// The index the four bits `n` name.
    pub const fn of(n: u32) -> Self {
        Self((n & 0xF) as u8)
    }

    /// `n`.
    pub const fn get(self) -> usize {
        self.0 as usize
    }
}

/// One of the virtual interface control registers (`ICH_*_EL2`, IHI 0069G
/// 12.4).
///
/// Closed (R5): the registers of a GICv3 virtual interface, with no
/// GICv4.1 or GICv5 additions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IchRegister {
    /// `ICH_AP0R<n>_EL2`, virtual Group 0's active priorities.
    Ap0r(ApIndex),
    /// `ICH_AP1R<n>_EL2`, virtual Group 1's active priorities.
    Ap1r(ApIndex),
    /// `ICH_HCR_EL2`, the virtual interface's control.
    Hcr,
    /// `ICH_VTR_EL2`, what the virtual interface implements.
    Vtr,
    /// `ICH_MISR_EL2`, the maintenance interrupts asserted.
    Misr,
    /// `ICH_EISR_EL2`, the List registers with an EOI maintenance interrupt.
    Eisr,
    /// `ICH_ELRSR_EL2`, the empty List registers.
    Elrsr,
    /// `ICH_VMCR_EL2`, the virtual machine's view of its CPU interface.
    Vmcr,
    /// `ICH_LR<n>_EL2`, a List register.
    Lr(LrIndex),
}

/// One of the controls of EL2 that trap an EL1 access to the GIC CPU
/// interface's System registers to EL2 (IHI 0069G 12.4.5, 12.2.23).
///
/// Closed (R5): the controls of a CPU interface with no SEIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GicTrap {
    /// `ICH_HCR_EL2.TC`: the registers common to both groups.
    Common,
    /// `ICH_HCR_EL2.TALL0`: Group 0's registers.
    Group0,
    /// `ICH_HCR_EL2.TALL1`: Group 1's registers.
    Group1,
    /// `ICH_HCR_EL2.TDIR`: writes of `ICC_DIR_EL1` and `ICV_DIR_EL1`.
    Deactivate,
    /// `ICC_SRE_EL2.Enable` clear: `ICC_SRE_EL1`.
    SreEl1,
}

/// The state of the trap controls of [`GicTrap`] an EL1 access meets: the
/// trap bits of `ICH_HCR_EL2` and `ICC_SRE_EL2.Enable`
/// ([`PhysBus::gic_el1_traps`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GicEl1Traps {
    ich_hcr: u64,
    icc_sre_el2: u64,
}

/// `ICH_HCR_EL2`'s trap bits: `TC` (10), `TALL0` (11), `TALL1` (12) and
/// `TDIR` (14).
const ICH_HCR_TC: u64 = 1 << 10;
const ICH_HCR_TALL0: u64 = 1 << 11;
const ICH_HCR_TALL1: u64 = 1 << 12;
const ICH_HCR_TDIR: u64 = 1 << 14;
/// `ICC_SRE_EL2.Enable`.
const ICC_SRE_EL2_ENABLE: u64 = 1 << 3;

impl GicEl1Traps {
    /// The controls of a CPU interface whose `ICH_HCR_EL2` is `ich_hcr` and
    /// whose `ICC_SRE_EL2` is `icc_sre_el2`.
    pub const fn of(ich_hcr: u64, icc_sre_el2: u64) -> Self {
        Self {
            ich_hcr,
            icc_sre_el2,
        }
    }

    /// Whether `trap` traps.
    pub const fn traps(self, trap: GicTrap) -> bool {
        match trap {
            GicTrap::Common => self.ich_hcr & ICH_HCR_TC != 0,
            GicTrap::Group0 => self.ich_hcr & ICH_HCR_TALL0 != 0,
            GicTrap::Group1 => self.ich_hcr & ICH_HCR_TALL1 != 0,
            GicTrap::Deactivate => self.ich_hcr & ICH_HCR_TDIR != 0,
            GicTrap::SreEl1 => self.icc_sre_el2 & ICC_SRE_EL2_ENABLE == 0,
        }
    }
}

/// A bus with no GIC whose CPU interface has System registers: an access to
/// one of them there is UNDEFINED.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NoGicSystemRegisters;

/// The guest-physical address space as the processor sees it.
///
/// Static dispatch only (R8): a machine names its bus type. `clock` is the
/// emulated time of the access, which is all a device needs to answer a read
/// of a counter or arm a deadline.
///
/// The processor reaches the bus for three purposes — data, instruction
/// fetch, and translation table walks, which read descriptors and, with
/// FEAT_HAF, swap them — and names the latter two with their own methods.
/// They default to ordinary reads and writes, which is what the board does;
/// a bus that tells the purposes apart (an observer reporting data accesses
/// only) overrides them.
pub trait PhysBus {
    /// Read `width` bytes at `addr`, little-endian, zero-extended to 64 bits.
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, clock: VmClock)
        -> Result<u64, BusFault>;

    /// Write the low `width` bytes of `value` at `addr`, little-endian.
    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        clock: VmClock,
    ) -> Result<(), BusFault>;

    /// Fetch the instruction word at `addr`.
    #[inline]
    fn fetch(&mut self, addr: PhysAddr, clock: VmClock) -> Result<u32, BusFault> {
        self.read(addr, AccessWidth::Word, clock)
            .map(|word| word as u32)
    }

    /// Read the translation table descriptor at `addr`.
    #[inline]
    fn read_descriptor(&mut self, addr: PhysAddr, clock: VmClock) -> Result<u64, BusFault> {
        self.read(addr, AccessWidth::Dword, clock)
    }

    /// Replace the translation table descriptor at `addr` with `new` if it
    /// is `expected`, as one atomic read-modify-write (Arm
    /// `AArch64_MemSwapTableDesc`): the read, and where it found `expected`
    /// the write, each refused as the bus refuses any. Atomic by the
    /// borrow: the processor holds the bus exclusively for the whole swap,
    /// and every other writer of guest memory — a device's DMA, a host —
    /// reaches it only through the same exclusive borrow, between the
    /// processor's accesses.
    #[inline]
    fn swap_descriptor(
        &mut self,
        addr: PhysAddr,
        expected: u64,
        new: u64,
        clock: VmClock,
    ) -> Result<DescriptorSwap, BusFault> {
        if self.read_descriptor(addr, clock)? != expected {
            return Ok(DescriptorSwap::Changed);
        }
        self.write(addr, AccessWidth::Dword, new, clock)?;
        Ok(DescriptorSwap::Swapped)
    }

    /// Whether the instructions fetched from an address may be kept. The
    /// default, [`FetchSource::Volatile`], has every instruction fetched as
    /// it executes.
    #[inline]
    fn fetch_source(&self, _addr: PhysAddr) -> FetchSource {
        FetchSource::Volatile
    }

    /// Whether an access since its owner last cleared the mark reached
    /// something other than memory, and so may have changed what the
    /// processor looks at between instructions: its interrupt inputs, the
    /// next timed event, what fetches read. The default, `true`, has the
    /// processor look after every instruction.
    #[inline]
    fn devices_touched(&self) -> bool {
        true
    }

    /// An `MRS` of the GIC CPU interface's System register `reg`, which
    /// the access's checks let through: its value, with the read's side
    /// effect (an acknowledge). The default is a bus with no such GIC.
    ///
    /// # Errors
    ///
    /// [`NoGicSystemRegisters`] where the bus has no GIC with a System
    /// register interface.
    #[inline]
    fn gic_register_read(&mut self, _reg: GicRegister) -> Result<u64, NoGicSystemRegisters> {
        Err(NoGicSystemRegisters)
    }

    /// An `MSR` of `value` to the GIC CPU interface's System register
    /// `reg`, as [`PhysBus::gic_register_read`] reads.
    ///
    /// # Errors
    ///
    /// [`NoGicSystemRegisters`] where the bus has no GIC with a System
    /// register interface.
    #[inline]
    fn gic_register_write(
        &mut self,
        _reg: GicRegister,
        _value: u64,
    ) -> Result<(), NoGicSystemRegisters> {
        Err(NoGicSystemRegisters)
    }

    /// The GIC CPU interface's controls that trap an EL1 access to its
    /// System registers ([`GicEl1Traps`]), which the processor reads
    /// before it lets such an access through. The default is a bus with no
    /// such GIC.
    ///
    /// # Errors
    ///
    /// [`NoGicSystemRegisters`] where the bus has no GIC with a System
    /// register interface.
    #[inline]
    fn gic_el1_traps(&self) -> Result<GicEl1Traps, NoGicSystemRegisters> {
        Err(NoGicSystemRegisters)
    }
}

#[cfg(test)]
#[path = "../tests/unit/bus.rs"]
mod tests;
