import Leanbox.Run
open Leanbox

def fnv (h v : UInt64) : UInt64 := Id.run do
  let mut h := h
  for i in [0:8] do
    h := (h ^^^ ((v >>> (8 * i.toUInt64)) &&& 0xFF)) * 0x100000001b3
  return h

def stateHash (m : Mach) : UInt64 := Id.run do
  let mut h : UInt64 := 0xcbf29ce484222325
  for i in [0:31] do h := fnv h (rr m.rf i.toUInt8)
  h := fnv h (spEl0 m.rf m.sys)
  h := fnv h (spEl1 m.rf m.sys)
  h := fnv h (m.nzcv <<< 28)
  h := fnv h m.sys.daif
  return h

def readFile (p : String) : IO ByteArray := IO.FS.readBinFile p

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

def console (m : Mach) : ByteArray := m.sys.board.uart.out

def hasText (b : ByteArray) (pat : ByteArray) : Bool := Id.run do
  if b.size < pat.size then return false
  for i in [0:b.size - pat.size + 1] do
    let mut ok := true
    for j in [0:pat.size] do
      if b[i + j]! != pat[j]! then ok := false; break
    if ok then return true
  return false

/-- One line per `k` retired instructions, as the oracle's `check` prints
them, until `max`, a stop, or `until` on the console (printed as `tag`). -/
partial def checkLoop (m : Mach) (k max : UInt64) (stopAt : ByteArray) (tag : String)
    (out : IO.FS.Stream) : IO Mach := do
  if m.retired >= max then return m
  let m := drive m (m.retired + k)
  let c := console m
  out.putStrLn s!"C {m.retired} {m.icnt} {hex m.pc} {String.ofList ((Nat.toDigits 16 (stateHash m).toNat).leftpad 16 (Char.ofNat 48))} {c.size}"
  if m.sys.power != 0 then
    out.putStrLn s!"STOP {m.sys.power} {m.sys.stopWhy}"
    return m
  -- the oracle searches the whole console; a match ends in the newest bytes
  if hasText (c.extract (c.size - min c.size (k.toNat + stopAt.size)) c.size) stopAt then
    out.putStrLn tag
    return m
  checkLoop m k max stopAt tag out

def regsLine (m : Mach) : String := Id.run do
  let mut s := ""
  for i in [0:31] do s := s ++ s!" {hex (rr m.rf i.toUInt8)}"
  s ++ s!" sp0 {hex (spEl0 m.rf m.sys)} sp1 {hex (spEl1 m.rf m.sys)} nzcv {hex (m.nzcv <<< 28)} daif {hex m.sys.daif}"

def run1 (m : Mach) (args : List String) (stopAt : String) (tag : String) : IO Unit := do
  let out ← IO.getStdout
  match args with
  | "check" :: k :: max :: rest =>
    let stopAt := (rest.headD stopAt).toUTF8
    let t0 ← IO.monoNanosNow
    let m ← checkLoop m k.toNat!.toUInt64 max.toNat!.toUInt64 stopAt tag out
    let t1 ← IO.monoNanosNow
    IO.eprintln s!"{m.retired} instructions in {(t1 - t0).toFloat / 1e9}s"
    IO.FS.writeBinFile "console.txt" (console m)
  | "detail" :: from_ :: count :: _ =>
    let m := drive m from_.toNat!.toUInt64
    let mut m := m
    for _ in [0:count.toNat!] do
      let pc := m.pc
      m := drive m (m.retired + 1)
      out.putStrLn s!"D {m.retired} t{m.icnt} pc {hex pc} ->{regsLine m}"
  | _ => IO.eprintln "usage: leanbox edk2|linux check K MAX [UNTIL] | detail FROM COUNT"

def main (args : List String) : IO UInt32 := do
  match args with
  | "edk2" :: rest =>
    let code ← readFile "/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd"
    let vars ← readFile "/usr/share/AAVMF/AAVMF_VARS.fd"
    let dtb ← readFile ((← IO.getEnv "EDK2_DTB").getD "oracle/ref/dtb.bin")
    run1 (boot code vars dtb) rest "Shell> " "SHELL"
  | "linux" :: rest =>
    let kernel ← readFile ((← IO.getEnv "LINUX_IMAGE").getD "/tmp/alp/Image")
    let initrd ← readFile ((← IO.getEnv "LINUX_INITRD").getD "/tmp/alp/initramfs-virt")
    let dtb ← readFile ((← IO.getEnv "LINUX_DTB").getD "oracle/ref/linux_dtb.bin")
    run1 (bootLinux kernel initrd dtb) rest "\x01\x02" "UNTIL"
  | _ => IO.eprintln "usage: leanbox edk2|linux check K MAX [UNTIL] | detail FROM COUNT"
  return 0
