//! The x87 and vector file, crossing the seam as the architecture's own
//! XSAVE area.
//!
//! The named-register exchange in [`crate::state`] cannot carry this file:
//! `WHV_REGISTER_NAME` stops at the XMM names and offers no YMM, ZMM, opmask
//! or x87 stack register at all. The platform's one complete window onto it is
//! `WHvGetVirtualProcessorXsaveState` and its counterpart, whose currency is
//! the XSAVE area — legacy `FXSAVE` region, header, then extended components.
//!
//! It must cross because whole stretches of guest code run on the shadow
//! processor — a converted slice, a burst behind an MMIO storm, a single
//! trapped instruction — and any of them may touch this file. A register file
//! that only one side of the seam could see would be two register files, and a
//! guest whose string routines are SSE2 reads whichever one the slice boundary
//! left it.
//!
//! The area is patched in place and handed back, never rebuilt from scratch,
//! so every component this port does not model — measured live on this
//! platform: the compacted form, with the host's supervisor CET pair in it —
//! crosses a write-back exactly as the platform produced it. The probe that
//! pins the platform contract is `rusty_box_whp`'s
//! `the_extended_state_area_round_trips_and_a_legacy_patch_is_a_register_write`.
//!
//! The x87 instruction and data pointers travel in their 64-bit form (a full
//! `FIP`/`FDP`, no selectors), which is the form a 64-bit host's XSAVE area
//! holds; the selector halves of the 32-bit form are not carried, and the
//! shadow's are set to zero on a read-back.

use crate::state::VpRegisters;
use rusty_box::cpu::arch_state::{FpuState, VcpuArchState};
use rusty_box_whp::WhpResult;

/// Fixed legacy-region offsets, identical in the standard and the compacted
/// form: only extended components move between the two.
const FCW: usize = 0;
const FSW: usize = 2;
const FTW_ABRIDGED: usize = 4;
const FOP: usize = 6;
const FIP: usize = 8;
const FDP: usize = 16;
const MXCSR: usize = 24;
/// Eight stack slots of sixteen bytes each: an 80-bit extended double and six
/// bytes of padding.
const ST0: usize = 32;
/// Sixteen XMM slots of sixteen bytes each.
const XMM0: usize = 160;
/// The header: `XSTATE_BV` at 512, `XCOMP_BV` at 520.
const XSTATE_BV: usize = 512;
const XCOMP_BV: usize = 520;
/// Where the first extended component sits in the compacted form — also the
/// least an area can be and still carry its own header, which is the floor
/// `rusty_box_whp`'s `read_xsave` enforces before a length reaches this
/// module.
const EXTENDED: usize = 576;

/// The `XCOMP_BV` bit that names the compacted form.
const COMPACTED: u64 = 1 << 63;

/// The architectural component indices this port models.
const X87: u32 = 0;
const SSE: u32 = 1;
const AVX: u32 = 2;
const OPMASK: u32 = 5;
const ZMM_HI256: u32 = 6;
const HI16_ZMM: u32 = 7;

/// The x87 and SSE init values the architecture defines for a component whose
/// `XSTATE_BV` bit is clear (SDM vol. 1, XRSTOR).
const FCW_INIT: u16 = 0x037F;
const TAG_ALL_EMPTY: u16 = 0xFFFF;
const MXCSR_INIT: u32 = 0x1F80;

/// How many architectural components the layout table describes: 0 through 18
/// are defined today, and a component past the table simply never matches a
/// span, which fails safe — nonzero shadow state for it is refused rather than
/// dropped.
const COMPONENTS: usize = 19;

/// How one extended component is sized and placed on this host, exactly as
/// `CPUID.(0xD, i)` reports it. All zero for a component the host lacks.
#[derive(Clone, Copy, Default, Debug)]
struct ComponentInfo {
    size: u32,
    /// Where the component sits in the STANDARD form. The compacted form
    /// ignores this and walks instead.
    offset: u32,
    /// Whether the compacted form aligns this component to sixty-four bytes.
    align64: bool,
}

/// This host's extended-state layout, captured once when an engine starts —
/// from the processor itself, because the area's extended offsets are the
/// host's own and no fixed table can know them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HostComponents {
    info: [ComponentInfo; COMPONENTS],
}

