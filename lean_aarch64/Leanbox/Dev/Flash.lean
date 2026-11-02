/-
  Intel/Sharp command-set CFI parallel flash: an exact port of
  rusty_box_aarch64/src/devices/cfi_flash.rs (`CfiFlash`), for lockstep runs
  against the Rust machine.

  The bank is QEMU `virt`'s: 32 bits wide, two 16-bit devices, manufacturer
  0x89, device 0x18, 256 KiB sectors, little-endian. The state holds the
  command state machine only; the array is the machine's storage and is
  passed to every access (cfi_flash.rs `FlashArray`): `writable = false` is
  `FlashArray::ReadOnly`, `writable = true` is `FlashArray::Writable`.

  Widths are `log2` (0 byte, 1 half, 2 word, 3 dword); offsets are relative
  to the bank's base. The array may be shorter than the bank: a read past its
  end reads erased flash (0xFF) and a write there changes nothing.
-/

namespace Leanbox.Dev.Flash

/-! ## Geometry (cfi_flash.rs constants) -/

-- cfi_flash.rs BANK_WIDTH
def BANK_WIDTH : UInt64 := 4
-- cfi_flash.rs `BANK_WIDTH.trailing_zeros()`
def BANK_SHIFT : UInt64 := 2
-- cfi_flash.rs DEVICE_WIDTH
def DEVICE_WIDTH : UInt64 := 2
-- cfi_flash.rs NUM_DEVICES
def NUM_DEVICES : UInt64 := BANK_WIDTH / DEVICE_WIDTH
-- cfi_flash.rs SECTOR_LEN (QEMU `VIRT_FLASH_SECTOR_SIZE`)
def SECTOR_LEN : UInt64 := 256 * 1024
-- cfi_flash.rs IDENT0 / IDENT1
def IDENT0 : UInt32 := 0x89
def IDENT1 : UInt32 := 0x18
-- cfi_flash.rs WRITEBLOCK_SIZE: `(1 << cfi_table[0x2A]) * NUM_DEVICES`
def WRITEBLOCK_SIZE : Nat := (1 <<< 0x0B) * NUM_DEVICES.toNat
-- cfi_flash.rs CFI_TABLE_LEN
def CFI_TABLE_LEN : Nat := 0x52

-- cfi_flash.rs STATUS_*
def STATUS_READY : UInt8 := 0x80
def STATUS_ERASE_ERROR : UInt8 := 0x20
def STATUS_PROGRAM_ERROR : UInt8 := 0x10

-- board.rs FLASH_BANK_SIZE: the window (and `len`) of each bank on the board
def FLASH_BANK_SIZE : UInt64 := 0x0400_0000

/-! ## Access widths (bus.rs AccessWidth) -/

-- bus.rs AccessWidth::bytes
def widthBytes (log2 : UInt8) : UInt64 :=
  match log2.toNat with
  | 0 => 1
  | 1 => 2
  | 2 => 4
  | _ => 8

-- bus.rs AccessWidth::value_mask
def valueMask (log2 : UInt8) : UInt64 :=
  match log2.toNat with
  | 0 => 0xFF
  | 1 => 0xFFFF
  | 2 => 0xFFFF_FFFF
  | _ => 0xFFFF_FFFF_FFFF_FFFF

/-! ## CFI table (cfi_flash.rs CfiFlash::new) -/

-- `u64::trailing_zeros`
def trailingZeros (x : UInt64) : Nat :=
  go x 0
where
  go (x : UInt64) (n : Nat) : Nat :=
    if n ≥ 64 then 64
    else if x &&& 1 == 1 then n
    else go (x >>> 1) (n + 1)
  termination_by 64 - n

