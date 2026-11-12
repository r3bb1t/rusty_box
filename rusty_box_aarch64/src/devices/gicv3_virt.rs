//! The virtualization support of the board's GICv3 CPU interface, for its
//! one processor with EL2: the virtual interface control registers
//! (`ICH_*_EL2`) a hypervisor programs, the virtual CPU interface
//! (`ICV_*_EL1`) its guest acknowledges and completes virtual interrupts
//! through, the virtual IRQ and FIQ it signals to the processor, the
//! maintenance interrupt, and `ICC_SRE_EL2`.
//!
//! Provenance (R7): Arm IHI 0069G (ID011821): 6.2 to 6.4 (the List
//! registers, the active priority registers, maintenance interrupts and
//! the virtual priority pseudocode), 12.3 (the `ICV_*_EL1` registers), 12.4
//! (the `ICH_*_EL2` registers), 12.2.23 (`ICC_SRE_EL2`), 12.20.1
//! (`VirtualReadIAR0`/`1`, `VirtualReadHPPIR0`/`1`, `VirtualWriteEOIR0`/`1`,
//! `VirtualWriteDIR`, `ICH_EISR_EL2`, `ICH_ELRSR_EL2`, `VPRIMask`, the
//! virtual paths of `ICC_PMR_EL1` and `ICC_RPR_EL1`), 2.2.3
//! (`VirtualIdentifierValid`) and B.7.2 (`CanSignalVirtualInt`,
//! `CanSignalVirtualInterrupt`, `HighestPriorityVirtualInterrupt`). What
//! `ICH_VTR_EL2` reports is the processor's ([`VirtualInterfaceShape`]).
//!
//! The model is a GICv3 virtual interface with no GICv4: no virtual LPIs by
//! direct injection (`GICH_VLPIR` is never pending), no SEIs
//! (`ICH_VTR_EL2.SEIS` 0, so `ICH_HCR_EL2.TSEI` is RES0), 16 bits of
//! virtual INTID (`IDbits` 0), no affinity level 3 (`A3V` 0), and the
//! System register interface only: `ICC_SRE_EL2.SRE`, `DFB` and `DIB` are
//! RAO/WI, `ICH_VMCR_EL2.VFIQEn` is RES1 and `VAckCtl` RES0 (12.4.8: "In
//! implementations where the Non-secure copy of ICC_SRE_EL1.SRE is always
//! 1"). `ICC_SRE_EL2.Enable` is a field, whose clear value traps EL1's
//! `ICC_SRE_EL1` to EL2.
//!
//! Where the specification leaves a choice to the implementation:
//! - Each reset value it leaves UNKNOWN is 0 — the List registers, the
//!   active priorities, `ICC_SRE_EL2.Enable`, `VEOIM` and `VCBPR` — but the
//!   binary points, which reset to their minimum, and `VFIQEn`, RES1.
//! - A virtual EOI drops the running priority before its checks
//!   (`IMPLEMENTATION DEFINED "Drop before checks"`, as the GICv2 model's
//!   every EOI drops it); each `ConstrainUnpredictableBool()` of an EOI is
//!   TRUE: an EOI with no active priority does nothing, and one whose
//!   interrupt is not of the highest active priority deactivates nothing.
//! - A write naming an INTID with bits set above the 16 implemented, or an
//!   LPI where none is allowed (UNPREDICTABLE), is ignored as a special
//!   INTID is.
//! - The List registers hold the 16 bits of `vINTID` and the 13 of
//!   `pINTID` (12.4.6: "only 13 bits of Physical INTID are required"),
//!   `EOI` among them, whatever `HW` says; the priority's unimplemented
//!   bits are RES0.
//! - The active priority registers past the preemption levels
//!   (`ICH_AP0R1_EL2` to `ICH_AP0R3_EL2` with five preemption bits) are
//!   UNDEFINED at the processor and hold nothing here.
//!
//! Every interface here has as many preemption bits as priority bits
//! (`ICH_VTR_EL2.PREbits` equal to `PRIbits`), so a priority's group under
//! the least binary point is its whole priority, and its active priority
//! bit is the priority shifted down past the bits not implemented.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::gicv2::{group_mask, preempts, PriorityBits, IDLE_PRIORITY, MAX_APRS, SPURIOUS};
use crate::bus::{ApIndex, IchRegister, IcvRegister, LrIndex};
use crate::cpu::VirtualLines;

/// INTID 25, the maintenance interrupt: "Arm strongly recommends that
/// maintenance interrupts are configured to use INTID 25" (IHI 0069G
/// 6.3.3), which BSA (DEN0094C) requires; the GICv2's own
/// ([`super::gicv2_virt::MAINTENANCE_INTID`]) too.
pub const MAINTENANCE_INTID: u32 = super::gicv2_virt::MAINTENANCE_INTID;

/// What a processor's virtual interface implements, as `ICH_VTR_EL2`
/// reports it: its List registers and its virtual priority and preemption
/// bits, and whether it has `ICH_HCR_EL2.TDIR`. The processor model
/// decides it ([`crate::CpuModel`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VirtualInterfaceShape {
    vtr: u32,
}

// ICH_VTR_EL2 (IHI 0069G 12.4.9).
const VTR_LIST_REGS: u32 = 0x1F;
const VTR_TDS: u32 = 1 << 19;
const VTR_NV4: u32 = 1 << 20;
const VTR_PREBITS_SHIFT: u32 = 26;
const VTR_PRIBITS_SHIFT: u32 = 29;

