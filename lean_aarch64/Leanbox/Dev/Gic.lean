/-
  GICv2 distributor and CPU interface for one processor, without the
  Security Extensions, plus the GICv2m MSI frame: an exact port of
  rusty_box_aarch64/src/devices/gicv2.rs (`GicV2`), for lockstep runs
  against the Rust machine. Snapshot save/restore is not ported.

  Shape (gicv2.rs module docs): with one processor every per-interrupt
  state is one bit, kept as 9 little-endian 32-bit words (`Bits`); an SGI's
  single source is its pending bit. Interrupt state is recomputed after
  every register write, every acknowledge and every input line change
  (`GicV2::update`), and the result is cached in `currentPending`,
  `irqLine` and `fiqLine`, which `irqOut`/`fiqOut` return.

  Widths are `log2 : UInt8` (0 byte, 1 half, 2 word, 3 dword: bus.rs
  `AccessWidth::from_log2`); a caller passes only 0..=3, and any larger
  value is treated as the dword. Offsets are `UInt64` relative to the
  region's base (board.rs `GIC_DIST_BASE`/`GIC_DIST_SIZE` 0x1000,
  `GIC_CPU_BASE`/`GIC_CPU_SIZE` 0x2000, `GIC_V2M_BASE`/`GIC_V2M_SIZE`
  0x1000); the caller decides which region an address is in.

  Width masking: board.rs `VirtBoard::write` masks a written value with
  `AccessWidth::value_mask` before the device sees it, and `VirtBoard::read`
  masks what `read_outside_ram` returns. Both maskings are applied here, in
  `distWrite`/`cpuWrite`/`v2mWrite` and in the values `distRead`/`cpuRead`/
  `v2mRead` return, so these functions give what the guest sees; masking
  again in a caller changes nothing.
-/

namespace Leanbox.Dev.Gic

/-! ## Constants (gicv2.rs) -/

-- gicv2.rs NUM_IRQ: SGIs, PPIs and QEMU `virt`'s 256 SPIs.
def NUM_IRQ : Nat := 288
-- gicv2.rs WORDS
def WORDS : Nat := NUM_IRQ / 32
-- gicv2.rs NR_SGIS
def NR_SGIS : Nat := 16
-- gicv2.rs MAXIRQ: interrupt IDs from here up are special.
def MAXIRQ : UInt32 := 1020
-- gicv2.rs SPURIOUS / SPURIOUS_GROUP1
def SPURIOUS : UInt32 := 1023
def SPURIOUS_GROUP1 : UInt32 := 1022
-- gicv2.rs IDLE_PRIORITY: the running priority while nothing is active.
def IDLE_PRIORITY : UInt16 := 0x100
-- gicv2.rs NR_APRS
def NR_APRS : Nat := 4
-- gicv2.rs MIN_BPR / MIN_ABPR
def MIN_BPR : UInt8 := 0
def MIN_ABPR : UInt8 := MIN_BPR + 1

-- gicv2.rs GICD_CTLR_EN_GRP0 / GICD_CTLR_EN_GRP1
def GICD_CTLR_EN_GRP0 : UInt32 := 1
def GICD_CTLR_EN_GRP1 : UInt32 := 2
-- gicv2.rs GICC_CTLR_ACK_CTL / FIQ_EN / CBPR / EOIMODE / V2_MASK
def GICC_CTLR_ACK_CTL : UInt32 := (1 : UInt32) <<< 2
def GICC_CTLR_FIQ_EN : UInt32 := (1 : UInt32) <<< 3
def GICC_CTLR_CBPR : UInt32 := (1 : UInt32) <<< 4
def GICC_CTLR_EOIMODE : UInt32 := (1 : UInt32) <<< 9
def GICC_CTLR_V2_MASK : UInt32 := 0x21F
-- gicv2.rs SGI_SOURCE_CPU0: processor 0's bit in a GICD_CPENDSGIRn/SPENDSGIRn byte.
def SGI_SOURCE_CPU0 : UInt8 := 1

-- gicv2.rs PERIPHERAL_ID (QEMU `gic_id_gicv2`), for offsets 0xFD0..=0xFFC.
def PERIPHERAL_ID : ByteArray :=
  ⟨#[0x04, 0x00, 0x00, 0x00, 0x90, 0xB4, 0x2B, 0x00, 0x0D, 0xF0, 0x05, 0xB1]⟩

-- gicv2.rs V2M_BASE_SPI / V2M_NUM_SPI: 64 SPIs from 80.
def V2M_BASE_SPI : UInt32 := 80
def V2M_NUM_SPI : UInt32 := 64
-- gicv2.rs V2M_MSI_TYPER / V2M_MSI_SETSPI_NS / V2M_MSI_IIDR
def V2M_MSI_TYPER : UInt64 := 0x008
def V2M_MSI_SETSPI_NS : UInt64 := 0x040
def V2M_MSI_IIDR : UInt64 := 0xFCC

-- The frame's `MSI_IIDR` under the default identity: identity.rs
-- `IdentityNames::RUSTY_BOX` gives `msi_frame_iidr: MsiFrameIidr::NONE`
-- (`MsiFrameIidr(0)`: no product, revision 0, no implementer), which
-- `BoardIdentity::RUSTY_BOX` copies and board.rs hands to
-- `GicV2::v2m_read` as `self.msi_frame_iidr`.
def MSI_FRAME_IIDR_RUSTY_BOX : UInt32 := 0

/-! ## Access widths (bus.rs AccessWidth) -/

-- bus.rs AccessWidth::value_mask, for the width `1 <<< log2` bytes.
@[inline] def valueMask (log2 : UInt8) : UInt64 :=
  if log2 == 0 then 0xFF
  else if log2 == 1 then 0xFFFF
  else if log2 == 2 then 0xFFFF_FFFF
  else 0xFFFF_FFFF_FFFF_FFFF

/-! ## Bits (gicv2.rs `Bits`): 9 words, interrupt `i` at bit `i % 32` of word `i / 32` -/

-- The mask of interrupt `i` in its word.
@[inline] def maskOf (i : Nat) : UInt32 := (1 : UInt32) <<< (i &&& 31).toUInt32

-- gicv2.rs Bits::get
@[inline] def bitIn (b : Array UInt32) (i : Nat) : Bool := b[i >>> 5]! &&& maskOf i != 0

-- u32::trailing_zeros of a nonzero word.
@[inline] def ctz32 (x : UInt32) : UInt32 := (Nat.log2 (x &&& (0 - x)).toNat).toUInt32

-- The `GICD_ICFGR` byte of four interrupts' edge bits `n` (bit k): bit 2k+1
-- of each pair is edge, bit 2k reads as zero (gicv2.rs GicV2::dist_readb).
@[inline] def spreadEdges (n : UInt32) : UInt8 :=
  (((n &&& 1) <<< 1) ||| ((n &&& 2) <<< 2) ||| ((n &&& 4) <<< 3) ||| ((n &&& 8) <<< 4)).toUInt8

-- The four edge bits a `GICD_ICFGR` byte `v` writes (bit 2k+1 to bit k).
@[inline] def gatherEdges (v : UInt8) : UInt32 :=
  let w : UInt32 := v.toUInt32
  ((w >>> 1) &&& (1 : UInt32)) ||| ((w >>> 2) &&& (2 : UInt32))
    ||| ((w >>> 3) &&& (4 : UInt32)) ||| ((w >>> 4) &&& (8 : UInt32))

/-- The two register sets of the CPU interface (gicv2.rs `CpuView`):
`primary` is `GICC_IAR`/`GICC_EOIR`/`GICC_HPPIR`, `group1Alias` is
`GICC_AIAR`/`GICC_AEOIR`/`GICC_AHPPIR`. -/
inductive CpuView where
  | primary
  | group1Alias
deriving DecidableEq, BEq