impl HostComponents {
    #[cfg(target_arch = "x86_64")]
    pub(crate) fn of_this_host() -> Self {
        let mut info = [ComponentInfo::default(); COMPONENTS];
        for (index, slot) in info.iter_mut().enumerate().skip(2) {
            let leaf = core::arch::x86_64::__cpuid_count(0xD, index as u32);
            *slot = ComponentInfo {
                size: leaf.eax,
                offset: leaf.ebx,
                align64: leaf.ecx & 2 != 0,
            };
        }
        Self { info }
    }

    /// On a host that cannot ask `CPUID`, no extended component has a place —
    /// which is consistent, because no such host runs the platform either.
    #[cfg(not(target_arch = "x86_64"))]
    pub(crate) fn of_this_host() -> Self {
        Self { info: [ComponentInfo::default(); COMPONENTS] }
    }

    /// Where component `index` sits in an area of `len` bytes whose header
    /// says `xcomp_bv`, or `None` when that area does not carry it.
    fn span(&self, xcomp_bv: u64, len: usize, index: u32) -> Option<core::ops::Range<usize>> {
        let sought = self.info.get(index as usize)?;
        if sought.size == 0 {
            return None;
        }
        let start = if xcomp_bv & COMPACTED != 0 {
            // The compacted form: components sit in index order, only the
            // ones `XCOMP_BV` names, each optionally aligned to sixty-four
            // bytes. The walk needs every earlier component's size, which is
            // why the table covers them all.
            if xcomp_bv & (1u64 << index) == 0 {
                return None;
            }
            let mut at = EXTENDED;
            for earlier in 2..index {
                if xcomp_bv & (1u64 << earlier) == 0 {
                    continue;
                }
                let info = self.info.get(earlier as usize)?;
                if info.align64 {
                    at = at.checked_next_multiple_of(64)?;
                }
                at = at.checked_add(info.size as usize)?;
            }
            if sought.align64 {
                at = at.checked_next_multiple_of(64)?;
            }
            at
        } else {
            sought.offset as usize
        };
        let end = start.checked_add(sought.size as usize)?;
        (end <= len).then_some(start..end)
    }
}

/// A component this port's shadow holds real state for, in an area with no
/// place to put it. Refused rather than dropped: a write-back that silently
/// left the partition's file behind the shadow's would be the exact divergence
/// this module exists to prevent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct UncarriedComponent {
    pub(crate) index: u32,
}

/// Little-endian reads and writes against the area, bounds-checked so a short
/// or hostile buffer reads as zero and takes no write rather than panicking —
/// the library stays panic-free, and [`XsaveArea::read_from`] refuses an area
/// shorter than [`LEAST_AREA`] up front.
fn u16_at(area: &[u8], at: usize) -> u16 {
    match area.get(at..at + 2).and_then(|bytes| bytes.try_into().ok()) {
        Some(bytes) => u16::from_le_bytes(bytes),
        None => 0,
    }
}

fn u32_at(area: &[u8], at: usize) -> u32 {
    match area.get(at..at + 4).and_then(|bytes| bytes.try_into().ok()) {
        Some(bytes) => u32::from_le_bytes(bytes),
        None => 0,
    }
}

fn u64_at(area: &[u8], at: usize) -> u64 {
    match area.get(at..at + 8).and_then(|bytes| bytes.try_into().ok()) {
        Some(bytes) => u64::from_le_bytes(bytes),
        None => 0,
    }
}

fn put(area: &mut [u8], at: usize, bytes: &[u8]) {
    if let Some(slot) = area.get_mut(at..at + bytes.len()) {
        slot.copy_from_slice(bytes);
    }
}

/// `FXSAVE`'s abridged tag: one bit per stack slot, set when the full tag says
/// anything but empty. The same packing the port's own `FXSAVE` performs
/// (Bochs proc_ctrl.cc `fxsave`).
const fn abridged_tag(full: u16) -> u8 {
    let mut abridged: u8 = 0;
    let mut slot = 0;
    while slot < 8 {
        if (full >> (slot * 2)) & 3 != 3 {
            abridged |= 1 << slot;
        }
        slot += 1;
    }
    abridged
}

/// The full tag from the abridged form: valid for present, empty otherwise —
/// the same reading the port's own `FXRSTOR` takes (Bochs proc_ctrl.cc
/// `fxrstor`), and the tag self-corrects at the next x87 instruction.
const fn full_tag(abridged: u8) -> u16 {
    let mut full: u16 = 0;
    let mut slot = 0;
    while slot < 8 {
        if abridged & (1 << slot) == 0 {
            full |= 3 << (slot * 2);
        }
        slot += 1;
    }
    full
}