impl VirtualInterfaceShape {
    /// The Cortex-A57's GICv3 CPU interface (DDI 0488H Table 8-13,
    /// `ICH_VTR_EL2` 0x90000003): four List registers, five priority and
    /// five preemption bits, 16 bits of virtual INTID, no SEIs, no affinity
    /// level 3; bit 20 (`nV4` in IHI 0069G) and bit 19 (`TDS`) are among
    /// the TRM's reserved RAZ bits [20:5], so `ICH_HCR_EL2.TDIR` is RES0.
    pub const CORTEX_A57: Self = Self::checked(0x9000_0003);

    /// The Armv9.2 model's on a CPU interface of `physical` priority bits,
    /// the board's own choice within IHI 0069G 12.4.9: sixteen List
    /// registers, the most there are; as many virtual priority bits as the
    /// CPU interface has, but at least five (12.4.9: "at least 32 levels")
    /// and at most seven (the most preemption bits there are), and as many
    /// preemption bits, so that every virtual active priority register a
    /// guest finds by `ICV_CTLR_EL1.PRIbits` is one the CPU interface's
    /// priority bits give an encoding (`ICC_AP1R<n>_EL1` is UNDEFINED past
    /// `NUM_GIC_PRIORITY_BITS`, virtual or not); 16 bits of virtual INTID;
    /// no SEIs, no affinity level 3; `nV4` 1, no direct injection, the only
    /// value a GICv3 permits; and `TDS` 1, `ICH_HCR_EL2.TDIR` implemented
    /// (FEAT_GICv3_TDIR), whose absence Arm deprecates.
    pub const fn armv9p2(physical: PriorityBits) -> Self {
        let bits: u32 = match physical {
            PriorityBits::Four | PriorityBits::Five => 4,
            PriorityBits::Six => 5,
            PriorityBits::Seven | PriorityBits::Eight => 6,
        };
        Self::checked(
            (bits << VTR_PRIBITS_SHIFT) | (bits << VTR_PREBITS_SHIFT) | VTR_NV4 | VTR_TDS | 15,
        )
    }

    /// The shape `vtr` reports, which this model implements: five to seven
    /// priority bits and as many preemption bits, 16 bits of virtual INTID,
    /// no SEIs, no affinity level 3, no masking of directly injected
    /// interrupts, and nothing in the reserved bits.
    const fn checked(vtr: u32) -> Self {
        let pri = (vtr >> VTR_PRIBITS_SHIFT) & 7;
        let pre = (vtr >> VTR_PREBITS_SHIFT) & 7;
        assert!(
            pri >= 4 && pri <= 6 && pre == pri,
            "five to seven bits, preempting at each"
        );
        assert!(
            vtr & !(VTR_LIST_REGS | VTR_TDS | VTR_NV4 | (0x3F << VTR_PREBITS_SHIFT)) == 0,
            "no IDbits, SEIS, A3V or DVIM"
        );
        Self { vtr }
    }

    /// `ICH_VTR_EL2`.
    pub const fn vtr(self) -> u64 {
        self.vtr as u64
    }

    /// The List registers, 1 to 16.
    pub const fn list_registers(self) -> usize {
        (self.vtr & VTR_LIST_REGS) as usize + 1
    }

    /// The virtual priority bits (`PRIbits` plus one), which are its
    /// preemption bits too.
    pub const fn priority_bits(self) -> PriorityBits {
        match (self.vtr >> VTR_PRIBITS_SHIFT) & 7 {
            4 => PriorityBits::Five,
            5 => PriorityBits::Six,
            _ => PriorityBits::Seven,
        }
    }

    /// Whether `ICH_AP0R<n>_EL2` and `ICH_AP1R<n>_EL2` exist (IHI 0069G
    /// Table 6-1): the first always, the second with six preemption bits
    /// or more, the third and fourth with seven.
    pub const fn has_active_priorities(self, n: ApIndex) -> bool {
        let bits = self.priority_bits().count();
        match n {
            ApIndex::Zero => true,
            ApIndex::One => bits >= 6,
            ApIndex::Two | ApIndex::Three => bits >= 7,
        }
    }

    /// Whether List register `n` exists.
    pub const fn has_list_register(self, n: LrIndex) -> bool {
        n.get() < self.list_registers()
    }

    /// Whether `ICH_HCR_EL2.TDIR` is implemented (`ICH_VTR_EL2.TDS`).
    const fn has_tdir(self) -> bool {
        self.vtr & VTR_TDS != 0
    }
}

// ICH_HCR_EL2 (IHI 0069G 12.4.5).
const HCR_EN: u64 = 1 << 0;
const HCR_UIE: u64 = 1 << 1;
const HCR_LRENPIE: u64 = 1 << 2;
const HCR_NPIE: u64 = 1 << 3;
const HCR_VGRP0EIE: u64 = 1 << 4;
const HCR_VGRP0DIE: u64 = 1 << 5;
const HCR_VGRP1EIE: u64 = 1 << 6;
const HCR_VGRP1DIE: u64 = 1 << 7;
const HCR_TC: u64 = 1 << 10;
const HCR_TALL0: u64 = 1 << 11;
const HCR_TALL1: u64 = 1 << 12;
const HCR_TDIR: u64 = 1 << 14;
const HCR_EOICOUNT_SHIFT: u32 = 27;
const HCR_EOICOUNT: u64 = 0x1F << HCR_EOICOUNT_SHIFT;

