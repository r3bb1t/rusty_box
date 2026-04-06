#![allow(private_interfaces, dead_code)]

//
// |---------------------------------------------|
// |             Segment Descriptor              |
// |---------------------------------------------|
// |33222222|2|2|2|2| 11 11 |1|11|1|11  |        |
// |10987654|3|2|1|0| 98 76 |5|43|2|1098|76543210|
// |--------|-|-|-|-|-------|-|--|-|----|--------|
// |Base    |G|D|L|A|Limit  |P|D |S|Type|Base    |
// |[31-24] | |/| |V|[19-16]| |P | |    |[23-16] |
// |        | |B| |L|       | |L | |    |        |
// |------------------------|--------------------|
// |       Base [15-0]      |    Limit [15-0]    |
// |------------------------|--------------------|
//

use crate::config::BxAddress;

#[derive(Debug, Default, Clone)]
pub(crate) struct BxSelector {
    /* bx_selector_t */
    pub(crate) value: u16, /* the 16bit value of the selector */
    /* the following fields are extracted from the value field in protected
    mode only.  They're used for sake of efficiency */
    pub(crate) index: u16, /* 13bit index extracted from value in protected mode */
    pub(crate) ti: u16,    /* table indicator bit extracted from value */
    pub(crate) rpl: u8,    /* RPL extracted from value */
}

pub(crate) struct Gate {
    param_count: u8, /* 5bits (0..31) #words/dword to copy from caller's
                      * stack to called procedure's stack. */
    dest_selector: u16,
    dest_offset: u32,
}

struct Taskgate {
    /* type 5: Task Gate Descriptor */
    tss_selector: u16, /* TSS segment selector */
}

//#[derive(Debug)]
//pub enum Descriptor {
//    Segment {
//        /// base address: 286=24bits, 386=32bits, long=64
//        base: BxAddress,
//        /// for efficiency, this contrived field is set to
//        ///  limit for byte granular, and
//        ///  `(limit << 12) | 0xfff` for page granular seg's
//        limit_scaled: u32,
//        /// granularity: 0=byte, 1=4K (page)
//        g: bool,
//        /// default size: 0=16bit, 1=32bit
//        d_b: bool,
//        /// long mode: 0=compat, 1=64 bit
//        l: bool,
//        ///  available for use by system
//        avl: bool, // available for use by system
//    },
//    Gate {
//        param_count: u8, // 5 bits (0..31) #words/dword to copy
//        dest_selector: u16,
//        dest_offset: u32,
//    },
//    TaskGate {
//        tss_selector: u16, // TSS segment selector
//    },
//}

#[derive(Clone, Copy)]
pub(crate) enum Descriptor {
    Segment(DescriptorSegment),
    Gate(DescriptorGate),
    TaskGate(DescriptorTaskGate),
}

#[derive(Clone, Copy)]
pub(crate) struct DescriptorSegment {
    /// base address: 286=24bits, 386=32bits, long=64
    pub(crate) base: BxAddress,
    /// for efficiency, this contrived field is set to
    ///  limit for byte granular, and
    ///  `(limit << 12) | 0xfff` for page granular seg's
    pub(crate) limit_scaled: u32,
    /// granularity: 0=byte, 1=4K (page)
    pub(crate) g: bool,
    /// default size: 0=16bit, 1=32bit
    pub(crate) d_b: bool,
    /// long mode: 0=compat, 1=64 bit
    pub(crate) l: bool,
    ///  available for use by system
    pub(crate) avl: bool, // available for use by system
}

#[derive(Clone, Copy)]
pub(super) struct DescriptorGate {
    pub(crate) param_count: u8, // 5 bits (0..31) #words/dword to copy
    pub(crate) dest_selector: u16,
    pub(crate) dest_offset: u32,
}

#[derive(Clone, Copy, Default)]
pub(super) struct DescriptorTaskGate {
    pub(super) tss_selector: u16, // TSS segment selector
}

impl Default for Descriptor {
    fn default() -> Self {
        Self::TaskGate(DescriptorTaskGate { tss_selector: 0 })
    }
}

