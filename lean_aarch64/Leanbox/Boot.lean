import Leanbox.Engine.Machine
/-!
# Booting

The machine's start states (machine.rs `start`, boot.rs): the firmware boot
(EDK2 from flash) and the direct kernel boot (an arm64 Linux `Image`, its
initramfs and the device tree).
-/
namespace Leanbox

/-- machine.rs `start` for the firmware boot: the processor at its reset
state at the base of flash, the devices reset, the device tree at the base
of RAM, then `sync_events` and the first `after_device_access`. -/
def boot (code vars dtb : ByteArray) : Mach :=
  let board := (Board.new code vars).resetDevices 0
  let sys := Sys.reset FLASH0_BASE 0 board
  let mem := dtb.copySlice 0 (zeros RAM_SIZE.toNat) 0 dtb.size
  let rf := zeros RF_BYTES
  let sys := syncEvents sys 0
  let sys := afterDeviceAccess sys 0
  { rf, mem, sys, pc := FLASH0_BASE, nzcv := 0, icnt := 0 }

/-- boot.rs `LinuxImage::parse`: the load offset and footprint from an
arm64 Image's header. -/
def imageLayout (img : ByteArray) : UInt64 × UInt64 :=
  let textOffset := ld64 img 8
  let imageSize := ld64 img 16
  let len := img.size.toUInt64
  if imageSize == 0 then (0x80000, len)
  else if textOffset < 0x1000 then (textOffset + 0x200000, max imageSize len)
  else (textOffset, max imageSize len)

def nextMultiple (x m : UInt64) : UInt64 := (x + m - 1) / m * m

/-- machine.rs `start` for a direct kernel boot, with boot.rs `layout`: the
kernel at its load offset, the initrd at min(RAM / 2, 128 MiB) or above the
kernel, the device tree at the next 2 MiB boundary; x0 the device tree, x4
the entry; the UART set up at 115200 baud as firmware leaves it. -/
def bootLinux (kernel initrd dtb : ByteArray) : Mach :=
  let (loadOff, footprint) := imageLayout kernel
  let kernelEnd := loadOff + footprint
  let initrdAt := nextMultiple (max (min (RAM_SIZE / 2) (128 * 1024 * 1024)) kernelEnd) 4096
  let initrdEnd := initrdAt + initrd.size.toUInt64
  let dtOff := nextMultiple initrdEnd (2 * 1024 * 1024)
  let entry := RAM_BASE + loadOff
  let board := (Board.new ByteArray.empty ByteArray.empty).resetDevices 0
  let sys := Sys.reset entry 0 board
  let mem := zeros RAM_SIZE.toNat
  let mem := dtb.copySlice 0 mem dtOff.toNat dtb.size
  let mem := kernel.copySlice 0 mem loadOff.toNat kernel.size
  let mem := if initrd.size != 0 then initrd.copySlice 0 mem initrdAt.toNat initrd.size else mem
  let rf := wr (wr (zeros RF_BYTES) 0 (RAM_BASE + dtOff)) 4 entry
  -- board.rs set_up_console at tick 0
  let b := sys.board
  let before := b.uart.nextDeadline
  let b := { b with uart := b.uart.setUpConsole 115200 0 }
  let b := if b.uart.nextDeadline != before then { b with deadlineMoved := true } else b
  let sys := { sys with board := b.syncDeviceLines }
  let sys := syncEvents sys 0
  let sys := afterDeviceAccess sys 0
  { rf, mem, sys, pc := entry, nzcv := 0, icnt := 0 }

end Leanbox
