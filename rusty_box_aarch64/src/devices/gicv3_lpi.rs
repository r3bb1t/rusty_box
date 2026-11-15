//! The GICv3 Redistributor's physical LPIs: `GICR_CTLR.EnableLPIs`,
//! `GICR_PROPBASER` and `GICR_PENDBASER`, the LPI Configuration table's
//! entries the Redistributor caches, and the LPIs' pending state.
//!
//! Provenance (R7): IHI 0069G 5.1 (LPIs, the LPI Configuration and Pending
//! tables), 12.11.2 (`GICR_CTLR`), 12.11.30 (`GICR_PENDBASER`), 12.11.31
//! (`GICR_PROPBASER`), and 2.2.2 for the INTID width. The choices the
//! specification leaves to an implementation:
//! - Fourteen bits of INTID ([`ID_BITS`]), the fewest a GIC with LPIs
//!   implements (2.2.2), so LPIs 8192 to 16383: 8192 LPIs, the fewest an
//!   implementation with LPIs supports (5.1), for one processor and the
//!   board's few devices. `GICD_TYPER.num_LPIs` is 0, the count following
//!   from `IDbits`. A `GICR_PROPBASER.IDbits` below 13 leaves every LPI out
//!   of range (12.11.31); one of 13 or more covers all of them.
//! - The cache (5.1.1 "Caching"): the Redistributor reads every entry of
//!   the LPI Configuration table as `EnableLPIs` goes from 0 to 1, reads an
//!   entry again at an ITS `INV` of an event that maps to it and every entry
//!   at an `INVALL` of a collection that targets it, and at no other time:
//!   a change software makes to the table takes effect once software
//!   invalidates it, and not before. With `EnableLPIs` 0 it caches nothing.
//! - The pending state lives in the Redistributor while `EnableLPIs` is 1
//!   (5.1.2: "maintained solely by the Redistributor"): it is read from the
//!   LPI Pending table as `EnableLPIs` goes from 0 to 1, unless
//!   `GICR_PENDBASER.PTZ` said the table is zero, and written back to it as
//!   `EnableLPIs` goes from 1 to 0, one bit per INTID from the table's
//!   second KiB (the bits of LPIs 8192 to 16383); the first KiB, whose use is
//!   IMPLEMENTATION DEFINED, the Redistributor neither reads nor writes.
//! - `EnableLPIs` can be cleared (`GICR_CTLR.CES` 1, as 12.11.2 recommends:
//!   an `EnableLPIs` that stays 1 once set is deprecated), and every write
//!   completes as it is made, so `GICR_CTLR.RWP` reads 0.
//! - A write to `GICR_PROPBASER` or `GICR_PENDBASER` while `EnableLPIs` is 1
//!   (UNPREDICTABLE, 5.1) is ignored. Their cacheability and shareability
//!   fields are programmable (a fixed value is deprecated) and, like the
//!   addresses, change nothing of how the board reaches its RAM; every
//!   address bit to bit 51 is writable.
//! - A table, or the part of one, outside RAM reads as zeros and takes no
//!   writes, as the board's devices reach RAM alone.
//! - No direct LPIs (`GICR_TYPER.DirectLPI` 0, `GICR_CTLR.IR` 0): with an
//!   ITS, `GICR_SETLPIR`, `GICR_CLRLPIR`, `GICR_INVLPIR`, `GICR_INVALLR` and
//!   `GICR_SYNCR` are not implemented (5.1), so they are RAZ/WI.
//!
//! LPIs are Group 1 (5.1, `GICD_CTLR.DS` 1) and edge-triggered, and have no
//! active state: acknowledging one clears its pending state, and an end of
//! interrupt drops the running priority without deactivating anything
//! (4.1, 12.20 `ICC_EOIR1_EL1`).

use rusty_box_core::SnapError;
use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use crate::devices::virtio::queue::GuestRam;

/// The first LPI's INTID (2.2.1).
pub(crate) const LPI_BASE: u32 = 8192;
/// The bits of INTID the Distributor and the Redistributor implement.
pub(crate) const ID_BITS: u32 = 14;
/// The LPIs those bits number, from [`LPI_BASE`].
pub(crate) const LPI_COUNT: usize = (1 << ID_BITS) - LPI_BASE as usize;
/// The words of a bitmap of one bit per LPI.
const LPI_WORDS: usize = LPI_COUNT / 32;
/// The words of a bitmap of one bit per word of the pending bitmap.
const SUMMARY_WORDS: usize = LPI_WORDS / 32;