// ICH_MISR_EL2 (IHI 0069G 12.4.7).
const MISR_EOI: u64 = 1 << 0;
const MISR_U: u64 = 1 << 1;
const MISR_LRENP: u64 = 1 << 2;
const MISR_NP: u64 = 1 << 3;
const MISR_VGRP0E: u64 = 1 << 4;
const MISR_VGRP0D: u64 = 1 << 5;
const MISR_VGRP1E: u64 = 1 << 6;
const MISR_VGRP1D: u64 = 1 << 7;

// ICH_VMCR_EL2 (IHI 0069G 12.4.8).
const VMCR_VENG0: u64 = 1 << 0;
const VMCR_VENG1: u64 = 1 << 1;
const VMCR_VFIQEN: u64 = 1 << 3;
const VMCR_VCBPR: u64 = 1 << 4;
const VMCR_VEOIM: u64 = 1 << 9;
const VMCR_VBPR1_SHIFT: u32 = 18;
const VMCR_VBPR0_SHIFT: u32 = 21;
const VMCR_VPMR_SHIFT: u32 = 24;
/// The control bits a write keeps: `VENG0`, `VENG1`, `VCBPR` and `VEOIM`.
const VMCR_CONTROLS: u64 = VMCR_VENG0 | VMCR_VENG1 | VMCR_VCBPR | VMCR_VEOIM;

// ICH_LR<n>_EL2 (IHI 0069G 12.4.6).
const LR_STATE_SHIFT: u32 = 62;
const LR_HW: u64 = 1 << 61;
const LR_GROUP: u64 = 1 << 60;
const LR_PRIORITY_SHIFT: u32 = 48;
const LR_PINTID_SHIFT: u32 = 32;
/// `pINTID`'s 13 bits.
const LR_PINTID: u64 = 0x1FFF << LR_PINTID_SHIFT;
/// With `HW` 0, bit 41, of `pINTID`: `EOI`.
const LR_EOI: u64 = 1 << 41;
/// `vINTID`'s 16 implemented bits (`ICH_VTR_EL2.IDbits` 0).
const LR_VINTID: u64 = 0xFFFF;

/// `ICC_SRE_EL2`: `SRE`, `DFB` and `DIB` RAO/WI, and `Enable` (IHI 0069G
/// 12.2.23).
const SRE_EL2_RAO: u64 = 0b111;
const SRE_EL2_ENABLE: u64 = 1 << 3;

/// The bits of a virtual INTID the interface implements (`VIDBits`).
const VID_BITS: u32 = 16;
/// The first special INTID and the first LPI (IHI 0069G B.7 `IsSpecial`,
/// `IsLPI`).
const FIRST_SPECIAL: u32 = 1020;
const FIRST_LPI: u32 = 8192;

/// A List register's `State` (IHI 0069G 12.4.6).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LrState {
    Invalid,
    Pending,
    Active,
    ActivePending,
}

impl LrState {
    const fn of(lr: u64) -> Self {
        match lr >> LR_STATE_SHIFT {
            0b00 => Self::Invalid,
            0b01 => Self::Pending,
            0b10 => Self::Active,
            _ => Self::ActivePending,
        }
    }

    const fn bits(self) -> u64 {
        (match self {
            Self::Invalid => 0b00,
            Self::Pending => 0b01,
            Self::Active => 0b10,
            Self::ActivePending => 0b11,
        }) << LR_STATE_SHIFT
    }

    const fn is_active(self) -> bool {
        matches!(self, Self::Active | Self::ActivePending)
    }
}

/// Which group an interrupt, an acknowledge or an end of interrupt is of.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Zero,
    One,
}

/// A physical interrupt a hardware List register's deactivation
/// deactivates in the Distributor or Redistributor (IHI 0069G 12.4.6 `HW`,
/// `Deactivate(pID)` of the pseudocode): its `pINTID`, if below 1020.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(crate) struct PhysicalDeactivation(pub(crate) Option<u32>);

/// The CPU interface's EL2 state: the virtual interface's registers and
/// `ICC_SRE_EL2.Enable`.
#[derive(Clone)]
pub(crate) struct VirtualInterface {
    shape: VirtualInterfaceShape,
    /// `ICH_HCR_EL2`, `EOIcount` included.
    hcr: u64,
    /// `ICH_VMCR_EL2`, `VFIQEn` set.
    vmcr: u64,
    /// `ICH_AP0R<n>_EL2` and `ICH_AP1R<n>_EL2`: one bit per active
    /// preemption level.
    ap0r: [u32; MAX_APRS],
    ap1r: [u32; MAX_APRS],
    lr: [u64; LrIndex::COUNT],
    /// `ICC_SRE_EL2`.
    sre_el2: u64,
}

impl VirtualInterface {
    /// The interface out of reset, of `shape`.
    pub(crate) const fn new(shape: VirtualInterfaceShape) -> Self {
        let bits = shape.priority_bits();
        Self {
            shape,
            hcr: 0,
            vmcr: VMCR_VFIQEN
                | (bits.min_bpr() as u64) << VMCR_VBPR0_SHIFT
                | (bits.min_abpr() as u64) << VMCR_VBPR1_SHIFT,
            ap0r: [0; MAX_APRS],
            ap1r: [0; MAX_APRS],
            lr: [0; LrIndex::COUNT],
            sre_el2: SRE_EL2_RAO,
        }
    }

