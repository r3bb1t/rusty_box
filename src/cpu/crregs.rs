use super::{cpuid::BxCpuIdTrait, decoder::instr_generated::BxInstructionGenerated, BxCpuC};

#[derive(Debug, Default)]
pub struct BxCr0 {
    pub val32: u32, // 32bit value of register
}

use paste::paste;

macro_rules! impl_crreg_accessors {
    // $name: the base name of the field
    // $bitnum: the bit index (0..31)
    ($struct_name:ident, $name:ident, $bitnum:expr) => {
        paste! {
            impl $struct_name {
                /// Returns the bit at position $bitnum as a bool.
                pub fn $name(&self) -> bool {
                    ((self.val32 >> $bitnum) & 1) != 0
                }

                /// Sets the bit at position $bitnum from the low bit of `val: u8`.
                pub fn [<set_ $name>](&mut self, val: u8) {
                    let mask = 1u32 << $bitnum;
                    // clear that bit, then or‐in the new value (0 or 1) shifted into place
                    self.val32 = (self.val32 & !mask)
                                | (((val != 0) as u32) << $bitnum);
                }
            }
        }
    };
}
// CR0 notes:
//   Each x86 level has its own quirks regarding how it handles
//   reserved bits.  I used DOS DEBUG.EXE in real mode on the
//   following processors, tried to clear bits 1..30, then tried
//   to set bits 1..30, to see how these bits are handled.
//   I found the following:
//
//   Processor    try to clear bits 1..30    try to set bits 1..30
//   386          7FFFFFF0                   7FFFFFFE
//   486DX2       00000010                   6005003E
//   Pentium      00000010                   7FFFFFFE
//   Pentium-II   00000010                   6005003E
//
// My assumptions:
//   All processors: bit 4 is hardwired to 1 (not true on all clones)
//   386: bits 5..30 of CR0 are also hardwired to 1
//   Pentium: reserved bits retain value set using mov cr0, reg32
//   486DX2/Pentium-II: reserved bits are hardwired to 0

impl_crreg_accessors!(BxCr0, pe, 0);
impl_crreg_accessors!(BxCr0, mp, 1);
impl_crreg_accessors!(BxCr0, em, 2);
impl_crreg_accessors!(BxCr0, ts, 3);

impl_crreg_accessors!(BxCr0, et, 4);
impl_crreg_accessors!(BxCr0, ne, 5);
impl_crreg_accessors!(BxCr0, wp, 16);
impl_crreg_accessors!(BxCr0, am, 18);
impl_crreg_accessors!(BxCr0, nw, 29);
impl_crreg_accessors!(BxCr0, cd, 30);

impl_crreg_accessors!(BxCr0, pg, 31);

impl BxCr0 {
    fn get32(&self) -> u32 {
        self.val32
    }
    fn set32(&mut self, val: u32) {
        self.val32 = val | 0x10
    }
}

#[derive(Debug, Default)]
pub struct BxCr4 {
    pub val32: u32, // 32bit value of register
}

impl_crreg_accessors!(BxCr4, vme, 0);
impl_crreg_accessors!(BxCr4, pvi, 1);
impl_crreg_accessors!(BxCr4, tsd, 2);
impl_crreg_accessors!(BxCr4, de, 3);
impl_crreg_accessors!(BxCr4, pse, 4);
impl_crreg_accessors!(BxCr4, pae, 5);
impl_crreg_accessors!(BxCr4, mce, 6);
impl_crreg_accessors!(BxCr4, pge, 7);
impl_crreg_accessors!(BxCr4, pce, 8);
impl_crreg_accessors!(BxCr4, osfxsr, 9);
impl_crreg_accessors!(BxCr4, osxmmexcpt, 10);
impl_crreg_accessors!(BxCr4, umip, 11);
impl_crreg_accessors!(BxCr4, la57, 12);

impl_crreg_accessors!(BxCr4, vmxe, 13);

impl_crreg_accessors!(BxCr4, smxe, 14);

impl_crreg_accessors!(BxCr4, fsgsbase, 16);

impl_crreg_accessors!(BxCr4, pcide, 17);
impl_crreg_accessors!(BxCr4, osxsave, 18);
impl_crreg_accessors!(BxCr4, keylocker, 19);
impl_crreg_accessors!(BxCr4, smep, 20);
impl_crreg_accessors!(BxCr4, smap, 21);
impl_crreg_accessors!(BxCr4, pke, 22);
impl_crreg_accessors!(BxCr4, cet, 23);
impl_crreg_accessors!(BxCr4, pks, 24);
impl_crreg_accessors!(BxCr4, uintr, 25);
impl_crreg_accessors!(BxCr4, lass, 27);
impl_crreg_accessors!(BxCr4, lam_supervisor, 28);