impl Descriptor {
    // -- Segment accessors --
    // Caller contract: only call segment_* on Segment variants.
    // Returns default (0/false) for wrong variant.

    #[inline(always)]
    pub(crate) fn segment_base(&self) -> BxAddress {
        match self {
            Self::Segment(s) => s.base,
            _ => 0,
        }
    }
    #[inline(always)]
    pub(crate) fn set_segment_base(&mut self, val: BxAddress) {
        match self {
            Self::Segment(s) => s.base = val,
            _ => {}
        }
    }

    #[inline(always)]
    pub(crate) fn segment_limit_scaled(&self) -> u32 {
        match self {
            Self::Segment(s) => s.limit_scaled,
            _ => 0,
        }
    }
    #[inline(always)]
    pub(crate) fn set_segment_limit_scaled(&mut self, val: u32) {
        match self {
            Self::Segment(s) => s.limit_scaled = val,
            _ => {}
        }
    }

    #[inline(always)]
    pub(crate) fn segment_g(&self) -> bool {
        match self {
            Self::Segment(s) => s.g,
            _ => false,
        }
    }
    #[inline(always)]
    pub(crate) fn set_segment_g(&mut self, val: bool) {
        match self {
            Self::Segment(s) => s.g = val,
            _ => {}
        }
    }

    #[inline(always)]
    pub(crate) fn segment_d_b(&self) -> bool {
        match self {
            Self::Segment(s) => s.d_b,
            _ => false,
        }
    }
    #[inline(always)]
    pub(crate) fn set_segment_d_b(&mut self, val: bool) {
        match self {
            Self::Segment(s) => s.d_b = val,
            _ => {}
        }
    }

    #[inline(always)]
    pub(crate) fn segment_l(&self) -> bool {
        match self {
            Self::Segment(s) => s.l,
            _ => false,
        }
    }
    #[inline(always)]
    pub(crate) fn set_segment_l(&mut self, val: bool) {
        match self {
            Self::Segment(s) => s.l = val,
            _ => {}
        }
    }

    #[inline(always)]
    pub(crate) fn segment_avl(&self) -> bool {
        match self {
            Self::Segment(s) => s.avl,
            _ => false,
        }
    }
    #[inline(always)]
    pub(crate) fn set_segment_avl(&mut self, val: bool) {
        match self {
            Self::Segment(s) => s.avl = val,
            _ => {}
        }
    }

    // -- Gate accessors --
    // Caller contract: only call gate_* on Gate variants.

    #[inline(always)]
    pub(crate) fn gate_dest_offset(&self) -> u32 {
        match self {
            Self::Gate(g) => g.dest_offset,
            _ => 0,
        }
    }
    #[inline(always)]
    pub(crate) fn set_gate_dest_offset(&mut self, val: u32) {
        match self {
            Self::Gate(g) => g.dest_offset = val,
            _ => {}
        }
    }

    #[inline(always)]
    pub(crate) fn gate_dest_selector(&self) -> u16 {
        match self {
            Self::Gate(g) => g.dest_selector,
            _ => 0,
        }
    }
    #[inline(always)]
    pub(crate) fn set_gate_dest_selector(&mut self, val: u16) {
        match self {
            Self::Gate(g) => g.dest_selector = val,
            _ => {}
        }
    }

    #[inline(always)]
    pub(crate) fn gate_param_count(&self) -> u8 {
        match self {
            Self::Gate(g) => g.param_count,
            _ => 0,
        }
    }

    // -- TaskGate accessors --
    // Caller contract: only call task_gate_* on TaskGate variants.

    #[inline(always)]
    pub(crate) fn task_gate_tss_selector(&self) -> u16 {
        match self {
            Self::TaskGate(t) => t.tss_selector,
            _ => 0,
        }
    }
}

