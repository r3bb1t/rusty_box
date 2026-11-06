import Leanbox.Dev.Gic
/-!
# GICv2 checks

The gicv2.rs unit tests as `#guard`s, checked when this module is built (`lake test`).
-/
namespace Leanbox.Dev.Gic

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
