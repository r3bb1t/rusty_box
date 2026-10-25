//! Intel/Sharp command-set CFI parallel flash, as QEMU's `virt` builds it:
//! a 32-bit bank of two 16-bit devices (`width` 4, `device-width` 2),
//! manufacturer 0x89, device 0x18, 256 KiB sectors, little-endian.
//!
//! Provenance (R7): ported from QEMU `hw/block/pflash_cfi01.c` —
//! `pflash_read`, `pflash_write`, `pflash_cfi_query`, `pflash_devid_query`,
//! `pflash_blk_write_start/flush/abort`, and the CFI table built in
//! `pflash_cfi01_fill_cfi_table`. The model's behaviour, including its
//! departures from real parts that EDK2's `VirtNorFlashDxe` is validated
//! against (erase happens when the setup command is written; programming
//! stores the value rather than AND-ing it into the array; a buffered write of
//! a single word reports a programming error), is QEMU's.

use crate::bus::{AccessWidth, RegOffset};

/// Bank width in bytes.
const BANK_WIDTH: u64 = 4;
/// Width of each device in the bank, in bytes.
const DEVICE_WIDTH: u64 = 2;
const NUM_DEVICES: u64 = BANK_WIDTH / DEVICE_WIDTH;
/// QEMU `VIRT_FLASH_SECTOR_SIZE`.
pub const SECTOR_LEN: u64 = 256 * 1024;
const IDENT0: u32 = 0x89;
const IDENT1: u32 = 0x18;
/// `1 << cfi_table[0x2A]` bytes per device, times the devices in the bank.
const WRITEBLOCK_SIZE: usize = (1 << 0x0B) * NUM_DEVICES as usize;
const CFI_TABLE_LEN: usize = 0x52;

const STATUS_READY: u8 = 0x80;
const STATUS_ERASE_ERROR: u8 = 0x20;
const STATUS_PROGRAM_ERROR: u8 = 0x10;

/// The backing store of one bank.
pub enum FlashStorage<'m> {
    /// Mapped read-only (QEMU `readonly=on`): commands are accepted, but
    /// erases and programs fail and set the error bits.
    ReadOnly(&'m [u8]),
    Writable(&'m mut [u8]),
}

impl FlashStorage<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::ReadOnly(b) => b,
            Self::Writable(b) => b,
        }
    }

    fn len(&self) -> u64 {
        self.bytes().len() as u64
    }
}

/// One CFI flash bank.
pub struct CfiFlash<'m> {
    storage: FlashStorage<'m>,
    cfi_table: [u8; CFI_TABLE_LEN],
    /// The command in progress; 0x00 is QEMU's encoding of read-array mode.
    cmd: u8,
    wcycle: u8,
    status: u8,
    counter: u32,
    /// Start of the block a buffered write is collecting, if one is open.
    blk_offset: Option<u64>,
    blk_bytes: [u8; WRITEBLOCK_SIZE],
}

impl<'m> CfiFlash<'m> {
    /// A bank over `storage`, whose length must be a whole number of sectors.
    pub fn new(storage: FlashStorage<'m>) -> Self {
        let blocks_per_device = (storage.len() / SECTOR_LEN).max(1);
        let sector_len_per_device = SECTOR_LEN / NUM_DEVICES;
        let device_len = sector_len_per_device * blocks_per_device;
        let mut t = [0u8; CFI_TABLE_LEN];
        t[0x10] = b'Q';
        t[0x11] = b'R';
        t[0x12] = b'Y';
        t[0x13] = 0x01;
        t[0x15] = 0x31;
        t[0x1B] = 0x45;
        t[0x1C] = 0x55;
        t[0x1F] = 0x07;
        t[0x20] = 0x07;
        t[0x21] = 0x0A;
        t[0x23] = 0x04;
        t[0x24] = 0x04;
        t[0x25] = 0x04;
        t[0x27] = device_len.trailing_zeros() as u8;
        t[0x28] = 0x02;
        t[0x2A] = 0x0B;
        t[0x2C] = 0x01;
        t[0x2D] = (blocks_per_device - 1) as u8;
        t[0x2E] = ((blocks_per_device - 1) >> 8) as u8;
        t[0x2F] = (sector_len_per_device >> 8) as u8;
        t[0x30] = (sector_len_per_device >> 16) as u8;
        t[0x31] = b'P';
        t[0x32] = b'R';
        t[0x33] = b'I';
        t[0x34] = b'1';
        t[0x35] = b'0';
        t[0x3F] = 0x01;
        Self {
            storage,
            cfi_table: t,
            cmd: 0,
            wcycle: 0,
            status: STATUS_READY,
            counter: 0,
            blk_offset: None,
            blk_bytes: [0; WRITEBLOCK_SIZE],
        }
    }