    pub(crate) const fn shape(&self) -> VirtualInterfaceShape {
        self.shape
    }

    /// `ICH_HCR_EL2`.
    pub(crate) const fn hcr(&self) -> u64 {
        self.hcr
    }

    /// `ICC_SRE_EL2`.
    pub(crate) const fn sre_el2(&self) -> u64 {
        self.sre_el2
    }

    /// The bits of `ICH_HCR_EL2` that hold something: the maintenance
    /// interrupt enables, `En`, `TC`, `TALL0`, `TALL1`, `EOIcount`, and
    /// `TDIR` where the shape has it.
    const fn hcr_mask(&self) -> u64 {
        let tdir = if self.shape.has_tdir() { HCR_TDIR } else { 0 };
        0xFF | HCR_TC | HCR_TALL0 | HCR_TALL1 | tdir | HCR_EOICOUNT
    }

    /// The bits of a List register that hold something.
    const fn lr_mask(&self) -> u64 {
        let priority = (self.shape.priority_bits().mask() as u64) << LR_PRIORITY_SHIFT;
        (0b11 << LR_STATE_SHIFT) | LR_HW | LR_GROUP | priority | LR_PINTID | LR_VINTID
    }

    // ----- the virtual interface control registers -----

    /// An `MRS` of `reg`. A List register or active priority register the
    /// shape lacks reads as zero, which no processor asks for (its access
    /// is UNDEFINED).
    pub(crate) fn control_read(&self, reg: IchRegister) -> u64 {
        match reg {
            IchRegister::Ap0r(n) => u64::from(self.ap0r[n.get()]),
            IchRegister::Ap1r(n) => u64::from(self.ap1r[n.get()]),
            IchRegister::Hcr => self.hcr,
            IchRegister::Vtr => self.shape.vtr(),
            IchRegister::Misr => self.misr(),
            IchRegister::Eisr => self.eisr(),
            IchRegister::Elrsr => self.elrsr(),
            IchRegister::Vmcr => self.vmcr,
            IchRegister::Lr(n) => self.lr[n.get()],
        }
    }

    /// An `MSR` of `value` to `reg`. A read-only register ignores it, which
    /// no processor asks for (its `MSR` is UNDEFINED).
    pub(crate) fn control_write(&mut self, reg: IchRegister, value: u64) {
        let bits = self.shape.priority_bits();
        match reg {
            IchRegister::Ap0r(n) => self.ap0r[n.get()] = value as u32 & bits.apr_bits(n.get()),
            IchRegister::Ap1r(n) => self.ap1r[n.get()] = value as u32 & bits.apr_bits(n.get()),
            IchRegister::Hcr => self.hcr = value & self.hcr_mask(),
            IchRegister::Vmcr => self.set_vmcr(value),
            IchRegister::Lr(n) => {
                if self.shape.has_list_register(n) {
                    self.lr[n.get()] = value & self.lr_mask();
                }
            }
            IchRegister::Vtr | IchRegister::Misr | IchRegister::Eisr | IchRegister::Elrsr => {}
        }
    }

    /// An `MSR` of `ICC_SRE_EL2`: `Enable` alone holds what is written.
    pub(crate) fn set_sre_el2(&mut self, value: u64) {
        self.sre_el2 = SRE_EL2_RAO | (value & SRE_EL2_ENABLE);
    }

    /// `ICH_VMCR_EL2` as `value` writes it: the priority mask's implemented
    /// bits (`VPRIMask`), each binary point at least its minimum (12.4.8:
    /// "An attempt to program the binary point field to a value less than
    /// the minimum value sets the field to the minimum value"), the
    /// controls, and `VFIQEn` set.
    fn set_vmcr(&mut self, value: u64) {
        let bits = self.shape.priority_bits();
        let vpmr = (value >> VMCR_VPMR_SHIFT) as u8 & bits.mask();
        let vbpr0 = (((value >> VMCR_VBPR0_SHIFT) & 7) as u8).max(bits.min_bpr());
        let vbpr1 = (((value >> VMCR_VBPR1_SHIFT) & 7) as u8).max(bits.min_abpr());
        self.vmcr = u64::from(vpmr) << VMCR_VPMR_SHIFT
            | u64::from(vbpr0) << VMCR_VBPR0_SHIFT
            | u64::from(vbpr1) << VMCR_VBPR1_SHIFT
            | (value & VMCR_CONTROLS)
            | VMCR_VFIQEN;
    }

    fn vpmr(&self) -> u8 {
        (self.vmcr >> VMCR_VPMR_SHIFT) as u8
    }

    fn vbpr0(&self) -> u8 {
        ((self.vmcr >> VMCR_VBPR0_SHIFT) & 7) as u8
    }

    fn vbpr1(&self) -> u8 {
        ((self.vmcr >> VMCR_VBPR1_SHIFT) & 7) as u8
    }

    fn vmcr_has(&self, bit: u64) -> bool {
        self.vmcr & bit != 0
    }

    /// The list registers the shape has.
    fn list(&self) -> &[u64] {
        &self.lr[..self.shape.list_registers()]
    }