/// `GICR_CTLR.EnableLPIs` and `CES`.
const GICR_CTLR_ENABLE_LPIS: u32 = 1 << 0;
const GICR_CTLR_CES: u32 = 1 << 1;
/// `GICR_PROPBASER`'s fields: `OuterCache` [58:56], `Physical_Address`
/// [51:12], `Shareability` [11:10], `InnerCache` [9:7] and `IDbits` [4:0].
const PROPBASER_FIELDS: u64 = 0x070F_FFFF_FFFF_FF9F;
const PROPBASER_ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;
const PROPBASER_ID_BITS: u64 = 0x1F;
/// `GICR_PENDBASER`'s fields: `PTZ` [62], `OuterCache` [58:56],
/// `Physical_Address` [51:16], `Shareability` [11:10] and `InnerCache`
/// [9:7].
const PENDBASER_FIELDS: u64 = 0x470F_FFFF_FFFF_0F80;
const PENDBASER_ADDRESS: u64 = 0x000F_FFFF_FFFF_0000;
/// `GICR_PENDBASER.PTZ`, write-only: the LPI Pending table is zero.
const PENDBASER_PTZ: u64 = 1 << 62;
/// The smallest `GICR_PROPBASER.IDbits` that covers an LPI (12.11.31): 14
/// bits, minus one.
const PROPBASER_MIN_ID_BITS: u64 = 13;
/// Where the pending bits of LPIs start in the LPI Pending table: the
/// byte of INTID 8192's bit.
const PENDING_LPI_BYTES: u64 = LPI_BASE as u64 / 8;
/// An LPI Configuration table entry's `Enable` bit and `Priority` field
/// (Table 5-1).
const CONFIG_ENABLE: u8 = 1 << 0;
const CONFIG_PRIORITY: u8 = 0xFC;
/// `GICR_PROPBASER` and `GICR_PENDBASER`, from `RD_base`.
pub(super) const GICR_PROPBASER: u64 = 0x0070;
pub(super) const GICR_PENDBASER: u64 = 0x0078;

/// Whether the Redistributor's LPIs are enabled (`GICR_CTLR.EnableLPIs`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LpiSupport {
    /// LPIs never become pending, and nothing is cached.
    Disabled,
    Enabled,
}

/// An LPI the Redistributor would forward: its INTID and its priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LpiCandidate {
    pub(crate) intid: u32,
    pub(crate) priority: u8,
}

/// The Redistributor's LPI state.
#[derive(Clone)]
pub(crate) struct Lpis {
    support: LpiSupport,
    /// `GICR_PROPBASER`, its fields as written.
    propbaser: u64,
    /// `GICR_PENDBASER`, its fields as written, `PTZ` among them though it
    /// reads as 0.
    pendbaser: u64,
    /// The LPI Configuration table's entries as the Redistributor last read
    /// them, LPI `8192 + n`'s at `n`.
    config: [u8; LPI_COUNT],
    /// The LPIs' pending state, LPI `8192 + n` bit `n % 32` of word
    /// `n / 32`.
    pending: [u32; LPI_WORDS],
    /// Bit `w % 32` of word `w / 32` set while word `w` of
    /// [`Lpis::pending`] has a bit set: what makes finding a pending LPI
    /// cost little while none is.
    summary: [u32; SUMMARY_WORDS],
}

impl Lpis {
    /// Out of reset: LPIs disabled, nothing cached or pending, and the base
    /// registers, whose reset values are UNKNOWN, zero.
    pub(crate) const fn new() -> Self {
        Self {
            support: LpiSupport::Disabled,
            propbaser: 0,
            pendbaser: 0,
            config: [0; LPI_COUNT],
            pending: [0; LPI_WORDS],
            summary: [0; SUMMARY_WORDS],
        }
    }

    /// The index of LPI `intid`, if it is one of the LPIs the GIC has.
    pub(crate) const fn index(intid: u32) -> Option<usize> {
        match intid.checked_sub(LPI_BASE) {
            Some(n) if (n as usize) < LPI_COUNT => Some(n as usize),
            Some(_) | None => None,
        }
    }

    /// Whether INTID `intid` is an LPI's, implemented or not.
    pub(crate) const fn is_lpi(intid: u32) -> bool {
        intid >= LPI_BASE
    }

    /// Whether `GICR_PROPBASER.IDbits` covers the LPIs: all of them or none.
    fn in_range(&self) -> bool {
        self.propbaser & PROPBASER_ID_BITS >= PROPBASER_MIN_ID_BITS
    }

    /// The index of LPI `intid`, if LPIs are enabled and it is one in range.
    fn live(&self, intid: u32) -> Option<usize> {
        match self.support {
            LpiSupport::Enabled if self.in_range() => Self::index(intid),
            LpiSupport::Enabled | LpiSupport::Disabled => None,
        }
    }

    /// `GICR_CTLR`: `EnableLPIs`, and `CES` set; `RWP`, `IR`, `UWP` and the
    /// `DPG` bits 0.
    pub(crate) fn ctlr(&self) -> u32 {
        GICR_CTLR_CES
            | match self.support {
                LpiSupport::Enabled => GICR_CTLR_ENABLE_LPIS,
                LpiSupport::Disabled => 0,
            }
    }

