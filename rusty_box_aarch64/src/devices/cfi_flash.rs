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
//!
//! The model holds the command state machine only. The array itself belongs
//! to the machine's storage and is lent to each access, so the same model
//! serves a bank the machine owns and one a caller lends it.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

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

/// The array of one bank, lent for one write.
pub enum FlashArray<'a> {
    /// Mapped read-only (QEMU `readonly=on`): commands are accepted, but
    /// erases and programs fail and set the error bits.
    ReadOnly(&'a [u8]),
    Writable(&'a mut [u8]),
}

impl FlashArray<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::ReadOnly(b) => b,
            Self::Writable(b) => b,
        }
    }
}

/// One CFI flash bank's command state.
pub struct CfiFlash {
    /// The bank's length in bytes: what its window decodes.
    len: u64,
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

/// The little-endian value of `bytes` bytes at `offset`, reading erased
/// flash (all ones) past the end of a short image.
fn array_read(store: &[u8], offset: u64, bytes: u64) -> u64 {
    let start = offset as usize;
    if let Some(slice) = start.checked_add(bytes as usize).and_then(|end| store.get(start..end)) {
        return crate::bus::le_value(slice);
    }
    let mut v = 0u64;
    for i in 0..bytes {
        let b = store.get((offset + i) as usize).copied().unwrap_or(0xFF);
        v |= u64::from(b) << (8 * i);
    }
    v
}

impl CfiFlash {
    /// A bank of `len` bytes, which must be a whole number of sectors.
    pub fn new(len: u64) -> Self {
        let blocks_per_device = (len / SECTOR_LEN).max(1);
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
            len,
            cfi_table: t,
            cmd: 0,
            wcycle: 0,
            status: STATUS_READY,
            counter: 0,
            blk_offset: None,
            blk_bytes: [0; WRITEBLOCK_SIZE],
        }
    }

    /// QEMU `pflash_reset`: back to read-array mode, any buffered write
    /// abandoned.
    pub fn reset(&mut self) {
        self.cmd = 0;
        self.wcycle = 0;
        self.status = STATUS_READY;
        self.counter = 0;
        self.blk_offset = None;
    }

    fn set_read_array(&mut self) {
        self.wcycle = 0;
        self.cmd = 0;
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
    fn read32(&mut self, store: &[u8], offset: u64, bytes: u64) -> u32 {
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
            0x00 => array_read(store, offset, bytes) as u32,
            _ => {
                self.set_read_array();
                array_read(store, offset, bytes) as u32
            }
        }
    }

    /// A read of the bank whose array is `store`.
    pub fn read(&mut self, store: &[u8], offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        if self.cmd == 0 {
            return array_read(store, off, width.bytes());
        }
        match width {
            AccessWidth::Dword => {
                u64::from(self.read32(store, off, 4)) | (u64::from(self.read32(store, off + 4, 4)) << 32)
            }
            _ => u64::from(self.read32(store, off, width.bytes())) & width.value_mask(),
        }
    }

    /// A write to the bank whose array is `array`.
    pub fn write(&mut self, mut array: FlashArray<'_>, offset: RegOffset, width: AccessWidth, value: u64) {
        let off = offset.get();
        match width {
            AccessWidth::Dword => {
                self.write32(&mut array, off, value as u32, 4);
                self.write32(&mut array, off + 4, (value >> 32) as u32, 4);
            }
            _ => self.write32(&mut array, off, value as u32, width.bytes()),
        }
    }

