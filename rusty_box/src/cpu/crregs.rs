#![allow(non_camel_case_types, unused_variables, unused_assignments, dead_code)]

use super::{cpuid::BxCpuIdTrait, decoder::Instruction, BxCpuC};

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

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct BxCr0: u32 {
        const PE = 1 << 0;
        const MP = 1 << 1;
        const EM = 1 << 2;
        const TS = 1 << 3;
        const ET = 1 << 4;
        const NE = 1 << 5;
        const WP = 1 << 16;
        const AM = 1 << 18;
        const NW = 1 << 29;
        const CD = 1 << 30;
        const PG = 1 << 31;
    }
}

impl BxCr0 {
    #[inline]
    pub fn pe(self) -> bool {
        self.contains(Self::PE)
    }
    #[inline]
    pub fn mp(self) -> bool {
        self.contains(Self::MP)
    }
    #[inline]
    pub fn em(self) -> bool {
        self.contains(Self::EM)
    }
    #[inline]
    pub fn ts(self) -> bool {
        self.contains(Self::TS)
    }
    #[inline]
    pub fn et(self) -> bool {
        self.contains(Self::ET)
    }
    #[inline]
    pub fn ne(self) -> bool {
        self.contains(Self::NE)
    }
    #[inline]
    pub fn wp(self) -> bool {
        self.contains(Self::WP)
    }
    #[inline]
    pub fn am(self) -> bool {
        self.contains(Self::AM)
    }
    #[inline]
    pub fn nw(self) -> bool {
        self.contains(Self::NW)
    }
    #[inline]
    pub fn cd(self) -> bool {
        self.contains(Self::CD)
    }
    #[inline]
    pub fn pg(self) -> bool {
        self.contains(Self::PG)
    }

    #[inline]
    pub(super) fn get32(self) -> u32 {
        self.bits()
    }
    #[inline]
    pub(super) fn set32(&mut self, val: u32) {
        // Bit 4 (ET) is hardwired to 1
        *self = Self::from_bits_retain(val | 0x10);
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct BxCr4: u64 {
        const VME = 1 << 0;
        const PVI = 1 << 1;
        const TSD = 1 << 2;
        const DE = 1 << 3;
        const PSE = 1 << 4;
        const PAE = 1 << 5;
        const MCE = 1 << 6;
        const PGE = 1 << 7;
        const PCE = 1 << 8;
        const OSFXSR = 1 << 9;
        const OSXMMEXCPT = 1 << 10;
        const UMIP = 1 << 11;
        const LA57 = 1 << 12;
        const VMXE = 1 << 13;
        const SMXE = 1 << 14;
        const FSGSBASE = 1 << 16;
        const PCIDE = 1 << 17;
        const OSXSAVE = 1 << 18;
        const KEYLOCKER = 1 << 19;
        const SMEP = 1 << 20;
        const SMAP = 1 << 21;
        const PKE = 1 << 22;
        const CET = 1 << 23;
        const PKS = 1 << 24;
        const UINTR = 1 << 25;
        const LASS = 1 << 27;
        const LAM_SUPERVISOR = 1 << 28;
        const FRED = 1u64 << 32;
    }
}

impl BxCr4 {
    #[inline]
    pub fn vme(self) -> bool {
        self.contains(Self::VME)
    }
    #[inline]
    pub fn pvi(self) -> bool {
        self.contains(Self::PVI)
    }
    #[inline]
    pub fn tsd(self) -> bool {
        self.contains(Self::TSD)
    }
    #[inline]
    pub fn de(self) -> bool {
        self.contains(Self::DE)
    }
    #[inline]
    pub fn pse(self) -> bool {
        self.contains(Self::PSE)
    }
    #[inline]
    pub fn pae(self) -> bool {
        self.contains(Self::PAE)
    }
    #[inline]
    pub fn mce(self) -> bool {
        self.contains(Self::MCE)
    }
    #[inline]
    pub fn pge(self) -> bool {
        self.contains(Self::PGE)
    }
    #[inline]
    pub fn pce(self) -> bool {
        self.contains(Self::PCE)
    }
    #[inline]
    pub fn osfxsr(self) -> bool {
        self.contains(Self::OSFXSR)
    }
    #[inline]
    pub fn osxmmexcpt(self) -> bool {
        self.contains(Self::OSXMMEXCPT)
    }
    #[inline]
    pub fn umip(self) -> bool {
        self.contains(Self::UMIP)
    }
    #[inline]
    pub fn la57(self) -> bool {
        self.contains(Self::LA57)
    }
    #[inline]
    pub fn vmxe(self) -> bool {
        self.contains(Self::VMXE)
    }
    #[inline]
    pub fn smxe(self) -> bool {
        self.contains(Self::SMXE)
    }
    #[inline]
    pub fn fsgsbase(self) -> bool {
        self.contains(Self::FSGSBASE)
    }
    #[inline]
    pub fn pcide(self) -> bool {
        self.contains(Self::PCIDE)
    }
    #[inline]
    pub fn osxsave(self) -> bool {
        self.contains(Self::OSXSAVE)
    }
    #[inline]
    pub fn keylocker(self) -> bool {
        self.contains(Self::KEYLOCKER)
    }
    #[inline]
    pub fn smep(self) -> bool {
        self.contains(Self::SMEP)
    }
    #[inline]
    pub fn smap(self) -> bool {
        self.contains(Self::SMAP)
    }
    #[inline]
    pub fn pke(self) -> bool {
        self.contains(Self::PKE)
    }
    #[inline]
    pub fn cet(self) -> bool {
        self.contains(Self::CET)
    }
    #[inline]
    pub fn pks(self) -> bool {
        self.contains(Self::PKS)
    }
    #[inline]
    pub fn uintr(self) -> bool {
        self.contains(Self::UINTR)
    }
    #[inline]
    pub fn lass(self) -> bool {
        self.contains(Self::LASS)
    }
    #[inline]
    pub fn lam_supervisor(self) -> bool {
        self.contains(Self::LAM_SUPERVISOR)
    }
    #[inline]
    pub fn fred(self) -> bool {
        self.contains(Self::FRED)
    }

