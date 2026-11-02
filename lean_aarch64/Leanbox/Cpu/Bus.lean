import Leanbox.Cpu.State
/-
The board's bus outside RAM: board.rs `read_outside_ram`,
`write_outside_ram`, `sync_device_lines`, `set_timer_lines`,
`advance_devices`, `next_deadline`, for a board with no virtio device and no
host serial input.
-/
namespace Leanbox

@[inline] def valueMask (log2 : UInt8) : UInt64 :=
  match log2 with | 0 => 0xFF | 1 => 0xFFFF | 2 => 0xFFFF_FFFF | _ => MISS

/-- `addr.offset_in(base, size)`. -/
@[inline] def within (a base size : UInt64) : Bool := a >= base && a - base < size

/-- board.rs `in_pci_window`. -/
def inPciWindow (a : UInt64) : Bool :=
  within a PCIE_MMIO_BASE PCIE_MMIO_SIZE || within a PCIE_PIO_BASE PCIE_PIO_SIZE
  || within a PCIE_ECAM_BASE PCIE_ECAM_SIZE || within a PCIE_MMIO_HIGH_BASE PCIE_MMIO_HIGH_SIZE

def minDeadline (a b : Option UInt64) : Option UInt64 :=
  match a, b with
  | some x, some y => some (min x y)
  | some x, none => some x
  | none, y => y

namespace Board

/-- board.rs `sync_device_lines`: the UART, the RTC and the GPIO (no virtio
transport is plugged). -/
def syncDeviceLines (b : Board) : Board :=
  let uart := b.uart.irq
  let rtc := b.rtc.irq
  let gpio := b.gpio.irq
  let g := b.gic.setLine IRQ_UART0.toUInt32 uart
  let g := g.setLine IRQ_RTC.toUInt32 rtc
  let g := g.setLine IRQ_GPIO.toUInt32 gpio
  { b with gic := g }

/-- board.rs `set_timer_lines`. -/
def setTimerLines (b : Board) (phys virt hyp : Bool) : Board :=
  let g := b.gic.setLine PPI_PHYSICAL_TIMER.toUInt32 phys
  let g := g.setLine PPI_VIRTUAL_TIMER.toUInt32 virt
  let g := g.setLine PPI_HYPERVISOR_TIMER.toUInt32 hyp
  { b with gic := g }

/-- board.rs `next_deadline`: the RTC's alarm and the UART's receive timeout
(no power button press is ever held). -/
def nextDeadline (b : Board) : Option UInt64 :=
  minDeadline b.rtc.nextDeadline b.uart.nextDeadline

/-- board.rs `next_deadline_after`. -/
def nextDeadlineAfter (b : Board) (now : UInt64) : Option UInt64 :=
  let f (d : Option UInt64) : Option UInt64 := match d with
    | some at_ => if at_ > now then some at_ else none
    | none => none
  minDeadline (f b.rtc.nextDeadline) (f b.uart.nextDeadline)

/-- board.rs `advance_devices` at tick `now`. -/
def advanceDevices (b : Board) (now : UInt64) : Board × Option UInt64 :=
  let b := { b with deadlineMoved := false }
  let b := { b with rtc := b.rtc.update now }
  let b := { b with uart := b.uart.update now }
  let b := b.syncDeviceLines
  (b, b.nextDeadline)

/-- board.rs `reset_devices` on a freshly built board at tick `now`. -/
def resetDevices (b : Board) (now : UInt64) : Board :=
  let b := { b with flash0 := b.flash0.reset, flash1 := b.flash1.reset }
  let b := { b with gic := Dev.Gic.State.new }
  let b := { b with uart := b.uart.reset }
  let b := { b with rtc := b.rtc.reset now }
  let b := { b with gpio := b.gpio.reset }
  b.syncDeviceLines

