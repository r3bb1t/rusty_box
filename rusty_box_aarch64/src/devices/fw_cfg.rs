//! QEMU firmware configuration interface, memory-mapped form.
//!
//! Provenance (R7): QEMU `hw/nvram/fw_cfg.c` (`fw_cfg_data_read`,
//! `fw_cfg_select`, `fw_cfg_init_mem_dma`, `fw_cfg_dma_mem_read`,
//! `fw_cfg_dma_mem_write`, `fw_cfg_dma_transfer`) and the items
//! `hw/arm/virt.c` installs. Layout at the device's base: data at +0 (1 to 8
//! bytes), the 16-bit big-endian selector at +8, the big-endian DMA address
//! at +16.
//!
//! This board offers the traditional and the DMA interface (`FW_CFG_ID` = 3,
//! as QEMU's `virt`) and no files: the file directory is empty. That is a
//! smaller configuration than QEMU's, and a legal one — firmware falls back
//! to the device tree for everything a missing item would have told it. The
//! numeric items QEMU's `virt` installs are present with this board's values.
//! EDK2's `QemuFwCfgMmioPei` turns DMA on from the device tree's region size
//! alone, so a 0x18-byte region must answer DMA.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use crate::bus::{AccessWidth, BusFault, RegOffset};
use crate::devices::virtio::queue::GuestRam;

const FW_CFG_SIGNATURE: u16 = 0x0000;
const FW_CFG_ID: u16 = 0x0001;
const FW_CFG_UUID: u16 = 0x0002;
const FW_CFG_NOGRAPHIC: u16 = 0x0004;
const FW_CFG_NB_CPUS: u16 = 0x0005;
const FW_CFG_BOOT_MENU: u16 = 0x000E;
const FW_CFG_FILE_DIR: u16 = 0x0019;

/// Feature bits in `FW_CFG_ID`: the traditional (port/MMIO) interface and
/// the DMA interface.
const FW_CFG_VERSION: u32 = 0x01;
const FW_CFG_VERSION_DMA: u32 = 0x02;

/// `FWCfgDmaAccess.control` bits (QEMU `qemu_fw_cfg.h`).
const DMA_CTL_ERROR: u32 = 0x01;
const DMA_CTL_READ: u32 = 0x02;
const DMA_CTL_SKIP: u32 = 0x04;
const DMA_CTL_SELECT: u32 = 0x08;
const DMA_CTL_WRITE: u32 = 0x10;

/// The DMA register's read value, QEMU's `FW_CFG_DMA_SIGNATURE`, in the
/// order a big-endian register presents it to memory.
const DMA_SIGNATURE: &[u8; 8] = b"QEMU CFG";

/// The DMA register's offset in the device, and its low word's.
const DMA_OFFSET: u64 = 16;
const DMA_LOW_OFFSET: u64 = 20;

/// Where an access of `width` at `off` starts in the 8-byte DMA register,
/// when it lies wholly inside it.
fn dma_register_span(off: u64, width: AccessWidth) -> Option<usize> {
    let start = off.checked_sub(DMA_OFFSET)?;
    (start + width.bytes() <= DMA_SIGNATURE.len() as u64).then_some(start as usize)
}

/// The longest item this board serves.
const MAX_ITEM: usize = 16;

/// The direction of a DMA transfer (`fw_cfg_dma_transfer`'s `read` and
/// `write` flags); a skip only moves the item's offset.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DmaMode {
    Read,
    Write,
    Skip,
}

