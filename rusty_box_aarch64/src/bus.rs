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
    /// inside a window of `len` bytes.
    #[inline]
    pub const fn offset_in(self, base: u64, len: u64) -> Option<RegOffset> {
        if self.0 >= base && self.0 - base < len {
            Some(RegOffset(self.0 - base))
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

/// Memory something other than the processor wrote — a device, by DMA —
/// for the processor to forget the code it decoded there
/// (`Cpu::forget_code`). Spans beyond what it keeps apart merge into their
/// hull, which only ever covers more.
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

/// The guest-physical address space as the processor sees it.
///
/// Static dispatch only (R8): a machine names its bus type. `clock` is the
/// emulated time of the access, which is all a device needs to answer a read
/// of a counter or arm a deadline.
///
/// The processor reaches the bus for three purposes — data, instruction
/// fetch, and translation table walks — and names the latter two with their
/// own methods. Both default to an ordinary read, which is what the board
/// does; a bus that tells the purposes apart (an observer reporting data
/// accesses only) overrides them.
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
}

#[cfg(test)]
#[path = "../tests/unit/bus.rs"]
mod tests;