-- cfi_flash.rs CfiFlash::new, the table QEMU `pflash_cfi01_fill_cfi_table` builds
def cfiTableFor (len : UInt64) : ByteArray := Id.run do
  let blocksPerDevice := max (len / SECTOR_LEN) 1
  let sectorLenPerDevice := SECTOR_LEN / NUM_DEVICES
  let deviceLen := sectorLenPerDevice * blocksPerDevice
  let entries : List (Nat × UInt8) :=
    [ (0x10, 0x51), (0x11, 0x52), (0x12, 0x59)            -- "QRY"
    , (0x13, 0x01), (0x15, 0x31)
    , (0x1B, 0x45), (0x1C, 0x55), (0x1F, 0x07), (0x20, 0x07), (0x21, 0x0A)
    , (0x23, 0x04), (0x24, 0x04), (0x25, 0x04)
    , (0x27, (trailingZeros deviceLen).toUInt8)
    , (0x28, 0x02), (0x2A, 0x0B), (0x2C, 0x01)
    , (0x2D, (blocksPerDevice - 1).toUInt8)
    , (0x2E, ((blocksPerDevice - 1) >>> 8).toUInt8)
    , (0x2F, (sectorLenPerDevice >>> 8).toUInt8)
    , (0x30, (sectorLenPerDevice >>> 16).toUInt8)
    , (0x31, 0x50), (0x32, 0x52), (0x33, 0x49), (0x34, 0x31), (0x35, 0x30)  -- "PRI10"
    , (0x3F, 0x01) ]
  let mut t := ByteArray.mk (Array.replicate CFI_TABLE_LEN 0)
  for (i, b) in entries do
    t := t.set! i b
  return t

/-! ## State (cfi_flash.rs `struct CfiFlash`) -/

structure State where
  /-- `len`: the bank's length in bytes, what its window decodes. -/
  len : UInt64
  /-- `cfi_table` (`CFI_TABLE_LEN` bytes). -/
  cfiTable : ByteArray
  /-- `cmd`: the command in progress; 0x00 is read-array mode. -/
  cmd : UInt8
  wcycle : UInt8
  status : UInt8
  counter : UInt32
  /-- `blk_offset`: start of the block a buffered write is collecting. -/
  blkOffset : Option UInt64
  /-- `blk_bytes` (`WRITEBLOCK_SIZE` bytes). -/
  blkBytes : ByteArray
deriving Inhabited

-- cfi_flash.rs CfiFlash::new
def State.new (len : UInt64) : State :=
  { len := len
    cfiTable := cfiTableFor len
    cmd := 0
    wcycle := 0
    status := STATUS_READY
    counter := 0
    blkOffset := none
    blkBytes := ByteArray.mk (Array.replicate WRITEBLOCK_SIZE 0) }

/-! ## The array (cfi_flash.rs array_read and the stores into `FlashArray`) -/

-- cfi_flash.rs array_read: the little-endian value of `bytes` bytes at
-- `offset`, reading erased flash (all ones) past the end of a short image.
def arrayRead (store : ByteArray) (offset : UInt64) (bytes : UInt64) : UInt64 := Id.run do
  let base := offset.toNat
  let mut v : UInt64 := 0
  for i in [0:bytes.toNat] do
    let idx := base + i
    let b : UInt8 := if h : idx < store.size then store[idx] else 0xFF
    v := v ||| (b.toUInt64 <<< (8 * i.toUInt64))
  return v

-- cfi_flash.rs CfiFlash::store, `None` arm with `FlashArray::Writable`: the
-- low `bytes` bytes of `value` at `offset`, those inside the array.
def storeBytes (arr : ByteArray) (offset : UInt64) (value : UInt32) (bytes : UInt64) : ByteArray := Id.run do
  let base := offset.toNat
  let mut a := arr
  for i in [0:bytes.toNat] do
    let idx := base + i
    if idx < a.size then
      a := a.set! idx (value >>> (8 * i.toUInt32)).toUInt8
  return a

-- cfi_flash.rs CfiFlash::write32, block erase: `store[start..end].fill(0xFF)`
-- when `start..end` lies in the array.
def eraseRange (arr : ByteArray) (start stop : Nat) : ByteArray := Id.run do
  let mut a := arr
  if start ≤ stop && stop ≤ a.size then
    for i in [start:stop] do
      a := a.set! i 0xFF
  return a

-- cfi_flash.rs CfiFlash::write32, cycle 2 of 0xE8: the block at `start` as
-- the array holds it, erased (0xFF) past the array's end.
def blockFrom (arr : ByteArray) (start : UInt64) : ByteArray := Id.run do
  let base := start.toNat
  let mut b := ByteArray.emptyWithCapacity WRITEBLOCK_SIZE
  for i in [0:WRITEBLOCK_SIZE] do
    let idx := base + i
    b := b.push (if h : idx < arr.size then arr[idx] else 0xFF)
  return b

namespace State

-- cfi_flash.rs CfiFlash::reads_array
def readsArray (s : State) : Bool := s.cmd == 0