pub struct FwCfg {
    selector: u16,
    offset: usize,
    /// The descriptor address being written, QEMU's `dma_addr`: the high
    /// word of a split write waits here for the low one.
    dma_addr: u64,
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
            dma_addr: 0,
        }
    }

    /// QEMU `fw_cfg_select`: a new item, read from its start.
    fn select(&mut self, key: u16) {
        self.selector = key;
        self.offset = 0;
    }

    /// QEMU `fw_cfg_reset`: the signature selected again. A DMA address half
    /// written before the reset stays, as QEMU's `dma_addr` does.
    pub fn reset(&mut self) {
        self.select(FW_CFG_SIGNATURE);
    }

    /// The bytes of the selected item; an unknown selector reads as empty,
    /// which QEMU answers with zeros (`FW_CFG_INVALID`).
    fn item(&self) -> Item {
        match self.selector {
            FW_CFG_SIGNATURE => Item::from(b"QEMU"),
            FW_CFG_ID => Item::from(&(FW_CFG_VERSION | FW_CFG_VERSION_DMA).to_le_bytes()),
            FW_CFG_UUID => Item::from(&[0u8; 16]),
            FW_CFG_NOGRAPHIC => Item::from(&1u16.to_le_bytes()),
            FW_CFG_NB_CPUS => Item::from(&1u16.to_le_bytes()),
            FW_CFG_BOOT_MENU => Item::from(&0u16.to_le_bytes()),
            // A big-endian count of zero files.
            FW_CFG_FILE_DIR => Item::from(&0u32.to_be_bytes()),
            _ => Item::from(&[]),
        }
    }

    /// A load from the device. QEMU's `virt` maps three regions here and
    /// rejects every other access with a decode error, which an Arm
    /// processor takes as a synchronous external abort
    /// (`arm_cpu_do_transaction_failed`): the data register answers only at
    /// its first byte (`fw_cfg_data_mem_valid`), the selector is write-only
    /// (`fw_cfg_ctl_mem_valid`), and +10..+16 decodes nothing.
    pub fn read(&mut self, offset: RegOffset, width: AccessWidth) -> Result<u64, BusFault> {
        let off = offset.get();
        if off == 0 {
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
            return Ok(v);
        }
        // QEMU `fw_cfg_dma_mem_read`: the signature, any width that stays in
        // the register, in memory order.
        if let Some(start) = dma_register_span(off, width) {
            let mut v = 0u64;
            for (i, &byte) in DMA_SIGNATURE[start..start + width.bytes() as usize].iter().enumerate() {
                v |= u64::from(byte) << (8 * i);
            }
            return Ok(v);
        }
        Err(BusFault::Unmapped)
    }

    /// A store to the device, rejected where QEMU's region callbacks reject
    /// it (see [`Self::read`]).
    pub fn write(
        &mut self,
        ram: &mut GuestRam<'_>,
        offset: RegOffset,
        width: AccessWidth,
        value: u64,
    ) -> Result<(), BusFault> {
        match (offset.get(), width) {
            // QEMU accepts and drops writes to the data register
            // (`fw_cfg_write`: write support removed in QEMU v2.4).
            (0, _) => {}
            // The selector is a 16-bit big-endian register.
            (8, AccessWidth::Half) => self.select((value as u16).swap_bytes()),
            // QEMU `fw_cfg_dma_mem_write` (with `fw_cfg_dma_mem_valid`): a
            // big-endian register taken as two words, high then low, or one
            // doubleword; the store that completes the address starts the
            // transfer.
            (DMA_OFFSET, AccessWidth::Word) => {
                self.dma_addr = u64::from((value as u32).swap_bytes()) << 32;
            }
            (DMA_LOW_OFFSET, AccessWidth::Word) => {
                self.dma_addr |= u64::from((value as u32).swap_bytes());
                self.dma_transfer(ram);
            }
            (DMA_OFFSET, AccessWidth::Dword) => {
                self.dma_addr = value.swap_bytes();
                self.dma_transfer(ram);
            }
            _ => return Err(BusFault::Unmapped),
        }
        Ok(())
    }

    /// QEMU `fw_cfg_dma_transfer`: performs the descriptor at the written
    /// address and stores its outcome in the descriptor's `control`, zero
    /// on success.
    fn dma_transfer(&mut self, ram: &mut GuestRam<'_>) {
        let desc = self.dma_addr;
        self.dma_addr = 0;
        let mut raw = [0u8; 16];
        if ram.read(desc, &mut raw).is_none() {
            ram.store(desc, &DMA_CTL_ERROR.to_be_bytes());
            return;
        }
        let word = |at: usize| u32::from_be_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
        let control = word(0);
        let mut length = u64::from(word(4));
        let mut address = (u64::from(word(8)) << 32) | u64::from(word(12));

        if control & DMA_CTL_SELECT != 0 {
            self.select((control >> 16) as u16);
        }
        let mode = if control & DMA_CTL_READ != 0 {
            DmaMode::Read
        } else if control & DMA_CTL_WRITE != 0 {
            DmaMode::Write
        } else if control & DMA_CTL_SKIP != 0 {
            DmaMode::Skip
        } else {
            length = 0;
            DmaMode::Skip
        };

        let item = self.item();
        let mut status = 0u32;
        while length > 0 && status & DMA_CTL_ERROR == 0 {
            let len = if self.offset >= item.len {
                // Past the item's end, or no item: a read fills zeros, a
                // write fails.
                match mode {
                    DmaMode::Read => {
                        if ram.fill(address, length, 0).is_none() {
                            status |= DMA_CTL_ERROR;
                        }
                    }
                    DmaMode::Write => status |= DMA_CTL_ERROR,
                    DmaMode::Skip => {}
                }
                length
            } else {
                let len = length.min((item.len - self.offset) as u64);
                let bytes = &item.bytes[self.offset..self.offset + len as usize];
                match mode {
                    DmaMode::Read => {
                        if ram.try_store(address, bytes).is_none() {
                            status |= DMA_CTL_ERROR;
                        }
                    }
                    // No item this board serves takes writes (QEMU's
                    // `allow_write`).
                    DmaMode::Write => status |= DMA_CTL_ERROR,
                    DmaMode::Skip => {}
                }
                self.offset += len as usize;
                len
            };
            address = address.wrapping_add(len);
            length -= len;
        }
        ram.store(desc, &status.to_be_bytes());
    }

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u16(self.selector)?;
        w.write_u32(self.offset as u32)?;
        w.write_u64(self.dma_addr)
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        self.selector = r.read_u16()?;
        let offset = r.read_u32()? as usize;
        if offset > MAX_ITEM {
            return Err(SnapError::Invalid(
                "fw_cfg read offset is past the longest item",
            ));
        }
        self.offset = offset;
        self.dma_addr = r.read_u64()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::RAM_BASE;

    /// EDK2's QemuFwCfgLib selects with `SwapBytes16(item)` and reads the
    /// signature bytes in order.
    #[test]
    fn the_signature_reads_in_memory_order() {
        let mut f = FwCfg::new();
        let mut backing = [0u8; 16];
        let mut ram = GuestRam::new(&mut backing);
        f.write(
            &mut ram,
            RegOffset::new(8),
            AccessWidth::Half,
            u64::from(FW_CFG_SIGNATURE.swap_bytes()),
        )
        .expect("a selector write");
        assert_eq!(
            f.read(RegOffset::new(0), AccessWidth::Word),
            Ok(u64::from(u32::from_le_bytes(*b"QEMU")))
        );
        f.write(
            &mut ram,
            RegOffset::new(8),
            AccessWidth::Half,
            u64::from(FW_CFG_ID.swap_bytes()),
        )
        .expect("a selector write");
        assert_eq!(f.read(RegOffset::new(0), AccessWidth::Dword), Ok(3));
    }

    /// Every access QEMU's `virt` rejects with a decode error faults, so the
    /// processor takes a synchronous external abort (DFSC 0b010000, Arm
    /// `EncodeLDFSC`); a write to the data register is accepted and dropped.
    #[test]
    fn the_registers_fault_where_qemu_rejects_the_access() {
        let mut f = FwCfg::new();
        let mut backing = [0u8; 16];
        let mut ram = GuestRam::new(&mut backing);
        let unmapped = Err(BusFault::Unmapped);
        // The data register answers only at its first byte.
        assert_eq!(f.read(RegOffset::new(1), AccessWidth::Byte), unmapped);
        // The selector is write-only, and only as a halfword.
        assert_eq!(f.read(RegOffset::new(8), AccessWidth::Half), unmapped);
        assert_eq!(f.write(&mut ram, RegOffset::new(8), AccessWidth::Word, 0), Err(BusFault::Unmapped));
        // Nothing decodes between the selector and the DMA register.
        assert_eq!(f.read(RegOffset::new(12), AccessWidth::Word), unmapped);
        // The DMA register takes words at +16 and +20 and a doubleword at +16.
        assert_eq!(f.write(&mut ram, RegOffset::new(16), AccessWidth::Byte, 0), Err(BusFault::Unmapped));
        assert_eq!(f.write(&mut ram, RegOffset::new(18), AccessWidth::Word, 0), Err(BusFault::Unmapped));
        assert_eq!(f.write(&mut ram, RegOffset::new(20), AccessWidth::Dword, 0), Err(BusFault::Unmapped));
        // A read must stay inside the register.
        assert_eq!(f.read(RegOffset::new(20), AccessWidth::Dword), unmapped);
        assert_eq!(f.write(&mut ram, RegOffset::new(0), AccessWidth::Dword, 0), Ok(()));
    }

    /// QEMU `fw_cfg_reset` selects the signature and leaves a half-written
    /// DMA address in place.
    #[test]
    fn a_reset_selects_the_signature_and_keeps_the_dma_high_word() {
        let mut f = FwCfg::new();
        let mut backing = [0u8; 16];
        let mut ram = GuestRam::new(&mut backing);
        f.write(&mut ram, RegOffset::new(8), AccessWidth::Half, u64::from(FW_CFG_ID.swap_bytes()))
            .expect("a selector write");
        f.write(&mut ram, RegOffset::new(16), AccessWidth::Word, u64::from(0x12u32.swap_bytes()))
            .expect("the high word");
        f.reset();
        assert_eq!(f.dma_addr, 0x12 << 32);
        assert_eq!(
            f.read(RegOffset::new(0), AccessWidth::Word),
            Ok(u64::from(u32::from_le_bytes(*b"QEMU")))
        );
    }

    /// A DMA descriptor at `desc`: control, length and address, big-endian.
    fn descriptor(ram: &mut [u8], desc: u64, control: u32, length: u32, address: u64) {
        let at = (desc - RAM_BASE) as usize;
        ram[at..at + 4].copy_from_slice(&control.to_be_bytes());
        ram[at + 4..at + 8].copy_from_slice(&length.to_be_bytes());
        ram[at + 8..at + 16].copy_from_slice(&address.to_be_bytes());
    }

    fn control(ram: &[u8], desc: u64) -> u32 {
        let at = (desc - RAM_BASE) as usize;
        u32::from_be_bytes(ram[at..at + 4].try_into().expect("four bytes"))
    }

    /// EDK2's `DmaReadBytes` writes the descriptor's address, big-endian, to
    /// the DMA register in one 64-bit store and polls `control` until the
    /// device clears it: the item's bytes land in RAM.
    #[test]
    fn a_dma_select_and_read_copies_the_item_and_clears_control() {
        let mut backing = [0u8; 0x400];
        let (desc, buf) = (RAM_BASE + 0x100, RAM_BASE + 0x200);
        let select_signature = u32::from(FW_CFG_SIGNATURE) << 16 | DMA_CTL_SELECT;
        descriptor(&mut backing, desc, select_signature | DMA_CTL_READ, 6, buf);
        let mut f = FwCfg::new();
        let mut ram = GuestRam::new(&mut backing);
        f.write(&mut ram, RegOffset::new(16), AccessWidth::Dword, desc.swap_bytes())
            .expect("the DMA address");
        assert_eq!(control(&backing, desc), 0);
        // Four signature bytes, then zeros past the item's end.
        assert_eq!(&backing[0x200..0x206], b"QEMU\0\0");
    }

    /// The two 32-bit halves: the high word arms, the low word starts the
    /// transfer. A write to an item that takes none reports ERROR, as QEMU's
    /// `allow_write` refuses it.
    #[test]
    fn a_dma_write_to_a_read_only_item_reports_error() {
        let mut backing = [0u8; 0x400];
        let desc = RAM_BASE + 0x100;
        descriptor(&mut backing, desc, DMA_CTL_WRITE, 4, RAM_BASE + 0x200);
        let mut f = FwCfg::new();
        let mut ram = GuestRam::new(&mut backing);
        let half = |v: u32| u64::from(v.swap_bytes());
        f.write(&mut ram, RegOffset::new(16), AccessWidth::Word, half((desc >> 32) as u32))
            .expect("the high word");
        f.write(&mut ram, RegOffset::new(20), AccessWidth::Word, half(desc as u32))
            .expect("the low word");
        assert_eq!(control(&backing, desc), DMA_CTL_ERROR);
    }

    /// A descriptor outside RAM is not read and nothing is stored: QEMU's
    /// `dma_memory_read` fails and its ERROR store to the same address
    /// fails too. The address is used up, as QEMU clears `dma_addr`.
    #[test]
    fn a_descriptor_outside_ram_writes_nothing() {
        let mut backing = [0xAAu8; 0x400];
        let mut f = FwCfg::new();
        let mut ram = GuestRam::new(&mut backing);
        let outside = RAM_BASE + 0x1000;
        f.write(&mut ram, RegOffset::new(16), AccessWidth::Dword, outside.swap_bytes())
            .expect("the DMA address");
        assert_eq!(ram.written(), crate::bus::Writes::NONE);
        assert_eq!(f.dma_addr, 0);
        assert!(backing.iter().all(|&b| b == 0xAA));
    }

    /// The DMA register reads back QEMU's "QEMU CFG" signature in memory order.
    #[test]
    fn the_dma_register_reads_the_signature() {
        let mut f = FwCfg::new();
        assert_eq!(
            f.read(RegOffset::new(16), AccessWidth::Dword),
            Ok(u64::from_le_bytes(*b"QEMU CFG"))
        );
        assert_eq!(
            f.read(RegOffset::new(20), AccessWidth::Word),
            Ok(u64::from(u32::from_le_bytes(*b" CFG")))
        );
    }
}