/// Whether two states disagree anywhere in the file the area carries — the
/// question that decides if a write-back can be skipped.
pub(crate) fn vector_file_differs(a: &VcpuArchState, b: &VcpuArchState) -> bool {
    a.fpu != b.fpu || a.vector != b.vector || a.opmask != b.opmask || a.mxcsr != b.mxcsr
}

/// The partition processor's extended-state area, as the platform last handed
/// it out.
pub(crate) struct XsaveArea {
    /// The platform's bytes, whole. Refreshed by [`XsaveArea::refresh_from`],
    /// patched by [`XsaveArea::patch`], and never built from anything else —
    /// which is what keeps the components this port does not model intact
    /// across a write-back.
    bytes: std::boxed::Box<[u8]>,
    /// How much of `bytes` the platform wrote last.
    len: usize,
    host: HostComponents,
}

/// More than any processor's XSAVE area today (this host's is 872 bytes), and
/// the platform reports how much it actually wrote.
const AREA_CAPACITY: usize = 4096;

impl XsaveArea {
    /// Read the processor's area for the first time.
    ///
    /// # Errors
    /// The platform's own refusal, or a contract refusal for an area too
    /// short to hold its own header — nothing after this constructor need
    /// doubt the header again.
    pub(crate) fn read_from(vp: &impl VpRegisters, host: HostComponents) -> WhpResult<Self> {
        let mut area = Self {
            bytes: std::vec![0u8; AREA_CAPACITY].into_boxed_slice(),
            len: 0,
            host,
        };
        area.refresh_from(vp)?;
        Ok(area)
    }

    /// Read the processor's area again, into the same buffer.
    ///
    /// # Errors
    /// As [`XsaveArea::read_from`]. The platform's own crate refuses an area
    /// too short to carry its own header, so a length stored here can index
    /// that header without doubting it.
    pub(crate) fn refresh_from(&mut self, vp: &impl VpRegisters) -> WhpResult<()> {
        self.len = vp.read_xsave(&mut self.bytes)?;
        Ok(())
    }

    /// Hand the area back to the processor.
    ///
    /// # Errors
    /// The platform's own refusal.
    pub(crate) fn write_to(&self, vp: &impl VpRegisters) -> WhpResult<()> {
        vp.write_xsave(self.area())
    }

    fn area(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }

