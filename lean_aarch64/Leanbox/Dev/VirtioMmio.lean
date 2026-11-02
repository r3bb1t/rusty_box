/-
  A virtio-mmio transport with no device behind it: an exact port of
  `read_empty_transport` in rusty_box_aarch64/src/devices/virtio_mmio.rs, for
  the board with no virtio devices plugged, whose transports have the legacy
  register layout (`Layout::Legacy`, `Version` 1).

  An empty transport answers the magic value, the version and the board's
  vendor ID, reads every other register (`DeviceID` included, 0 meaning "no
  device") as zero, and ignores writes. Offsets are within one transport
  (board.rs read_outside_ram passes `off % TRANSPORT_SIZE`); widths are
  `log2` (0 byte, 1 half, 2 word, 3 dword).
-/

namespace Leanbox.Dev.VirtioMmio

-- devices/virtio/mod.rs MAGIC: "virt", little-endian
def MAGIC : UInt32 := 0x7472_6976
-- virtio_mmio.rs LEGACY_VERSION
def LEGACY_VERSION : UInt32 := 1
-- devices/virtio/mod.rs MODERN_VERSION: `Version` of a non-legacy transport
def MODERN_VERSION : UInt32 := 2
-- virtio_mmio.rs TRANSPORT_SIZE: bytes of register space per transport
def TRANSPORT_SIZE : UInt64 := 0x200
-- virtio_mmio.rs TRANSPORT_COUNT: transports `virt` creates
def TRANSPORT_COUNT : UInt64 := 32

-- identity.rs IdentityNames::RUSTY_BOX `virtio_vendor`
-- (`VirtioVendorId::from_ascii(*b"RBOX")`, the little-endian bytes "RBOX"),
-- which BoardIdentity::RUSTY_BOX::virtio_vendor() returns.
def VENDOR_ID : UInt32 := 0x584F_4252

-- bus.rs AccessWidth::value_mask
def valueMask (log2 : UInt8) : UInt64 :=
  match log2.toNat with
  | 0 => 0xFF
  | 1 => 0xFFFF
  | 2 => 0xFFFF_FFFF
  | _ => 0xFFFF_FFFF_FFFF_FFFF

-- virtio_mmio.rs read_empty_transport, its `register` closure with
-- `Layout::Legacy`: exact offsets, no decoding of the low bits.
def register (off : UInt64) : UInt64 :=
  (if off == 0x000 then MAGIC
  else if off == 0x004 then LEGACY_VERSION
  else if off == 0x00C then VENDOR_ID
  else 0).toUInt64

-- board.rs read_outside_ram: the register offset within its transport, for
-- an offset into the whole virtio-mmio window.
def transportOffset (windowOff : UInt64) : UInt64 := windowOff % TRANSPORT_SIZE

-- virtio_mmio.rs read_empty_transport (`Layout::Legacy`, the board's vendor):
-- an 8-byte read is `off` then `off + 4`, low word first; a narrower read is
-- the register at `off` masked to the width.
def readEmpty (off : UInt64) (log2 : UInt8) : UInt64 :=
  if log2 ≥ 3 then register off ||| (register (off + 4) <<< 32)
  else register off &&& valueMask log2

/-! ## Checks -/

-- "RBOX" read as a little-endian word
#guard VENDOR_ID == (0x52 : UInt32) ||| ((0x42 : UInt32) <<< 8) ||| ((0x4F : UInt32) <<< 16)
  ||| ((0x58 : UInt32) <<< 24)
#guard readEmpty 0x000 2 == 0x7472_6976
#guard readEmpty 0x004 2 == 1
#guard readEmpty 0x008 2 == 0
#guard readEmpty 0x00C 2 == 0x584F_4252
-- the whole-word value masked to a narrower width, at the register's own
-- offset only
#guard readEmpty 0x000 0 == 0x76
#guard readEmpty 0x00C 1 == 0x4252
#guard readEmpty 0x001 0 == 0
-- an 8-byte read is two registers, low first
#guard readEmpty 0x000 3 == 0x0000_0001_7472_6976
#guard readEmpty 0x008 3 == 0x584F_4252_0000_0000
#guard readEmpty 0x1FC 3 == 0
-- the last transport's VendorID (tests/identity.rs
-- a_virtio_transport_reads_the_identitys_vendor)
#guard readEmpty (transportOffset ((TRANSPORT_COUNT - 1) * TRANSPORT_SIZE + 0x00C)) 2 == 0x584F_4252

end Leanbox.Dev.VirtioMmio