    /// A write of `v` to `GICR_CTLR`: `EnableLPIs` from 0 to 1 loads the
    /// tables, from 1 to 0 writes the pending state back and forgets
    /// everything cached.
    pub(crate) fn write_ctlr(&mut self, ram: &mut GuestRam<'_>, v: u32) {
        match (self.support, v & GICR_CTLR_ENABLE_LPIS != 0) {
            (LpiSupport::Disabled, true) => {
                self.support = LpiSupport::Enabled;
                self.load(ram);
            }
            (LpiSupport::Enabled, false) => {
                self.write_back(ram);
                self.support = LpiSupport::Disabled;
                self.config = [0; LPI_COUNT];
                self.pending = [0; LPI_WORDS];
                self.summary = [0; SUMMARY_WORDS];
            }
            (LpiSupport::Disabled, false) | (LpiSupport::Enabled, true) => {}
        }
    }

    /// Read the LPI Configuration table, and the LPI Pending table unless
    /// `PTZ` said it is zero.
    fn load(&mut self, ram: &GuestRam<'_>) {
        self.pending = [0; LPI_WORDS];
        self.summary = [0; SUMMARY_WORDS];
        if !self.in_range() {
            self.config = [0; LPI_COUNT];
            return;
        }
        read_or_zeros(ram, self.propbaser & PROPBASER_ADDRESS, &mut self.config);
        if self.pendbaser & PENDBASER_PTZ != 0 {
            return;
        }
        let mut bytes = [0u8; LPI_COUNT / 8];
        read_or_zeros(ram, self.pending_bits_address(), &mut bytes);
        for (w, chunk) in bytes.chunks_exact(4).enumerate() {
            let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            self.pending[w] = word;
            if word != 0 {
                self.summary[w / 32] |= 1 << (w % 32);
            }
        }
    }

    /// Write the pending state of the LPIs in range to the LPI Pending
    /// table.
    fn write_back(&self, ram: &mut GuestRam<'_>) {
        if !self.in_range() {
            return;
        }
        let mut bytes = [0u8; LPI_COUNT / 8];
        for (chunk, word) in bytes.chunks_exact_mut(4).zip(self.pending) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        ram.store(self.pending_bits_address(), &bytes);
    }

    /// Where the LPIs' bits of the LPI Pending table are.
    fn pending_bits_address(&self) -> u64 {
        (self.pendbaser & PENDBASER_ADDRESS) + PENDING_LPI_BYTES
    }

    /// Word `half` (0 the low, 1 the high) of `GICR_PROPBASER` (`base`
    /// [`GICR_PROPBASER`]) or `GICR_PENDBASER`; `PTZ` reads as 0.
    pub(crate) fn base_word(&self, base: u64, half: u64) -> u32 {
        let value = if base == GICR_PROPBASER {
            self.propbaser
        } else {
            self.pendbaser & !PENDBASER_PTZ
        };
        (value >> (32 * half)) as u32
    }

    /// A write of `v` to word `half` of `GICR_PROPBASER` (`base`
    /// [`GICR_PROPBASER`]) or `GICR_PENDBASER`, ignored while LPIs are
    /// enabled.
    pub(crate) fn write_base_word(&mut self, base: u64, half: u64, v: u32) {
        if self.support == LpiSupport::Enabled {
            return;
        }
        let shift = 32 * half;
        let (register, fields) = if base == GICR_PROPBASER {
            (&mut self.propbaser, PROPBASER_FIELDS)
        } else {
            (&mut self.pendbaser, PENDBASER_FIELDS)
        };
        let kept = *register & !(0xFFFF_FFFF << shift);
        *register = (kept | u64::from(v) << shift) & fields;
    }

    /// Make LPI `intid` pending, if LPIs are enabled and it is in range;
    /// otherwise it is lost (5.1).
    pub(crate) fn set_pending(&mut self, intid: u32) {
        if let Some(n) = self.live(intid) {
            self.pending[n / 32] |= 1 << (n % 32);
            self.summary[n / 1024] |= 1 << ((n / 32) % 32);
        }
    }

    /// Clear LPI `intid`'s pending state.
    pub(crate) fn clear_pending(&mut self, intid: u32) {
        if let Some(n) = Self::index(intid) {
            let w = n / 32;
            self.pending[w] &= !(1 << (n % 32));
            if self.pending[w] == 0 {
                self.summary[w / 32] &= !(1 << (w % 32));
            }
        }
    }

    /// Whether LPI `intid` is pending.
    pub(crate) fn is_pending(&self, intid: u32) -> bool {
        Self::index(intid).is_some_and(|n| self.pending[n / 32] & (1 << (n % 32)) != 0)
    }