    /// Size of the bank in bytes.
    pub fn len(&self) -> u64 {
        self.storage.len()
    }

    pub fn is_empty(&self) -> bool {
        self.storage.len() == 0
    }

    /// The array's current contents (for a front end that persists the
    /// variable store).
    pub fn contents(&self) -> &[u8] {
        self.storage.bytes()
    }

    fn set_read_array(&mut self) {
        self.wcycle = 0;
        self.cmd = 0;
    }

    fn data_read(&self, offset: u64, bytes: u64) -> u64 {
        let store = self.storage.bytes();
        let start = offset as usize;
        if let Some(slice) = store.get(start..start + bytes as usize) {
            return crate::bus::le_value(slice);
        }
        // Past the end of a short image: erased flash.
        let mut v = 0u64;
        for i in 0..bytes {
            let b = store.get((offset + i) as usize).copied().unwrap_or(0xFF);
            v |= u64::from(b) << (8 * i);
        }
        v
    }

    /// Replicate one device's response across the bank.
    fn replicate(resp: u32) -> u32 {
        resp | (resp << 16)
    }

    fn cfi_query(&self, offset: u64) -> u32 {
        let boff = (offset >> BANK_WIDTH.trailing_zeros()) as usize;
        match self.cfi_table.get(boff) {
            Some(b) => Self::replicate(u32::from(*b)),
            None => 0,
        }
    }

    fn devid_query(&self, offset: u64) -> u32 {
        let boff = offset >> BANK_WIDTH.trailing_zeros();
        match boff & 0xFF {
            0 => Self::replicate(IDENT0),
            1 => Self::replicate(IDENT1),
            _ => 0,
        }
    }

    /// QEMU `pflash_read`, for accesses up to the bank width.
    fn read32(&mut self, offset: u64, bytes: u64) -> u32 {
        match self.cmd {
            0x10 | 0x20 | 0x28 | 0x40 | 0x50 | 0x60 | 0x70 | 0xE8 => {
                let s = u32::from(self.status);
                if bytes > DEVICE_WIDTH {
                    s | (s << 16)
                } else {
                    s
                }
            }
            0x90 => self.devid_query(offset),
            0x98 => self.cfi_query(offset),
            0x00 => self.data_read(offset, bytes) as u32,
            _ => {
                self.set_read_array();
                self.data_read(offset, bytes) as u32
            }
        }
    }