bitflags::bitflags! {
    /// Segment cache validity and access-permission flags.
    ///
    /// Cached in `BxDescriptor::valid` — set during segment load and
    /// checked on every memory access to avoid re-parsing the descriptor.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct SegAccess: u32 {
        /// Descriptor cache entry is populated and valid
        const VALID      = 0x01;
        /// Read access allowed (segment-limit check passed)
        const ROK        = 0x02;
        /// Write access allowed (segment-limit check passed)
        const WOK        = 0x04;
        /// Read access allowed with 4 GB granularity (page-granular shortcut)
        const ROK4G      = 0x08;
        /// Write access allowed with 4 GB granularity (page-granular shortcut)
        const WOK4G      = 0x10;
        /// All read/write + granularity bits (convenience combo)
        const ALL_ACCESS = Self::ROK.bits() | Self::WOK.bits()
                         | Self::ROK4G.bits() | Self::WOK4G.bits();
    }
}

// ---- Backward-compat aliases (prefer SegAccess::<NAME> in new code) ----
pub(super) const SEG_VALID_CACHE: u32   = SegAccess::VALID.bits();
pub(super) const SEG_ACCESS_ROK: u32    = SegAccess::ROK.bits();
pub(super) const SEG_ACCESS_WOK: u32    = SegAccess::WOK.bits();
pub(super) const SEG_ACCESS_ROK4_G: u32 = SegAccess::ROK4G.bits();
pub(super) const SEG_ACCESS_WOK4_G: u32 = SegAccess::WOK4G.bits();

bitflags::bitflags! {
    /// Access-rights byte of a segment descriptor (byte 5 of the 8-byte entry).
    ///
    /// Layout: `P | DPL(2) | S | TYPE(4)`
    ///  - P (bit 7): segment present
    ///  - DPL (bits 5-6): descriptor privilege level (0-3)
    ///  - S (bit 4): 1 = code/data segment, 0 = system/gate descriptor
    ///  - TYPE (bits 0-3): segment type (see `SegTypeBits`)
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct AccessRights: u8 {
        /// Bit 7 — Present
        const PRESENT  = 0x80;
        /// Bit 6 — DPL high bit
        const DPL_HI   = 0x40;
        /// Bit 5 — DPL low bit
        const DPL_LO   = 0x20;
        /// Bit 4 — Segment (1 = code/data, 0 = system/gate)
        const SEGMENT  = 0x10;
        /// Bits 0-3 — Type field (see SegTypeBits)
        const TYPE_MASK = 0x0F;
    }
}

impl AccessRights {
    /// Both DPL bits combined (mask = 0x60)
    pub const DPL_MASK: AccessRights = Self::DPL_HI.union(Self::DPL_LO);

    /// Extract DPL value (0-3) from the access rights byte
    #[inline]
    pub const fn dpl(self) -> u8 {
        (self.bits() >> 5) & 0x03
    }
}

#[derive(Default, Clone)]
pub(crate) struct BxDescriptor {
    pub(crate) valid: u32, // Holds above values, Or'd together. Used to
    // hold only 0 or 1 once.
    pub(crate) p: bool,       /* present */
    pub(crate) dpl: u8,       /* descriptor privilege level 0..3 */
    pub(crate) segment: bool, /* 0 = system/gate, 1 = data/code segment */
    pub r#type: u8,           /* For system & gate descriptors:
                               *  0 = invalid descriptor (reserved)
                               *  1 = 286 available Task State Segment (TSS)
                               *  2 = LDT descriptor
                               *  3 = 286 busy Task State Segment (TSS)
                               *  4 = 286 call gate
                               *  5 = task gate
                               *  6 = 286 interrupt gate
                               *  7 = 286 trap gate
                               *  8 = (reserved)
                               *  9 = 386 available TSS
                               * 10 = (reserved)
                               * 11 = 386 busy TSS
                               * 12 = 386 call gate
                               * 13 = (reserved)
                               * 14 = 386 interrupt gate
                               * 15 = 386 trap gate */
    pub(crate) u: Descriptor,
}

impl BxDescriptor {
    #[inline]
    pub fn is_present(&self) -> bool {
        self.p
    }

    pub fn is_long64_segment(&self) -> bool {
        self.u.segment_l()
    }