impl BxCr4 {
    fn get32(&self) -> u32 {
        self.val32
    }
    fn set32(&mut self, val: u32) {
        self.val32 = val
    }
}

#[derive(Debug, Default)]
pub struct BxDr6 {
    pub val32: u32, // 32bit value of register
}

impl_crreg_accessors!(BxDr6, b0, 0);
impl_crreg_accessors!(BxDr6, b1, 1);
impl_crreg_accessors!(BxDr6, b2, 2);
impl_crreg_accessors!(BxDr6, b3, 3);

impl_crreg_accessors!(BxDr6, bd, 13);
impl_crreg_accessors!(BxDr6, bs, 14);
impl_crreg_accessors!(BxDr6, bt, 15);

impl BxDr6 {
    fn get32(&self) -> u32 {
        self.val32
    }
    fn set32(&mut self, val: u32) {
        self.val32 = val
    }
}

#[derive(Debug, Default)]
pub struct BxDr7 {
    pub val32: u32, // 32bit value of register
}

macro_rules! impl_drreg_accessors {
    ($ty:ident, $name:ident, $mask:literal, $shift:expr) => {
        paste! {
            impl $ty {
                #[inline]
                pub fn $name(&self) -> u32 {
                    (self.val32 & $mask) >> $shift
                }
            }
        }
    };
}

impl_crreg_accessors!(BxDr7, l0, 0);
impl_crreg_accessors!(BxDr7, g0, 1);
impl_crreg_accessors!(BxDr7, l1, 2);
impl_crreg_accessors!(BxDr7, g1, 3);
impl_crreg_accessors!(BxDr7, l2, 4);
impl_crreg_accessors!(BxDr7, g2, 5);
impl_crreg_accessors!(BxDr7, l3, 6);
impl_crreg_accessors!(BxDr7, g3, 7);
impl_crreg_accessors!(BxDr7, le, 8);
impl_crreg_accessors!(BxDr7, ge, 9);
impl_crreg_accessors!(BxDr7, gd, 13);

impl_drreg_accessors!(BxDr7, r_w0, 0x00030000, 16);
impl_drreg_accessors!(BxDr7, len0, 0x000C0000, 18);
impl_drreg_accessors!(BxDr7, r_w1, 0x00300000, 20);
impl_drreg_accessors!(BxDr7, len1, 0x00C00000, 22);
impl_drreg_accessors!(BxDr7, r_w2, 0x03000000, 24);
impl_drreg_accessors!(BxDr7, len2, 0x0C000000, 26);
impl_drreg_accessors!(BxDr7, r_w3, 0x30000000, 28);
impl_drreg_accessors!(BxDr7, len3, 0xC0000000, 30);

impl_drreg_accessors!(BxDr7, bp_enabled, 0xFF, 0);

impl BxDr7 {
    fn get32(&self) -> u32 {
        self.val32
    }
    fn set32(&mut self, val: u32) {
        self.val32 = val
    }
}

#[derive(Debug, Default)]
pub struct BxEfer {
    pub val32: u32,
}

impl_crreg_accessors!(BxEfer, sce, 0);

impl_crreg_accessors!(BxEfer, lme, 8);
impl_crreg_accessors!(BxEfer, lma, 10);

impl_crreg_accessors!(BxEfer, nxe, 11);

impl_crreg_accessors!(BxEfer, svme, 12); /* AMD Secure Virtual Machine */
impl_crreg_accessors!(BxEfer, lmsle, 13); /* AMD Long Mode Segment Limit */
impl_crreg_accessors!(BxEfer, ffxsr, 14);
impl_crreg_accessors!(BxEfer, tce, 15); /* AMD Translation Cache Extensions */

impl BxEfer {
    fn get32(&self) -> u32 {
        self.val32
    }
    fn set32(&mut self, val: u32) {
        self.val32 = val
    }
}