    #[inline]
    pub(super) fn get(self) -> u64 {
        self.bits()
    }
    #[inline]
    pub(super) fn set_val(&mut self, val: u64) {
        *self = Self::from_bits_retain(val);
    }
    #[inline]
    pub(super) fn get32(self) -> u32 {
        self.bits() as u32
    }
    #[inline]
    pub(super) fn set32(&mut self, val: u32) {
        self.set_val(val as u64);
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct BxDr6: u32 {
        const B0 = 1 << 0;   // Breakpoint 0 condition detected
        const B1 = 1 << 1;   // Breakpoint 1 condition detected
        const B2 = 1 << 2;   // Breakpoint 2 condition detected
        const B3 = 1 << 3;   // Breakpoint 3 condition detected
        const BD = 1 << 13;  // Debug register access detected
        const BS = 1 << 14;  // Single step
        const BT = 1 << 15;  // Task switch
    }
}

impl BxDr6 {
    #[inline]
    pub fn b0(self) -> bool {
        self.contains(Self::B0)
    }
    #[inline]
    pub fn set_b0(&mut self, val: u8) {
        self.set(Self::B0, val != 0);
    }
    #[inline]
    pub fn b1(self) -> bool {
        self.contains(Self::B1)
    }
    #[inline]
    pub fn set_b1(&mut self, val: u8) {
        self.set(Self::B1, val != 0);
    }
    #[inline]
    pub fn b2(self) -> bool {
        self.contains(Self::B2)
    }
    #[inline]
    pub fn set_b2(&mut self, val: u8) {
        self.set(Self::B2, val != 0);
    }
    #[inline]
    pub fn b3(self) -> bool {
        self.contains(Self::B3)
    }
    #[inline]
    pub fn set_b3(&mut self, val: u8) {
        self.set(Self::B3, val != 0);
    }
    #[inline]
    pub fn bd(self) -> bool {
        self.contains(Self::BD)
    }
    #[inline]
    pub fn set_bd(&mut self, val: u8) {
        self.set(Self::BD, val != 0);
    }
    #[inline]
    pub fn bs(self) -> bool {
        self.contains(Self::BS)
    }
    #[inline]
    pub fn set_bs(&mut self, val: u8) {
        self.set(Self::BS, val != 0);
    }
    #[inline]
    pub fn bt(self) -> bool {
        self.contains(Self::BT)
    }
    #[inline]
    pub fn set_bt(&mut self, val: u8) {
        self.set(Self::BT, val != 0);
    }

    #[inline]
    pub(super) fn get32(self) -> u32 {
        self.bits()
    }
    #[inline]
    pub(super) fn set32(&mut self, val: u32) {
        *self = Self::from_bits_retain(val);
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct BxDr7: u32 {
        const L0 = 1 << 0;   // Local breakpoint 0 enable
        const G0 = 1 << 1;   // Global breakpoint 0 enable
        const L1 = 1 << 2;   // Local breakpoint 1 enable
        const G1 = 1 << 3;   // Global breakpoint 1 enable
        const L2 = 1 << 4;   // Local breakpoint 2 enable
        const G2 = 1 << 5;   // Global breakpoint 2 enable
        const L3 = 1 << 6;   // Local breakpoint 3 enable
        const G3 = 1 << 7;   // Global breakpoint 3 enable
        const LE = 1 << 8;   // Local exact breakpoint enable
        const GE = 1 << 9;   // Global exact breakpoint enable
        const GD = 1 << 13;  // General detect enable
    }
}

impl BxDr7 {
    // Single-bit accessors (matching old API)
    #[inline]
    pub fn l0(self) -> bool {
        self.contains(Self::L0)
    }
    #[inline]
    pub fn set_l0(&mut self, val: u8) {
        self.set(Self::L0, val != 0);
    }
    #[inline]
    pub fn g0(self) -> bool {
        self.contains(Self::G0)
    }
    #[inline]
    pub fn set_g0(&mut self, val: u8) {
        self.set(Self::G0, val != 0);
    }
    #[inline]
    pub fn l1(self) -> bool {
        self.contains(Self::L1)
    }
    #[inline]
    pub fn set_l1(&mut self, val: u8) {
        self.set(Self::L1, val != 0);
    }
    #[inline]
    pub fn g1(self) -> bool {
        self.contains(Self::G1)
    }
    #[inline]
    pub fn set_g1(&mut self, val: u8) {
        self.set(Self::G1, val != 0);
    }
    #[inline]
    pub fn l2(self) -> bool {
        self.contains(Self::L2)
    }
    #[inline]
    pub fn set_l2(&mut self, val: u8) {
        self.set(Self::L2, val != 0);
    }
    #[inline]
    pub fn g2(self) -> bool {
        self.contains(Self::G2)
    }
    #[inline]
    pub fn set_g2(&mut self, val: u8) {
        self.set(Self::G2, val != 0);
    }
    #[inline]
    pub fn l3(self) -> bool {
        self.contains(Self::L3)
    }
    #[inline]
    pub fn set_l3(&mut self, val: u8) {
        self.set(Self::L3, val != 0);
    }
    #[inline]
    pub fn g3(self) -> bool {
        self.contains(Self::G3)
    }
    #[inline]
    pub fn set_g3(&mut self, val: u8) {
        self.set(Self::G3, val != 0);
    }
    #[inline]
    pub fn le(self) -> bool {
        self.contains(Self::LE)
    }
    #[inline]
    pub fn set_le(&mut self, val: u8) {
        self.set(Self::LE, val != 0);
    }
    #[inline]
    pub fn ge(self) -> bool {
        self.contains(Self::GE)
    }
    #[inline]
    pub fn set_ge(&mut self, val: u8) {
        self.set(Self::GE, val != 0);
    }
    #[inline]
    pub fn gd(self) -> bool {
        self.contains(Self::GD)
    }
    #[inline]
    pub fn set_gd(&mut self, val: u8) {
        self.set(Self::GD, val != 0);
    }