-- cfi_flash.rs CfiFlash::reset (QEMU `pflash_cfi01_system_reset`): back to
-- read-array mode, any buffered write abandoned; `blk_bytes` is kept.
def reset (s : State) : State :=
  { s with cmd := 0, wcycle := 0, status := STATUS_READY, counter := 0, blkOffset := none }

-- cfi_flash.rs CfiFlash::set_read_array
def setReadArray (s : State) : State := { s with wcycle := 0, cmd := 0 }

-- cfi_flash.rs CfiFlash::replicate
def replicate (resp : UInt32) : UInt32 := resp ||| (resp <<< 16)

-- cfi_flash.rs CfiFlash::cfi_query
def cfiQuery (s : State) (offset : UInt64) : UInt32 :=
  let boff := (offset >>> BANK_SHIFT).toNat
  if h : boff < s.cfiTable.size then replicate s.cfiTable[boff].toUInt32 else 0

-- cfi_flash.rs CfiFlash::devid_query
def devidQuery (offset : UInt64) : UInt32 :=
  let boff := offset >>> BANK_SHIFT
  match (boff &&& 0xFF).toNat with
  | 0 => replicate IDENT0
  | 1 => replicate IDENT1
  | _ => 0

-- cfi_flash.rs CfiFlash::read32 (QEMU `pflash_read`), for accesses up to the
-- bank width.
def read32 (s : State) (store : ByteArray) (offset bytes : UInt64) : State × UInt32 :=
  match s.cmd.toNat with
  | 0x10 | 0x20 | 0x28 | 0x40 | 0x50 | 0x60 | 0x70 | 0xE8 =>
    let st := s.status.toUInt32
    (s, if bytes > DEVICE_WIDTH then st ||| (st <<< 16) else st)
  | 0x90 => (s, devidQuery offset)
  | 0x98 => (s, s.cfiQuery offset)
  | 0x00 => (s, (arrayRead store offset bytes).toUInt32)
  | _ => (s.setReadArray, (arrayRead store offset bytes).toUInt32)

-- cfi_flash.rs CfiFlash::read: a read of the bank whose array is `arr`.
def read (s : State) (arr : ByteArray) (off : UInt64) (log2 : UInt8) : State × UInt64 :=
  if s.cmd == 0 then (s, arrayRead arr off (widthBytes log2))
  else if log2 ≥ 3 then
    let (s, lo) := s.read32 arr off 4
    let (s, hi) := s.read32 arr (off + 4) 4
    (s, lo.toUInt64 ||| (hi.toUInt64 <<< 32))
  else
    let (s, v) := s.read32 arr off (widthBytes log2)
    (s, v.toUInt64 &&& valueMask log2)

-- cfi_flash.rs CfiFlash::store: into the open buffered write's block, or,
-- with none open, into a writable array.
def store (s : State) (arr : ByteArray) (writable : Bool) (offset : UInt64) (value : UInt32)
    (bytes : UInt64) : State × ByteArray :=
  match s.blkOffset with
  | some blk =>
    if offset < blk || offset + bytes > blk + WRITEBLOCK_SIZE.toUInt64 then
      ({ s with status := s.status ||| STATUS_PROGRAM_ERROR }, arr)
    else
      let b := s.blkBytes
      let s := { s with blkBytes := ByteArray.empty }
      let base := (offset - blk).toNat
      let b := Id.run do
        let mut b := b
        for i in [0:bytes.toNat] do
          if base + i < b.size then
            b := b.set! (base + i) (value >>> (8 * i.toUInt32)).toUInt8
        return b
      ({ s with blkBytes := b }, arr)
  | none =>
    if writable then (s, storeBytes arr offset value bytes) else (s, arr)