    /// `ICH_EISR_EL2`: the List registers invalid with `HW` 0 and `EOI` 1.
    fn eisr(&self) -> u64 {
        self.list()
            .iter()
            .enumerate()
            .filter(|(_, &lr)| {
                LrState::of(lr) == LrState::Invalid && lr & LR_HW == 0 && lr & LR_EOI != 0
            })
            .fold(0, |bits, (n, _)| bits | 1 << n)
    }

    /// `ICH_ELRSR_EL2`: the List registers invalid with `HW` 1 or `EOI` 0.
    fn elrsr(&self) -> u64 {
        self.list()
            .iter()
            .enumerate()
            .filter(|(_, &lr)| {
                LrState::of(lr) == LrState::Invalid && (lr & LR_HW != 0 || lr & LR_EOI == 0)
            })
            .fold(0, |bits, (n, _)| bits | 1 << n)
    }

    /// `ICH_MISR_EL2` (IHI 0069G 12.4.7). "No List register is in pending
    /// state": a List register active and pending is in a state of its own
    /// (12.4.6 `State`).
    fn misr(&self) -> u64 {
        let hcr = self.hcr;
        let enabled = |control: u64| hcr & control != 0;
        let valid = self
            .list()
            .iter()
            .filter(|&&lr| LrState::of(lr) != LrState::Invalid)
            .count();
        let none_pending = !self
            .list()
            .iter()
            .any(|&lr| LrState::of(lr) == LrState::Pending);
        let grp0 = self.vmcr_has(VMCR_VENG0);
        let grp1 = self.vmcr_has(VMCR_VENG1);
        let conditions = [
            (MISR_EOI, self.eisr() != 0),
            (MISR_U, enabled(HCR_UIE) && valid <= 1),
            (MISR_LRENP, enabled(HCR_LRENPIE) && hcr & HCR_EOICOUNT != 0),
            (MISR_NP, enabled(HCR_NPIE) && none_pending),
            (MISR_VGRP0E, enabled(HCR_VGRP0EIE) && grp0),
            (MISR_VGRP0D, enabled(HCR_VGRP0DIE) && !grp0),
            (MISR_VGRP1E, enabled(HCR_VGRP1EIE) && grp1),
            (MISR_VGRP1D, enabled(HCR_VGRP1DIE) && !grp1),
        ];
        conditions
            .into_iter()
            .filter(|&(_, asserted)| asserted)
            .fold(0, |misr, (bit, _)| misr | bit)
    }

    /// The maintenance interrupt's level: a `ICH_MISR_EL2` bit set while
    /// `ICH_HCR_EL2.En` is (6.3.3: "generated only when the global enable
    /// bit ... is set to 1", level-sensitive).
    pub(crate) fn maintenance(&self) -> bool {
        self.hcr & HCR_EN != 0 && self.misr() != 0
    }

    // ----- prioritization -----

    fn group(&self, n: usize) -> Group {
        if self.lr[n] & LR_GROUP != 0 {
            Group::One
        } else {
            Group::Zero
        }
    }

    fn priority(&self, n: usize) -> u8 {
        (self.lr[n] >> LR_PRIORITY_SHIFT) as u8
    }

    fn vintid(&self, n: usize) -> u32 {
        (self.lr[n] & LR_VINTID) as u32
    }

    /// Whether the virtual CPU interface signals interrupts of `group`
    /// (`VENG0`, `VENG1`).
    fn group_enabled(&self, group: Group) -> bool {
        match group {
            Group::Zero => self.vmcr_has(VMCR_VENG0),
            Group::One => self.vmcr_has(VMCR_VENG1),
        }
    }

    /// B.7.2 `HighestPriorityVirtualInterrupt`: the pending List register
    /// of an enabled group with the numerically lowest priority, the lowest
    /// numbered among equals.
    fn highest_priority(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        for n in 0..self.shape.list_registers() {
            if LrState::of(self.lr[n]) != LrState::Pending || !self.group_enabled(self.group(n)) {
                continue;
            }
            if best.is_none_or(|b| self.priority(n) < self.priority(b)) {
                best = Some(n);
            }
        }
        best
    }

    /// The shift from a priority to its preemption level, a bit of the
    /// active priority registers: the bits below the implemented ones.
    fn level_shift(&self) -> u32 {
        8 - u32::from(self.shape.priority_bits().count())
    }

    /// The preemption level of `priority`: its group under the least binary
    /// point (6.4 `VPriorityGroup`), shifted down.
    fn level(&self, priority: u8) -> usize {
        usize::from(priority & self.shape.priority_bits().mask()) >> self.level_shift()
    }

    /// The highest active preemption level, if one is active (6.4
    /// `GetHighestActiveVPriority`, `VPriorityBitsSet`).
    fn highest_active_level(&self) -> Option<usize> {
        (0..MAX_APRS).find_map(|n| {
            let active = self.ap0r[n] | self.ap1r[n];
            (active != 0).then(|| n * 32 + active.trailing_zeros() as usize)
        })
    }

    /// The running priority (`ICV_RPR_EL1`): the highest active preemption
    /// level's priority, or [`IDLE_PRIORITY`] while none is active.
    fn running_priority(&self) -> u16 {
        self.highest_active_level()
            .map_or(IDLE_PRIORITY, |level| (level << self.level_shift()) as u16)
    }

