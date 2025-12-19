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

use core::{default, ffi::c_uint};

use crate::config::BxAddress;

#[derive(Debug, Default, Clone)]
pub(crate) struct BxSelector {
    /* bx_selector_t */
    pub(super) value: u16, /* the 16bit value of the selector */
    /* the following fields are extracted from the value field in protected
    mode only.  They're used for sake of efficiency */
    pub(super) index: u16, /* 13bit index extracted from value in protected mode */
    pub(super) ti: u16,    /* table indicator bit extracted from value */
    pub(super) rpl: u8,    /* RPL extracted from value */
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
pub(super) union Descriptor {
    pub segment: DescriptorSegment,
    pub gate: DescriptorGate,
    pub task_gate: DescriptorTaskGate,
}

#[derive(Clone, Copy)]
pub(super) struct DescriptorSegment {
    /// base address: 286=24bits, 386=32bits, long=64
    pub base: BxAddress,
    /// for efficiency, this contrived field is set to
    ///  limit for byte granular, and
    ///  `(limit << 12) | 0xfff` for page granular seg's
    pub limit_scaled: u32,
    /// granularity: 0=byte, 1=4K (page)
    pub g: bool,
    /// default size: 0=16bit, 1=32bit
    pub d_b: bool,
    /// long mode: 0=compat, 1=64 bit
    pub l: bool,
    ///  available for use by system
    pub avl: bool, // available for use by system
}

#[derive(Clone, Copy)]
struct DescriptorGate {
    pub param_count: u8, // 5 bits (0..31) #words/dword to copy
    pub dest_selector: u16,
    pub dest_offset: u32,
}

#[derive(Clone, Copy, Default)]
struct DescriptorTaskGate {
    tss_selector: u16, // TSS segment selector
}

impl Default for Descriptor {
    fn default() -> Self {
        Self {
            task_gate: DescriptorTaskGate { tss_selector: 0 },
        }
    }
}

pub(super) const SEG_VALID_CACHE: u32 = 0x01;
pub(super) const SEG_ACCESS_ROK: u32 = 0x02;
pub(super) const SEG_ACCESS_WOK: u32 = 0x04;
pub(super) const SEG_ACCESS_ROK4_G: u32 = 0x08;
pub(super) const SEG_ACCESS_WOK4_G: u32 = 0x10;

#[derive(Default, Clone)]
pub(crate) struct BxDescriptor {
    pub valid: u32, // Holds above values, Or'd together. Used to
    // hold only 0 or 1 once.
    pub p: bool,       /* present */
    pub dpl: u8,       /* descriptor privilege level 0..3 */
    pub segment: bool, /* 0 = system/gate, 1 = data/code segment */
    pub r#type: u8,    /* For system & gate descriptors:
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
    pub u: Descriptor,
}

impl BxDescriptor {
    #[inline]
    pub fn is_present(&self) -> bool {
        self.segment
    }

    pub fn is_long64_segment(&self) -> bool {
        unsafe { self.u.segment.l }
    }
}

// Define constants for segment types
const BX_SEGMENT_CODE: u8 = 0x8;
const BX_SEGMENT_DATA_EXPAND_DOWN: u8 = 0x4;
const BX_SEGMENT_CODE_CONFORMING: u8 = 0x4;
const BX_SEGMENT_DATA_WRITE: u8 = 0x2;
const BX_SEGMENT_CODE_READ: u8 = 0x2;
const BX_SEGMENT_ACCESSED: u8 = 0x1;

#[derive(Debug)]
pub enum SegmentType {
    Code,
    DataExpandDown,
    CodeConforming,
    DataWrite,
    CodeRead,
    Accessed,
}

// Hack since i can't assign values in definition
impl From<SegmentType> for u8 {
    fn from(value: SegmentType) -> Self {
        match value {
            SegmentType::Code => 0x8,           // 8
            SegmentType::DataExpandDown => 0x4, // 4
            SegmentType::CodeConforming => 0x4, // 4 (this will be handled in checks)
            SegmentType::DataWrite => 0x2,      // 2
            SegmentType::CodeRead => 0x2,       // 2 (this will be handled in checks)
            SegmentType::Accessed => 0x1,       // 1
        }
    }
}

impl SegmentType {
    pub fn is_code_segment(ty: u8) -> bool {
        ty & u8::from(SegmentType::Code) != 0
    }

    pub fn is_code_segment_conforming(ty: u8) -> bool {
        ty & u8::from(SegmentType::CodeConforming) != 0
    }

    pub fn is_data_segment_expand_down(ty: u8) -> bool {
        ty & u8::from(SegmentType::DataExpandDown) != 0
    }

    pub fn is_code_segment_readable(ty: u8) -> bool {
        ty & u8::from(SegmentType::CodeRead) != 0
    }

    pub fn is_data_segment_writable(ty: u8) -> bool {
        ty & u8::from(SegmentType::DataWrite) != 0
    }

    pub fn is_segment_accessed(ty: u8) -> bool {
        ty & u8::from(SegmentType::Accessed) != 0
    }

    // New methods based on the provided macros
    pub fn is_data_segment(ty: u8) -> bool {
        !Self::is_code_segment(ty)
    }

    pub fn is_code_segment_non_conforming(ty: u8) -> bool {
        !Self::is_code_segment_conforming(ty)
    }
}

pub fn is_code_segment(type_: u8) -> bool {
    type_ & BX_SEGMENT_CODE != 0
}

pub fn is_code_segment_conforming(type_: u8) -> bool {
    type_ & BX_SEGMENT_CODE_CONFORMING != 0
}

pub fn is_data_segment_expand_down(type_: u8) -> bool {
    type_ & BX_SEGMENT_DATA_EXPAND_DOWN != 0
}

pub fn is_code_segment_readable(type_: u8) -> bool {
    type_ & BX_SEGMENT_CODE_READ != 0
}

pub fn is_data_segment_writable(type_: u8) -> bool {
    type_ & BX_SEGMENT_DATA_WRITE != 0
}

pub fn is_segment_accessed(type_: u8) -> bool {
    type_ & BX_SEGMENT_ACCESSED != 0
}

// New functions based on the provided macros
pub fn is_data_segment(type_: u8) -> bool {
    !is_code_segment(type_)
}

pub fn is_code_segment_non_conforming(type_: u8) -> bool {
    !is_code_segment_conforming(type_)
}

#[derive(Debug, Clone, Default)]
pub(super) enum SystemAndGateDescriptorEnum {
    #[default] // FIXME: delete this
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
pub(super) struct BxSegmentReg {
    pub(super) selector: BxSelector,
    pub(super) cache: BxDescriptor, // Idk if really option
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