/-- The six one-bit-per-interrupt sets of gicv2.rs `GicV2`. -/
inductive Bank where
  | enabled
  | pending
  | active
  | group1
  | level
  | edge

/-! ## State (gicv2.rs `struct GicV2`) -/

structure State where
  /-- `GICD_CTLR`. -/
  distCtlr : UInt32
  enabled : Array UInt32
  /-- QEMU `irq_state[].pending`: set by an edge, software or an SGI. A
  level-sensitive interrupt is also pending while its line is high. -/
  pending : Array UInt32
  active : Array UInt32
  group1 : Array UInt32
  /-- The level of each input line. -/
  level : Array UInt32
  /-- Edge-triggered (`GICD_ICFGR` bit 2n+1). -/
  edge : Array UInt32
  /-- `GICD_IPRIORITYR`, one byte per interrupt (288 bytes). -/
  priority : ByteArray
  /-- `GICC_CTLR`. -/
  cpuCtlr : UInt32
  pmr : UInt8
  bpr : UInt8
  abpr : UInt8
  /-- QEMU `running_priority`: the group priority of the highest-priority
  active interrupt, or `IDLE_PRIORITY`. -/
  runningPriority : UInt16
  /-- QEMU `apr`: one bit per active Group 0 preemption level (4 words). -/
  apr : Array UInt32
  /-- QEMU `nsapr`: one bit per active Group 1 preemption level (4 words). -/
  nsapr : Array UInt32
  /-- QEMU `current_pending`, recomputed by `update`. -/
  currentPending : UInt32
  /-- gicv2.rs `irq_out`, recomputed by `update`. -/
  irqLine : Bool
  /-- gicv2.rs `fiq_out`, recomputed by `update`. -/
  fiqLine : Bool
deriving Inhabited

-- gicv2.rs GicV2::new (QEMU `arm_gic_common_reset_hold`): SGIs enabled and
-- edge-triggered, every other interrupt disabled, level-sensitive, Group 0
-- and at priority 0.
def State.new : State :=
  { distCtlr := 0
    enabled := #[0xFFFF, 0, 0, 0, 0, 0, 0, 0, 0]
    pending := Array.replicate WORDS 0
    active := Array.replicate WORDS 0
    group1 := Array.replicate WORDS 0
    level := Array.replicate WORDS 0
    edge := #[0xFFFF, 0, 0, 0, 0, 0, 0, 0, 0]
    priority := ⟨Array.replicate NUM_IRQ 0⟩
    cpuCtlr := 0
    pmr := 0
    bpr := MIN_BPR
    abpr := MIN_ABPR
    runningPriority := IDLE_PRIORITY
    apr := Array.replicate NR_APRS 0
    nsapr := Array.replicate NR_APRS 0
    currentPending := SPURIOUS
    irqLine := false
    fiqLine := false }

namespace State

-- gicv2.rs GicV2::irq_out: the IRQ output to the processor.
@[inline] def irqOut (s : State) : Bool := s.irqLine

-- gicv2.rs GicV2::fiq_out: the FIQ output to the processor.
@[inline] def fiqOut (s : State) : Bool := s.fiqLine

/-! ### Field updates

Each takes the array out of the state before changing it, so that a
uniquely owned state changes its array in place. -/

@[inline] def bank (s : State) : Bank → Array UInt32
  | .enabled => s.enabled
  | .pending => s.pending
  | .active => s.active
  | .group1 => s.group1
  | .level => s.level
  | .edge => s.edge