-- cfi_flash.rs CfiFlash::write32 (QEMU `pflash_write`)
def write32 (s : State) (arr : ByteArray) (writable : Bool) (offset : UInt64) (value : UInt32)
    (bytes : UInt64) : State × ByteArray :=
  let readOnly := !writable
  let cmd := value.toUInt8
  match s.wcycle.toNat with
  | 0 =>
    match cmd.toNat with
    | 0x00 | 0xF0 | 0xFF => (s.setReadArray, arr)
    | 0x10 | 0x40 | 0x60 | 0x98 => ({ s with wcycle := s.wcycle + 1, cmd := cmd }, arr)
    | 0x20 =>
      let start := offset &&& ~~~(SECTOR_LEN - 1)
      let (s, arr) :=
        if writable then
          let stop := min (start + SECTOR_LEN).toNat arr.size
          (s, eraseRange arr start.toNat stop)
        else ({ s with status := s.status ||| STATUS_ERASE_ERROR }, arr)
      ({ s with status := s.status ||| STATUS_READY, wcycle := s.wcycle + 1, cmd := cmd }, arr)
    | 0x50 => ({ s with status := 0 }.setReadArray, arr)
    | 0x70 | 0x90 => ({ s with cmd := cmd }, arr)
    | 0xE8 =>
      ({ s with status := s.status ||| STATUS_READY, wcycle := s.wcycle + 1, cmd := cmd }, arr)
    | _ => (s.setReadArray, arr)
  | 1 =>
    match s.cmd.toNat with
    | 0x10 | 0x40 =>
      let (s, arr) :=
        if readOnly then ({ s with status := s.status ||| STATUS_PROGRAM_ERROR }, arr)
        else s.store arr writable offset value bytes
      ({ s with status := s.status ||| STATUS_READY, wcycle := 0 }, arr)
    | 0x20 | 0x28 =>
      if cmd == 0xD0 then ({ s with wcycle := 0, status := s.status ||| STATUS_READY }, arr)
      else (s.setReadArray, arr)
    | 0xE8 => ({ s with counter := value &&& 0xFFFF, wcycle := s.wcycle + 1 }, arr)
    | 0x60 =>
      if cmd == 0xD0 || cmd == 0x01 then
        ({ s with wcycle := 0, status := s.status ||| STATUS_READY }, arr)
      else (s.setReadArray, arr)
    -- In query mode only Read Array leaves; anything else stays.
    | 0x98 => if cmd == 0xFF then (s.setReadArray, arr) else (s, arr)
    | _ => (s.setReadArray, arr)
  | 2 =>
    if s.cmd == 0xE8 then
      let s :=
        if s.blkOffset.isNone && s.counter != 0 then
          let start := offset &&& ~~~(WRITEBLOCK_SIZE.toUInt64 - 1)
          { s with blkBytes := blockFrom arr start, blkOffset := some start }
        else s
      let (s, arr) :=
        if !readOnly && s.blkOffset.isSome then s.store arr writable offset value bytes
        else ({ s with status := s.status ||| STATUS_PROGRAM_ERROR }, arr)
      let s := { s with status := s.status ||| STATUS_READY }
      let s : State :=
        if s.counter == (0 : UInt32) then { s with wcycle := s.wcycle + 1 }
        else { s with counter := s.counter - 1 }
      (s, arr)
    else (s.setReadArray, arr)
  | 3 =>
    if s.cmd == 0xE8 && cmd == 0xD0 && s.status &&& STATUS_PROGRAM_ERROR == 0 then
      -- Confirm: commit the collected block.
      let arr :=
        match s.blkOffset with
        | some blk =>
          if writable then
            let base := blk.toNat
            let stop := min (base + WRITEBLOCK_SIZE) arr.size
            if base ≤ stop then Id.run do
              let mut a := arr
              for i in [0:stop - base] do
                a := a.set! (base + i) s.blkBytes[i]!
              return a
            else arr
          else arr
        | none => arr
      ({ s with blkOffset := none, wcycle := 0, status := s.status ||| STATUS_READY }, arr)
    else
      ({ s with blkOffset := none }.setReadArray, arr)
  | _ => (s.setReadArray, arr)

-- cfi_flash.rs CfiFlash::write: a write to the bank whose array is `arr`
-- (`writable = false` is `FlashArray::ReadOnly`). An 8-byte access is two
-- 4-byte command cycles, the low word first.
def write (s : State) (arr : ByteArray) (writable : Bool) (off : UInt64) (log2 : UInt8)
    (v : UInt64) : State × ByteArray :=
  if log2 ≥ 3 then
    let (s, arr) := s.write32 arr writable off v.toUInt32 4
    s.write32 arr writable (off + 4) (v >>> 32).toUInt32 4
  else
    s.write32 arr writable off v.toUInt32 (widthBytes log2)

end State

/-! ## Checks reproducing cfi_flash.rs unit tests -/

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