    // Multi-bit field accessors (R/W and LEN fields, 2 bits each)
    #[inline]
    pub fn r_w0(self) -> u32 {
        (self.bits() & 0x0003_0000) >> 16
    }
    #[inline]
    pub fn len0(self) -> u32 {
        (self.bits() & 0x000C_0000) >> 18
    }
    #[inline]
    pub fn r_w1(self) -> u32 {
        (self.bits() & 0x0030_0000) >> 20
    }
    #[inline]
    pub fn len1(self) -> u32 {
        (self.bits() & 0x00C0_0000) >> 22
    }
    #[inline]
    pub fn r_w2(self) -> u32 {
        (self.bits() & 0x0300_0000) >> 24
    }
    #[inline]
    pub fn len2(self) -> u32 {
        (self.bits() & 0x0C00_0000) >> 26
    }
    #[inline]
    pub fn r_w3(self) -> u32 {
        (self.bits() & 0x3000_0000) >> 28
    }
    #[inline]
    pub fn len3(self) -> u32 {
        (self.bits() & 0xC000_0000) >> 30
    }
    #[inline]
    pub fn bp_enabled(self) -> u32 {
        self.bits() & 0xFF
    }

    #[inline]
    pub(super) fn get32(self) -> u32 {
        self.bits()
    }
    #[inline]
    pub(super) fn set32(&mut self, val: u32) {
        *self = Self::from_bits_retain(val);
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct BxEfer: u32 {
        const SCE   = 1 << 0;  // System Call Extensions
        const LME   = 1 << 8;  // Long Mode Enable
        const LMA   = 1 << 10; // Long Mode Active
        const NXE   = 1 << 11; // No-Execute Enable
        const SVME  = 1 << 12; // AMD Secure Virtual Machine Enable
        const LMSLE = 1 << 13; // AMD Long Mode Segment Limit Enable
        const FFXSR = 1 << 14; // Fast FXSAVE/FXRSTOR
        const TCE   = 1 << 15; // AMD Translation Cache Extensions
    }
}

impl BxEfer {
    #[inline]
    pub fn sce(&self) -> bool {
        self.contains(Self::SCE)
    }
    #[inline]
    pub fn set_sce(&mut self, val: u8) {
        self.set(Self::SCE, val != 0);
    }
    #[inline]
    pub fn lme(&self) -> bool {
        self.contains(Self::LME)
    }
    #[inline]
    pub fn set_lme(&mut self, val: u8) {
        self.set(Self::LME, val != 0);
    }
    #[inline]
    pub fn lma(&self) -> bool {
        self.contains(Self::LMA)
    }
    #[inline]
    pub fn set_lma(&mut self, val: u8) {
        self.set(Self::LMA, val != 0);
    }
    #[inline]
    pub fn nxe(&self) -> bool {
        self.contains(Self::NXE)
    }
    #[inline]
    pub fn set_nxe(&mut self, val: u8) {
        self.set(Self::NXE, val != 0);
    }
    #[inline]
    pub fn svme(&self) -> bool {
        self.contains(Self::SVME)
    }
    #[inline]
    pub fn set_svme(&mut self, val: u8) {
        self.set(Self::SVME, val != 0);
    }
    #[inline]
    pub fn lmsle(&self) -> bool {
        self.contains(Self::LMSLE)
    }
    #[inline]
    pub fn set_lmsle(&mut self, val: u8) {
        self.set(Self::LMSLE, val != 0);
    }
    #[inline]
    pub fn ffxsr(&self) -> bool {
        self.contains(Self::FFXSR)
    }
    #[inline]
    pub fn set_ffxsr(&mut self, val: u8) {
        self.set(Self::FFXSR, val != 0);
    }
    #[inline]
    pub fn tce(&self) -> bool {
        self.contains(Self::TCE)
    }
    #[inline]
    pub fn set_tce(&mut self, val: u8) {
        self.set(Self::TCE, val != 0);
    }

    #[inline]
    pub(super) fn get32(self) -> u32 {
        self.bits()
    }
    #[inline]
    pub(super) fn set32(&mut self, val: u32) {
        *self = Self::from_bits_retain(val);
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
    pub(crate) value: u32,
}

impl Xcr0 {
    #[inline]
    pub(super) fn get32(&self) -> u32 {
        self.value
    }