    /// The mask of List register `n`'s group priority under its group's
    /// binary point (6.4 `VGroupBits`: `VBPR0` for Group 0, or with `VCBPR`
    /// for both, `VBPR1` minus one for Group 1).
    fn group_mask(&self, n: usize) -> u8 {
        group_mask(
            self.group(n) == Group::One,
            self.vmcr_has(VMCR_VCBPR),
            self.vbpr0(),
            self.vbpr1(),
        )
    }

    /// B.7.2 `CanSignalVirtualInt` of List register `n`: the interface
    /// enabled, the interrupt pending, of a priority above `VPMR`, and with
    /// nothing active or a group priority above the running priority's.
    fn can_signal(&self, n: usize) -> bool {
        let priority = self.priority(n);
        self.hcr & HCR_EN != 0
            && LrState::of(self.lr[n]) == LrState::Pending
            && priority < self.vpmr()
            && preempts(priority, self.group_mask(n), self.running_priority())
    }

    /// The List register B.7.2 `CanSignalVirtualInterrupt` signals, if any.
    fn signalled(&self) -> Option<usize> {
        self.highest_priority().filter(|&n| self.can_signal(n))
    }

    /// The virtual IRQ and FIQ to the processor: the signalled interrupt's,
    /// Group 1 as a virtual IRQ and Group 0, with `VFIQEn` RES1, as a
    /// virtual FIQ (12.4.6 `Group`).
    pub(crate) fn outputs(&self) -> VirtualLines {
        match self.signalled().map(|n| self.group(n)) {
            None => VirtualLines::NONE,
            Some(Group::Zero) => VirtualLines {
                irq: false,
                fiq: true,
            },
            Some(Group::One) => VirtualLines {
                irq: true,
                fiq: false,
            },
        }
    }

    // ----- the virtual CPU interface -----

    /// An `MRS` of `reg`, with the read's side effect: `ICV_IAR0_EL1` and
    /// `ICV_IAR1_EL1` acknowledge the interrupt they return. A write-only
    /// register reads as zero, which no processor asks for.
    pub(crate) fn virtual_read(&mut self, reg: IcvRegister) -> u64 {
        u64::from(match reg {
            IcvRegister::Pmr => u32::from(self.vpmr()),
            IcvRegister::Iar0 => self.acknowledge(Group::Zero),
            IcvRegister::Iar1 => self.acknowledge(Group::One),
            IcvRegister::Hppir0 => self.highest_pending(Group::Zero),
            IcvRegister::Hppir1 => self.highest_pending(Group::One),
            IcvRegister::Bpr0 => u32::from(self.vbpr0()),
            IcvRegister::Bpr1 => u32::from(if self.vmcr_has(VMCR_VCBPR) {
                (self.vbpr0() + 1).min(7)
            } else {
                self.vbpr1()
            }),
            IcvRegister::Ap0r(n) => self.ap0r[n.get()],
            IcvRegister::Ap1r(n) => self.ap1r[n.get()],
            IcvRegister::Rpr => u32::from(self.running_priority().min(0xFF)),
            IcvRegister::Ctlr => self.ctlr(),
            IcvRegister::Igrpen0 => u32::from(self.vmcr_has(VMCR_VENG0)),
            IcvRegister::Igrpen1 => u32::from(self.vmcr_has(VMCR_VENG1)),
            IcvRegister::Eoir0 | IcvRegister::Eoir1 | IcvRegister::Dir => 0,
        })
    }

    /// `ICV_CTLR_EL1` (12.3.5): `PRIbits` from `ICH_VTR_EL2`, `EOImode` and
    /// `CBPR` from `ICH_VMCR_EL2`, and the rest 0: 16 bits of INTID, no
    /// SEIs, no affinity level 3, no range selector, no extended INTIDs.
    fn ctlr(&self) -> u32 {
        let pri_bits = u32::from(self.shape.priority_bits().count()) - 1;
        let eoimode = u32::from(self.vmcr_has(VMCR_VEOIM));
        let cbpr = u32::from(self.vmcr_has(VMCR_VCBPR));
        pri_bits << 8 | eoimode << 1 | cbpr
    }

    /// An `MSR` of `value` to `reg`, and the physical interrupt a hardware
    /// List register's deactivation deactivates. A read-only register
    /// ignores it, which no processor asks for.
    pub(crate) fn virtual_write(&mut self, reg: IcvRegister, value: u64) -> PhysicalDeactivation {
        let bits = self.shape.priority_bits();
        let low = value as u32;
        let set = |vmcr: u64, bit: u64, on: bool| if on { vmcr | bit } else { vmcr & !bit };
        match reg {
            // 12.20.1 `ICC_PMR_EL1`: `ICH_VMCR_EL2.VPMR = data<7:0> AND
            // VPRIMask()`.
            IcvRegister::Pmr => {
                let vpmr = u64::from(low as u8 & bits.mask());
                self.vmcr = (self.vmcr & !(0xFF << VMCR_VPMR_SHIFT)) | vpmr << VMCR_VPMR_SHIFT;
            }
            IcvRegister::Bpr0 => {
                let point = u64::from((low as u8 & 7).max(bits.min_bpr()));
                self.vmcr = (self.vmcr & !(7 << VMCR_VBPR0_SHIFT)) | point << VMCR_VBPR0_SHIFT;
            }
            IcvRegister::Bpr1 => {
                if !self.vmcr_has(VMCR_VCBPR) {
                    let point = u64::from((low as u8 & 7).max(bits.min_abpr()));
                    self.vmcr = (self.vmcr & !(7 << VMCR_VBPR1_SHIFT)) | point << VMCR_VBPR1_SHIFT;
                }
            }
            IcvRegister::Ap0r(n) => self.ap0r[n.get()] = low & bits.apr_bits(n.get()),
            IcvRegister::Ap1r(n) => self.ap1r[n.get()] = low & bits.apr_bits(n.get()),
            IcvRegister::Ctlr => {
                let vmcr = set(self.vmcr, VMCR_VEOIM, low & 2 != 0);
                self.vmcr = set(vmcr, VMCR_VCBPR, low & 1 != 0);
            }
            IcvRegister::Igrpen0 => self.vmcr = set(self.vmcr, VMCR_VENG0, low & 1 != 0),
            IcvRegister::Igrpen1 => self.vmcr = set(self.vmcr, VMCR_VENG1, low & 1 != 0),
            IcvRegister::Eoir0 => return self.end_of_interrupt(Group::Zero, value),
            IcvRegister::Eoir1 => return self.end_of_interrupt(Group::One, value),
            IcvRegister::Dir => return self.deactivate(value),
            IcvRegister::Iar0
            | IcvRegister::Iar1
            | IcvRegister::Hppir0
            | IcvRegister::Hppir1
            | IcvRegister::Rpr => {}
        }
        PhysicalDeactivation(None)
    }

