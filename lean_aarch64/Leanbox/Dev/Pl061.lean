/-!
# PL061 GPIO

  Arm PrimeCell PL061 GPIO: an exact port of
  rusty_box_aarch64/src/devices/pl061.rs (`Pl061`), for lockstep runs
  against the Rust machine.

  Eight lines; on `virt` only line 3 is wired, to the power button. A line's
  level is what `GPIODATA` drives on an output under software control, and
  what the outside drives otherwise. Interrupt detection watches the levels,
  so an output that changes level is an edge too.

  Widths are `log2` (0 byte, 1 half, 2 word, 3 dword); offsets are relative
  to the device's base.
-/

namespace Leanbox.Dev.Pl061

/-! ## Register offsets (pl061.rs `GPIODATA_LAST` ... `GPIOPCELLID3`) -/

def GPIODATA_LAST : UInt64 := 0x3FC
def GPIODIR : UInt64 := 0x400
def GPIOIS : UInt64 := 0x404
def GPIOIBE : UInt64 := 0x408
def GPIOIEV : UInt64 := 0x40C
def GPIOIE : UInt64 := 0x410
def GPIORIS : UInt64 := 0x414
def GPIOMIS : UInt64 := 0x418
def GPIOIC : UInt64 := 0x41C
def GPIOAFSEL : UInt64 := 0x420
def GPIOPERIPHID0 : UInt64 := 0xFE0
def GPIOPCELLID3 : UInt64 := 0xFFC

-- pl061.rs PL061_ID (DDI 0190B Table 3-1)
def PL061_ID : Array UInt8 := #[0x61, 0x10, 0x04, 0x00, 0x0D, 0xF0, 0x05, 0xB1]

-- pl061.rs POWER_BUTTON_LINE: the line QEMU's `virt` wires its power button to
def POWER_BUTTON_LINE : UInt8 := 3

-- pl061.rs register_at: the interface decodes PADDR[11:2]
def registerAt (offset : UInt64) : UInt64 := offset &&& 0xFFC

/-! ## State (pl061.rs `struct Pl061`) -/

structure State where
  /-- `data`: `GPIODATA`, what an output line drives. -/
  data : UInt8 := 0
  /-- `external`: the level the outside drives on each line. -/
  external : UInt8 := 0
  /-- `old_levels`: the lines' levels when interrupt detection last looked. -/
  oldLevels : UInt8 := 0
  dir : UInt8 := 0
  isense : UInt8 := 0
  ibe : UInt8 := 0
  iev : UInt8 := 0
  im : UInt8 := 0
  /-- `edges`: the edge detection latches, which `GPIOIC` clears. -/
  edges : UInt8 := 0
  afsel : UInt8 := 0
deriving Inhabited, BEq, Repr

-- pl061.rs Pl061::new (`Default`: every register and level zero)
def State.new : State := {}

namespace State

-- pl061.rs Pl061::reset: every register clears; the lines become inputs at
-- the levels the outside drives, which latch no edge.
def reset (s : State) : State :=
  { external := s.external, oldLevels := s.external }

-- pl061.rs Pl061::driven: outputs under software control
def driven (s : State) : UInt8 := s.dir &&& ~~~s.afsel

-- pl061.rs Pl061::levels
def levels (s : State) : UInt8 :=
  let d := s.driven
  (s.data &&& d) ||| (s.external &&& ~~~d)

-- pl061.rs Pl061::raw_status: `GPIORIS`
def rawStatus (s : State) : UInt8 :=
  let atLevel := ~~~(s.levels ^^^ s.iev)
  (s.edges &&& ~~~s.isense) ||| (atLevel &&& s.isense)

-- pl061.rs Pl061::irq
def irq (s : State) : Bool := s.rawStatus &&& s.im != 0

-- pl061.rs Pl061::detect_edges
def detectEdges (s : State) : State :=
  let lv := s.levels
  let changed := (lv ^^^ s.oldLevels) &&& ~~~s.isense
  let wanted := s.ibe ||| ~~~(lv ^^^ s.iev)
  { s with oldLevels := lv, edges := s.edges ||| (changed &&& wanted) }

-- pl061.rs Pl061::set_input: drive `line` to `level` from outside
def setInput (s : State) (line : UInt8) (level : Bool) : State :=
  let mask : UInt8 := (1 : UInt8) <<< (line &&& 7)
  let s := { s with external := (s.external &&& ~~~mask) ||| (if level then mask else 0) }
  s.detectEdges

-- pl061.rs Pl061::register
def register (s : State) (offset : UInt64) : UInt64 :=
  let reg := registerAt offset
  (if reg ≤ GPIODATA_LAST then s.levels &&& (reg >>> 2).toUInt8
  else if reg == GPIODIR then s.dir
  else if reg == GPIOIS then s.isense
  else if reg == GPIOIBE then s.ibe
  else if reg == GPIOIEV then s.iev
  else if reg == GPIOIE then s.im
  else if reg == GPIORIS then s.rawStatus
  else if reg == GPIOMIS then s.rawStatus &&& s.im
  else if reg == GPIOAFSEL then s.afsel
  else if GPIOPERIPHID0 ≤ reg && reg ≤ GPIOPCELLID3 then
    PL061_ID[((reg - GPIOPERIPHID0) >>> 2).toNat]!
  else 0).toUInt64

-- pl061.rs Pl061::read via bus.rs read_by_words: an 8-byte access is
-- `offset` then `offset + 4`, low word first; any other width is one
-- register, unshifted. A read changes nothing.
def read (s : State) (off : UInt64) (log2 : UInt8) : State × UInt64 :=
  if log2 ≥ 3 then (s, s.register off ||| (s.register (off + 4) <<< 32))
  else (s, s.register off)

-- pl061.rs Pl061::set_register: every write, reserved locations included,
-- ends with edge detection.
def setRegister (s : State) (offset : UInt64) (v : UInt8) : State :=
  let reg := registerAt offset
  let s :=
    if reg ≤ GPIODATA_LAST then
      let mask := (reg >>> 2).toUInt8
      { s with data := (s.data &&& ~~~mask) ||| (v &&& mask) }
    else if reg == GPIODIR then { s with dir := v }
    else if reg == GPIOIS then { s with isense := v }
    else if reg == GPIOIBE then { s with ibe := v }
    else if reg == GPIOIEV then { s with iev := v }
    else if reg == GPIOIE then { s with im := v }
    else if reg == GPIOIC then { s with edges := s.edges &&& ~~~v }
    else if reg == GPIOAFSEL then { s with afsel := v }
    else s
  s.detectEdges

-- pl061.rs Pl061::write via bus.rs write_by_words (`word as u8`)
def write (s : State) (off : UInt64) (log2 : UInt8) (v : UInt64) : State :=
  if log2 ≥ 3 then
    let s := s.setRegister off (v &&& 0xFFFF_FFFF).toUInt8
    s.setRegister (off + 4) (v >>> 32).toUInt8
  else s.setRegister off v.toUInt8

end State

end Leanbox.Dev.Pl061