    #[inline]
    pub(super) fn set32(&mut self, val: u32) {
        self.value = val
    }
}

/// XCR0 / XSAVE state-component bit positions.
/// The discriminant equals the bit number in XCR0 and the XSAVE component
/// index used throughout xsave/xrstor dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(super) enum Xcr0Component {
    Fpu = 0,
    Sse = 1,
    Ymm = 2,
    /// MPX bound registers — Intel deprecated MPX in 2019.
    Bndregs = 3,
    /// MPX configuration — Intel deprecated MPX in 2019.
    Bndcfg = 4,
    Opmask = 5,
    ZmmHi256 = 6,
    HiZmm = 7,
    Pt = 8,
    Pkru = 9,
    Pasid = 10,
    CetU = 11,
    CetS = 12,
    Hdc = 13,
    Uintr = 14,
    Lbr = 15,
    Hwp = 16,
    Xtilecfg = 17,
    Xtiledata = 18,
    Apx = 19,
}

impl Xcr0Component {
    /// Convert a bit index (0..32) to a defined XCR0 component, if any.
    #[inline]
    pub(super) fn from_bit(bit: u32) -> Option<Self> {
        Some(match bit {
            0 => Self::Fpu,
            1 => Self::Sse,
            2 => Self::Ymm,
            3 => Self::Bndregs,
            4 => Self::Bndcfg,
            5 => Self::Opmask,
            6 => Self::ZmmHi256,
            7 => Self::HiZmm,
            8 => Self::Pt,
            9 => Self::Pkru,
            10 => Self::Pasid,
            11 => Self::CetU,
            12 => Self::CetS,
            13 => Self::Hdc,
            14 => Self::Uintr,
            15 => Self::Lbr,
            16 => Self::Hwp,
            17 => Self::Xtilecfg,
            18 => Self::Xtiledata,
            19 => Self::Apx,
            _ => return None,
        })
    }
}

#[allow(clippy::upper_case_acronyms)]
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
type XSavePtr_tR = fn(&Instruction, usize);
type XRestorPtr_tR = fn(&Instruction, usize);

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    pub(super) fn xsave_xrestor_init(&mut self) {
        //self
    }
}

// =========================================================================
// MOV Rd, CRn / MOV CRn, Rd / LMSW -- Control Register Instructions
// Matching Bochs crregs.cc
// =========================================================================

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ----- MOV Rd, CRn (reads) -----
    // All MOV CRn require CPL=0, matching Bochs crregs.cc

    /// Helper: check CPL=0 for privileged instructions, #GP(0) otherwise
    /// Matches Bochs crregs.cc CPL check at start of every MOV CRn/DRn
    fn check_cpl0_for_cr_dr(&mut self) -> super::Result<()> {
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        Ok(())
    }

    pub fn mov_rd_cr0(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let val_32 = self.cr0.get32();
        let gpr = instr.src() as usize;
        self.set_gpr32(gpr, val_32);

        Ok(())
    }

    pub fn mov_rd_cr2(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let val_32 = self.cr2 as u32;
        let gpr = instr.src() as usize;
        self.set_gpr32(gpr, val_32);

        Ok(())
    }

    pub fn mov_rd_cr3(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let gpr = instr.src();
        // Bochs vmexit.cc VMexit_CR3_Read — gated on CR3_READ_VMEXIT.
        if self.in_vmx_guest && self.vmexit_check_cr3_read(gpr)? {
            return Ok(());
        }
        let val_32 = self.cr3 as u32;
        self.set_gpr32(usize::from(gpr), val_32);

        Ok(())
    }

    pub fn mov_rd_cr4(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let val_32 = self.cr4.get32();
        let gpr = instr.src() as usize;
        self.set_gpr32(gpr, val_32);

        Ok(())
    }

    // ----- MOV CRn, Rd (writes) -----

    pub fn mov_cr0_rd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        self.invalidate_prefetch_q();

        let src = instr.src1();
        let raw_val_32 = self.get_gpr32(usize::from(src));

        // Bochs vmexit.cc VMexit_CR0_Write — either VMEXIT or merge the value
        // so masked (pinned) bits retain their hardware state.
        let val_32 = if self.in_vmx_guest {
            let (exited, merged) =
                self.vmexit_check_cr0_write(u64::from(raw_val_32), src)?;
            if exited {
                return Ok(());
            }
            merged as u32
        } else {
            raw_val_32
        };

        let old_cr0 = self.cr0.get32();

