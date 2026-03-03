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

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct BxCr4: u32 {
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
    pub(super) fn get32(self) -> u32 {
        self.bits()
    }
    #[inline]
    pub(super) fn set32(&mut self, val: u32) {
        *self = Self::from_bits_retain(val);
    }
}

#[derive(Debug, Default)]
pub struct BxDr6 {
    pub(crate) val32: u32, // 32bit value of register
}

impl_crreg_accessors!(BxDr6, b0, 0);
impl_crreg_accessors!(BxDr6, b1, 1);
impl_crreg_accessors!(BxDr6, b2, 2);
impl_crreg_accessors!(BxDr6, b3, 3);

impl_crreg_accessors!(BxDr6, bd, 13);
impl_crreg_accessors!(BxDr6, bs, 14);
impl_crreg_accessors!(BxDr6, bt, 15);

impl BxDr6 {
    #[inline]
    pub(super) fn get32(&self) -> u32 {
        self.val32
    }
    #[inline]
    pub(super) fn set32(&mut self, val: u32) {
        self.val32 = val
    }
}

#[derive(Debug, Default)]
pub struct BxDr7 {
    pub(crate) val32: u32, // 32bit value of register
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
    pub(super) fn get32(&self) -> u32 {
        self.val32
    }
    pub(super) fn set32(&mut self, val: u32) {
        self.val32 = val
    }
}