    fn store(&mut self, array: &mut FlashArray<'_>, offset: u64, value: u32, bytes: u64) {
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
                if let FlashArray::Writable(store) = array {
                    for i in 0..bytes {
                        if let Some(b) = store.get_mut((offset + i) as usize) {
                            *b = (value >> (8 * i)) as u8;
                        }
                    }
                }
            }
        }
    }

    /// QEMU `pflash_write`.
    fn write32(&mut self, array: &mut FlashArray<'_>, offset: u64, value: u32, bytes: u64) {
        let read_only = matches!(array, FlashArray::ReadOnly(_));
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
                    if let FlashArray::Writable(store) = array {
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
                    if read_only {
                        self.status |= STATUS_PROGRAM_ERROR;
                    } else {
                        self.store(array, offset, value, bytes);
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
                        let store = array.bytes();
                        for (i, b) in self.blk_bytes.iter_mut().enumerate() {
                            *b = store.get(start as usize + i).copied().unwrap_or(0xFF);
                        }
                        self.blk_offset = Some(start);
                    }
                    if !read_only && self.blk_offset.is_some() {
                        self.store(array, offset, value, bytes);
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
                    if let (Some(blk), FlashArray::Writable(store)) = (self.blk_offset, array) {
                        let end = (blk as usize).saturating_add(WRITEBLOCK_SIZE).min(store.len());
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

    /// The command state, for a snapshot. The array is the machine's storage
    /// and is saved with it.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_bytes(&[self.cmd, self.wcycle, self.status])?;
        w.write_u32(self.counter)?;
        match self.blk_offset {
            Some(blk) => {
                w.write_bool(true)?;
                w.write_u64(blk)?;
            }
            None => w.write_bool(false)?,
        }
        w.write_bytes(&self.blk_bytes)
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let mut head = [0u8; 3];
        r.read_bytes(&mut head)?;
        [self.cmd, self.wcycle, self.status] = head;
        if self.wcycle > 3 {
            return Err(SnapError::Invalid("CFI flash write cycle is past the last one a command has"));
        }
        self.counter = r.read_u32()?;
        self.blk_offset = if r.read_bool()? { Some(r.read_u64()?) } else { None };
        // A buffered write only ever opens on a write-block boundary inside
        // the bank (`write32`'s second cycle).
        if self
            .blk_offset
            .is_some_and(|blk| blk % WRITEBLOCK_SIZE as u64 != 0 || blk >= self.len)
        {
            return Err(SnapError::Invalid("CFI flash buffered write is not at a write block in the bank"));
        }
        r.read_bytes(&mut self.blk_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot sink in a fixed buffer, so the test needs no allocator.
    struct Image {
        bytes: [u8; 64 + WRITEBLOCK_SIZE],
        len: usize,
    }

    impl SnapWrite for Image {
        fn write_bytes(&mut self, bytes: &[u8]) -> SnapResult {
            let end = self.len + bytes.len();
            self.bytes.get_mut(self.len..end).ok_or(SnapError::Truncated)?.copy_from_slice(bytes);
            self.len = end;
            Ok(())
        }
    }

    /// A snapshot naming a buffered write outside the bank, or off a block
    /// boundary, is refused rather than kept for the confirm cycle to use.
    #[test]
    fn a_restored_buffered_write_must_lie_on_a_block_in_the_bank() {
        let bank = SECTOR_LEN * 4;
        let image = |blk: u64| -> Result<Image, SnapError> {
            let mut w = Image {
                bytes: [0; 64 + WRITEBLOCK_SIZE],
                len: 0,
            };
            w.write_bytes(&[0xE8, 2, STATUS_READY])?;
            w.write_u32(3)?;
            w.write_bool(true)?;
            w.write_u64(blk)?;
            w.write_bytes(&[0xAA; WRITEBLOCK_SIZE])?;
            Ok(w)
        };
        let restore = |blk: u64| {
            let w = image(blk)?;
            CfiFlash::new(bank).restore(&mut &w.bytes[..w.len])
        };
        assert_eq!(restore(WRITEBLOCK_SIZE as u64), Ok(()));
        for wrong in [bank, u64::MAX - 1, WRITEBLOCK_SIZE as u64 + 4] {
            assert!(matches!(restore(wrong), Err(SnapError::Invalid(_))), "{wrong:#x}");
        }
    }

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
        let mut f = CfiFlash::new(backing.len() as u64);
        let at = |o: u64| RegOffset::new(o);
        let write = |f: &mut CfiFlash, backing: &mut [u8], o: u64, v: u64| {
            f.write(FlashArray::Writable(backing), at(o), AccessWidth::Word, v)
        };

        write(&mut f, &mut backing, 0x40000, dual(0x20));
        write(&mut f, &mut backing, 0x40000, dual(0xD0));
        write(&mut f, &mut backing, 0x40000, dual(0x70));
        assert_eq!(f.read(&backing, at(0x40000), AccessWidth::Word) & 0x0080_0080, 0x0080_0080);

        write(&mut f, &mut backing, 0x40000, dual(0xE8));
        write(&mut f, &mut backing, 0x40000, dual(3));
        for (i, w) in [0x1111_2222u64, 0x3333_4444, 0x5555_6666, 0x7777_8888].iter().enumerate() {
            write(&mut f, &mut backing, 0x40000 + 4 * i as u64, *w);
        }
        write(&mut f, &mut backing, 0x40000, dual(0xD0));
        write(&mut f, &mut backing, 0x40000, dual(0xFF));
        assert_eq!(f.read(&backing, at(0x40004), AccessWidth::Word), 0x3333_4444);
        assert_eq!(f.read(&backing, at(0x40010), AccessWidth::Word), 0xFFFF_FFFF, "erased by the sector erase");
    }

    #[test]
    fn device_id_and_cfi_query_replicate_across_the_bank() {
        let mut backing = [0u8; SECTOR_LEN as usize];
        let mut f = CfiFlash::new(backing.len() as u64);
        f.write(FlashArray::Writable(&mut backing), RegOffset::new(0), AccessWidth::Word, dual(0x90));
        assert_eq!(f.read(&backing, RegOffset::new(0), AccessWidth::Word), 0x0089_0089);
        assert_eq!(f.read(&backing, RegOffset::new(4), AccessWidth::Word), 0x0018_0018);
        f.write(FlashArray::Writable(&mut backing), RegOffset::new(0), AccessWidth::Word, dual(0x98));
        assert_eq!(f.read(&backing, RegOffset::new(0x10 * 4), AccessWidth::Word), 0x0051_0051);
    }

    #[test]
    fn a_read_only_bank_reports_erase_errors_and_keeps_its_data() {
        let backing = [0xABu8; SECTOR_LEN as usize];
        let mut f = CfiFlash::new(backing.len() as u64);
        f.write(FlashArray::ReadOnly(&backing), RegOffset::new(0), AccessWidth::Word, dual(0x20));
        assert_ne!(f.read(&backing, RegOffset::new(0), AccessWidth::Word) & 0x0020_0020, 0);
        f.write(FlashArray::ReadOnly(&backing), RegOffset::new(0), AccessWidth::Word, dual(0xFF));
        assert_eq!(f.read(&backing, RegOffset::new(0), AccessWidth::Byte), 0xAB);
    }

    #[test]
    fn a_command_state_snapshot_round_trips_mid_buffered_write() {
        let mut backing = [0xFFu8; SECTOR_LEN as usize];
        let mut f = CfiFlash::new(backing.len() as u64);
        f.write(FlashArray::Writable(&mut backing), RegOffset::new(0), AccessWidth::Word, dual(0xE8));
        f.write(FlashArray::Writable(&mut backing), RegOffset::new(0), AccessWidth::Word, dual(1));
        f.write(FlashArray::Writable(&mut backing), RegOffset::new(0), AccessWidth::Word, 0x1234_5678);
        let mut image = std::vec::Vec::new();
        f.save(&mut image).expect("save");
        let mut restored = CfiFlash::new(backing.len() as u64);
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(source.is_empty());
        // Finishing the write on the restored bank commits what was collected
        // before the snapshot.
        restored.write(FlashArray::Writable(&mut backing), RegOffset::new(4), AccessWidth::Word, 0x9ABC_DEF0);
        restored.write(FlashArray::Writable(&mut backing), RegOffset::new(0), AccessWidth::Word, dual(0xD0));
        restored.write(FlashArray::Writable(&mut backing), RegOffset::new(0), AccessWidth::Word, dual(0xFF));
        assert_eq!(restored.read(&backing, RegOffset::new(0), AccessWidth::Word), 0x1234_5678);
        assert_eq!(restored.read(&backing, RegOffset::new(4), AccessWidth::Word), 0x9ABC_DEF0);
    }
}