const XSAVE_HEADER_LEN: u32 = 64;
const XSAVE_FPU_STATE_LEN: u32 = 160;
const XSAVE_SSE_STATE_LEN: u32 = 256;
const XSAVE_YMM_STATE_LEN: u32 = 256;
const XSAVE_OPMASK_STATE_LEN: u32 = 64;
const XSAVE_ZMM_HI256_STATE_LEN: u32 = 512;
const XSAVE_HI_ZMM_STATE_LEN: u32 = 1024;
const XSAVE_PT_STATE_LEN: u32 = 128;
const XSAVE_PKRU_STATE_LEN: u32 = 8;
const XSAVE_PASID_STATE_LEN: u32 = 8;
const XSAVE_CET_U_STATE_LEN: u32 = 16;
const XSAVE_CET_S_STATE_LEN: u32 = 24;
const XSAVE_HDC_STATE_LEN: u32 = 8;
const XSAVE_UINTR_STATE_LEN: u32 = 48;
const XSAVE_LBR_STATE_LEN: u32 = 808;
const XSAVE_HWP_STATE_LEN: u32 = 8;
const XSAVE_XTILECFG_STATE_LEN: u32 = 64;
const XSAVE_XTILEDATA_STATE_LEN: u32 = 8192;
const XSAVE_APX_STATE_LEN: u32 = 128;

const XSAVE_FPU_STATE_OFFSET: u32 = 0;
const XSAVE_SSE_STATE_OFFSET: u32 = 160;
const XSAVE_YMM_STATE_OFFSET: u32 = 576;
const XSAVE_OPMASK_STATE_OFFSET: u32 = 1088;
const XSAVE_ZMM_HI256_STATE_OFFSET: u32 = 1152;
const XSAVE_HI_ZMM_STATE_OFFSET: u32 = 1664;
const XSAVE_PKRU_STATE_OFFSET: u32 = 2688;
const XSAVE_XTILECFG_STATE_OFFSET: u32 = 2752;
const XSAVE_XTILEDATA_STATE_OFFSET: u32 = 2816;
const XSAVE_APX_STATE_OFFSET: u32 = 960; // repurpose deprecated BND (MPX) state

#[derive(Debug, Default)]
pub struct Xcr0 {
    pub value: u32,
}

#[derive(Debug)]
enum Xcr0Enum {
    BxXcr0FpuBit = 0,
    BxXcr0SseBit = 1,
    BxXcr0YmmBit = 2,
    BxXcr0BndregsBit = 3, // not implemented, deprecated
    BxXcr0BndcfgBit = 4,  // not implemented, deprecated
    BxXcr0OpmaskBit = 5,
    BxXcr0ZmmHi256Bit = 6,
    BxXcr0HiZmmBit = 7,
    BxXcr0PtBit = 8, // not implemented yet
    BxXcr0PkruBit = 9,
    BxXcr0PasidBit = 10, // not implemented yet
    BxXcr0CetUBit = 11,
    BxXcr0CetSBit = 12,
    BxXcr0HdcBit = 13, // not implemented yet
    BxXcr0UintrBit = 14,
    BxXcr0LbrBit = 15, // not implemented yet
    BxXcr0HwpBit = 16, // not implemented yet
    BxXcr0XtilecfgBit = 17,
    BxXcr0XtiledataBit = 18,
    BxXcr0ApxBit = 19,
    BxXcr0Last, // make sure it is < 32
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MSR {
    /// MSR index
    pub(crate) index: u32,
    /// MSR type: 1 - lin address, 2 - phy address
    pub(crate) r#type: u32,
    /// current MSR value
    pub(crate) val64: u64,
    /// reset value
    pub(crate) reset_value: u64,
    /// r/o bits - fault on write
    pub(crate) reserved: u64,
    /// hardwired bits - ignored on write
    pub(crate) ignored: u64,
}

impl MSR {
    const BX_LIN_ADDRESS_MSR: u32 = 1;
    const BX_PHY_ADDRESS_MSR: u32 = 2;
}

//struct XSaveRestoreStateHelper {
//  len: usize,
//  offset: usize,
//  XSaveStateInUsePtr_tR xstate_in_use_method;
//  XSavePtr_tR xsave_method;
//  XRestorPtr_tR xrstor_method;
//  XRestorInitPtr_tR xrstor_init_method;
//}

type XSaveStateInUsePtr_tR = fn() -> bool;
type XSavePtr_tR = fn(&BxInstructionGenerated, usize);
type XRestorPtr_tR = fn(&BxInstructionGenerated, usize);

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn xsave_xrestor_init(&mut self) {
        //self
    }
}
