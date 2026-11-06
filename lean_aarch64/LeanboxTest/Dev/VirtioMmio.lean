import Leanbox.Dev.VirtioMmio
/-!
# virtio-mmio checks

The empty transport's registers as `#guard`s, checked when this module is built (`lake test`).
-/
namespace Leanbox.Dev.VirtioMmio

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
