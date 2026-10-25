//! QEMU firmware configuration interface, memory-mapped form.
//!
//! Provenance (R7): QEMU `hw/nvram/fw_cfg.c` (`fw_cfg_data_read`,
//! `fw_cfg_select`, `fw_cfg_init_mem_wide`) and the items `hw/arm/virt.c`
//! installs. Layout at the device's base: data at +0 (1 to 8 bytes), the
//! 16-bit big-endian selector at +8, the DMA address at +16.
//!
//! This board offers the traditional interface only (`FW_CFG_ID` = 1, no
//! DMA) and no files: the file directory is empty. That is a smaller
//! configuration than QEMU's, and a legal one — firmware falls back to the
//! device tree for everything a missing item would have told it. The numeric
//! items QEMU's `virt` installs are present with this board's values.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use crate::bus::{AccessWidth, RegOffset};

const FW_CFG_SIGNATURE: u16 = 0x0000;
const FW_CFG_ID: u16 = 0x0001;
const FW_CFG_UUID: u16 = 0x0002;
const FW_CFG_NOGRAPHIC: u16 = 0x0004;
const FW_CFG_NB_CPUS: u16 = 0x0005;
const FW_CFG_BOOT_MENU: u16 = 0x000E;
const FW_CFG_FILE_DIR: u16 = 0x0019;

/// Feature bits in `FW_CFG_ID`: the traditional (port/MMIO) interface.
const FW_CFG_VERSION: u32 = 0x01;

/// The longest item this board serves.
const MAX_ITEM: usize = 16;

pub struct FwCfg {
    selector: u16,
    offset: usize,
}

impl Default for FwCfg {
    fn default() -> Self {
        Self::new()
    }
}

/// One item's bytes.
struct Item {
    bytes: [u8; MAX_ITEM],
    len: usize,
}

impl Item {
    fn from(slice: &[u8]) -> Self {
        let mut bytes = [0u8; MAX_ITEM];
        let len = slice.len().min(MAX_ITEM);
        bytes[..len].copy_from_slice(&slice[..len]);
        Self { bytes, len }
    }
}

impl FwCfg {
    pub fn new() -> Self {
        Self {
            selector: FW_CFG_SIGNATURE,
            offset: 0,
        }
    }

    /// The bytes of the selected item; an unknown selector reads as empty,
    /// which QEMU answers with zeros (`FW_CFG_INVALID`).
    fn item(&self) -> Item {
        match self.selector {
            FW_CFG_SIGNATURE => Item::from(b"QEMU"),
            FW_CFG_ID => Item::from(&FW_CFG_VERSION.to_le_bytes()),
            FW_CFG_UUID => Item::from(&[0u8; 16]),
            FW_CFG_NOGRAPHIC => Item::from(&1u16.to_le_bytes()),
            FW_CFG_NB_CPUS => Item::from(&1u16.to_le_bytes()),
            FW_CFG_BOOT_MENU => Item::from(&0u16.to_le_bytes()),
            // A big-endian count of zero files.
            FW_CFG_FILE_DIR => Item::from(&0u32.to_be_bytes()),
            _ => Item::from(&[]),
        }
    }

    pub fn read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        match offset.get() {
            0 => {
                // The next `width` bytes of the item, in memory order: a
                // little-endian load sees the first byte in its low bits.
                // Past the end of the item the bytes read as zero.
                let item = self.item();
                let mut v = 0u64;
                for i in 0..width.bytes() as usize {
                    if self.offset < item.len {
                        v |= u64::from(item.bytes[self.offset]) << (8 * i);
                        self.offset += 1;
                    }
                }
                v
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        // The selector is a 16-bit big-endian register; any other write is
        // ignored, as QEMU ignores writes to the data register.
        if offset.get() == 8 && width == AccessWidth::Half {
            self.selector = (value as u16).swap_bytes();
            self.offset = 0;
        }
    }

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u16(self.selector)?;
        w.write_u32(self.offset as u32)
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        self.selector = r.read_u16()?;
        let offset = r.read_u32()? as usize;
        if offset > MAX_ITEM {
            return Err(SnapError::Invalid("fw_cfg read offset is past the longest item"));
        }
        self.offset = offset;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EDK2's QemuFwCfgLib selects with `SwapBytes16(item)` and reads the
    /// signature bytes in order.
    #[test]
    fn the_signature_reads_in_memory_order() {
        let mut f = FwCfg::new();
        f.write(RegOffset::new(8), AccessWidth::Half, u64::from(FW_CFG_SIGNATURE.swap_bytes()));
        assert_eq!(f.read(RegOffset::new(0), AccessWidth::Word), u64::from(u32::from_le_bytes(*b"QEMU")));
        f.write(RegOffset::new(8), AccessWidth::Half, u64::from(FW_CFG_ID.swap_bytes()));
        assert_eq!(f.read(RegOffset::new(0), AccessWidth::Dword), 1);
    }
}