-- Word `w` of set `b` becomes `v`.
def setBankWord (s : State) (b : Bank) (w : Nat) (v : UInt32) : State :=
  match b with
  | .enabled =>
    let a := s.enabled
    let s := { s with enabled := #[] }
    { s with enabled := a.set! w v }
  | .pending =>
    let a := s.pending
    let s := { s with pending := #[] }
    { s with pending := a.set! w v }
  | .active =>
    let a := s.active
    let s := { s with active := #[] }
    { s with active := a.set! w v }
  | .group1 =>
    let a := s.group1
    let s := { s with group1 := #[] }
    { s with group1 := a.set! w v }
  | .level =>
    let a := s.level
    let s := { s with level := #[] }
    { s with level := a.set! w v }
  | .edge =>
    let a := s.edge
    let s := { s with edge := #[] }
    { s with edge := a.set! w v }

-- gicv2.rs Bits::set
@[inline] def setBit (s : State) (b : Bank) (i : Nat) (v : Bool) : State :=
  let w := i >>> 5
  let m := maskOf i
  let x := (s.bank b)[w]!
  s.setBankWord b w (if v then x ||| m else x &&& ~~~m)

-- gicv2.rs GicV2::set_each with `value = true`, for the byte of interrupts
-- `irq..irq + 8` (`irq` a multiple of 8 below NUM_IRQ, so all eight exist).
@[inline] def orByte (s : State) (b : Bank) (irq : Nat) (v : UInt8) : State :=
  let w := irq >>> 5
  let x := (s.bank b)[w]!
  s.setBankWord b w (x ||| (v.toUInt32 <<< (irq &&& 31).toUInt32))

-- gicv2.rs GicV2::set_each with `value = false`, as `orByte`.
@[inline] def andNotByte (s : State) (b : Bank) (irq : Nat) (v : UInt8) : State :=
  let w := irq >>> 5
  let x := (s.bank b)[w]!
  s.setBankWord b w (x &&& ~~~(v.toUInt32 <<< (irq &&& 31).toUInt32))

-- Interrupts `irq..irq + 8` take the bits of `v` (gicv2.rs
-- GicV2::dist_writeb, `GICD_IGROUPR`).
@[inline] def assignByte (s : State) (b : Bank) (irq : Nat) (v : UInt8) : State :=
  let w := irq >>> 5
  let sh := (irq &&& 31).toUInt32
  let x := (s.bank b)[w]!
  s.setBankWord b w ((x &&& ~~~((0xFF : UInt32) <<< sh)) ||| (v.toUInt32 <<< sh))

-- Interrupts `first..first + 4` take the bits of `n` (`first` a multiple of
-- 4 below NUM_IRQ, so all four exist).
@[inline] def assignNibble (s : State) (b : Bank) (first : Nat) (n : UInt32) : State :=
  let w := first >>> 5
  let sh := (first &&& 31).toUInt32
  let x := (s.bank b)[w]!
  s.setBankWord b w ((x &&& ~~~((0xF : UInt32) <<< sh)) ||| (n <<< sh))

def setPriority (s : State) (i : Nat) (v : UInt8) : State :=
  let p := s.priority
  let s := { s with priority := ByteArray.empty }
  { s with priority := p.set! i v }

-- Word `w` of `nsapr` (group 1) or `apr` (group 0) becomes `v`.
def setAprWord (s : State) (group1 : Bool) (w : Nat) (v : UInt32) : State :=
  if group1 then
    let a := s.nsapr
    let s := { s with nsapr := #[] }
    { s with nsapr := a.set! w v }
  else
    let a := s.apr
    let s := { s with apr := #[] }
    { s with apr := a.set! w v }

/-! ### Choosing the signalled interrupt -/

-- gicv2.rs GicV2::test_pending (QEMU `gic_test_pending`)
@[inline] def testPending (s : State) (i : Nat) : Bool :=
  bitIn s.pending i || (!bitIn s.edge i && bitIn s.level i)

-- The search key of a candidate: its priority above its ID, so the least
-- key is the numerically lowest priority, the lowest ID among equals.
def NO_CANDIDATE : UInt32 := IDLE_PRIORITY.toUInt32 <<< 16

-- The inner loop of gicv2.rs GicV2::best_irq over one word's candidates
-- `c` (bits of interrupts `base..base + 32`); `fuel` is 32, one bit cleared
-- per step.
def scanCandidates (pri : ByteArray) (base : UInt32) (c : UInt32) (key : UInt32) :
    Nat → UInt32
  | 0 => key
  | fuel + 1 =>
    if c == 0 then key
    else
      let i := base + ctz32 c
      let k := (pri[i.toNat]!.toUInt32 <<< 16) ||| i
      scanCandidates pri base (c &&& (c - 1)) (if k < key then k else key) fuel

-- The outer loop of gicv2.rs GicV2::best_irq over words `w..WORDS`; `g0`/`g1`
-- are all-ones when Group 0/Group 1 take part.
def scanWords (s : State) (g0 g1 : UInt32) (w : Nat) (key : UInt32) : Nat → UInt32
  | 0 => key
  | fuel + 1 =>
    let pend := s.pending[w]! ||| (s.level[w]! &&& ~~~s.edge[w]!)
    let grp := s.group1[w]!
    let inGroups := (grp &&& g1) ||| (~~~grp &&& g0)
    let c := s.enabled[w]! &&& pend &&& ~~~s.active[w]! &&& inGroups
    scanWords s g0 g1 (w + 1) (scanCandidates s.priority (w * 32).toUInt32 c key 32) fuel

-- gicv2.rs GicV2::best_irq (QEMU `gic_get_best_irq`): the enabled, pending,
-- inactive interrupt of `groups` with the numerically lowest priority, the
-- lowest ID among equals; NUM_IRQ when there is none.
def bestIrq (s : State) (groups : UInt32) : Nat :=
  let g0 : UInt32 := if groups &&& GICD_CTLR_EN_GRP0 != 0 then 0xFFFF_FFFF else 0
  let g1 : UInt32 := if groups &&& GICD_CTLR_EN_GRP1 != 0 then 0xFFFF_FFFF else 0
  let key := scanWords s g0 g1 0 NO_CANDIDATE WORDS
  if key < NO_CANDIDATE then (key &&& 0xFFFF).toNat else NUM_IRQ

-- gicv2.rs GicV2::signalled_groups: the groups the distributor forwards and
-- the CPU interface signals.
@[inline] def signalledGroups (s : State) : UInt32 :=
  s.distCtlr &&& s.cpuCtlr &&& (GICD_CTLR_EN_GRP0 ||| GICD_CTLR_EN_GRP1)

-- gicv2.rs GicV2::group_priority (QEMU `gic_get_group_priority`): the
-- priority's bits above the binary point, GICC_ABPR minus one for a Group 1
-- interrupt unless CBPR is set, GICC_BPR otherwise.
def groupPriority (s : State) (i : Nat) : UInt8 :=
  let bpr := if s.cpuCtlr &&& GICC_CTLR_CBPR == 0 && bitIn s.group1 i then s.abpr - 1 else s.bpr
  s.priority[i]! &&& ((0xFF : UInt32) <<< ((bpr &&& 7) + 1).toUInt32).toUInt8

-- gicv2.rs GicV2::preempts: the group priority is higher than the running
-- priority.
@[inline] def preempts (s : State) (i : Nat) : Bool :=
  (s.groupPriority i).toUInt16 < s.runningPriority

-- The current pending interrupt gicv2.rs GicV2::update chooses: the best of
-- the signalled groups if it beats the priority mask, else SPURIOUS.
def chooseCurrent (s : State) : UInt32 :=
  let groups := s.signalledGroups
  if groups == 0 then SPURIOUS
  else
    let best := s.bestIrq groups
    if best ≥ NUM_IRQ then SPURIOUS
    else if s.priority[best]! ≥ s.pmr then SPURIOUS
    else best.toUInt32

-- gicv2.rs GicV2::update (QEMU `gic_update_internal`): the current pending
-- interrupt is signalled if it also preempts the running priority, as FIQ if
-- it is Group 0 and FIQEn is set, as IRQ otherwise.
def update (s : State) : State :=
  let cp := s.chooseCurrent
  if cp == SPURIOUS then { s with currentPending := SPURIOUS, irqLine := false, fiqLine := false }
  else
    let i := cp.toNat
    if !s.preempts i then { s with currentPending := cp, irqLine := false, fiqLine := false }
    else
      let fiq := !bitIn s.group1 i && s.cpuCtlr &&& GICC_CTLR_FIQ_EN != 0
      { s with currentPending := cp, irqLine := !fiq, fiqLine := fiq }

-- gicv2.rs GicV2::set_line (QEMU `gic_set_irq` with `gic_set_irq_generic`):
-- drive input line `intid` (a PPI or SPI) to `level`. An SGI ID, an ID past
-- the last interrupt, or the level the line already has changes nothing.
def setLine (s : State) (intid : UInt32) (level : Bool) : State :=
  let i := intid.toNat
  if !(NR_SGIS ≤ i && i < NUM_IRQ) || bitIn s.level i == level then s
  else
    let s := s.setBit .level i level
    let s := if level && bitIn s.edge i then s.setBit .pending i true else s
    s.update

/-! ### Distributor -/

-- gicv2.rs GicV2::bank_irq: the first interrupt a byte of a
-- one-bit-per-interrupt bank at `base` describes; it exists if below NUM_IRQ.
@[inline] def bankIrq (off base : UInt64) : Nat := ((off - base) * 8).toNat

-- gicv2.rs Bits::byte, for `irq` a multiple of 8 below NUM_IRQ.
@[inline] def byteOf (b : Array UInt32) (irq : Nat) : UInt8 :=
  (b[irq >>> 5]! >>> (irq &&& 31).toUInt32).toUInt8

-- gicv2.rs GicV2::sgi_sources: SGI `sgi`'s GICD_CPENDSGIRn/SPENDSGIRn byte.
@[inline] def sgiSources (s : State) (sgi : Nat) : UInt8 :=
  if bitIn s.pending sgi then SGI_SOURCE_CPU0 else 0

-- A one-bit-per-interrupt bank's byte at `off` of a bank at `base`.
@[inline] def bankByte (b : Array UInt32) (off base : UInt64) : UInt8 :=
  let irq := bankIrq off base
  if irq < NUM_IRQ then byteOf b irq else 0

-- gicv2.rs GicV2::dist_readb (QEMU `gic_dist_readb`)
def distReadb (s : State) (off : UInt64) : UInt8 :=
  if off == 0x000 then s.distCtlr.toUInt8
  else if off == 0x004 then (NUM_IRQ / 32 - 1).toUInt8
  else if off == 0x008 then 0x3B
  else if off == 0x009 then 0x04
  else if 0x080 ≤ off && off ≤ 0x0FF then bankByte s.group1 off 0x080
  else if 0x100 ≤ off && off ≤ 0x17F then bankByte s.enabled off 0x100
  else if 0x180 ≤ off && off ≤ 0x1FF then bankByte s.enabled off 0x180
  else if 0x200 ≤ off && off ≤ 0x2FF then
    let irq := bankIrq off (if off < 0x280 then 0x200 else 0x280)
    if irq < NUM_IRQ then
      let w := irq >>> 5
      ((s.pending[w]! ||| (s.level[w]! &&& ~~~s.edge[w]!)) >>> (irq &&& 31).toUInt32).toUInt8
    else 0
  else if 0x300 ≤ off && off ≤ 0x37F then bankByte s.active off 0x300
  else if 0x380 ≤ off && off ≤ 0x3FF then bankByte s.active off 0x380
  else if 0x400 ≤ off && off ≤ 0x7FF then
    let i := (off - 0x400).toNat
    if i < NUM_IRQ then s.priority[i]! else 0
  -- GICD_ITARGETSR: RAZ/WI on a uniprocessor GIC.
  else if 0x800 ≤ off && off ≤ 0xBFF then 0
  else if 0xC00 ≤ off && off ≤ 0xEFF then
    let first := ((off - 0xC00) * 4).toNat
    if first < NUM_IRQ then
      spreadEdges ((s.edge[first >>> 5]! >>> (first &&& 31).toUInt32) &&& 0xF)
    else 0
  else if 0xF10 ≤ off && off ≤ 0xF1F then s.sgiSources (off - 0xF10).toNat
  else if 0xF20 ≤ off && off ≤ 0xF2F then s.sgiSources (off - 0xF20).toNat
  else if 0xFD0 ≤ off && off ≤ 0xFFF && off % 4 == 0 then
    PERIPHERAL_ID[((off - 0xFD0) >>> 2).toNat]!
  else 0

-- gicv2.rs GicV2::dist_readl: four byte reads, little-endian.
def distReadl (s : State) (off : UInt64) : UInt32 :=
  (s.distReadb off).toUInt32
    ||| ((s.distReadb (off + 1)).toUInt32 <<< 8)
    ||| ((s.distReadb (off + 2)).toUInt32 <<< 16)
    ||| ((s.distReadb (off + 3)).toUInt32 <<< 24)

-- gicv2.rs GicV2::dist_read: a byte, two bytes, a word of bytes, or two
-- words (`off` then `off + 4`). Reading changes nothing; the state is
-- returned for the shape every region's read shares. Masked to the width
-- as board.rs VirtBoard::read masks it.
def distRead (s : State) (off : UInt64) (log2 : UInt8) : State × UInt64 :=
  let v : UInt64 :=
    if log2 == 0 then (s.distReadb off).toUInt64
    else if log2 == 1 then
      (s.distReadb off).toUInt64 ||| ((s.distReadb (off + 1)).toUInt64 <<< 8)
    else if log2 == 2 then (s.distReadl off).toUInt64
    else (s.distReadl off).toUInt64 ||| ((s.distReadl (off + 4)).toUInt64 <<< 32)
  (s, v &&& valueMask log2)

-- gicv2.rs GicV2::dist_writeb (QEMU `gic_dist_writeb`)
def distWriteb (s : State) (off : UInt64) (v : UInt8) : State :=
  if off == 0x000 then { s with distCtlr := v.toUInt32 &&& (GICD_CTLR_EN_GRP0 ||| GICD_CTLR_EN_GRP1) }
  else if 0x080 ≤ off && off ≤ 0x0FF then
    let irq := bankIrq off 0x080
    if irq < NUM_IRQ then s.assignByte .group1 irq v else s
  else if 0x100 ≤ off && off ≤ 0x17F then
    let irq := bankIrq off 0x100
    -- SGIs cannot be disabled: any write enables them.
    if irq < NUM_IRQ then s.orByte .enabled irq (if irq < NR_SGIS then 0xFF else v) else s
  else if 0x180 ≤ off && off ≤ 0x1FF then
    let irq := bankIrq off 0x180
    if irq < NUM_IRQ then s.andNotByte .enabled irq (if irq < NR_SGIS then 0 else v) else s
  else if 0x200 ≤ off && off ≤ 0x27F then
    -- SGIs are made pending only through GICD_SGIR and GICD_SPENDSGIR.
    let irq := bankIrq off 0x200
    if irq < NUM_IRQ then s.orByte .pending irq (if irq < NR_SGIS then 0 else v) else s
  else if 0x280 ≤ off && off ≤ 0x2FF then
    let irq := bankIrq off 0x280
    if irq < NUM_IRQ then s.andNotByte .pending irq (if irq < NR_SGIS then 0 else v) else s
  else if 0x300 ≤ off && off ≤ 0x37F then
    let irq := bankIrq off 0x300
    if irq < NUM_IRQ then s.orByte .active irq v else s
  else if 0x380 ≤ off && off ≤ 0x3FF then
    let irq := bankIrq off 0x380
    if irq < NUM_IRQ then s.andNotByte .active irq v else s
  -- QEMU `gic_dist_set_priority`: with 8 priority bits nothing is masked off.
  else if 0x400 ≤ off && off ≤ 0x7FF then
    let i := (off - 0x400).toNat
    if i < NUM_IRQ then s.setPriority i v else s
  else if 0xC00 ≤ off && off ≤ 0xEFF then
    let first := ((off - 0xC00) * 4).toNat
    if first < NUM_IRQ then
      -- SGIs stay edge-triggered.
      let v := if first < NR_SGIS then v ||| 0xAA else v
      s.assignNibble .edge first (gatherEdges v)
    else s
  -- GICD_CPENDSGIRn and GICD_SPENDSGIRn: a 1 removes or adds the SGI's
  -- pending state from that source processor, a 0 has no effect; only
  -- processor 0's bit exists.
  else if 0xF10 ≤ off && off ≤ 0xF1F && v &&& SGI_SOURCE_CPU0 != 0 then
    s.setBit .pending (off - 0xF10).toNat false
  else if 0xF20 ≤ off && off ≤ 0xF2F && v &&& SGI_SOURCE_CPU0 != 0 then
    s.setBit .pending (off - 0xF20).toNat true
  -- GICD_CTLR's upper bytes, GICD_ITARGETSR, the SGI bits of processors that
  -- do not exist, and the rest: ignored.
  else s

-- gicv2.rs GicV2::dist_writew: two byte writes.
def distWritew (s : State) (off : UInt64) (v : UInt16) : State :=
  (s.distWriteb off v.toUInt8).distWriteb (off + 1) (v >>> 8).toUInt8

-- gicv2.rs GicV2::write_sgir (`GICD_SGIR`, from processor 0). The target
-- filter "all others" names only processors that do not exist; the
-- reserved filter 3 names all eight, this one included.
def writeSgir (s : State) (v : UInt32) : State :=
  let sgi := (v &&& 0xF).toNat
  let filter := (v >>> 24) &&& 3
  let targets : UInt32 :=
    if filter == 0 then (v >>> 16) &&& 0xFF
    else if filter == 1 then 0xFF ^^^ 1
    else if filter == 2 then 1
    else 0xFF
  if targets &&& 1 != 0 then s.setBit .pending sgi true else s

-- gicv2.rs GicV2::dist_writel (QEMU `gic_dist_writel`): GICD_SGIR is
-- written only whole; any other word is two half-word writes.
def distWritel (s : State) (off : UInt64) (v : UInt32) : State :=
  if off == 0xF00 then s.writeSgir v
  else (s.distWritew off v.toUInt16).distWritew (off + 2) (v >>> 16).toUInt16

-- gicv2.rs GicV2::dist_write, then GicV2::update. The value is first masked
-- to the width, as board.rs VirtBoard::write masks it.
def distWrite (s : State) (off : UInt64) (log2 : UInt8) (v : UInt64) : State :=
  let v := v &&& valueMask log2
  let s :=
    if log2 == 0 then s.distWriteb off v.toUInt8
    else if log2 == 1 then s.distWritew off v.toUInt16
    else if log2 == 2 then s.distWritel off v.toUInt32
    else (s.distWritel off v.toUInt32).distWritel (off + 4) (v >>> 32).toUInt32
  s.update

/-! ### CPU interface -/

-- gicv2.rs GicV2::current_pending_irq (QEMU `gic_get_current_pending_irq`):
-- the GICC_HPPIR or GICC_AHPPIR value. Through the primary registers a
-- Group 1 interrupt reads as 1022 unless AckCtl is set; through the Group 1
-- aliases a Group 0 interrupt reads as 1023.
def currentPendingIrq (s : State) (view : CpuView) : UInt32 :=
  let irq := s.currentPending
  if irq ≥ MAXIRQ then irq
  else
    let g1 := bitIn s.group1 irq.toNat
    match view with
    | .primary => if g1 && s.cpuCtlr &&& GICC_CTLR_ACK_CTL == 0 then SPURIOUS_GROUP1 else irq
    | .group1Alias => if !g1 then SPURIOUS else irq

-- gicv2.rs GicV2::activate (QEMU `gic_activate_irq`)
def activate (s : State) (i : Nat) : State :=
  let prio := s.groupPriority i
  let lvl := (prio >>> (MIN_BPR + 1)).toNat
  let g1 := bitIn s.group1 i
  let w := lvl >>> 5
  let x := (if g1 then s.nsapr else s.apr)[w]!
  let s := s.setAprWord g1 w (x ||| maskOf lvl)
  let s := { s with runningPriority := prio.toUInt16 }
  s.setBit .active i true

-- One active-priority word's contribution to gicv2.rs
-- GicV2::prio_from_apr_bits: the priority of its lowest set bit.
@[inline] def aprPriority (i : Nat) (a : UInt32) : UInt16 :=
  ((i.toUInt32 * 32 + ctz32 a) <<< (MIN_BPR + 1).toUInt32).toUInt16

-- gicv2.rs GicV2::prio_from_apr_bits (QEMU `gic_get_prio_from_apr_bits`)
def prioFromAprBits (s : State) : UInt16 :=
  let a0 := s.apr[0]! ||| s.nsapr[0]!
  let a1 := s.apr[1]! ||| s.nsapr[1]!
  let a2 := s.apr[2]! ||| s.nsapr[2]!
  let a3 := s.apr[3]! ||| s.nsapr[3]!
  if a0 != 0 then aprPriority 0 a0
  else if a1 != 0 then aprPriority 1 a1
  else if a2 != 0 then aprPriority 2 a2
  else if a3 != 0 then aprPriority 3 a3
  else IDLE_PRIORITY

-- The first nonzero word of an active-priority bank, NR_APRS if none.
def firstNonzero (a : Array UInt32) : Nat :=
  if a[0]! != 0 then 0
  else if a[1]! != 0 then 1
  else if a[2]! != 0 then 2
  else if a[3]! != 0 then 3
  else NR_APRS

-- gicv2.rs GicV2::drop_prio (QEMU `gic_drop_prio`): clear the lowest set
-- bit of the group's active priority registers.
def dropPrio (s : State) (group1 : Bool) : State :=
  let w := firstNonzero (if group1 then s.nsapr else s.apr)
  let s :=
    if w < NR_APRS then
      let x := (if group1 then s.nsapr else s.apr)[w]!
      s.setAprWord group1 w (x &&& (x - 1))
    else s
  { s with runningPriority := s.prioFromAprBits }

-- gicv2.rs GicV2::acknowledge (QEMU `gic_acknowledge_irq`): the GICC_IAR or
-- GICC_AIAR value. An interrupt that does not preempt the running priority
-- reads as 1023 and stays pending. CPUID is RAZ.
def acknowledge (s : State) (view : CpuView) : State × UInt32 :=
  let irq := s.currentPendingIrq view
  if irq ≥ MAXIRQ then (s, irq)
  else
    let i := irq.toNat
    if !s.preempts i then (s, SPURIOUS)
    else
      let s := s.activate i
      let s := s.setBit .pending i false
      (s.update, irq)

-- gicv2.rs GicV2::eoi_split
@[inline] def eoiSplit (s : State) : Bool := s.cpuCtlr &&& GICC_CTLR_EOIMODE != 0

-- gicv2.rs GicV2::complete (QEMU `gic_complete_irq`, GICC_EOIR/GICC_AEOIR):
-- ignored for an interrupt that does not exist, while nothing is active,
-- and for a Group 0 interrupt through the Group 1 alias; otherwise the
-- priority drops and, unless EOImode splits the two, the interrupt
-- deactivates.
def complete (s : State) (irq : UInt32) (view : CpuView) : State :=
  let i := irq.toNat
  if i ≥ NUM_IRQ || s.runningPriority == IDLE_PRIORITY then s
  else
    let g1 := bitIn s.group1 i
    if view == .group1Alias && !g1 then s
    else
      let s := s.dropPrio g1
      if !s.eoiSplit then s.setBit .active i false else s

-- gicv2.rs GicV2::deactivate (QEMU `gic_deactivate_irq`, GICC_DIR):
-- ignored unless EOImode is set.
def deactivate (s : State) (irq : UInt32) : State :=
  let i := irq.toNat
  if irq ≥ MAXIRQ || i ≥ NUM_IRQ || !s.eoiSplit then s
  else s.setBit .active i false

-- gicv2.rs GicV2::cpu_read_register (QEMU `gic_cpu_read`, with GICC_AIAR
-- and GICC_AHPPIR). The register at exactly `off`; every other offset reads
-- as zero.
def cpuReadRegister (s : State) (off : UInt64) : State × UInt32 :=
  if off == 0x00 then (s, s.cpuCtlr)
  else if off == 0x04 then (s, s.pmr.toUInt32)
  else if off == 0x08 then (s, s.bpr.toUInt32)
  else if off == 0x0C then s.acknowledge .primary
  else if off == 0x14 then (s, (min s.runningPriority 0xFF).toUInt32)
  else if off == 0x18 then (s, s.currentPendingIrq .primary)
  else if off == 0x1C then (s, s.abpr.toUInt32)
  else if off == 0x20 then s.acknowledge .group1Alias
  else if off == 0x28 then (s, s.currentPendingIrq .group1Alias)
  else if off == 0xD0 || off == 0xD4 || off == 0xD8 || off == 0xDC then
    (s, s.apr[((off - 0xD0) / 4).toNat]!)
  else if off == 0xE0 || off == 0xE4 || off == 0xE8 || off == 0xEC then
    (s, s.nsapr[((off - 0xE0) / 4).toNat]!)
  else if off == 0xFC then (s, ((2 : UInt32) <<< 16) ||| 0x43B)
  else (s, 0)

-- gicv2.rs GicV2::cpu_read via bus.rs read_by_words: an 8-byte access is
-- the register at `off` then the one at `off + 4`; any other width is the
-- register at `off`, masked to the width as board.rs VirtBoard::read masks
-- it (a byte read of GICC_IAR acknowledges).
def cpuRead (s : State) (off : UInt64) (log2 : UInt8) : State × UInt64 :=
  if log2 ≥ 3 then
    let (s, lo) := s.cpuReadRegister off
    let (s, hi) := s.cpuReadRegister (off + 4)
    (s, lo.toUInt64 ||| (hi.toUInt64 <<< 32))
  else
    let (s, v) := s.cpuReadRegister off
    (s, v.toUInt64 &&& valueMask log2)

-- gicv2.rs GicV2::cpu_write_register (QEMU `gic_cpu_write`, with GICC_AEOIR)
def cpuWriteRegister (s : State) (off : UInt64) (v : UInt32) : State :=
  if off == 0x00 then { s with cpuCtlr := v &&& GICC_CTLR_V2_MASK }
  else if off == 0x04 then { s with pmr := v.toUInt8 }
  -- QEMU raises a written binary point to `GIC_MIN_BPR`, which is 0, so
  -- every value stands.
  else if off == 0x08 then { s with bpr := v.toUInt8 &&& 7 }
  else if off == 0x10 then s.complete (v &&& 0x3FF) .primary
  else if off == 0x1C then { s with abpr := max (v.toUInt8 &&& 7) MIN_ABPR }
  else if off == 0x24 then s.complete (v &&& 0x3FF) .group1Alias
  else if off == 0xD0 || off == 0xD4 || off == 0xD8 || off == 0xDC then
    let s := s.setAprWord false ((off - 0xD0) / 4).toNat v
    { s with runningPriority := s.prioFromAprBits }
  else if off == 0xE0 || off == 0xE4 || off == 0xE8 || off == 0xEC then
    let s := s.setAprWord true ((off - 0xE0) / 4).toNat v
    { s with runningPriority := s.prioFromAprBits }
  else if off == 0x1000 then s.deactivate (v &&& 0x3FF)
  else s

-- gicv2.rs GicV2::cpu_write via bus.rs write_by_words (`word as u32`), then
-- GicV2::update. The value is first masked to the width, as board.rs
-- VirtBoard::write masks it.
def cpuWrite (s : State) (off : UInt64) (log2 : UInt8) (v : UInt64) : State :=
  let v := v &&& valueMask log2
  let s :=
    if log2 ≥ 3 then
      (s.cpuWriteRegister off (v &&& 0xFFFF_FFFF).toUInt32).cpuWriteRegister (off + 4)
        (v >>> 32).toUInt32
    else s.cpuWriteRegister off v.toUInt32
  s.update

/-! ### GICv2m MSI frame -/

-- gicv2.rs GicV2::v2m_register (QEMU `gicv2m_read` for a 4-byte access),
-- with `MSI_IIDR` the board identity's.
def v2mRegister (off : UInt64) (iidr : UInt32) : UInt32 :=
  if off == V2M_MSI_TYPER then (V2M_BASE_SPI <<< 16) ||| V2M_NUM_SPI
  else if off == V2M_MSI_IIDR then iidr
  else 0

-- gicv2.rs GicV2::v2m_read on a board whose identity gives `MSI_IIDR` as
-- `iidr`: a byte or half-word read is zero, a word the register, a dword
-- the registers at `off` and `off + 4`. Masked to the width as board.rs
-- VirtBoard::read masks it.
def v2mReadWith (s : State) (off : UInt64) (log2 : UInt8) (iidr : UInt32) : State × UInt64 :=
  let v : UInt64 :=
    if log2 ≤ 1 then 0
    else if log2 == 2 then (v2mRegister off iidr).toUInt64
    else (v2mRegister off iidr).toUInt64 ||| ((v2mRegister (off + 4) iidr).toUInt64 <<< 32)
  (s, v &&& valueMask log2)

-- gicv2.rs GicV2::v2m_read on the default board (BoardIdentity::RUSTY_BOX).
def v2mRead (s : State) (off : UInt64) (log2 : UInt8) : State × UInt64 :=
  s.v2mReadWith off log2 MSI_FRAME_IIDR_RUSTY_BOX

-- gicv2.rs GicV2::v2m_write_register (QEMU `gicv2m_write`, `gicv2m_set_irq`):
-- MSI_SETSPI_NS pulses the SPI's input line, which latches it pending only
-- if it is edge-triggered.
def v2mWriteRegister (s : State) (off : UInt64) (v : UInt32) : State :=
  if off == V2M_MSI_SETSPI_NS then
    let spi := v &&& 0x3FF
    if V2M_BASE_SPI ≤ spi && spi < V2M_BASE_SPI + V2M_NUM_SPI then
      (s.setLine spi true).setLine spi false
    else s
  else s

-- gicv2.rs GicV2::v2m_write: a byte write is ignored, a half-word or word
-- write is one register, a dword the registers at `off` and `off + 4`. The
-- value is first masked to the width, as board.rs VirtBoard::write masks it.
def v2mWrite (s : State) (off : UInt64) (log2 : UInt8) (v : UInt64) : State :=
  let v := v &&& valueMask log2
  if log2 == 0 then s
  else if log2 ≤ 2 then s.v2mWriteRegister off v.toUInt32
  else (s.v2mWriteRegister off v.toUInt32).v2mWriteRegister (off + 4) (v >>> 32).toUInt32

end State

/-! ## Checks reproducing gicv2.rs unit tests -/

namespace Tests
open State

abbrev G := StateM State

-- gicv2.rs tests `dist`, `cpu`, `cpu_get`, `dist_get` (word accesses).
def dist (off : UInt64) (v : UInt32) : G Unit := modify (·.distWrite off 2 v.toUInt64)
def distB (off : UInt64) (v : UInt8) : G Unit := modify (·.distWrite off 0 v.toUInt64)
def cpu (off : UInt64) (v : UInt32) : G Unit := modify (·.cpuWrite off 2 v.toUInt64)
def cpuGet (off : UInt64) : G UInt32 :=
  modifyGet fun g => let (g, v) := g.cpuRead off 2; (v.toUInt32, g)
def distGet (off : UInt64) : G UInt32 :=
  modifyGet fun g => let (g, v) := g.distRead off 2; (v.toUInt32, g)
def distGetB (off : UInt64) : G UInt64 :=
  modifyGet fun g => let (g, v) := g.distRead off 0; (v, g)
def irq : G Bool := do return (← get).irqOut
def line (i : UInt32) (l : Bool) : G Unit := modify (·.setLine i l)

def CTLR : UInt64 := 0x00
def PMR : UInt64 := 0x04
def BPR : UInt64 := 0x08
def IAR : UInt64 := 0x0C
def EOIR : UInt64 := 0x10
def RPR : UInt64 := 0x14
def HPPIR : UInt64 := 0x18
def ABPR : UInt64 := 0x1C
def AIAR : UInt64 := 0x20
def AEOIR : UInt64 := 0x24
def AHPPIR : UInt64 := 0x28
def APR0 : UInt64 := 0xD0
def NSAPR0 : UInt64 := 0xE0
def DIR : UInt64 := 0x1000

-- gicv2.rs tests `enabled`: both groups enabled at the distributor and the
-- CPU interface, no priority masked.
def enabled : G Unit := do
  dist 0x000 3
  cpu CTLR 3
  cpu PMR 0xFF

-- gicv2.rs tests `edge_triggered`
def edgeTriggered (i : UInt32) : G Unit :=
  distB (0xC00 + (i / 4).toUInt64) ((2 : UInt32) <<< (2 * (i % 4))).toUInt8

-- gicv2.rs tests `raise`: enable SPI `i` at `prio`, and raise its line.
def raise (i : UInt32) (prio : UInt8) : G Unit := do
  distB (0x400 + i.toUInt64) prio
  dist (0x100 + (i / 32).toUInt64 * 4) ((1 : UInt32) <<< (i % 32))
  line i true

def run {α : Type} (t : G α) : α := Id.run (t.run' State.new)

-- reset values: GICD_TYPER, GICD_IIDR, the peripheral IDs, GICC_IIDR, the
-- running priority, and the MSI frame's MSI_TYPER and MSI_IIDR.
#guard run (distGet 0x004) == 8
#guard run (distGet 0x008) == 0x43B
#guard ((List.range 12).map fun k =>
    run (distGetB (0xFD0 + 4 * k.toUInt64))) ==
  [0x04, 0x00, 0x00, 0x00, 0x90, 0xB4, 0x2B, 0x00, 0x0D, 0xF0, 0x05, 0xB1]
#guard run (distGetB 0xFD1) == 0
#guard run (cpuGet 0xFC) == 0x0002_043B
#guard run (cpuGet RPR) == 0xFF
#guard run (cpuGet ABPR) == 1
#guard (State.new.v2mRead V2M_MSI_TYPER 2).2 == 0x0050_0040
#guard (State.new.v2mRead V2M_MSI_IIDR 2).2 == 0
#guard (State.new.v2mRead V2M_MSI_TYPER 3).2 == 0x0050_0040
#guard (State.new.v2mReadWith V2M_MSI_IIDR 2 0x1230_5678).2 == 0x1230_5678
#guard (State.new.v2mReadWith (V2M_MSI_IIDR - 4) 3 0x1230_5678).2 == 0x1230_5678_0000_0000

-- width masking at the board: a byte write of GICC_CTLR keeps the low byte
-- only, and a byte read of it returns its low byte.
#guard run (do cpu CTLR 0x21F; modify (·.cpuWrite CTLR 0 0x2FF); cpuGet CTLR) == 0x1F
#guard run (do cpu CTLR 0x21F
               modifyGet fun g => let (g, v) := g.cpuRead CTLR 0; (v, g)) == 0x1F
#guard run (do cpu CTLR 0x21F
               modifyGet fun g => let (g, v) := g.cpuRead (CTLR + 1) 0; (v, g)) == 0

-- a_level_ppi_is_acknowledged_and_completed
def tLevelPpi : G Bool := do
  dist 0x000 1
  cpu CTLR 1
  cpu PMR 0xFF
  dist 0x100 ((1 : UInt32) <<< 30)
  line 30 true
  let mut ok ← irq
  ok := ok && (← cpuGet IAR) == 30
  ok := ok && !(← irq)
  line 30 false
  cpu EOIR 30
  ok := ok && !(← irq)
  ok := ok && (← cpuGet IAR) == 1023
  return ok
#guard run tLevelPpi

-- pmr_masks_lower_priority_interrupts
def tPmr : G Bool := do
  enabled
  cpu PMR 0x80
  raise 32 0x80
  let mut ok := !(← irq)
  ok := ok && (← cpuGet HPPIR) == 1023
  cpu PMR 0x81
  ok := ok && (← irq)
  return ok
#guard run tPmr

-- sgis_are_enabled_from_reset_and_stay_enabled
def tSgisEnabled : G Bool := do
  let mut ok := (← distGet 0x100) &&& 0xFFFF == 0xFFFF
  dist 0x180 0xFFFF_FFFF
  ok := ok && (← distGet 0x100) == 0xFFFF
  dist 0xC00 0
  ok := ok && (← distGet 0xC00) == 0xAAAA_AAAA
  return ok
#guard run tSgisEnabled

-- an_sgi_on_a_uniprocessor_has_one_source_and_no_cpuid
def tSgiOneSource : G Bool := do
  enabled
  dist 0xF00 (((2 : UInt32) <<< 24) ||| 5)
  let mut ok ← irq
  ok := ok && (← cpuGet IAR) == 5
  cpu EOIR 5
  distB (0xF20 + 7) 0x48
  ok := ok && (← distGetB (0xF20 + 7)) == 0
  ok := ok && !(← irq)
  ok := ok && (← cpuGet HPPIR) == 1023
  distB (0xF20 + 7) 0x49
  ok := ok && (← distGetB (0xF10 + 7)) == 1
  ok := ok && (← cpuGet HPPIR) == 7
  ok := ok && (← cpuGet IAR) == 7
  cpu EOIR 7
  ok := ok && (← cpuGet IAR) == 1023
  ok := ok && (← distGet 0xF24) == 0
  distB (0xF20 + 2) 1
  distB (0xF10 + 2) 0xFE
  ok := ok && (← cpuGet HPPIR) == 2
  distB (0xF10 + 2) 1
  ok := ok && (← cpuGet HPPIR) == 1023
  return ok
#guard run tSgiOneSource

-- a_zero_spendsgir_byte_pends_nothing
def tZeroSpendsgir : G Bool := do
  enabled
  distB (0xF20 + 4) 0
  let mut ok := !(← irq)
  ok := ok && (← cpuGet HPPIR) == 1023
  dist 0xF20 0x0000_0001
  ok := ok && (← distGet 0x200) &&& 0xF == 1
  ok := ok && (← distGet 0xF20) == 1
  ok := ok && (← cpuGet IAR) == 0
  cpu EOIR 0
  ok := ok && (← cpuGet IAR) == 1023
  return ok
#guard run tZeroSpendsgir

-- sgir_target_filters_on_a_uniprocessor
def tSgirFilters : G Bool := do
  enabled
  dist 0xF00 (((1 : UInt32) <<< 24) ||| 1)
  let mut ok := !(← irq)
  dist 0xF00 (((1 : UInt32) <<< 16) ||| 2)
  ok := ok && (← cpuGet IAR) == 2
  dist 0xF00 (((3 : UInt32) <<< 24) ||| 3)
  ok := ok && (← cpuGet HPPIR) == 3
  distB 0xF00 4
  ok := ok && (← distGet 0x200) &&& ((1 : UInt32) <<< 4) == 0
  return ok
#guard run tSgirFilters

-- the_running_priority_is_the_group_priority
def tRunningPriority : G Bool := do
  enabled
  let mut ok := (← cpuGet RPR) == 0xFF
  cpu BPR 2
  raise 40 0x45
  ok := ok && (← cpuGet IAR) == 40
  ok := ok && (← cpuGet RPR) == 0x40
  ok := ok && (← cpuGet (APR0 + 4)) == 1
  ok := ok && (← cpuGet APR0) == 0
  raise 41 0x41
  ok := ok && !(← irq)
  raise 42 0x3F
  ok := ok && (← irq)
  ok := ok && (← cpuGet IAR) == 42
  ok := ok && (← cpuGet RPR) == 0x38
  cpu EOIR 42
  ok := ok && (← cpuGet RPR) == 0x40
  cpu EOIR 40
  ok := ok && (← cpuGet RPR) == 0xFF
  return ok
#guard run tRunningPriority

-- preemption_compares_group_priorities_under_each_groups_binary_point
def tPreemptionBinaryPoints : G Bool := do
  enabled
  cpu BPR 2
  cpu ABPR 1
  dist 0x084 ((1 : UInt32) <<< 8)
  raise 40 0x42
  let mut ok := (← cpuGet AIAR) == 40
  ok := ok && (← cpuGet RPR) == 0x42
  raise 42 0x48
  ok := ok && !(← irq)
  raise 41 0x43
  ok := ok && (← irq)
  ok := ok && (← cpuGet HPPIR) == 41
  ok := ok && (← cpuGet IAR) == 41
  ok := ok && (← cpuGet RPR) == 0x40
  cpu EOIR 41
  ok := ok && (← cpuGet RPR) == 0x42
  cpu AEOIR 40
  ok := ok && (← cpuGet RPR) == 0xFF
  return ok
#guard run tPreemptionBinaryPoints

-- group_1_uses_the_aliased_binary_point_unless_cbpr
def tAliasedBinaryPoint : G Bool := do
  enabled
  cpu CTLR (3 ||| GICC_CTLR_ACK_CTL)
  dist 0x084 ((1 : UInt32) <<< 8)
  cpu ABPR 4
  raise 40 0x5F
  let mut ok := (← cpuGet IAR) == 40
  ok := ok && (← cpuGet RPR) == 0x50
  ok := ok && (← cpuGet (NSAPR0 + 4)) == (1 : UInt32) <<< 8
  ok := ok && (← cpuGet (APR0 + 4)) == 0
  cpu EOIR 40
  cpu CTLR (3 ||| GICC_CTLR_ACK_CTL ||| GICC_CTLR_CBPR)
  line 40 false
  line 40 true
  ok := ok && (← cpuGet IAR) == 40
  ok := ok && (← cpuGet RPR) == 0x5E
  return ok
#guard run tAliasedBinaryPoint

-- a_group_1_interrupt_reads_1022_without_ackctl
def tGroup1Reads1022 : G Bool := do
  enabled
  dist 0x084 ((1 : UInt32) <<< 8)
  raise 40 0x10
  let mut ok ← irq
  ok := ok && (← cpuGet HPPIR) == 1022
  ok := ok && (← cpuGet IAR) == 1022
  ok := ok && (← irq)
  return ok
#guard run tGroup1Reads1022

-- a_group_the_distributor_does_not_forward_hides_nothing
def tUnforwardedGroup : G Bool := do
  enabled
  cpu CTLR (3 ||| GICC_CTLR_ACK_CTL)
  dist 0x000 1
  dist 0x084 ((1 : UInt32) <<< 8)
  raise 40 0x10
  raise 41 0x20
  let mut ok ← irq
  ok := ok && (← cpuGet HPPIR) == 41
  ok := ok && (← cpuGet AHPPIR) == 1023
  ok := ok && (← cpuGet AIAR) == 1023
  ok := ok && (← cpuGet IAR) == 41
  cpu EOIR 41
  line 41 false
  ok := ok && !(← irq)
  ok := ok && (← cpuGet IAR) == 1023
  ok := ok && (← distGet 0x304) == 0
  dist 0x000 3
  ok := ok && (← cpuGet IAR) == 40
  return ok
#guard run tUnforwardedGroup

-- a_group_the_cpu_interface_does_not_signal_hides_nothing
def tUnsignalledGroup : G Bool := do
  enabled
  cpu CTLR 2
  dist 0x084 ((1 : UInt32) <<< 9)
  raise 40 0x10
  raise 41 0x20
  let mut ok ← irq
  ok := ok && (← cpuGet HPPIR) == 1022
  ok := ok && (← cpuGet AHPPIR) == 41
  ok := ok && (← cpuGet AIAR) == 41
  cpu AEOIR 41
  line 41 false
  ok := ok && !(← irq)
  ok := ok && (← cpuGet IAR) == 1023
  ok := ok && (← distGet 0x304) == 0
  return ok
#guard run tUnsignalledGroup

-- eoi_while_idle_and_dir_without_eoimode_are_ignored
def tIdleEoiAndDir : G Bool := do
  enabled
  dist 0x304 ((1 : UInt32) <<< 8)
  cpu EOIR 40
  let mut ok := (← distGet 0x304) == (1 : UInt32) <<< 8
  cpu DIR 40
  ok := ok && (← distGet 0x304) == (1 : UInt32) <<< 8
  cpu CTLR (3 ||| GICC_CTLR_EOIMODE)
  cpu DIR 40
  ok := ok && (← distGet 0x304) == 0
  return ok
#guard run tIdleEoiAndDir

-- eoimode_splits_priority_drop_from_deactivation
def tEoiModeSplit : G Bool := do
  enabled
  cpu CTLR (3 ||| GICC_CTLR_EOIMODE)
  raise 40 0x20
  let mut ok := (← cpuGet IAR) == 40
  line 40 false
  cpu EOIR 40
  ok := ok && (← cpuGet RPR) == 0xFF
  ok := ok && (← distGet 0x304) == (1 : UInt32) <<< 8
  cpu DIR 40
  ok := ok && (← distGet 0x304) == 0
  return ok
#guard run tEoiModeSplit

-- ctlr_keeps_qemus_bits
#guard run (do enabled; cpu CTLR 0xFFFF_FFFF; cpuGet CTLR) == GICC_CTLR_V2_MASK

-- the_aliased_registers_handle_group_1
def tAliasesGroup1 : G Bool := do
  enabled
  dist 0x084 ((1 : UInt32) <<< 8)
  raise 40 0x10
  let mut ok := (← cpuGet IAR) == 1022
  ok := ok && (← cpuGet AHPPIR) == 40
  ok := ok && (← cpuGet AIAR) == 40
  ok := ok && (← distGet 0x304) == (1 : UInt32) <<< 8
  ok := ok && (← cpuGet RPR) == 0x10
  ok := ok && (← cpuGet NSAPR0) == (1 : UInt32) <<< 8
  ok := ok && (← cpuGet AHPPIR) == 1023
  line 40 false
  cpu AEOIR 40
  ok := ok && (← cpuGet RPR) == 0xFF
  ok := ok && (← distGet 0x304) == 0
  raise 41 0x20
  ok := ok && (← cpuGet AHPPIR) == 1023
  ok := ok && (← cpuGet AIAR) == 1023
  ok := ok && (← distGet 0x304) == 0
  ok := ok && (← cpuGet IAR) == 41
  cpu AEOIR 41
  ok := ok && (← cpuGet RPR) == 0x20
  cpu EOIR 41
  ok := ok && (← cpuGet RPR) == 0xFF
  return ok
#guard run tAliasesGroup1

-- an_eight_byte_access_is_two_registers
def tDword : G Bool := do
  enabled
  raise 40 0x10
  let pair ← modifyGet fun g => let (g, v) := g.cpuRead BPR 3; (v, g)
  let mut ok := pair == (40 : UInt64) <<< 32
  modify (·.cpuWrite EOIR 3 40)
  ok := ok && (← cpuGet RPR) == 0xFF
  modify (·.distWrite (0x400 + 40) 3 0x0000_0099_0000_0077)
  ok := ok && (← distGetB (0x400 + 40)) == 0x77
  ok := ok && (← distGetB (0x400 + 44)) == 0x99
  return ok
#guard run tDword

-- an_msi_latches_only_an_edge_triggered_spi
def tMsi : G Bool := do
  enabled
  dist (0x100 + 80 / 32 * 4) ((1 : UInt32) <<< (80 % 32))
  modify (·.v2mWrite V2M_MSI_SETSPI_NS 2 80)
  let mut ok := !(← irq)
  edgeTriggered 80
  modify (·.v2mWrite V2M_MSI_SETSPI_NS 0 80)
  ok := ok && !(← irq)
  modify (·.v2mWrite V2M_MSI_SETSPI_NS 1 80)
  ok := ok && (← cpuGet IAR) == 80
  let g ← get
  ok := ok && (g.v2mRead V2M_MSI_TYPER 0).2 == 0
  ok := ok && (g.v2mRead V2M_MSI_TYPER 2).2 == ((80 : UInt64) <<< 16) ||| 64
  return ok
#guard run tMsi

-- a_snapshot_round_trips_the_priority_state, on the state the snapshot
-- holds (save/restore is not ported).
def tPriorityState : G Bool := do
  enabled
  cpu BPR 1
  raise 40 0x45
  dist 0xF00 (((2 : UInt32) <<< 24) ||| 3)
  let mut ok := (← cpuGet IAR) == 3
  ok := ok && (← cpuGet RPR) == 0
  cpu EOIR 3
  ok := ok && (← cpuGet IAR) == 40
  ok := ok && (← cpuGet RPR) == 0x44
  return ok
#guard run tPriorityState

-- the_aliased_registers_stay_group_1_with_ackctl
def tAliasesWithAckCtl : G Bool := do
  enabled
  cpu CTLR (3 ||| GICC_CTLR_ACK_CTL)
  dist 0x084 ((1 : UInt32) <<< 8)
  raise 40 0x20
  raise 41 0x10
  let mut ok := (← cpuGet AHPPIR) == 1023
  ok := ok && (← cpuGet AIAR) == 1023
  ok := ok && (← distGet 0x304) == 0
  ok := ok && (← cpuGet HPPIR) == 41
  ok := ok && (← cpuGet IAR) == 41
  cpu AEOIR 41
  ok := ok && (← cpuGet RPR) == 0x10
  ok := ok && (← distGet 0x304) == (1 : UInt32) <<< 9
  line 41 false
  cpu EOIR 41
  ok := ok && (← cpuGet AHPPIR) == 40
  ok := ok && (← cpuGet AIAR) == 40
  ok := ok && (← cpuGet RPR) == 0x20
  cpu AEOIR 40
  ok := ok && (← cpuGet RPR) == 0xFF
  ok := ok && (← distGet 0x304) == 0
  return ok
#guard run tAliasesWithAckCtl

-- gicv2.rs GicV2::update: a Group 0 interrupt is signalled as FIQ while
-- FIQEn is set, a Group 1 interrupt as IRQ.
def tFiq : G Bool := do
  enabled
  cpu CTLR (3 ||| GICC_CTLR_FIQ_EN)
  raise 40 0x10
  let g ← get
  let mut ok := g.fiqOut && !g.irqOut
  dist 0x084 ((1 : UInt32) <<< 8)
  let g ← get
  ok := ok && g.irqOut && !g.fiqOut
  return ok
#guard run tFiq

end Tests

end Leanbox.Dev.Gic