    /// Get Access Rights byte from descriptor
    /// Based on get_ar_byte in segment_ctrl_pro.cc:380
    pub(super) fn get_ar_byte(&self) -> u8 {
        let mut ar = AccessRights::empty();
        if self.p { ar |= AccessRights::PRESENT; }
        ar |= AccessRights::from_bits_retain(((self.dpl & 0x03) << 5) | if self.segment { AccessRights::SEGMENT.bits() } else { 0 });
        ar |= AccessRights::from_bits_retain(self.r#type & 0x0F);
        ar.bits()
    }

    /// Set Access Rights byte in descriptor
    /// Based on set_ar_byte in segment_ctrl_pro.cc:395
    pub(super) fn set_ar_byte(&mut self, ar_byte: u8) {
        let ar = AccessRights::from_bits_retain(ar_byte);
        self.p = ar.contains(AccessRights::PRESENT);
        self.dpl = (ar_byte >> 5) & 0x03;
        self.segment = ar.contains(AccessRights::SEGMENT);
        self.r#type = ar_byte & 0x0F;
    }
}

bitflags::bitflags! {
    /// Segment descriptor type-field bits (low nibble of the access byte).
    ///
    /// For **code** segments bit 3 is set; for **data** segments it is clear.
    /// Bit 2 has different meaning depending on code vs data:
    ///   - Code: conforming (1 = conforming, 0 = non-conforming)
    ///   - Data: expand-down (1 = expand-down, 0 = expand-up)
    /// Bit 1:
    ///   - Code: readable (1 = readable, 0 = execute-only)
    ///   - Data: writable (1 = writable, 0 = read-only)
    /// Bit 0: accessed (set by CPU on first access)
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct SegTypeBits: u8 {
        /// Bit 3 — code segment flag (1 = code, 0 = data)
        const CODE       = 0x8;
        /// Bit 2 — conforming (code) / expand-down (data)
        const CONFORMING = 0x4;
        /// Bit 2 alias for data segments
        const EXPAND_DOWN = 0x4;
        /// Bit 1 — readable (code) / writable (data)
        const READABLE   = 0x2;
        /// Bit 1 alias for data segments
        const WRITABLE   = 0x2;
        /// Bit 0 — accessed (set by CPU on segment load)
        const ACCESSED   = 0x1;
    }
}

impl SegTypeBits {
    /// Wrap a raw type nibble value
    #[inline]
    pub const fn from_raw(ty: u8) -> Self {
        Self::from_bits_retain(ty & 0x0F)
    }
}

// Convenience free functions (matching original Bochs BX_SEGMENT_* macros)
#[inline] pub fn is_code_segment(ty: u8) -> bool              { SegTypeBits::from_raw(ty).contains(SegTypeBits::CODE) }
#[inline] pub fn is_data_segment(ty: u8) -> bool              { !is_code_segment(ty) }
#[inline] pub fn is_code_segment_conforming(ty: u8) -> bool   { SegTypeBits::from_raw(ty).contains(SegTypeBits::CONFORMING) }
#[inline] pub fn is_code_segment_non_conforming(ty: u8) -> bool { !is_code_segment_conforming(ty) }
#[inline] pub fn is_data_segment_expand_down(ty: u8) -> bool  { SegTypeBits::from_raw(ty).contains(SegTypeBits::EXPAND_DOWN) }
#[inline] pub fn is_code_segment_readable(ty: u8) -> bool     { SegTypeBits::from_raw(ty).contains(SegTypeBits::READABLE) }
#[inline] pub fn is_data_segment_writable(ty: u8) -> bool     { SegTypeBits::from_raw(ty).contains(SegTypeBits::WRITABLE) }
#[inline] pub fn is_segment_accessed(ty: u8) -> bool          { SegTypeBits::from_raw(ty).contains(SegTypeBits::ACCESSED) }

// Keep the SegmentType enum for backward compat with existing match-based code
#[derive(Debug)]
pub enum SegmentType {
    Code,
    DataExpandDown,
    CodeConforming,
    DataWrite,
    CodeRead,
    Accessed,
}

impl From<SegmentType> for u8 {
    fn from(value: SegmentType) -> Self {
        match value {
            SegmentType::Code => SegTypeBits::CODE.bits(),
            SegmentType::DataExpandDown => SegTypeBits::EXPAND_DOWN.bits(),
            SegmentType::CodeConforming => SegTypeBits::CONFORMING.bits(),
            SegmentType::DataWrite => SegTypeBits::WRITABLE.bits(),
            SegmentType::CodeRead => SegTypeBits::READABLE.bits(),
            SegmentType::Accessed => SegTypeBits::ACCESSED.bits(),
        }
    }
}

impl SegmentType {
    pub fn is_code_segment(ty: u8) -> bool { is_code_segment(ty) }
    pub fn is_code_segment_conforming(ty: u8) -> bool { is_code_segment_conforming(ty) }
    pub fn is_data_segment_expand_down(ty: u8) -> bool { is_data_segment_expand_down(ty) }
    pub fn is_code_segment_readable(ty: u8) -> bool { is_code_segment_readable(ty) }
    pub fn is_data_segment_writable(ty: u8) -> bool { is_data_segment_writable(ty) }
    pub fn is_segment_accessed(ty: u8) -> bool { is_segment_accessed(ty) }
    pub fn is_data_segment(ty: u8) -> bool { is_data_segment(ty) }
    pub fn is_code_segment_non_conforming(ty: u8) -> bool { is_code_segment_non_conforming(ty) }
}

#[derive(Debug, Clone, Default)]
pub(super) enum SystemAndGateDescriptorEnum {
    #[default]
    BxGateTypeNone = 0x0,
    BxSysSegmentAvail286Tss = 0x1,
    BxSysSegmentLdt = 0x2,
    BxSysSegmentBusy286Tss = 0x3,
    Bx286CallGate = 0x4,
    BxTaskGate = 0x5,
    Bx286InterruptGate = 0x6,
    Bx286TrapGate = 0x7,
    /* 0x8 reserved */
    BxSysSegmentAvail386Tss = 0x9,
    /* 0xa reserved */
    BxSysSegmentBusy386Tss = 0xb,
    Bx386CallGate = 0xc,
    /* 0xd reserved */
    Bx386InterruptGate = 0xe,
    Bx386TrapGate = 0xf,
}

#[derive(Debug, Default, Clone)]
pub(super) enum BxDataAndCodeDescriptorEnum {
    DataReadOnly = 0x0,
    DataReadOnlyAccessed = 0x1,
    DataReadWrite = 0x2,
    #[default]
    DataReadWriteAccessed = 0x3,
    DataReadOnlyExpandDown = 0x4,
    DataReadOnlyExpandDownAccessed = 0x5,
    DataReadWriteExpandDown = 0x6,
    DataReadWriteExpandDownAccessed = 0x7,
    CodeExecOnly = 0x8,
    CodeExecOnlyAccessed = 0x9,
    CodeExecRead = 0xa,
    CodeExecReadAccessed = 0xb,
    CodeExecOnlyConforming = 0xc,
    CodeExecOnlyConformingAccessed = 0xd,
    CodeExecReadConforming = 0xe,
    CodeExecReadConformingAccessed = 0xf,
}

#[derive(Default, Clone)]
pub(crate) struct BxSegmentReg {
    pub(crate) selector: BxSelector,
    pub(crate) cache: BxDescriptor, // Idk if really option
}

#[derive(Debug, Default)]
pub struct BxGlobalSegmentReg {
    /// base address: 24bits=286,32bits=386,64bits=x86-64
    pub(super) base: BxAddress,
    pub(super) limit: u16, /* limit, 16bits */
}

//Bit8u get_ar_byte(const bx_descriptor_t *d);
//void  set_ar_byte(bx_descriptor_t *d, Bit8u ar_byte);
//void  parse_descriptor(Bit32u dword1, Bit32u dword2, bx_descriptor_t *temp);
