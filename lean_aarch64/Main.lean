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

def console (m : Mach) : ByteArray := m.sys.board.uart.out

def hasShell (b : ByteArray) : Bool := Id.run do
  let pat := "Shell> ".toUTF8
  if b.size < pat.size then return false
  for i in [0:b.size - pat.size + 1] do
    let mut ok := true
    for j in [0:pat.size] do
      if b[i + j]! != pat[j]! then ok := false; break
    if ok then return true
  return false

partial def checkLoop (m : Mach) (k max : UInt64) (out : IO.FS.Stream) : IO Mach := do
  if m.retired >= max then return m
  let m := drive m (m.retired + k)
  let c := console m
  out.putStrLn s!"C {m.retired} {m.icnt} {hex m.pc} {String.ofList ((Nat.toDigits 16 (stateHash m).toNat).leftpad 16 (Char.ofNat 48))} {c.size}"
  if m.sys.power != 0 then
    out.putStrLn s!"STOP {m.sys.power} {m.sys.stopWhy}"
    return m
  if hasShell (c.extract (c.size - min c.size 64) c.size) then
    out.putStrLn "SHELL"
    return m
  checkLoop m k max out

def regsLine (m : Mach) : String := Id.run do
  let mut s := ""
  for i in [0:31] do s := s ++ s!" {hex (rr m.rf i.toUInt8)}"
  s ++ s!" sp0 {hex (spEl0 m.rf m.sys)} sp1 {hex (spEl1 m.rf m.sys)} nzcv {hex (m.nzcv <<< 28)} daif {hex m.sys.daif}"

def main (args : List String) : IO UInt32 := do
  let code ← readFile "/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd"
  let vars ← readFile "/usr/share/AAVMF/AAVMF_VARS.fd"
  let dtb ← readFile (args.getD 3 "oracle/ref/dtb.bin")
  let m := boot code vars dtb
  let out ← IO.getStdout
  match args with
  | "check" :: k :: max :: _ =>
    let t0 ← IO.monoNanosNow
    let m ← checkLoop m k.toNat!.toUInt64 max.toNat!.toUInt64 out
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
  | _ => IO.eprintln "usage: leanbox check K MAX | detail FROM COUNT"
  return 0