    /// 12.20.1 `VirtualReadIAR0` (`group` 0) or `VirtualReadIAR1`: the
    /// highest priority virtual interrupt, if it can be signalled and is of
    /// the register's group, made active, its preemption level set in its
    /// group's active priority register; 1023 otherwise. A special INTID
    /// in a List register (UNPREDICTABLE) is returned as it is and changes
    /// nothing.
    fn acknowledge(&mut self, group: Group) -> u32 {
        let Some(n) = self.signalled() else {
            return SPURIOUS;
        };
        if self.group(n) != group {
            return SPURIOUS;
        }
        let vintid = self.vintid(n);
        if (FIRST_SPECIAL..FIRST_SPECIAL + 4).contains(&vintid) {
            return vintid;
        }
        self.lr[n] = (self.lr[n] & !LrState::ActivePending.bits()) | LrState::Active.bits();
        let level = self.level(self.priority(n));
        let bank = match group {
            Group::Zero => &mut self.ap0r,
            Group::One => &mut self.ap1r,
        };
        bank[level / 32] |= 1 << (level % 32);
        vintid
    }

    /// 12.20.1 `VirtualReadHPPIR0` or `VirtualReadHPPIR1`: the highest
    /// priority virtual interrupt if it is of the register's group,
    /// whatever its priority; 1023 otherwise.
    fn highest_pending(&self, group: Group) -> u32 {
        match self.highest_priority() {
            Some(n) if self.group(n) == group => self.vintid(n),
            Some(_) | None => SPURIOUS,
        }
    }

    /// 2.2.3 `VirtualIdentifierValid`: the INTID `data` names, if it is
    /// valid — no bit set above the 16 implemented, not an LPI unless
    /// `lpis` allows one, not special. The cases the pseudocode makes
    /// UNPREDICTABLE are taken as invalid.
    fn valid_identifier(data: u64, lpis: Lpis) -> Option<u32> {
        if data >> VID_BITS != 0 {
            return None;
        }
        let intid = data as u32;
        let lpi = intid >= FIRST_LPI;
        match lpis {
            Lpis::Allowed => {}
            Lpis::Refused if lpi => return None,
            Lpis::Refused => {}
        }
        (!(FIRST_SPECIAL..FIRST_SPECIAL + 4).contains(&intid)).then_some(intid)
    }

    /// 6.4 `FindActiveVirtualInterrupt`: the List register active, or
    /// active and pending, whose `vINTID` is `intid`.
    fn find_active(&self, intid: u32) -> Option<usize> {
        (0..self.shape.list_registers())
            .find(|&n| LrState::of(self.lr[n]).is_active() && self.vintid(n) == intid)
    }

    /// The `ICH_HCR_EL2.EOIcount` increment of a deactivation that finds no
    /// List register, wrapping at 31.
    fn count_unmatched_eoi(&mut self) {
        let count = ((self.hcr >> HCR_EOICOUNT_SHIFT) + 1) & 0x1F;
        self.hcr = (self.hcr & !HCR_EOICOUNT) | count << HCR_EOICOUNT_SHIFT;
    }

    /// 6.4 `VPriorityDrop`: clear the highest active preemption level,
    /// Group 0's where both groups have it.
    fn drop_priority(&mut self) {
        for n in 0..MAX_APRS {
            let (g0, g1) = (self.ap0r[n], self.ap1r[n]);
            if g0 | g1 == 0 {
                continue;
            }
            let bit = (g0 | g1) & (g0 | g1).wrapping_neg();
            if g0 & bit != 0 {
                self.ap0r[n] &= !bit;
            } else {
                self.ap1r[n] &= !bit;
            }
            return;
        }
    }

