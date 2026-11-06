import Leanbox.Dev.Flash
/-!
# CFI flash checks

The cfi_flash.rs unit tests as `#guard`s, checked when this module is built (`lake test`).
-/
namespace Leanbox.Dev.Flash

namespace Tests
open State

-- cfi_flash.rs tests `dual`
def dual (cmd : UInt32) : UInt64 := (cmd ||| (cmd <<< 16)).toUInt64

def wrW (p : State × ByteArray) (o v : UInt64) : State × ByteArray :=
  p.1.write p.2 true o 2 v
def rdW (p : State × ByteArray) (o : UInt64) : State × UInt64 := p.1.read p.2 o 2

-- edk2s_erase_and_buffered_program_sequence_round_trips
def tEdk2 : Bool := Id.run do
  let mut backing := ByteArray.mk (Array.replicate (2 * SECTOR_LEN.toNat) 0)
  for i in [0x40000:0x40010] do
    backing := backing.set! i 0x5A
  let mut p := (State.new backing.size.toUInt64, backing)
  p := wrW p 0x40000 (dual 0x20)
  p := wrW p 0x40000 (dual 0xD0)
  p := wrW p 0x40000 (dual 0x70)
  let (s1, st) := rdW p 0x40000
  p := (s1, p.2)
  let mut ok := st &&& 0x0080_0080 == 0x0080_0080
  p := wrW p 0x40000 (dual 0xE8)
  p := wrW p 0x40000 (dual 3)
  let words : List UInt64 := [0x1111_2222, 0x3333_4444, 0x5555_6666, 0x7777_8888]
  for (w, i) in words.zipIdx do
    p := wrW p (0x40000 + 4 * i.toUInt64) w
  p := wrW p 0x40000 (dual 0xD0)
  p := wrW p 0x40000 (dual 0xFF)
  let (s2, a) := rdW p 0x40004
  p := (s2, p.2)
  let (_, b) := rdW p 0x40010
  ok := ok && a == 0x3333_4444 && b == 0xFFFF_FFFF
  -- the whole buffered block landed in the array, and the rest of the
  -- sector is erased
  ok := ok && (rdW p 0x40000).2 == 0x1111_2222 && (rdW p 0x4000C).2 == 0x7777_8888
    && (rdW p 0x7FFFC).2 == 0xFFFF_FFFF && (rdW p 0x3FFFC).2 == 0
  return ok
#guard tEdk2

-- device_id_and_cfi_query_replicate_across_the_bank
def tDevid : Bool :=
  let p := (State.new SECTOR_LEN, ByteArray.mk (Array.replicate SECTOR_LEN.toNat 0))
  let p := wrW p 0 (dual 0x90)
  let a := (rdW p 0).2
  let b := (rdW p 4).2
  let p := wrW p 0 (dual 0x98)
  let c := (rdW p (0x10 * 4)).2
  a == 0x0089_0089 && b == 0x0018_0018 && c == 0x0051_0051
#guard tDevid

-- a_read_only_bank_reports_erase_errors_and_keeps_its_data
def tReadOnly : Bool :=
  let arr := ByteArray.mk (Array.replicate SECTOR_LEN.toNat 0xAB)
  let s := State.new SECTOR_LEN
  let (s, arr) := s.write arr false 0 2 (dual 0x20)
  let (s, st) := s.read arr 0 2
  let (s, arr) := s.write arr false 0 2 (dual 0xFF)
  st &&& 0x0020_0020 != 0 && (s.read arr 0 0).2 == 0xAB && arr.size == SECTOR_LEN.toNat
    && (s.read arr 0x3FFFC 2).2 == 0xABAB_ABAB
#guard tReadOnly

-- a_command_state_snapshot_round_trips_mid_buffered_write, without the
-- snapshot: finishing the buffered write commits what was collected.
def tBuffered : Bool :=
  let p := (State.new SECTOR_LEN, ByteArray.mk (Array.replicate SECTOR_LEN.toNat 0xFF))
  let p := wrW p 0 (dual 0xE8)
  let p := wrW p 0 (dual 1)
  let p := wrW p 0 0x1234_5678
  let p := wrW p 4 0x9ABC_DEF0
  let p := wrW p 0 (dual 0xD0)
  let p := wrW p 0 (dual 0xFF)
  (rdW p 0).2 == 0x1234_5678 && (rdW p 4).2 == 0x9ABC_DEF0
#guard tBuffered

-- A buffered write of a single word (count 0) opens no block and reports a
-- programming error, as QEMU's model does (cfi_flash.rs module docs); the
-- confirm then leaves the array alone and drops to read-array mode.
def tSingleWordBuffered : Bool :=
  let p := (State.new SECTOR_LEN, ByteArray.mk (Array.replicate SECTOR_LEN.toNat 0xFF))
  let p := wrW p 0 (dual 0xE8)
  let p := wrW p 0 (dual 0)
  let p := wrW p 0 0x1234_5678
  let (s, st) := rdW p 0
  let p := wrW (s, p.2) 0 (dual 0xD0)
  st == 0x0090_0090 && p.1.readsArray && (rdW p 0).2 == 0xFFFF_FFFF
#guard tSingleWordBuffered

-- Word program (0x40) stores the value; status mode until Read Array.
def tWordProgram : Bool :=
  let p := (State.new SECTOR_LEN, ByteArray.mk (Array.replicate SECTOR_LEN.toNat 0xFF))
  let p := wrW p 8 (dual 0x40)
  let p := wrW p 8 0x0000_00F0
  let (s, st) := rdW p 8
  let p := wrW (s, p.2) 8 (dual 0xFF)
  st == 0x0080_0080 && !s.readsArray && (rdW p 8).2 == 0xF0
#guard tWordProgram

-- A short array reads erased flash past its end; an 8-byte array read is one
-- little-endian load; the bank's own CFI table for the board's 64 MiB bank.
def tShortAndTable : Bool :=
  let arr := ByteArray.mk #[1, 2, 3, 4, 5, 6]
  let s := State.new FLASH_BANK_SIZE
  (s.read arr 0 3).2 == 0xFFFF_0605_0403_0201
    && (s.read arr 4 2).2 == 0xFFFF_0605
    && (s.read arr 0x100 2).2 == 0xFFFF_FFFF
    && (s.cfiTable.toList.drop 0x27).take 10 == [25, 2, 0, 0x0B, 0, 1, 0xFF, 0, 0, 2]
#guard tShortAndTable

-- Query mode stays for anything but Read Array; status words carry both
-- devices only on accesses wider than a device; an 8-byte read in query
-- mode is two bank reads.
def tQueryStays : Bool :=
  let p := (State.new SECTOR_LEN, ByteArray.mk (Array.replicate SECTOR_LEN.toNat 0))
  let p := wrW p 0 (dual 0x98)
  let p := wrW p 0 (dual 0x70)
  let q := (p.1.read p.2 (0x10 * 4) 3).2
  let p := wrW p 0 (dual 0xFF)
  let p := wrW p 0 (dual 0x70)
  q == 0x0052_0052_0051_0051 && (p.1.read p.2 0 1).2 == 0x80 && (rdW p 0).2 == 0x0080_0080
#guard tQueryStays

end Tests

end Leanbox.Dev.Flash