        // Bochs check_CR0(): PG without PE is illegal, NW without CD is illegal
        let new_cr0 = BxCr0::from_bits_retain(val_32);
        if new_cr0.contains(BxCr0::PG) && !new_cr0.contains(BxCr0::PE) {
            tracing::trace!("MOV CR0: PG=1 without PE=1, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        if new_cr0.contains(BxCr0::NW) && !new_cr0.contains(BxCr0::CD) {
            tracing::trace!("MOV CR0: NW=1 without CD=1, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let pg = new_cr0.contains(BxCr0::PG);

        // Bochs crregs.cc — Long mode activation/deactivation
        // When enabling paging (PG: 0→1) with EFER.LME=1: activate long mode
        if !self.cr0.pg() && pg {
            if self.efer.lme() {
                if !self.cr4.pae() {
                    tracing::trace!(
                        "MOV CR0: attempt to enter long mode without CR4.PAE, #GP(0)"
                    );
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                // SAFETY: segment cache populated during segment load; union read matches descriptor type
                let cs_l = self.sregs[super::decoder::BxSegregs::Cs as usize]
                    .cache
                    .u
                    .segment_l();
                if cs_l {
                    tracing::trace!("MOV CR0: attempt to enter long mode with CS.L=1, #GP(0)");
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                // TSS must be 386 or later (type > 3)
                if self.tr.cache.r#type <= 3 {
                    tracing::trace!(
                        "MOV CR0: attempt to enter long mode with TSS286 in TR, #GP(0)"
                    );
                    return self.exception(super::cpu::Exception::Gp, 0);
                }
                // Bochs crregs.cc — set EFER.LMA=1
                self.efer.set_lma(1);
                tracing::trace!("MOV CR0: Long mode activated (EFER.LMA=1)");
            }
        } else if self.cr0.pg() && !pg {
            // When disabling paging (PG: 1→0) with EFER.LMA=1: deactivate long mode
            if self.cpu_mode == super::cpu::CpuMode::Long64 {
                tracing::trace!("MOV CR0: attempt to leave 64-bit mode directly, #GP(0)");
                return self.exception(super::cpu::Exception::Gp, 0);
            }
            if self.efer.lma() {
                // Bochs crregs.cc — clear EFER.LMA
                self.efer.set_lma(0);
                tracing::trace!("MOV CR0: Long mode deactivated (EFER.LMA=0)");
            }
        }

        // Bochs SetCR0() (crregs.cc): mask reserved bits for CPU level 6
        let cr0_allowed = BxCr0::PG | BxCr0::CD | BxCr0::NW | BxCr0::AM | BxCr0::WP
            | BxCr0::NE | BxCr0::ET | BxCr0::TS | BxCr0::EM | BxCr0::MP | BxCr0::PE;
        let val_32 = val_32 & cr0_allowed.bits();

        // Bochs crregs.cc — PDPTR check when enabling paging with PAE
        if pg && self.cr4.pae() && !self.long_mode() {
            self.load_pdptrs();
        }

        // Track PM↔RM transitions for diagnostics
        #[cfg(debug_assertions)] {
            let old_pe = BxCr0::from_bits_retain(old_cr0).contains(BxCr0::PE);
            let new_pe = BxCr0::from_bits_retain(val_32).contains(BxCr0::PE);
            if old_pe && !new_pe {
                self.diag_pm_to_rm_count += 1;
            } else if !old_pe && new_pe {
                self.diag_rm_to_pm_count += 1;
            }
        }

        self.cr0.set32(val_32);

        // Bochs crregs.cc — mode change handlers (BEFORE TLB flush)
        // Note: Bochs calls handleCpuModeChange here, but our code has historically
        // only called update_fetch_mode_mask. Adding the full handler set caused
        // Alpine to break (cpu_mode transitions to LongCompat too early before
        // far JMP loads 64-bit CS). Keep the full Bochs-matching set but ensure
        // correct ordering.
        self.handle_alignment_check();
        self.handle_cpu_mode_change();
        self.handle_fpu_mmx_mode_change();
        self.handle_sse_mode_change();
        self.handle_avx_mode_change();

        // Bochs crregs.cc — TLB flush only if PG, WP, or PE changed
        if (old_cr0 & 0x80010001) != (val_32 & 0x80010001) {
            self.tlb_flush();
        }

        // Bochs crregs.cc: WP change flips the pkey disable-to-SYS-write
        // mapping inside set_PKeys. Recompute when WP bit differs.
        if (old_cr0 & BxCr0::WP.bits()) != (val_32 & BxCr0::WP.bits()) {
            self.set_pkeys(self.pkru, self.pkrs);
        }

        // Bochs crregs.cc
        self.linaddr_width = if self.cr4.la57() { 57 } else { 48 };

        // BOCHS BX_INSTR_TLB_CNTRL with MovCr0 kind
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_tlb() {
            self.instrumentation.fire_tlb_cntrl(
                super::instrumentation::TlbCntrl::MovCr0 { new_value: val_32 as u64 },
            );
        }

        tracing::trace!(
            "MOV CR0, r32: {:#010x} -> {:#010x} (PE={}, PG={}, LMA={})",
            old_cr0,
            val_32,
            self.cr0.pe(),
            (val_32 >> 31) & 1,
            self.efer.lma(),
        );
        Ok(())
    }

    pub fn mov_cr2_rd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        self.cr2 = val_32 as u64;

        Ok(())
    }

    pub fn mov_cr3_rd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        // Bochs crregs.cc — invalidate prefetch queue before CR3 change
        self.invalidate_prefetch_q();
        let src = instr.src1();
        let src_idx = usize::from(src);

        // Bochs crregs.cc: In long mode, CR3 gets full 64-bit value
        let mut val = if self.long_mode() {
            self.get_gpr64(src_idx)
        } else {
            u64::from(self.get_gpr32(src_idx))
        };

        // Bochs crregs.cc — allow NOFLUSH hint (bit 63) when PCIDE is set,
        // but ignore the hint: always clear it before storing to CR3
        if self.cr4.pcide() {
            val &= !(1u64 << 63);
        }

        // Bochs vmexit.cc VMexit_CR3_Write — gated on CR3_WRITE_VMEXIT, with
        // a fast-path when the new value matches any enabled CR3-target value.
        if self.in_vmx_guest && self.vmexit_check_cr3_write(val, src)? {
            return Ok(());
        }

        self.cr3 = val;

        // In PAE mode (but not long mode), validate and cache PDPTE entries.
        // Bochs crregs.cc calls CheckPDPTR() which reads 4 PDPTE entries from
        // physical memory at (cr3 & 0xFFFFFFE0) + n*8 and validates reserved bits.
        if self.cr4.pae() && !self.efer.lma() {
            self.load_pdptrs();
        }

        // Always flush ALL TLB entries including global on CR3 write.
        // This ensures stale global entries (like GDT page mapped RW→RO)
        // don't persist. Bochs uses flush_non_global when PGE is enabled,
        // but that requires the kernel to INVLPG global pages when PTEs change.
        // Our kernel doesn't INVLPG the GDT page after remapping RW→RO.
        self.tlb_flush();


        // BOCHS BX_INSTR_TLB_CNTRL with MovCr3 kind
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_tlb() {
            self.instrumentation.fire_tlb_cntrl(
                super::instrumentation::TlbCntrl::MovCr3 { new_value: val },
            );
        }


        Ok(())
    }