    /// Clear every LPI's pending state.
    pub(crate) fn clear_all_pending(&mut self) {
        self.pending = [0; LPI_WORDS];
        self.summary = [0; SUMMARY_WORDS];
    }

    /// Read LPI `intid`'s LPI Configuration table entry again, if the
    /// Redistributor caches it.
    pub(crate) fn invalidate(&mut self, ram: &GuestRam<'_>, intid: u32) {
        if let Some(n) = self.live(intid) {
            let address = (self.propbaser & PROPBASER_ADDRESS) + n as u64;
            read_or_zeros(ram, address, &mut self.config[n..n + 1]);
        }
    }

    /// Read every LPI Configuration table entry again, if the Redistributor
    /// caches them.
    pub(crate) fn invalidate_all(&mut self, ram: &GuestRam<'_>) {
        if self.support == LpiSupport::Enabled && self.in_range() {
            read_or_zeros(ram, self.propbaser & PROPBASER_ADDRESS, &mut self.config);
        }
    }

    /// LPI `intid`'s priority, of the bits `mask` implements: its cached
    /// configuration's `Priority`, bits [1:0] 0 (Table 5-1).
    pub(crate) fn priority(&self, intid: u32, mask: u8) -> u8 {
        Self::index(intid).map_or(0, |n| self.config[n] & CONFIG_PRIORITY & mask)
    }

    /// The pending, enabled LPI of the numerically lowest priority (of the
    /// bits `mask` implements), the lowest INTID among equals.
    pub(crate) fn best(&self, mask: u8) -> Option<LpiCandidate> {
        let mut best: Option<LpiCandidate> = None;
        for (s, &summary) in self.summary.iter().enumerate() {
            let mut words = summary;
            while words != 0 {
                let w = s * 32 + words.trailing_zeros() as usize;
                words &= words - 1;
                let mut bits = self.pending[w];
                while bits != 0 {
                    let n = w * 32 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let config = self.config[n];
                    if config & CONFIG_ENABLE == 0 {
                        continue;
                    }
                    let priority = config & CONFIG_PRIORITY & mask;
                    if best.is_none_or(|b| priority < b.priority) {
                        best = Some(LpiCandidate {
                            intid: LPI_BASE + n as u32,
                            priority,
                        });
                    }
                }
            }
        }
        best
    }

    // ----- snapshots -----

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_bool(self.support == LpiSupport::Enabled)?;
        w.write_u64(self.propbaser)?;
        w.write_u64(self.pendbaser)?;
        w.write_bytes(&self.config)?;
        for word in self.pending {
            w.write_u32(word)?;
        }
        Ok(())
    }

    /// The state an image holds, refused where no Redistributor could be in
    /// it: a base register with a bit no field holds, or LPIs pending or
    /// configuration cached while LPIs are disabled.
    pub(crate) fn restore<R: SnapRead>(r: &mut R) -> SnapResult<Self> {
        let support = if r.read_bool()? {
            LpiSupport::Enabled
        } else {
            LpiSupport::Disabled
        };
        let propbaser = r.read_u64()?;
        let pendbaser = r.read_u64()?;
        let mut lpis = Self::new();
        r.read_bytes(&mut lpis.config)?;
        for word in lpis.pending.iter_mut() {
            *word = r.read_u32()?;
        }
        if propbaser & !PROPBASER_FIELDS != 0 || pendbaser & !PENDBASER_FIELDS != 0 {
            return Err(SnapError::Invalid(
                "GICv3 LPI table base register with a bit no field holds",
            ));
        }
        let holds_state =
            lpis.config.iter().any(|&c| c != 0) || lpis.pending.iter().any(|&w| w != 0);
        let empty = support == LpiSupport::Disabled
            || propbaser & PROPBASER_ID_BITS < PROPBASER_MIN_ID_BITS;
        if empty && holds_state {
            return Err(SnapError::Invalid(
                "GICv3 LPI state cached while no LPI is in range",
            ));
        }
        lpis.support = support;
        lpis.propbaser = propbaser;
        lpis.pendbaser = pendbaser;
        for (w, &word) in lpis.pending.iter().enumerate() {
            if word != 0 {
                lpis.summary[w / 32] |= 1 << (w % 32);
            }
        }
        Ok(lpis)
    }
}

/// `out.len()` bytes of guest memory at `address`, a byte outside RAM read
/// as zero.
pub(super) fn read_or_zeros(ram: &GuestRam<'_>, address: u64, out: &mut [u8]) {
    if ram.read(address, out).is_some() {
        return;
    }
    for (i, byte) in out.iter_mut().enumerate() {
        let mut one = [0u8; 1];
        *byte = match ram.read(address.wrapping_add(i as u64), &mut one) {
            Some(()) => one[0],
            None => 0,
        };
    }
}