    /// Copy the x87 and vector file out of the area into `state`, applying
    /// the architecture's init values for every component the area marks
    /// init or does not carry.
    pub(crate) fn fill(&self, state: &mut VcpuArchState) {
        let area = self.area();
        let xstate_bv = u64_at(area, XSTATE_BV);
        let xcomp_bv = u64_at(area, XCOMP_BV);

        if xstate_bv & (1 << X87) != 0 {
            state.fpu = FpuState {
                control_word: u16_at(area, FCW),
                status_word: u16_at(area, FSW),
                tag_word: full_tag(area.get(FTW_ABRIDGED).copied().unwrap_or(0)),
                opcode: u16_at(area, FOP),
                instruction_pointer: u64_at(area, FIP),
                data_pointer: u64_at(area, FDP),
                instruction_selector: 0,
                data_selector: 0,
                stack: core::array::from_fn(|slot| {
                    let at = ST0 + slot * 16;
                    (u64_at(area, at), u16_at(area, at + 8))
                }),
            };
        } else {
            state.fpu = FpuState {
                control_word: FCW_INIT,
                tag_word: TAG_ALL_EMPTY,
                ..FpuState::default()
            };
        }

        if xstate_bv & (1 << SSE) != 0 {
            state.mxcsr = u32_at(area, MXCSR);
            for (slot, reg) in state.vector.iter_mut().enumerate().take(16) {
                let at = XMM0 + slot * 16;
                if let (Some(low), Some(from)) =
                    (reg.get_mut(..16), area.get(at..at + 16))
                {
                    low.copy_from_slice(from);
                }
            }
        } else {
            state.mxcsr = MXCSR_INIT;
            for reg in state.vector.iter_mut().take(16) {
                if let Some(low) = reg.get_mut(..16) {
                    low.fill(0);
                }
            }
        }

        let avx = (xstate_bv & (1 << AVX) != 0)
            .then(|| self.host.span(xcomp_bv, area.len(), AVX))
            .flatten();
        for (slot, reg) in state.vector.iter_mut().enumerate().take(16) {
            let Some(high) = reg.get_mut(16..32) else { continue };
            match avx
                .as_ref()
                .and_then(|span| area.get(span.start + slot * 16..span.start + slot * 16 + 16))
            {
                Some(from) => high.copy_from_slice(from),
                None => high.fill(0),
            }
        }

        let opmask = (xstate_bv & (1 << OPMASK) != 0)
            .then(|| self.host.span(xcomp_bv, area.len(), OPMASK))
            .flatten();
        for (slot, mask) in state.opmask.iter_mut().enumerate() {
            *mask = match &opmask {
                Some(span) => u64_at(area, span.start + slot * 8),
                None => 0,
            };
        }

        let zmm_hi = (xstate_bv & (1 << ZMM_HI256) != 0)
            .then(|| self.host.span(xcomp_bv, area.len(), ZMM_HI256))
            .flatten();
        for (slot, reg) in state.vector.iter_mut().enumerate().take(16) {
            let Some(high) = reg.get_mut(32..64) else { continue };
            match zmm_hi
                .as_ref()
                .and_then(|span| area.get(span.start + slot * 32..span.start + slot * 32 + 32))
            {
                Some(from) => high.copy_from_slice(from),
                None => high.fill(0),
            }
        }

        let hi16 = (xstate_bv & (1 << HI16_ZMM) != 0)
            .then(|| self.host.span(xcomp_bv, area.len(), HI16_ZMM))
            .flatten();
        for (slot, reg) in state.vector.iter_mut().enumerate().skip(16) {
            let upper = slot - 16;
            match hi16
                .as_ref()
                .and_then(|span| area.get(span.start + upper * 64..span.start + upper * 64 + 64))
            {
                Some(from) => reg.copy_from_slice(from),
                None => reg.fill(0),
            }
        }
    }