/-- board.rs `read_outside_ram`, then the bus's width mask: `none` is
`BusFault::Unmapped`. -/
def read (b : Board) (a : UInt64) (log2 : UInt8) (now : UInt64) : Board × Option UInt64 :=
  let m := valueMask log2
  if within a FLASH0_BASE FLASH_BANK_SIZE then
    let (f, v) := b.flash0.read b.code (a - FLASH0_BASE) log2
    ({ b with flash0 := f }, some (v &&& m))
  else if within a FLASH1_BASE FLASH_BANK_SIZE then
    let (f, v) := b.flash1.read b.vars (a - FLASH1_BASE) log2
    ({ b with flash1 := f }, some (v &&& m))
  else
  let b := { b with touched := true }
  if within a GIC_DIST_BASE GIC_DIST_SIZE then
    let (g, v) := b.gic.distRead (a - GIC_DIST_BASE) log2
    ({ b with gic := g }, some (v &&& m))
  else if within a GIC_CPU_BASE GIC_CPU_SIZE then
    let (g, v) := b.gic.cpuRead (a - GIC_CPU_BASE) log2
    ({ b with gic := g }, some (v &&& m))
  else if within a GIC_V2M_BASE GIC_V2M_SIZE then
    let (g, v) := b.gic.v2mRead (a - GIC_V2M_BASE) log2
    ({ b with gic := g }, some (v &&& m))
  else if within a UART0_BASE PRIMECELL_SIZE then
    let before := b.uart.nextDeadline
    let (u, v) := b.uart.read (a - UART0_BASE) log2 now
    let b := { b with uart := u }
    let b := if b.uart.nextDeadline != before then { b with deadlineMoved := true } else b
    (b.syncDeviceLines, some (v &&& m))
  else if within a RTC_BASE PRIMECELL_SIZE then
    let (r, v) := b.rtc.read (a - RTC_BASE) log2 now
    ({ b with rtc := r }, some (v &&& m))
  else if within a GPIO_BASE PRIMECELL_SIZE then
    let (g, v) := b.gpio.read (a - GPIO_BASE) log2
    ({ b with gpio := g }, some (v &&& m))
  else if within a VIRTIO_MMIO_BASE (Dev.VirtioMmio.TRANSPORT_SIZE * Dev.VirtioMmio.TRANSPORT_COUNT) then
    let off := (a - VIRTIO_MMIO_BASE) % Dev.VirtioMmio.TRANSPORT_SIZE
    (b, some (Dev.VirtioMmio.readEmpty off log2 &&& m))
  else if inPciWindow a then (b, some m)
  else (b, none)

/-- board.rs `write_outside_ram` (the value already masked to the width):
`false` is `BusFault::Unmapped`. -/
def write (b : Board) (a : UInt64) (log2 : UInt8) (v : UInt64) (now : UInt64) : Board × Bool :=
  let b := { b with touched := true }
  if within a FLASH0_BASE FLASH_BANK_SIZE then
    let arr := b.code
    let b := { b with code := ByteArray.empty }
    let (f, arr) := b.flash0.write arr false (a - FLASH0_BASE) log2 v
    ({ b with flash0 := f, code := arr }, true)
  else if within a FLASH1_BASE FLASH_BANK_SIZE then
    let arr := b.vars
    let b := { b with vars := ByteArray.empty }
    let (f, arr) := b.flash1.write arr true (a - FLASH1_BASE) log2 v
    ({ b with flash1 := f, vars := arr }, true)
  else if within a GIC_DIST_BASE GIC_DIST_SIZE then
    ({ b with gic := b.gic.distWrite (a - GIC_DIST_BASE) log2 v }, true)
  else if within a GIC_CPU_BASE GIC_CPU_SIZE then
    ({ b with gic := b.gic.cpuWrite (a - GIC_CPU_BASE) log2 v }, true)
  else if within a GIC_V2M_BASE GIC_V2M_SIZE then
    ({ b with gic := b.gic.v2mWrite (a - GIC_V2M_BASE) log2 v }, true)
  else if within a UART0_BASE PRIMECELL_SIZE then
    let before := b.uart.nextDeadline
    let b := { b with uart := b.uart.write (a - UART0_BASE) log2 v now }
    let b := if b.uart.nextDeadline != before then { b with deadlineMoved := true } else b
    (b.syncDeviceLines, true)
  else if within a RTC_BASE PRIMECELL_SIZE then
    let (r, moved) := b.rtc.write (a - RTC_BASE) log2 v now
    let b := { b with rtc := r }
    let b := if moved then { b with deadlineMoved := true } else b
    (b.syncDeviceLines, true)
  else if within a GPIO_BASE PRIMECELL_SIZE then
    let b := { b with gpio := b.gpio.write (a - GPIO_BASE) log2 v }
    (b.syncDeviceLines, true)
  else if within a VIRTIO_MMIO_BASE (Dev.VirtioMmio.TRANSPORT_SIZE * Dev.VirtioMmio.TRANSPORT_COUNT) then
    (b, true)
  else if inPciWindow a then (b, true)
  else (b, false)

/-- board.rs `fetch_source`: whether a fetch from flash at `a` reads the
array (the bank is in read-array mode). -/
def flashReadsArray (b : Board) (a : UInt64) : Bool :=
  if within a FLASH0_BASE FLASH_BANK_SIZE then b.flash0.readsArray
  else if within a FLASH1_BASE FLASH_BANK_SIZE then b.flash1.readsArray
  else false

end Board
end Leanbox