    // load_pdptrs is defined in paging.rs where page_walk_read_qword is accessible.

    pub fn mov_cr4_rd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        self.invalidate_prefetch_q();

        let src = instr.src1();
        let raw_val_32 = u64::from(self.get_gpr32(usize::from(src)));

        // Bochs vmexit.cc VMexit_CR4_Write — VMEXIT or merge per mask/shadow.
        let val_32 = if self.in_vmx_guest {
            let (exited, merged) =
                self.vmexit_check_cr4_write(raw_val_32, src)?;
            if exited {
                return Ok(());
            }
            merged
        } else {
            raw_val_32
        };

        // Bochs check_CR4(): reject unsupported bits using cr4_suppmask
        // computed at reset from CPUID features (matches crregs.cc)
        if (val_32 & !self.cr4_suppmask) != 0 {
            tracing::trace!(
                "MOV CR4: unsupported bits set {:#010x} (mask={:#010x}), #GP(0)",
                val_32 & !self.cr4_suppmask,
                self.cr4_suppmask
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let new_cr4 = BxCr4::from_bits_retain(val_32);

        // Bochs crregs.cc — long-mode checks
        // (1) Cannot clear CR4.PAE when EFER.LMA=1
        if self.efer.lma() && !new_cr4.contains(BxCr4::PAE) {
            tracing::trace!("MOV CR4: attempt to clear PAE while EFER.LMA=1, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        // (2) Cannot change CR4.LA57 when EFER.LMA=1
        if self.efer.lma()
            && (new_cr4.contains(BxCr4::LA57) != self.cr4.contains(BxCr4::LA57))
        {
            tracing::trace!("MOV CR4: attempt to change LA57 while EFER.LMA=1, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        // (3) Cannot set CR4.PCIDE when EFER.LMA=0
        if !self.efer.lma() && new_cr4.contains(BxCr4::PCIDE) {
            tracing::trace!("MOV CR4: attempt to set PCIDE while EFER.LMA=0, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let old_cr4 = self.cr4.get();
        self.cr4.set_val(val_32);

        // Bochs: TLB flush only if paging-related bits changed
        // BX_CR4_FLUSH_TLB_MASK = PSE|PAE|PGE|PCIDE|SMEP|SMAP
        let cr4_flush_tlb_mask = BxCr4::PSE.bits()
            | BxCr4::PAE.bits()
            | BxCr4::PGE.bits()
            | BxCr4::PCIDE.bits()
            | BxCr4::SMEP.bits()
            | BxCr4::SMAP.bits();
        if (old_cr4 ^ val_32) & cr4_flush_tlb_mask != 0 {
            self.tlb_flush();
        }

        // Bochs crregs.cc — mode change handlers after CR4 write
        self.handle_fpu_mmx_mode_change();
        self.handle_sse_mode_change();
        self.handle_avx_mode_change();

        // Bochs crregs.cc: PKE or PKS flip changes the pkey allow-mask layout.
        let pke_pks_mask = BxCr4::PKE.bits() | BxCr4::PKS.bits();
        if (old_cr4 ^ val_32) & pke_pks_mask != 0 {
            self.set_pkeys(self.pkru, self.pkrs);
        }

        // BOCHS BX_INSTR_TLB_CNTRL with MovCr4 kind
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_tlb() {
            self.instrumentation.fire_tlb_cntrl(
                super::instrumentation::TlbCntrl::MovCr4 { new_value: val_32 },
            );
        }

        // Bochs: update linaddr_width based on LA57 (5-level paging support)
        self.linaddr_width = if self.cr4.la57() { 57 } else { 48 };

        Ok(())
    }

    // ----- 64-bit MOV CRn, Rq (writes in long mode) -----

    /// MOV CR0, Rq — Bochs crregs.cc
    /// Reads full 64-bit register; upper 32 bits must be zero (#GP if not).
    pub fn mov_cr0_rq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        self.invalidate_prefetch_q();

        let src = instr.src1() as usize;
        let val_64 = self.get_gpr64(src);

        // Bochs check_CR0(): upper 32 bits must be zero
        if (val_64 >> 32) != 0 {
            tracing::trace!("MOV CR0 (64-bit): upper 32 bits non-zero {:#018x}, #GP(0)", val_64);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Delegate to 32-bit handler for the actual CR0 logic
        // Temporarily set the GPR to val_64 low 32 bits so mov_cr0_rd reads it
        // Actually, just inline the same logic with the known value
        let val_32 = val_64 as u32;
        let old_cr0 = self.cr0.get32();

        // Bochs check_CR0(): PG without PE is illegal, NW without CD is illegal
        let new_cr0 = BxCr0::from_bits_retain(val_32);
        if new_cr0.contains(BxCr0::PG) && !new_cr0.contains(BxCr0::PE) {
            tracing::trace!("MOV CR0 (64-bit): PG=1 without PE=1, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        if new_cr0.contains(BxCr0::NW) && !new_cr0.contains(BxCr0::CD) {
            tracing::trace!("MOV CR0 (64-bit): NW=1 without CD=1, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Call the shared CR0 write logic (reuse mov_cr0_rd body)
        // We need to set the GPR temporarily so the 32-bit handler reads the right value
        self.set_gpr32(src, val_32);
        self.mov_cr0_rd(instr)
    }

    /// MOV CR2, Rq — Bochs crregs.cc
    /// Full 64-bit store (CR2 holds the faulting linear address in long mode).
    pub fn mov_cr2_rq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let src = instr.src1() as usize;
        let val_64 = self.get_gpr64(src);
        self.cr2 = val_64;

        Ok(())
    }

    /// MOV CR4, Rq — Bochs crregs.cc
    /// Reads full 64-bit register; upper 32 bits must be zero (#GP if not).
    pub fn mov_cr4_rq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        self.invalidate_prefetch_q();

        let src = instr.src1() as usize;
        let val_64 = self.get_gpr64(src);

        // Bochs check_CR4(): upper 32 bits must be zero
        if (val_64 >> 32) != 0 {
            tracing::trace!("MOV CR4 (64-bit): upper 32 bits non-zero {:#018x}, #GP(0)", val_64);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Delegate to 32-bit handler with the low 32 bits
        self.set_gpr32(src, val_64 as u32);
        self.mov_cr4_rd(instr)
    }

    // ----- LMSW -----

    /// LMSW - Load Machine Status Word
    /// Based on Bochs crregs.cc
    pub fn lmsw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        // CPL must be 0 (CPL is always 0 in real mode)
        // Based on Bochs crregs.cc
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::trace!("LMSW: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let is_memory = !instr.mod_c0();
        let mut linear_addr: u64 = 0;
        let mut msw = if instr.mod_c0() {
            // For Group 7 (0F 01): b1=0x101, (b1 & 0x0F)==0x01 → Ed,Gd branch: DST=rm, SRC1=nnn
            // So dst() = rm = actual register. Matches Bochs: BX_READ_16BIT_REG(i->src()) where
            // Bochs's i->src()=rm. In our decoder src1()=nnn (opcode ext), dst()=rm.
            self.get_gpr16(instr.dst() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = super::decoder::BxSegregs::from(instr.seg());
            linear_addr = self.get_laddr64(seg as usize, u64::from(eaddr));
            self.v_read_word(seg, eaddr)?
        };

        // Bochs vmexit.cc VMexit_LMSW: the intercept inspects the raw source
        // value and either VMEXITs or returns the masked-merged value.
        if self.in_vmx_guest {
            let (exited, merged) =
                self.vmexit_check_lmsw(u32::from(msw), is_memory, linear_addr)?;
            if exited {
                return Ok(());
            }
            msw = merged as u16;
        }

        // LMSW cannot clear PE (Bochs crregs.cc)
        if self.cr0.pe() {
            msw |= 1;
        }

        // LMSW only affects last 4 bits (Bochs crregs.cc)
        msw &= 0xF;

        let old_cr0 = self.cr0.get32();
        let cr0_val = (old_cr0 & 0xFFFFFFF0) | msw as u32;

        // Use same path as MOV CR0 — SetCR0 equivalent
        // (Bochs crregs.cc calls SetCR0)
        self.cr0.set32(cr0_val);

        // TLB flush if PG, PE, or WP changed (Bochs crregs.cc)
        let tlb_relevant = BxCr0::PG | BxCr0::WP | BxCr0::PE;
        if (old_cr0 & tlb_relevant.bits()) != (cr0_val & tlb_relevant.bits()) {
            self.tlb_flush();
        }

        // handleAlignmentCheck + handleCpuModeChange (Bochs crregs.cc)
        self.handle_cpu_context_change();

        tracing::trace!(
            "LMSW: msw={:#06x}, CR0={:#010x} (PE={})",
            msw,
            cr0_val,
            self.cr0.pe()
        );
        Ok(())
    }

    // =========================================================================
    // Allow-mask computation functions
    // Matching Bochs crregs.cc
    // =========================================================================

    /// Compute CR4 supported bits mask from CPUID features.
    /// Matches Bochs crregs.cc get_cr4_allow_mask()
    pub(super) fn get_cr4_allow_mask(&self) -> u64 {
        use super::decoder::features::X86Feature;

        let mut allow = 0u64;

        // VME → CR4.VME + CR4.PVI
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaVme) {
            allow |= BxCr4::VME.bits() | BxCr4::PVI.bits();
        }
        // Pentium → CR4.TSD
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaPentium) {
            allow |= BxCr4::TSD.bits();
        }
        // Debug Extensions → CR4.DE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaDebugExtensions) {
            allow |= BxCr4::DE.bits();
        }
        // PSE → CR4.PSE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaPse) {
            allow |= BxCr4::PSE.bits();
        }
        // PAE → CR4.PAE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaPae) {
            allow |= BxCr4::PAE.bits();
        }
        // MCE always allowed (Bochs crregs.cc)
        allow |= BxCr4::MCE.bits();
        // PGE → CR4.PGE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaPge) {
            allow |= BxCr4::PGE.bits();
        }
        // PCE always allowed for CPU level >= 6 (Bochs crregs.cc)
        allow |= BxCr4::PCE.bits();
        // SSE → CR4.OSFXSR + CR4.OSXMMEXCPT
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaSse) {
            allow |= BxCr4::OSFXSR.bits() | BxCr4::OSXMMEXCPT.bits();
        }
        // VMX → CR4.VMXE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaVmx) {
            allow |= BxCr4::VMXE.bits();
        }
        // SMX → CR4.SMXE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaSmx) {
            allow |= BxCr4::SMXE.bits();
        }
        // PCID → CR4.PCIDE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaPcid) {
            allow |= BxCr4::PCIDE.bits();
        }
        // FSGSBASE → CR4.FSGSBASE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaFsgsbase) {
            allow |= BxCr4::FSGSBASE.bits();
        }
        // XSAVE → CR4.OSXSAVE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaXsave) {
            allow |= BxCr4::OSXSAVE.bits();
        }
        // SMEP → CR4.SMEP
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaSmep) {
            allow |= BxCr4::SMEP.bits();
        }
        // SMAP → CR4.SMAP
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaSmap) {
            allow |= BxCr4::SMAP.bits();
        }
        // PKU → CR4.PKE
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaPku) {
            allow |= BxCr4::PKE.bits();
        }
        // UMIP → CR4.UMIP
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaUmip) {
            allow |= BxCr4::UMIP.bits();
        }
        // LA57 → CR4.LA57
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaLa57) {
            allow |= BxCr4::LA57.bits();
        }
        // CET → CR4.CET
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaCet) {
            allow |= BxCr4::CET.bits();
        }
        // PKS → CR4.PKS
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaPks) {
            allow |= BxCr4::PKS.bits();
        }
        // UINTR → CR4.UINTR
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaUintr) {
            allow |= BxCr4::UINTR.bits();
        }
        // LASS → CR4.LASS
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaLass) {
            allow |= BxCr4::LASS.bits();
        }
        // FRED → CR4.FRED
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaFred) {
            allow |= BxCr4::FRED.bits();
        }

        allow
    }

    /// Compute EFER supported bits mask from CPUID features.
    /// Matches Bochs crregs.cc get_efer_allow_mask()
    pub(super) fn get_efer_allow_mask(&self) -> u32 {
        use super::decoder::features::X86Feature;

        let mut allow = 0u32;

        // NX → EFER.NXE (bit 11)
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaNx) {
            allow |= 1 << 11; // BX_EFER_NXE_MASK
        }
        // SYSCALL_SYSRET_LEGACY → EFER.SCE (bit 0)
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaSyscallSysretLegacy) {
            allow |= 1 << 0; // BX_EFER_SCE_MASK
        }
        // Long mode → SCE + LME + LMA
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaLongMode) {
            allow |= (1 << 0) | (1 << 8) | (1 << 10); // SCE | LME | LMA
                                                      // FFXSR → EFER.FFXSR (bit 14)
            if self.bx_cpuid_support_isa_extension(X86Feature::IsaFfxsr) {
                allow |= 1 << 14;
            }
            // SVM → EFER.SVME (bit 12)
            if self.bx_cpuid_support_isa_extension(X86Feature::IsaSvm) {
                allow |= 1 << 12;
            }
            // TCE → EFER.TCE (bit 15)
            if self.bx_cpuid_support_isa_extension(X86Feature::IsaTce) {
                allow |= 1 << 15;
            }
        }

        allow
    }

    /// Compute XCR0 supported bits mask from CPUID features.
    /// Matches Bochs crregs.cc get_xcr0_allow_mask()
    pub(super) fn get_xcr0_allow_mask(&self) -> u32 {
        use super::decoder::features::X86Feature;

        // FPU (bit 0) and SSE (bit 1) always present
        let mut allow = (1u32 << 0) | (1u32 << 1);

        // AVX → YMM (bit 2)
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaAvx) {
            allow |= 1 << 2;
        }
        // AVX-512 → OPMASK (bit 5), ZMM_HI256 (bit 6), HI_ZMM (bit 7)
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaAvx512) {
            allow |= (1 << 5) | (1 << 6) | (1 << 7);
        }
        // PKU → PKRU (bit 9)
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaPku) {
            allow |= 1 << 9;
        }
        // AMX → XTILECFG (bit 17) + XTILEDATA (bit 18)
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaAmx) {
            allow |= (1 << 17) | (1 << 18);
        }

        allow
    }

    /// Compute IA32_XSS supported bits mask from CPUID features.
    /// Matches Bochs crregs.cc get_ia32_xss_allow_mask()
    pub(super) fn get_ia32_xss_allow_mask(&self) -> u32 {
        use super::decoder::features::X86Feature;

        let mut allow = 0u32;

        // CET → CET_U (bit 11) + CET_S (bit 12)
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaCet) {
            allow |= (1 << 11) | (1 << 12);
        }
        // UINTR → UINTR (bit 14)
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaUintr) {
            allow |= 1 << 14;
        }

        allow
    }

    // =========================================================================
    // MOV Rq, CRn — 64-bit CR reads (long mode)
    // Matching Bochs crregs.cc MOV_RqCR0 / MOV_RqCR2 / MOV_RqCR3 / MOV_RqCR4
    // =========================================================================

    pub fn mov_rq_cr0(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let val = self.cr0.get32() as u64;
        self.set_gpr64(instr.src() as usize, val);
        Ok(())
    }

    pub fn mov_rq_cr2(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        self.set_gpr64(instr.src() as usize, self.cr2);
        Ok(())
    }

    pub fn mov_rq_cr3(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let gpr = instr.src();
        // Bochs vmexit.cc VMexit_CR3_Read — gated on CR3_READ_VMEXIT.
        if self.in_vmx_guest && self.vmexit_check_cr3_read(gpr)? {
            return Ok(());
        }
        self.set_gpr64(usize::from(gpr), self.cr3);
        Ok(())
    }

    pub fn mov_rq_cr4(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let val = self.cr4.get();
        self.set_gpr64(instr.src() as usize, val);
        Ok(())
    }

    // =========================================================================
    // MOV Rq, DRn / MOV DRn, Rq — 64-bit DR reads/writes (long mode)
    // Matching Bochs crregs.cc MOV_RqDq / MOV_DqRq
    // =========================================================================

    pub fn mov_rq_dq(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        // Reuse 32-bit handler which already reads DR correctly
        self.mov_rd_dd(instr)
    }

    pub fn mov_dq_rq(&mut self, instr: &super::decoder::Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        // Reuse 32-bit handler which already writes DR correctly
        self.mov_dd_rd(instr)
    }
}