    /// Write the shadow's x87 and vector file into the area, leaving every
    /// component this port does not model exactly as the platform produced
    /// it.
    ///
    /// The x87 and SSE components are always written and always marked live —
    /// their init values are ordinary values, and the platform accepts both
    /// marks whatever the guest's `XCR0` says (measured by the probe). An
    /// extended component is marked live only while the shadow holds nonzero
    /// state for it, because all-zero IS its init state and the mark is then
    /// a claim the area need not make.
    ///
    /// # Errors
    /// [`UncarriedComponent`] when the shadow holds nonzero state for a
    /// component the area has no place for.
    pub(crate) fn patch(&mut self, state: &VcpuArchState) -> Result<(), UncarriedComponent> {
        let len = self.len;
        let host = self.host;
        let xcomp_bv = u64_at(self.area(), XCOMP_BV);
        let mut xstate_bv = u64_at(self.area(), XSTATE_BV);
        let area = &mut self.bytes;

        put(area, FCW, &state.fpu.control_word.to_le_bytes());
        put(area, FSW, &state.fpu.status_word.to_le_bytes());
        put(area, FTW_ABRIDGED, &[abridged_tag(state.fpu.tag_word)]);
        put(area, FOP, &state.fpu.opcode.to_le_bytes());
        put(area, FIP, &state.fpu.instruction_pointer.to_le_bytes());
        put(area, FDP, &state.fpu.data_pointer.to_le_bytes());
        for (slot, (significand, sign_exp)) in state.fpu.stack.iter().enumerate() {
            let at = ST0 + slot * 16;
            put(area, at, &significand.to_le_bytes());
            put(area, at + 8, &sign_exp.to_le_bytes());
        }
        xstate_bv |= 1 << X87;

        put(area, MXCSR, &state.mxcsr.to_le_bytes());
        for (slot, reg) in state.vector.iter().enumerate().take(16) {
            if let Some(low) = reg.get(..16) {
                put(area, XMM0 + slot * 16, low);
            }
        }
        xstate_bv |= 1 << SSE;

        let mut extended = |index: u32,
                            live: bool,
                            write: &mut dyn FnMut(&mut [u8], core::ops::Range<usize>)|
         -> Result<u64, UncarriedComponent> {
            match host.span(xcomp_bv, len, index) {
                Some(span) if live => {
                    write(area, span);
                    Ok(1 << index)
                }
                Some(_) => Ok(0),
                None if live => Err(UncarriedComponent { index }),
                None => Ok(0),
            }
        };

        let ymm_high_live = state
            .vector
            .iter()
            .take(16)
            .any(|reg| reg.get(16..32).is_some_and(|high| high.iter().any(|byte| *byte != 0)));
        let mut live_bits = extended(AVX, ymm_high_live, &mut |area, span| {
            for (slot, reg) in state.vector.iter().enumerate().take(16) {
                if let Some(high) = reg.get(16..32) {
                    put(area, span.start + slot * 16, high);
                }
            }
        })?;

        let opmask_live = state.opmask.iter().any(|mask| *mask != 0);
        live_bits |= extended(OPMASK, opmask_live, &mut |area, span| {
            for (slot, mask) in state.opmask.iter().enumerate() {
                put(area, span.start + slot * 8, &mask.to_le_bytes());
            }
        })?;

        let zmm_high_live = state
            .vector
            .iter()
            .take(16)
            .any(|reg| reg.get(32..64).is_some_and(|high| high.iter().any(|byte| *byte != 0)));
        live_bits |= extended(ZMM_HI256, zmm_high_live, &mut |area, span| {
            for (slot, reg) in state.vector.iter().enumerate().take(16) {
                if let Some(high) = reg.get(32..64) {
                    put(area, span.start + slot * 32, high);
                }
            }
        })?;

        let hi16_live = state
            .vector
            .iter()
            .skip(16)
            .any(|reg| reg.iter().any(|byte| *byte != 0));
        live_bits |= extended(HI16_ZMM, hi16_live, &mut |area, span| {
            for (slot, reg) in state.vector.iter().enumerate().skip(16) {
                put(area, span.start + (slot - 16) * 64, reg);
            }
        })?;

        // A live mark for a component the shadow holds at init is withdrawn,
        // so the mask says exactly what the data says.
        for index in [AVX, OPMASK, ZMM_HI256, HI16_ZMM] {
            xstate_bv &= !(1u64 << index);
        }
        xstate_bv |= live_bits;
        put(area, XSTATE_BV, &xstate_bv.to_le_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layout like the one measured on this host: the compacted form with
    /// AVX at 576 and a supervisor pair behind it, 872 bytes in all.
    fn measured_host() -> HostComponents {
        let mut info = [ComponentInfo::default(); COMPONENTS];
        info[AVX as usize] = ComponentInfo { size: 256, offset: 576, align64: false };
        info[11] = ComponentInfo { size: 16, offset: 0, align64: false };
        info[12] = ComponentInfo { size: 24, offset: 0, align64: false };
        HostComponents { info }
    }

    const MEASURED_XCOMP: u64 = COMPACTED | (1 << 12) | (1 << 11) | 0b111;
    const MEASURED_LEN: usize = 872;

    fn area_of(xstate_bv: u64, xcomp_bv: u64) -> std::vec::Vec<u8> {
        let mut area = std::vec![0u8; MEASURED_LEN];
        area[XSTATE_BV..XSTATE_BV + 8].copy_from_slice(&xstate_bv.to_le_bytes());
        area[XCOMP_BV..XCOMP_BV + 8].copy_from_slice(&xcomp_bv.to_le_bytes());
        area
    }

    fn boxed(area: std::vec::Vec<u8>, host: HostComponents) -> XsaveArea {
        let len = area.len();
        XsaveArea { bytes: area.into_boxed_slice(), len, host }
    }

    /// The compacted walk lands AVX at 576 on the measured layout, and a
    /// component the mask omits has no span at all.
    #[test]
    fn the_compacted_walk_places_avx_where_the_platform_put_it() {
        let host = measured_host();
        assert_eq!(host.span(MEASURED_XCOMP, MEASURED_LEN, AVX), Some(576..832));
        assert_eq!(host.span(MEASURED_XCOMP & !(1 << AVX), MEASURED_LEN, AVX), None);
        assert_eq!(host.span(MEASURED_XCOMP, MEASURED_LEN, OPMASK), None);
        // Too short an area carries no component, whatever the mask claims.
        assert_eq!(host.span(MEASURED_XCOMP, 600, AVX), None);
    }

    /// What the shadow writes, the shadow reads back — through the area and
    /// its init-state conventions, in both marked and init forms.
    #[test]
    fn the_vector_file_round_trips_through_the_area() {
        let mut state = VcpuArchState::default();
        state.fpu.control_word = 0x027F;
        state.fpu.status_word = 0x3800;
        state.fpu.tag_word = 0xFFFD; // ST0 present-zero, the rest empty
        state.fpu.stack[0] = (0x8000_0000_0000_0000, 0x3FFF);
        state.mxcsr = 0x1FA0;
        state.vector[3][..16].copy_from_slice(&[0xA5; 16]);
        state.vector[7][16..32].copy_from_slice(&[0x5A; 16]);

        let mut area = boxed(area_of(0, MEASURED_XCOMP), measured_host());
        area.patch(&state).expect("every live component has a span");

        let mut read_back = VcpuArchState::default();
        area.fill(&mut read_back);

        assert_eq!(read_back.fpu.control_word, 0x027F);
        assert_eq!(read_back.fpu.status_word, 0x3800);
        // The abridged tag keeps presence, not kind: present-zero returns as
        // present-valid, exactly as the port's own FXRSTOR reads it.
        assert_eq!(read_back.fpu.tag_word & 3, 0);
        assert_eq!(read_back.fpu.stack[0], (0x8000_0000_0000_0000, 0x3FFF));
        assert_eq!(read_back.mxcsr, 0x1FA0);
        assert_eq!(read_back.vector[3][..16], [0xA5; 16]);
        assert_eq!(read_back.vector[7][16..32], [0x5A; 16]);
        assert_eq!(read_back.vector[7][..16], [0; 16]);
        assert_eq!(read_back.vector[3][16..32], [0; 16]);
    }

    /// An area marked all-init fills the shadow with the architecture's init
    /// values, not with zeros pretending to be a control word.
    #[test]
    fn an_init_area_fills_the_shadow_with_init_values() {
        let area = boxed(area_of(0, MEASURED_XCOMP), measured_host());
        let mut state = VcpuArchState::default();
        state.fpu.control_word = 0xDEAD;
        state.mxcsr = 0xDEAD_BEEF;
        state.vector[2] = [0xFF; 64];
        area.fill(&mut state);

        assert_eq!(state.fpu.control_word, FCW_INIT);
        assert_eq!(state.fpu.tag_word, TAG_ALL_EMPTY);
        assert_eq!(state.mxcsr, MXCSR_INIT);
        assert_eq!(state.vector[2], [0; 64]);
    }

    /// Nonzero shadow state for a component the area cannot hold is refused,
    /// never dropped.
    #[test]
    fn state_with_no_place_in_the_area_is_refused() {
        let mut state = VcpuArchState::default();
        state.opmask[1] = 0x1234;
        let mut area = boxed(area_of(0, MEASURED_XCOMP), measured_host());
        assert_eq!(
            area.patch(&state),
            Err(UncarriedComponent { index: OPMASK }),
            "an opmask this host cannot carry must be refused"
        );
    }

    /// A patch touches the components it carries and nothing else — the
    /// supervisor state behind them comes back byte for byte.
    #[test]
    fn a_patch_leaves_unmodelled_components_untouched() {
        let mut raw = area_of(1 << 11, MEASURED_XCOMP);
        raw[832..872].fill(0xC7); // the supervisor pair's bytes
        let mut area = boxed(raw, measured_host());

        let mut state = VcpuArchState::default();
        state.vector[0][..16].copy_from_slice(&[0x11; 16]);
        area.patch(&state).expect("a live XMM has a legacy slot");

        assert!(area.area()[832..872].iter().all(|byte| *byte == 0xC7));
        let xstate_bv = u64_at(area.area(), XSTATE_BV);
        assert_eq!(xstate_bv & (1 << 11), 1 << 11, "the supervisor mark survives");
        assert_eq!(xstate_bv & 0b11, 0b11, "x87 and SSE are marked live");
    }

    /// The tag word survives the abridged form in the one dimension it keeps:
    /// which slots hold anything.
    #[test]
    fn the_abridged_tag_keeps_presence_in_both_directions() {
        assert_eq!(abridged_tag(0xFFFF), 0);
        assert_eq!(abridged_tag(0x0000), 0xFF);
        assert_eq!(full_tag(0), TAG_ALL_EMPTY);
        assert_eq!(full_tag(0xFF), 0);
        assert_eq!(abridged_tag(full_tag(0b1010_0101)), 0b1010_0101);
    }
}