#[derive(Debug, Default)]
pub struct BxEfer {
    pub(crate) val32: u32,
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
    #[inline]
    pub(super) fn get32(&self) -> u32 {
        self.val32
    }
    #[inline]
    pub(super) fn set32(&mut self, val: u32) {
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
type XSavePtr_tR = fn(&Instruction, usize);
type XRestorPtr_tR = fn(&Instruction, usize);

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn xsave_xrestor_init(&mut self) {
        //self
    }
}

// =========================================================================
// MOV Rd, CRn / MOV CRn, Rd / LMSW -- Control Register Instructions
// Matching Bochs crregs.cc
// =========================================================================

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
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
        tracing::trace!("MOV r32, CR0: {:#010x} -> reg{}", val_32, gpr);
        Ok(())
    }

    pub fn mov_rd_cr2(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let val_32 = self.cr2 as u32;
        let gpr = instr.src() as usize;
        self.set_gpr32(gpr, val_32);
        tracing::trace!("MOV r32, CR2: {:#010x} -> reg{}", val_32, gpr);
        Ok(())
    }

    pub fn mov_rd_cr3(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let val_32 = self.cr3 as u32;
        let gpr = instr.src() as usize;
        self.set_gpr32(gpr, val_32);
        tracing::trace!("MOV r32, CR3: {:#010x} -> reg{}", val_32, gpr);
        Ok(())
    }

    pub fn mov_rd_cr4(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let val_32 = self.cr4.get32();
        let gpr = instr.src() as usize;
        self.set_gpr32(gpr, val_32);
        tracing::trace!("MOV r32, CR4: {:#010x} -> reg{}", val_32, gpr);
        Ok(())
    }

    // ----- MOV CRn, Rd (writes) -----

    pub fn mov_cr0_rd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        self.invalidate_prefetch_q();

        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        let old_cr0 = self.cr0.get32();

        // Bochs check_CR0(): PG without PE is illegal, NW without CD is illegal
        if (val_32 & (1 << 31)) != 0 && (val_32 & 1) == 0 {
            // PG=1, PE=0 → #GP(0)
            tracing::debug!("MOV CR0: PG=1 without PE=1, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        if (val_32 & (1 << 29)) != 0 && (val_32 & (1 << 30)) == 0 {
            // NW=1, CD=0 → #GP(0)
            tracing::debug!("MOV CR0: NW=1 without CD=1, #GP(0)");
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        // Bochs SetCR0() (crregs.cc): mask reserved bits for CPU level 6
        let val_32 = val_32 & 0xe005003f;

        self.cr0.set32(val_32);

        // Bochs: TLB flush if PG, PE, or WP changed
        if (old_cr0 & 0x80010001) != (val_32 & 0x80010001) {
            self.tlb_flush();
        }

        // Match Bochs handleCpuModeChange + handleAlignmentCheck
        self.handle_cpu_context_change();

        tracing::trace!(
            "MOV CR0, r32: {:#010x} -> {:#010x} (PE={}, PG={})",
            old_cr0,
            val_32,
            self.cr0.pe(),
            (val_32 >> 31) & 1
        );
        Ok(())
    }

    pub fn mov_cr2_rd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        self.cr2 = val_32 as u64;
        tracing::trace!("MOV CR2, r32: {:#010x}", val_32);
        Ok(())
    }

    pub fn mov_cr3_rd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        // Bochs crregs.cc:463 — invalidate prefetch queue before CR3 change
        self.invalidate_prefetch_q();
        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);
        self.cr3 = val_32 as u64;

        if self.cr4.pge() {
            self.tlb_flush_non_global();
        } else {
            self.tlb_flush();
        }

        tracing::trace!("MOV CR3, r32: {:#010x}", val_32);
        Ok(())
    }

    pub fn mov_cr4_rd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.check_cpl0_for_cr_dr()?;
        self.invalidate_prefetch_q();

        let src = instr.src1() as usize;
        let val_32 = self.get_gpr32(src);

        // Bochs check_CR4(): reject unsupported bits.
        // cr4_suppmask for Skylake-X: VME|PVI|TSD|DE|PSE|PAE|MCE|PGE|PCE|OSFXSR|OSXMMEXCPT|UMIP|SMEP|SMAP
        // We allow the bits that our CPUID model advertises support for.
        // For a 32-bit emulation: bits 0-10, 11(UMIP), 20(SMEP), 21(SMAP)
        const CR4_SUPPMASK: u32 = (1 << 0)  // VME
            | (1 << 1)  // PVI
            | (1 << 2)  // TSD
            | (1 << 3)  // DE
            | (1 << 4)  // PSE
            | (1 << 5)  // PAE
            | (1 << 6)  // MCE
            | (1 << 7)  // PGE
            | (1 << 8)  // PCE
            | (1 << 9)  // OSFXSR
            | (1 << 10) // OSXMMEXCPT
            | (1 << 11) // UMIP
            | (1 << 18) // OSXSAVE
            | (1 << 20) // SMEP
            | (1 << 21); // SMAP

        if (val_32 & !CR4_SUPPMASK) != 0 {
            tracing::debug!(
                "MOV CR4: unsupported bits set {:#010x} (mask={:#010x}), #GP(0)",
                val_32 & !CR4_SUPPMASK,
                CR4_SUPPMASK
            );
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let old_cr4 = self.cr4.get32();
        self.cr4.set32(val_32);

        // Bochs: TLB flush only if paging-related bits changed
        // BX_CR4_FLUSH_TLB_MASK = PSE|PAE|PGE|PCIDE|SMEP|SMAP
        const CR4_FLUSH_TLB_MASK: u32 =
            (1 << 4) | (1 << 5) | (1 << 7) | (1 << 17) | (1 << 20) | (1 << 21);
        if (old_cr4 ^ val_32) & CR4_FLUSH_TLB_MASK != 0 {
            self.tlb_flush();
        }

        tracing::trace!("MOV CR4, r32: {:#010x}", val_32);
        Ok(())
    }

    // ----- LMSW -----

    /// LMSW - Load Machine Status Word
    /// Based on Bochs crregs.cc:870-913
    pub fn lmsw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        // CPL must be 0 (CPL is always 0 in real mode)
        // Based on Bochs crregs.cc:874
        let cpl = self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .rpl;
        if cpl != 0 {
            tracing::debug!("LMSW: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let mut msw = if instr.mod_c0() {
            // For Group 7 (0F 01): b1=0x101, (b1 & 0x0F)==0x01 → Ed,Gd branch: DST=rm, SRC1=nnn
            // So dst() = rm = actual register. Matches Bochs: BX_READ_16BIT_REG(i->src()) where
            // Bochs's i->src()=rm. In our decoder src1()=nnn (opcode ext), dst()=rm.
            self.get_gpr16(instr.dst() as usize)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = super::decoder::BxSegregs::from(instr.seg());
            self.read_virtual_word(seg, eaddr)?
        };

        // LMSW cannot clear PE (Bochs crregs.cc:903-905)
        if self.cr0.pe() {
            msw |= 1;
        }

        // LMSW only affects last 4 bits (Bochs crregs.cc:907)
        msw &= 0xF;

        let old_cr0 = self.cr0.get32();
        let cr0_val = (old_cr0 & 0xFFFFFFF0) | msw as u32;

        // Use same path as MOV CR0 — SetCR0 equivalent
        // (Bochs crregs.cc:910 calls SetCR0)
        self.cr0.set32(cr0_val);

        // TLB flush if PG, PE, or WP changed (Bochs crregs.cc:1158)
        if (old_cr0 & 0x80010001) != (cr0_val & 0x80010001) {
            self.tlb_flush();
        }

        // handleAlignmentCheck + handleCpuModeChange (Bochs crregs.cc:1142-1145)
        self.handle_cpu_context_change();

        tracing::debug!(
            "LMSW: msw={:#06x}, CR0={:#010x} (PE={})",
            msw,
            cr0_val,
            self.cr0.pe()
        );
        Ok(())
    }
}