    pub fn read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        if self.cmd == 0 {
            return self.data_read(off, width.bytes());
        }
        match width {
            AccessWidth::Dword => {
                u64::from(self.read32(off, 4)) | (u64::from(self.read32(off + 4, 4)) << 32)
            }
            _ => u64::from(self.read32(off, width.bytes())) & width.value_mask(),
        }
    }

    pub fn write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        let off = offset.get();
        match width {
            AccessWidth::Dword => {
                self.write32(off, value as u32, 4);
                self.write32(off + 4, (value >> 32) as u32, 4);
            }
            _ => self.write32(off, value as u32, width.bytes()),
        }
    }

    fn store(&mut self, offset: u64, value: u32, bytes: u64) {
        match self.blk_offset {
            Some(blk) => {
                if offset < blk || offset + bytes > blk + WRITEBLOCK_SIZE as u64 {
                    self.status |= STATUS_PROGRAM_ERROR;
                    return;
                }
                for i in 0..bytes {
                    if let Some(b) = self.blk_bytes.get_mut((offset - blk + i) as usize) {
                        *b = (value >> (8 * i)) as u8;
                    }
                }
            }
            None => {
                if let FlashStorage::Writable(store) = &mut self.storage {
                    for i in 0..bytes {
                        if let Some(b) = store.get_mut((offset + i) as usize) {
                            *b = (value >> (8 * i)) as u8;
                        }
                    }
                }
            }
        }
    }

    fn read_only(&self) -> bool {
        matches!(self.storage, FlashStorage::ReadOnly(_))
    }

    /// QEMU `pflash_write`.
    fn write32(&mut self, offset: u64, value: u32, bytes: u64) {
        let cmd = value as u8;
        match self.wcycle {
            0 => match cmd {
                0x00 | 0xF0 | 0xFF => self.set_read_array(),
                0x10 | 0x40 | 0x60 | 0x98 => {
                    self.wcycle += 1;
                    self.cmd = cmd;
                }
                0x20 => {
                    let start = offset & !(SECTOR_LEN - 1);
                    if let FlashStorage::Writable(store) = &mut self.storage {
                        let end = (start + SECTOR_LEN).min(store.len() as u64);
                        if let Some(region) = store.get_mut(start as usize..end as usize) {
                            region.fill(0xFF);
                        }
                    } else {
                        self.status |= STATUS_ERASE_ERROR;
                    }
                    self.status |= STATUS_READY;
                    self.wcycle += 1;
                    self.cmd = cmd;
                }
                0x50 => {
                    self.status = 0;
                    self.set_read_array();
                }
                0x70 | 0x90 => self.cmd = cmd,
                0xE8 => {
                    self.status |= STATUS_READY;
                    self.wcycle += 1;
                    self.cmd = cmd;
                }
                _ => self.set_read_array(),
            },
            1 => match self.cmd {
                0x10 | 0x40 => {
                    if self.read_only() {
                        self.status |= STATUS_PROGRAM_ERROR;
                    } else {
                        self.store(offset, value, bytes);
                    }
                    self.status |= STATUS_READY;
                    self.wcycle = 0;
                }
                0x20 | 0x28 => match cmd {
                    0xD0 => {
                        self.wcycle = 0;
                        self.status |= STATUS_READY;
                    }
                    _ => self.set_read_array(),
                },
                0xE8 => {
                    self.counter = value & 0xFFFF;
                    self.wcycle += 1;
                }
                0x60 => match cmd {
                    0xD0 | 0x01 => {
                        self.wcycle = 0;
                        self.status |= STATUS_READY;
                    }
                    _ => self.set_read_array(),
                },
                // In query mode only Read Array leaves; anything else stays.
                0x98 if cmd == 0xFF => self.set_read_array(),
                0x98 => {}
                _ => self.set_read_array(),
            },
            2 => match self.cmd {
                0xE8 => {
                    if self.blk_offset.is_none() && self.counter != 0 {
                        let start = offset & !(WRITEBLOCK_SIZE as u64 - 1);
                        let store = self.storage.bytes();
                        for (i, b) in self.blk_bytes.iter_mut().enumerate() {
                            *b = store.get(start as usize + i).copied().unwrap_or(0xFF);
                        }
                        self.blk_offset = Some(start);
                    }
                    if !self.read_only() && self.blk_offset.is_some() {
                        self.store(offset, value, bytes);
                    } else {
                        self.status |= STATUS_PROGRAM_ERROR;
                    }
                    self.status |= STATUS_READY;
                    if self.counter == 0 {
                        self.wcycle += 1;
                    } else {
                        self.counter -= 1;
                    }
                }
                _ => self.set_read_array(),
            },
            3 => match self.cmd {
                // Confirm: commit the collected block, unless a programming
                // error was recorded while collecting it.
                0xE8 if cmd == 0xD0 && self.status & STATUS_PROGRAM_ERROR == 0 => {
                    if let (Some(blk), FlashStorage::Writable(store)) = (self.blk_offset, &mut self.storage) {
                        let end = (blk as usize + WRITEBLOCK_SIZE).min(store.len());
                        let n = end.saturating_sub(blk as usize);
                        if let (Some(dst), Some(src)) = (store.get_mut(blk as usize..end), self.blk_bytes.get(..n)) {
                            dst.copy_from_slice(src);
                        }
                    }
                    self.blk_offset = None;
                    self.wcycle = 0;
                    self.status |= STATUS_READY;
                }
                _ => {
                    self.blk_offset = None;
                    self.set_read_array();
                }
            },
            _ => self.set_read_array(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dual(cmd: u32) -> u64 {
        u64::from(cmd | (cmd << 16))
    }

    /// The sequence EDK2's VirtNorFlashDxe uses: erase a sector, poll status
    /// until both devices are ready, buffered-program four words, confirm,
    /// return to read-array mode and read the data back.
    #[test]
    fn edk2s_erase_and_buffered_program_sequence_round_trips() {
        let mut backing = [0u8; (SECTOR_LEN * 2) as usize];
        backing[0x40000..0x40010].fill(0x5A);
        let mut f = CfiFlash::new(FlashStorage::Writable(&mut backing));
        let at = |o: u64| RegOffset::new(o);

        f.write(at(0x40000), AccessWidth::Word, dual(0x20));
        f.write(at(0x40000), AccessWidth::Word, dual(0xD0));
        f.write(at(0x40000), AccessWidth::Word, dual(0x70));
        assert_eq!(f.read(at(0x40000), AccessWidth::Word) & 0x0080_0080, 0x0080_0080);

        f.write(at(0x40000), AccessWidth::Word, dual(0xE8));
        f.write(at(0x40000), AccessWidth::Word, dual(3));
        for (i, w) in [0x1111_2222u64, 0x3333_4444, 0x5555_6666, 0x7777_8888].iter().enumerate() {
            f.write(at(0x40000 + 4 * i as u64), AccessWidth::Word, *w);
        }
        f.write(at(0x40000), AccessWidth::Word, dual(0xD0));
        f.write(at(0x40000), AccessWidth::Word, dual(0xFF));
        assert_eq!(f.read(at(0x40004), AccessWidth::Word), 0x3333_4444);
        assert_eq!(f.read(at(0x40010), AccessWidth::Word), 0xFFFF_FFFF, "erased by the sector erase");
    }

    #[test]
    fn device_id_and_cfi_query_replicate_across_the_bank() {
        let mut backing = [0u8; SECTOR_LEN as usize];
        let mut f = CfiFlash::new(FlashStorage::Writable(&mut backing));
        f.write(RegOffset::new(0), AccessWidth::Word, dual(0x90));
        assert_eq!(f.read(RegOffset::new(0), AccessWidth::Word), 0x0089_0089);
        assert_eq!(f.read(RegOffset::new(4), AccessWidth::Word), 0x0018_0018);
        f.write(RegOffset::new(0), AccessWidth::Word, dual(0x98));
        assert_eq!(f.read(RegOffset::new(0x10 * 4), AccessWidth::Word), 0x0051_0051);
    }

    #[test]
    fn a_read_only_bank_reports_erase_errors_and_keeps_its_data() {
        let backing = [0xABu8; SECTOR_LEN as usize];
        let mut f = CfiFlash::new(FlashStorage::ReadOnly(&backing));
        f.write(RegOffset::new(0), AccessWidth::Word, dual(0x20));
        assert_ne!(f.read(RegOffset::new(0), AccessWidth::Word) & 0x0020_0020, 0);
        f.write(RegOffset::new(0), AccessWidth::Word, dual(0xFF));
        assert_eq!(f.read(RegOffset::new(0), AccessWidth::Byte), 0xAB);
    }
}