    /// 12.20.1 `VirtualWriteEOIR0` (`group` 0) or `VirtualWriteEOIR1`,
    /// written `data`: with a valid INTID and an active priority, the
    /// priority drop, then the deactivation, unless `VEOIM` splits the two,
    /// of the active List register of the INTID, where it is of the
    /// register's group and of the highest active priority; with no List
    /// register for it, and `VEOIM` 0, an `EOIcount` increment instead.
    fn end_of_interrupt(&mut self, group: Group, data: u64) -> PhysicalDeactivation {
        let Some(intid) = Self::valid_identifier(data, Lpis::Allowed) else {
            return PhysicalDeactivation(None);
        };
        let Some(highest) = self.highest_active_level() else {
            return PhysicalDeactivation(None);
        };
        self.drop_priority();
        let Some(n) = self.find_active(intid) else {
            if intid < FIRST_LPI && !self.vmcr_has(VMCR_VEOIM) {
                self.count_unmatched_eoi();
            }
            return PhysicalDeactivation(None);
        };
        if self.group(n) != group || self.level(self.priority(n)) != highest {
            return PhysicalDeactivation(None);
        }
        if self.vmcr_has(VMCR_VEOIM) && intid < FIRST_LPI {
            return PhysicalDeactivation(None);
        }
        self.deactivate_lr(n)
    }

    /// 12.20.1 `VirtualWriteDIR`, written `data`: with `VEOIM` set and a
    /// valid INTID other than an LPI, the deactivation of its active List
    /// register, or with none an `EOIcount` increment.
    fn deactivate(&mut self, data: u64) -> PhysicalDeactivation {
        if !self.vmcr_has(VMCR_VEOIM) {
            return PhysicalDeactivation(None);
        }
        let Some(intid) = Self::valid_identifier(data, Lpis::Refused) else {
            return PhysicalDeactivation(None);
        };
        match self.find_active(intid) {
            Some(n) => self.deactivate_lr(n),
            None => {
                self.count_unmatched_eoi();
                PhysicalDeactivation(None)
            }
        }
    }

    /// Deactivate List register `n` (it stays pending if it was active and
    /// pending), and for a hardware interrupt the physical interrupt its
    /// `pINTID` names, below 1020.
    fn deactivate_lr(&mut self, n: usize) -> PhysicalDeactivation {
        let lr = self.lr[n];
        let rest = match LrState::of(lr) {
            LrState::ActivePending => LrState::Pending,
            LrState::Invalid | LrState::Pending | LrState::Active => LrState::Invalid,
        };
        self.lr[n] = (lr & !LrState::ActivePending.bits()) | rest.bits();
        if lr & LR_HW == 0 {
            return PhysicalDeactivation(None);
        }
        let physical = ((lr & LR_PINTID) >> LR_PINTID_SHIFT) as u32;
        PhysicalDeactivation((physical < FIRST_SPECIAL).then_some(physical))
    }

    // ----- snapshots -----

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u64(self.hcr)?;
        w.write_u64(self.vmcr)?;
        for word in self.ap0r.iter().chain(self.ap1r.iter()) {
            w.write_u32(*word)?;
        }
        for lr in self.list() {
            w.write_u64(*lr)?;
        }
        w.write_u64(self.sre_el2)
    }

    /// Reads the interface's section of the image, of an interface of
    /// `shape`, and refuses one whose registers hold bits they cannot:
    /// nothing changes before the caller accepts the result.
    pub(crate) fn restore<R: SnapRead>(
        shape: VirtualInterfaceShape,
        r: &mut R,
    ) -> Result<Self, SnapError> {
        let mut restored = Self::new(shape);
        let hcr = r.read_u64()?;
        let vmcr = r.read_u64()?;
        let mut ap0r = [0u32; MAX_APRS];
        let mut ap1r = [0u32; MAX_APRS];
        for word in ap0r.iter_mut().chain(ap1r.iter_mut()) {
            *word = r.read_u32()?;
        }
        let mut lr = [0u64; LrIndex::COUNT];
        for slot in lr.iter_mut().take(shape.list_registers()) {
            *slot = r.read_u64()?;
        }
        let sre_el2 = r.read_u64()?;
        if hcr & !restored.hcr_mask() != 0 {
            return Err(SnapError::Invalid("ICH_HCR_EL2 with a bit no field holds"));
        }
        restored.set_vmcr(vmcr);
        if restored.vmcr != vmcr {
            return Err(SnapError::Invalid(
                "ICH_VMCR_EL2 with a value its fields cannot hold",
            ));
        }
        let bits = shape.priority_bits();
        if (0..MAX_APRS).any(|n| (ap0r[n] | ap1r[n]) & !bits.apr_bits(n) != 0) {
            return Err(SnapError::Invalid(
                "virtual active priority of a preemption level that does not exist",
            ));
        }
        if lr.iter().any(|&lr| lr & !restored.lr_mask() != 0) {
            return Err(SnapError::Invalid(
                "List register with a bit no field holds",
            ));
        }
        if sre_el2 & !(SRE_EL2_RAO | SRE_EL2_ENABLE) != 0 || sre_el2 & SRE_EL2_RAO != SRE_EL2_RAO {
            return Err(SnapError::Invalid("ICC_SRE_EL2 its fields cannot hold"));
        }
        restored.hcr = hcr;
        restored.ap0r = ap0r;
        restored.ap1r = ap1r;
        restored.lr = lr;
        restored.sre_el2 = sre_el2;
        Ok(restored)
    }
}

/// Whether an INTID written may name an LPI ([`VirtualInterface::valid_identifier`]).
///
/// Closed (R5).
#[derive(Clone, Copy)]
enum Lpis {
    Allowed,
    Refused,
}

#[cfg(test)]
#[path = "../../tests/unit/gicv3_virt.rs"]
mod tests;
