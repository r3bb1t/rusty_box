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

/-- A console script item (as the oracle's): `until:TEXT` waits for TEXT
on the console after the previous item, `type:TEXT` sends TEXT to the UART
(board.rs `send_serial`); plain TEXT is `until:`. `\r` and `\n` stand for
carriage return and line feed. -/
def scriptItem (arg : String) : Bool × ByteArray :=
  let unescape (t : String) : ByteArray := ((t.replace "\\r" "\r").replace "\\n" "\n").toUTF8
  if arg.startsWith "type:" then (true, unescape (arg.drop 5).toString)
  else if arg.startsWith "until:" then (false, unescape (arg.drop 6).toString)
  else (false, unescape arg)

/-- Send `bytes` to the UART at the machine's tick, then what the next run's
start does (machine.rs `run_until`: `after_device_access`). -/
def sendSerial (m : Mach) (bytes : ByteArray) : Mach × Nat :=
  let (b, n) := m.sys.board.sendSerial bytes m.icnt
  ({ m with sys := afterDeviceAccess { m.sys with board := b } m.icnt }, n)

/-- One line per `k` retired instructions, as the oracle's `check` prints
them, until `max`, a stop, or the end of the script (printed as `tag`). The
script is acted on after each line: every `until` satisfied by the console
since the previous item, and every `type` after it. The console is drained
from the UART after each step and kept here, as the oracle keeps it. -/
partial def checkLoop (m : Mach) (k max : UInt64) (script : Array (Bool × ByteArray))
    (pos mark : Nat) (con : ByteArray) (tag : String) (out : IO.FS.Stream) : IO (Mach × ByteArray) := do
  if m.retired >= max then return (m, con)
  let m := drive m (m.retired + k)
  let (u, o) := m.sys.board.uart.takeOutput
  let m := { m with sys := { m.sys with board := { m.sys.board with uart := u } } }
  let con := con ++ o
  out.putStrLn s!"C {m.retired} {m.icnt} {hex m.pc} {String.ofList ((Nat.toDigits 16 (stateHash m).toNat).leftpad 16 (Char.ofNat 48))} {con.size}"
  if m.sys.power != 0 then
    out.putStrLn s!"STOP {m.sys.power} {m.sys.stopWhy}"
    return (m, con)
  let mut m := m
  let mut pos := pos
  let mut mark := mark
  while pos < script.size do
    let (isType, text) := script[pos]!
    if isType then
      let (m', n) := sendSerial m text
      m := m'
      if n != text.size then out.putStrLn s!"SHORT {n} of {text.size}"
      mark := con.size
      pos := pos + 1
    else if hasText (con.extract mark con.size) text then
      mark := con.size
      pos := pos + 1
    else break
  if pos >= script.size then
    out.putStrLn tag
    return (m, con)
  checkLoop m k max script pos mark con tag out

def regsLine (m : Mach) : String := Id.run do
  let mut s := ""
  for i in [0:31] do s := s ++ s!" {hex (rr m.rf i.toUInt8)}"
  s ++ s!" sp0 {hex (spEl0 m.rf m.sys)} sp1 {hex (spEl1 m.rf m.sys)} nzcv {hex (m.nzcv <<< 28)} daif {hex m.sys.daif}"

def run1 (m : Mach) (args : List String) (stopAt : String) (tag : String) : IO Unit := do
  let out ← IO.getStdout
  match args with
  | "check" :: k :: max :: rest =>
    let script := (if rest.isEmpty then [stopAt] else rest).toArray.map scriptItem
    let t0 ← IO.monoNanosNow
    let (m, con) ← checkLoop m k.toNat!.toUInt64 max.toNat!.toUInt64 script 0 0 ByteArray.empty tag out
    let t1 ← IO.monoNanosNow
    IO.eprintln s!"{m.retired} instructions in {(t1 - t0).toFloat / 1e9}s"
    IO.FS.writeBinFile "console.txt" con
  | "detail" :: from_ :: count :: _ =>
    let m := drive m from_.toNat!.toUInt64
    let mut m := m
    for _ in [0:count.toNat!] do
      let pc := m.pc
      m := drive m (m.retired + 1)
      out.putStrLn s!"D {m.retired} t{m.icnt} pc {hex pc} ->{regsLine m}"
  | _ => IO.eprintln "usage: leanbox edk2|linux check K MAX [until:TEXT|type:TEXT]... | detail FROM COUNT; leanbox bare TESTS"

def parseHex (x : String) : UInt64 :=
  (x.foldl (fun acc c => acc * 16 + (if c.isDigit then c.toNat - 48 else c.toNat - 87)) 0).toUInt64

/-- The Lean side of `oracle bare` (tools/difftest_gen.py's tests): the same
start state per line, one instruction, the same line out. -/
def bare (path : String) : IO Unit := do
  let text ← IO.FS.readFile path
  let out ← IO.getStdout
  let board := (Board.new ByteArray.empty ByteArray.empty).resetDevices 0
  let sys0 := syncEvents (Sys.reset RAM_BASE 0 board) 0
  let mut m : Mach := { rf := zeros RF_BYTES, mem := zeros RAM_SIZE.toNat, sys := sys0, pc := RAM_BASE, nzcv := 0, icnt := 0 }
  let mut idx := 0
  for line in text.splitOn "\n" do
    if line.isEmpty then continue
    let f := (line.splitOn " ").toArray.map parseHex
    let pc := RAM_BASE + 0x10_0000 + 4 * idx.toUInt64
    let mut mem := st32 m.mem (pc - RAM_BASE).toUSize f[0]!
    let mut h : UInt64 := 0x9E37_79B9_7F4A_7C15 ^^^ idx.toUInt64
    for k in [0:0x2000] do
      h := h ^^^ (h <<< 13); h := h ^^^ (h >>> 7); h := h ^^^ (h <<< 17)
      mem := sb mem (0x1000 + k).toUSize h
    let mut rf := m.rf
    for i in [0:31] do rf := wr rf i.toUInt8 f[3 + i]!
    rf := wr rf SP f[1]!
    for i in [0:32] do rf := wv rf i.toUInt8 f[34 + 2 * i]! f[35 + 2 * i]!
    let vb : UInt64 := RAM_BASE + 0x200000
    let s0 := m.sys
    let sys : Sys := { s0 with el := 1, spsel := true, daif := 0x3C0, il := false, vbar := vb }
    let sys : Sys := { sys with cpacr := 0x300000, esr := 0, far := 0, elr := 0, spsr := 0 }
    let sys : Sys := { sys with monArmed := false, power := 0, fpsr := 0, fpcr := 0 }
    m := { m with rf, mem, sys, pc, nzcv := f[2]! &&& 0xF }
    m := drive m (m.retired + 1)
    let mut mh : UInt64 := 0xcbf29ce484222325
    for k in [0:0x2000] do mh := (mh ^^^ gb m.mem (0x1000 + k).toUSize) * 0x100000001b3
    let stop := if m.sys.power == 0 then "BudgetExhausted" else s!"STOP{m.sys.power}"
    let mut s := s!"T {idx} {stop} pc {hex m.pc} nzcv {hex m.nzcv}"
    for i in [0:31] do s := s ++ s!" {hex (rr m.rf i.toUInt8)}"
    s := s ++ s!" sp {hex (spEl1 m.rf m.sys)}"
    for i in [0:32] do s := s ++ s!" {hex (vlo m.rf i.toUInt8)}:{hex (vhi m.rf i.toUInt8)}"
    s := s ++ s!" esr {hex m.sys.esr} far {hex m.sys.far} elr {hex m.sys.elr} spsr {hex m.sys.spsr} fpsr {hex m.sys.fpsr} mem {hex mh}"
    out.putStrLn s
    if m.sys.power != 0 then m := { m with sys := { m.sys with power := 0 } }
    idx := idx + 1

def main (args : List String) : IO UInt32 := do
  match args with
  | "edk2" :: rest =>
    let code ← readFile "/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd"
    let vars ← readFile "/usr/share/AAVMF/AAVMF_VARS.fd"
    let dtb ← readFile ((← IO.getEnv "EDK2_DTB").getD "oracle/ref/dtb.bin")
    run1 (boot code vars dtb) rest "Shell> " "SHELL"
  | ["bare", path] => bare path
  | "linux" :: rest =>
    let kernel ← readFile ((← IO.getEnv "LINUX_IMAGE").getD "/tmp/alp/Image")
    let initrd ← readFile ((← IO.getEnv "LINUX_INITRD").getD "/tmp/alp/initramfs-virt")
    let dtb ← readFile ((← IO.getEnv "LINUX_DTB").getD "oracle/ref/linux_dtb.bin")
    run1 (bootLinux kernel initrd dtb) rest "\x01\x02" "UNTIL"
  | _ => IO.eprintln "usage: leanbox edk2|linux check K MAX [until:TEXT|type:TEXT]... | detail FROM COUNT; leanbox bare TESTS"
  return 0
