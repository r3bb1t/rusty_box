//! Intel/Sharp command-set (0001, JEDEC JEP137) CFI parallel flash: a
//! 32-bit bank of two 16-bit devices side by side (`width` 4,
//! `device-width` 2), 256 KiB sectors, little-endian.
//!
//! Provenance (R7): the command state machine is ported from QEMU
//! `hw/block/pflash_cfi01.c` — `pflash_read`, `pflash_write`,
//! `pflash_cfi_query`, `pflash_devid_query` and
//! `pflash_blk_write_start/flush/abort`. Its behaviour, including its
//! departures from real parts that EDK2's `VirtNorFlashDxe` is validated
//! against (erase happens when the setup command is written; programming
//! stores the value rather than AND-ing it into the array; a buffered write of
//! a single word reports a programming error), is QEMU's.
//!
//! What a device says of itself is the board's. Its Read Identifier space
//! gives the identity's manufacturer and device codes ([`FlashIdentity`],
//! by default no manufacturer and no part) at offsets 0 and 1, and every
//! block unlocked at offset 2, the block status EDK2's driver reads before
//! it erases or programs. Its CFI query table (JEDEC JESD68.01) holds what
//! the CFI specification and the command set fix, the geometry of the
//! board's banks, and timings and supply voltages describing the device as
//! it is: every operation done within the access that starts it, and no
//! supply to draw ([`CfiFlash::new`] gives each field).
//!
//! The model holds the command state machine only. The array itself belongs
//! to the machine's storage and is lent to each access, so the same model
//! serves a bank the machine owns and one a caller lends it.

use rusty_box_core::SnapError;
use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use crate::bus::{AccessWidth, RegOffset};
use crate::identity::FlashIdentity;

/// Bank width in bytes.
const BANK_WIDTH: u64 = 4;
/// Width of each device in the bank, in bytes.
const DEVICE_WIDTH: u64 = 2;
const NUM_DEVICES: u64 = BANK_WIDTH / DEVICE_WIDTH;
/// The bank's erase block, which stock EDK2's variable store is laid out
/// in (`FdtNorFlashQemuLib.c`, `VarStore.fdf.inc`).
pub const SECTOR_LEN: u64 = 256 * 1024;
/// The bank's write buffer: 4 KiB, the smallest page an Arm translation
/// regime maps (the 4 KB granule), so one buffered program can write a
/// whole such page and, aligned to its own size, never two.
const WRITEBLOCK_SIZE: usize = 4096;
/// The erase block regions the query table describes: one, the banks'
/// sectors being uniform.
const ERASE_REGIONS: usize = 1;
/// Where the Intel/Sharp extended query table starts: right after the
/// query structure's last erase block region, which starts at 0x2D and
/// takes four bytes.
const PRI_TABLE: usize = 0x2D + 4 * ERASE_REGIONS;
/// The query table's length: the CFI query structure and the extended
/// query table of version 1.0, which ends with its Vpp optimum at offset
/// 0xD of it. Every offset beyond reads 0.
const CFI_TABLE_LEN: usize = PRI_TABLE + 0xE;
/// The lowest and highest supply voltages the CFI query structure's Vcc
/// fields can state (a BCD volt digit in bits 7:4 and a BCD 100 mV digit in
/// bits 3:0): 0.0 V and 9.9 V. The device has no supply, so it programs
/// and erases at any.
const VCC_ANY_MIN: u8 = 0x00;
const VCC_ANY_MAX: u8 = 0x99;
/// What a Vpp field reads for a device with no Vpp pin.
const NO_VPP_PIN: u8 = 0x00;
/// The device interface code of a 16-bit asynchronous device, a device
/// [`DEVICE_WIDTH`] bytes wide (JEP137; 0x0000 is 8-bit, 0x0002 8- or
/// 16-bit).
const INTERFACE_X16: u8 = 0x01;
const _: () = assert!(DEVICE_WIDTH == 2);

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
    /// What each device's Read Identifier space gives at offsets 0 and 1.
    identity: FlashIdentity,
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
    if let Some(slice) = start
        .checked_add(bytes as usize)
        .and_then(|end| store.get(start..end))
    {
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
    /// A bank of `len` bytes, which must be a whole number of sectors, of
    /// devices identified as `identity`.
    ///
    /// Each device's CFI query table, by offset in the device's query
    /// space (two bytes a field are little-endian):
    ///
    /// - 0x10–0x12 "QRY", and 0x13–0x14 0x0001, the primary command set,
    ///   Intel/Sharp's, which the device implements (JESD68.01, JEP137).
    /// - 0x15–0x16 the extended query table's address, 0x0031, the first
    ///   offset after the one erase block region.
    /// - 0x17–0x1A 0: no alternate command set, so no table for one.
    /// - 0x1B–0x1C Vcc 0.0 V to 9.9 V, every voltage the fields can state;
    ///   0x1D–0x1E 0, no Vpp pin.
    /// - 0x1F–0x22 the typical times, 2^n µs to program a word and to
    ///   program a full buffer, 2^n ms to erase a block and the chip: the
    ///   shortest each can state, 1 µs, 2 µs (0 would say the device has no
    ///   buffered program) and 1 ms, as the device finishes every operation
    ///   within the access that starts it; chip erase 0, as the command set
    ///   has none. 0x23–0x26, the longest times, 2^n times the typical
    ///   ones: 0, once each, the typical time also being the longest, and 0
    ///   for chip erase, which the device does not have.
    /// - 0x27 the device's size, 2^n bytes: half the bank's.
    /// - 0x28–0x29 0x0001, a 16-bit asynchronous interface, the device's
    ///   width.
    /// - 0x2A–0x2B the write buffer, 2^n bytes a device: half of the
    ///   bank's 4 KiB, 2^11.
    /// - 0x2C one erase block region, and 0x2D–0x30 its blocks less one and
    ///   their size in 256 bytes: the bank's sectors, each half in each
    ///   device.
    /// - 0x31–0x35, the extended query table: "PRI" and version 1.0, the
    ///   table's first, whose fields hold every feature the device has.
    ///   0x36–0x39 0, none of the optional features (chip erase, suspend,
    ///   block locking, protection registers, page and synchronous reads,
    ///   partitions); 0x3A 0, no suspend; 0x3B–0x3C 0, no block status
    ///   bits, so Read Identifier offset 2 reports no block locked;
    ///   0x3D Vcc 0.0 V, the least voltage of the range, as the device
    ///   works alike at each; 0x3E 0, no Vpp pin.
    pub fn new(len: u64, identity: FlashIdentity) -> Self {
        let blocks_per_device = (len / SECTOR_LEN).max(1);
        let sector_len_per_device = SECTOR_LEN / NUM_DEVICES;
        let device_len = sector_len_per_device * blocks_per_device;
        let buffer_per_device = WRITEBLOCK_SIZE as u64 / NUM_DEVICES;
        let mut t = [0u8; CFI_TABLE_LEN];
        t[0x10] = b'Q';
        t[0x11] = b'R';
        t[0x12] = b'Y';
        t[0x13] = 0x01;
        t[0x15] = PRI_TABLE as u8;
        t[0x16] = (PRI_TABLE >> 8) as u8;
        t[0x1B] = VCC_ANY_MIN;
        t[0x1C] = VCC_ANY_MAX;
        t[0x1D] = NO_VPP_PIN;
        t[0x1E] = NO_VPP_PIN;
        // Word program 2^0 µs, buffer program 2^1 µs, block erase 2^0 ms;
        // no chip erase; each longest time 2^0 times the typical one.
        t[0x1F] = 0;
        t[0x20] = 1;
        t[0x21] = 0;
        t[0x22] = 0;
        t[0x23] = 0;
        t[0x24] = 0;
        t[0x25] = 0;
        t[0x26] = 0;
        t[0x27] = device_len.trailing_zeros() as u8;
        t[0x28] = INTERFACE_X16;
        t[0x2A] = buffer_per_device.trailing_zeros() as u8;
        t[0x2C] = ERASE_REGIONS as u8;
        t[0x2D] = (blocks_per_device - 1) as u8;
        t[0x2E] = ((blocks_per_device - 1) >> 8) as u8;
        t[0x2F] = (sector_len_per_device >> 8) as u8;
        t[0x30] = (sector_len_per_device >> 16) as u8;
        let p = PRI_TABLE;
        t[p] = b'P';
        t[p + 1] = b'R';
        t[p + 2] = b'I';
        t[p + 3] = b'1';
        t[p + 4] = b'0';
        // Feature support (4 bytes), suspend functions and the block status
        // register mask (2 bytes): none.
        t[p + 0xC] = VCC_ANY_MIN;
        t[p + 0xD] = NO_VPP_PIN;
        Self {
            len,
            identity,
            cfi_table: t,
            cmd: 0,
            wcycle: 0,
            status: STATUS_READY,
            counter: 0,
            blk_offset: None,
            blk_bytes: [0; WRITEBLOCK_SIZE],
        }
    }

    /// Whether a read returns the array, without changing anything (QEMU
    /// `pflash_read` in read-array mode, the mode `pflash_mem_read_with_attrs`
    /// serves straight from the ROM device).
    pub fn reads_array(&self) -> bool {
        self.cmd == 0
    }

    /// QEMU `pflash_cfi01_system_reset`: back to read-array mode, any
    /// buffered write abandoned.
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

    /// The Read Identifier space: the manufacturer code at offset 0, the
    /// device code at 1, and 0 elsewhere, the block status at 2 among it,
    /// no block being locked.
    fn devid_query(&self, offset: u64) -> u32 {
        let boff = offset >> BANK_WIDTH.trailing_zeros();
        match boff & 0xFF {
            0 => Self::replicate(u32::from(self.identity.manufacturer.code())),
            1 => Self::replicate(u32::from(self.identity.device.get())),
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
                u64::from(self.read32(store, off, 4))
                    | (u64::from(self.read32(store, off + 4, 4)) << 32)
            }
            _ => u64::from(self.read32(store, off, width.bytes())) & width.value_mask(),
        }
    }

    /// A write to the bank whose array is `array`.
    pub fn write(
        &mut self,
        mut array: FlashArray<'_>,
        offset: RegOffset,
        width: AccessWidth,
        value: u64,
    ) {
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
                        let end = (blk as usize)
                            .saturating_add(WRITEBLOCK_SIZE)
                            .min(store.len());
                        let n = end.saturating_sub(blk as usize);
                        if let (Some(dst), Some(src)) =
                            (store.get_mut(blk as usize..end), self.blk_bytes.get(..n))
                        {
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
            return Err(SnapError::Invalid(
                "CFI flash write cycle is past the last one a command has",
            ));
        }
        self.counter = r.read_u32()?;
        self.blk_offset = if r.read_bool()? {
            Some(r.read_u64()?)
        } else {
            None
        };
        // A buffered write only ever opens on a write-block boundary inside
        // the bank (`write32`'s second cycle).
        if self
            .blk_offset
            .is_some_and(|blk| blk % WRITEBLOCK_SIZE as u64 != 0 || blk >= self.len)
        {
            return Err(SnapError::Invalid(
                "CFI flash buffered write is not at a write block in the bank",
            ));
        }
        r.read_bytes(&mut self.blk_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{FlashDeviceCode, FlashManufacturer};

    /// A snapshot sink in a fixed buffer, so the test needs no allocator.
    struct Image {
        bytes: [u8; 64 + WRITEBLOCK_SIZE],
        len: usize,
    }

    impl SnapWrite for Image {
        fn write_bytes(&mut self, bytes: &[u8]) -> SnapResult {
            let end = self.len + bytes.len();
            self.bytes
                .get_mut(self.len..end)
                .ok_or(SnapError::Truncated)?
                .copy_from_slice(bytes);
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
            CfiFlash::new(bank, FlashIdentity::NONE).restore(&mut &w.bytes[..w.len])
        };
        assert_eq!(restore(WRITEBLOCK_SIZE as u64), Ok(()));
        for wrong in [bank, u64::MAX - 1, WRITEBLOCK_SIZE as u64 + 4] {
            assert!(
                matches!(restore(wrong), Err(SnapError::Invalid(_))),
                "{wrong:#x}"
            );
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
        let mut f = CfiFlash::new(backing.len() as u64, FlashIdentity::NONE);
        let at = |o: u64| RegOffset::new(o);
        let write = |f: &mut CfiFlash, backing: &mut [u8], o: u64, v: u64| {
            f.write(FlashArray::Writable(backing), at(o), AccessWidth::Word, v)
        };

        write(&mut f, &mut backing, 0x40000, dual(0x20));
        write(&mut f, &mut backing, 0x40000, dual(0xD0));
        write(&mut f, &mut backing, 0x40000, dual(0x70));
        assert_eq!(
            f.read(&backing, at(0x40000), AccessWidth::Word) & 0x0080_0080,
            0x0080_0080
        );

        write(&mut f, &mut backing, 0x40000, dual(0xE8));
        write(&mut f, &mut backing, 0x40000, dual(3));
        for (i, w) in [0x1111_2222u64, 0x3333_4444, 0x5555_6666, 0x7777_8888]
            .iter()
            .enumerate()
        {
            write(&mut f, &mut backing, 0x40000 + 4 * i as u64, *w);
        }
        write(&mut f, &mut backing, 0x40000, dual(0xD0));
        write(&mut f, &mut backing, 0x40000, dual(0xFF));
        assert_eq!(
            f.read(&backing, at(0x40004), AccessWidth::Word),
            0x3333_4444
        );
        assert_eq!(
            f.read(&backing, at(0x40010), AccessWidth::Word),
            0xFFFF_FFFF,
            "erased by the sector erase"
        );
    }

    /// The board's bank: 64 MiB.
    const BANK: u64 = 0x0400_0000;

    /// What the guest reads of query space offset `offset` of each device,
    /// with both devices' halves of the bank word: the word at `offset`
    /// bank words into the bank.
    fn bank_query(f: &mut CfiFlash, offset: u64) -> u64 {
        f.read(&[], RegOffset::new(offset * BANK_WIDTH), AccessWidth::Word)
    }

    /// Read Identifier, as a guest gives it: the command to both devices,
    /// at an address of the block it asks about.
    fn read_identifier(f: &mut CfiFlash) {
        f.write(
            FlashArray::ReadOnly(&[]),
            RegOffset::new(2 * BANK_WIDTH),
            AccessWidth::Word,
            dual(0x90),
        );
    }

    /// Each device of the bank gives the identity's manufacturer and device
    /// codes at Read Identifier offsets 0 and 1 and no block locked at 2:
    /// by default no manufacturer (0x00, which no JEP106 code reads as) and
    /// no part; a host's codes where it gives them.
    #[test]
    fn read_identifier_gives_the_identitys_codes_in_each_device() {
        let maker = match FlashManufacturer::jep106(0x14) {
            Ok(maker) => maker,
            Err(e) => panic!("{e}"),
        };
        let host = FlashIdentity {
            manufacturer: maker,
            device: FlashDeviceCode::new(0xBEEF),
        };
        for (identity, [manufacturer, device]) in [
            (FlashIdentity::NONE, [0, 0]),
            // Number 0x14, two ones, so the parity bit: 0x94.
            (host, [0x0094_0094, 0xBEEF_BEEF]),
        ] {
            let mut f = CfiFlash::new(BANK, identity);
            read_identifier(&mut f);
            assert_eq!(bank_query(&mut f, 0), manufacturer);
            assert_eq!(bank_query(&mut f, 1), device);
            assert_eq!(bank_query(&mut f, 2), 0, "no block locked");
            // A block's Read Identifier space repeats in each block.
            assert_eq!(bank_query(&mut f, SECTOR_LEN / BANK_WIDTH), manufacturer);
            assert_eq!(bank_query(&mut f, SECTOR_LEN / BANK_WIDTH + 2), 0);
        }
    }

    /// The CFI query table each device of a 64 MiB bank gives, as a guest
    /// reads it after Read Query at offset 0x55 (JESD68.01's address for
    /// the command): every byte to past the extended query table, each
    /// device's in its half of the bank word.
    #[test]
    fn the_cfi_query_table_describes_the_boards_device() {
        let mut expected = [0u8; 0x48];
        for (offset, bytes) in [
            (0x10, &b"QRY"[..]),
            // Command set 0001 and its extended table at 0x31; no
            // alternate.
            (0x13, &[0x01, 0x00, 0x31, 0x00, 0, 0, 0, 0][..]),
            // Vcc 0.0 V to 9.9 V; no Vpp pin.
            (0x1B, &[0x00, 0x99, 0x00, 0x00][..]),
            // Program a word in 1 µs and a buffer in 2 µs, erase a block in
            // 1 ms, no chip erase; each longest time the typical one.
            (0x1F, &[0, 1, 0, 0, 0, 0, 0, 0][..]),
            // 32 MiB a device, 16-bit, a 2 KiB buffer, one region of 256
            // blocks of 128 KiB.
            (
                0x27,
                &[25, 0x01, 0x00, 11, 0x00, 1, 0xFF, 0x00, 0x00, 0x02][..],
            ),
            // "PRI" version 1.0, no optional feature, no suspend, no block
            // status bits, Vcc optimum 0.0 V, no Vpp pin.
            (0x31, &b"PRI10"[..]),
            (0x36, &[0, 0, 0, 0, 0, 0, 0, 0, 0][..]),
        ] {
            expected[offset..offset + bytes.len()].copy_from_slice(bytes);
        }
        let mut f = CfiFlash::new(BANK, FlashIdentity::NONE);
        f.write(
            FlashArray::ReadOnly(&[]),
            RegOffset::new(0x55 * BANK_WIDTH),
            AccessWidth::Word,
            dual(0x98),
        );
        for (offset, byte) in expected.iter().enumerate() {
            assert_eq!(
                bank_query(&mut f, offset as u64),
                dual(u32::from(*byte)),
                "query offset {offset:#x}"
            );
        }
        // Read Array leaves query mode.
        f.write(
            FlashArray::ReadOnly(&[]),
            RegOffset::new(0),
            AccessWidth::Word,
            dual(0xFF),
        );
        assert!(f.reads_array());
    }

    /// Each device's write buffer is the size its query table gives, half
    /// of the bank's: a buffered write of a whole 4 KiB page is kept, and
    /// one word past it fails with a programming error.
    #[test]
    fn the_write_buffer_is_the_size_the_query_table_gives() {
        let mut f = CfiFlash::new(BANK, FlashIdentity::NONE);
        f.write(
            FlashArray::ReadOnly(&[]),
            RegOffset::new(0x55 * BANK_WIDTH),
            AccessWidth::Word,
            dual(0x98),
        );
        let per_device = 1u64 << (bank_query(&mut f, 0x2A) & 0xFF);
        assert_eq!(per_device * NUM_DEVICES, 4096);
        let mut backing = std::vec![0xFFu8; 2 * 4096];
        let write = |f: &mut CfiFlash, backing: &mut [u8], o: u64, v: u64| {
            f.write(
                FlashArray::Writable(backing),
                RegOffset::new(o),
                AccessWidth::Word,
                v,
            )
        };
        let program_error = |f: &mut CfiFlash, backing: &mut [u8]| {
            write(f, backing, 0, dual(0x70));
            f.read(backing, RegOffset::new(0), AccessWidth::Word)
                & dual(u32::from(STATUS_PROGRAM_ERROR))
                != 0
        };
        write(&mut f, &mut backing, 0, dual(0xFF));
        let words = 4096 / BANK_WIDTH;
        // Buffered program: the count is the bank words less one.
        write(&mut f, &mut backing, 0, dual(0xE8));
        write(&mut f, &mut backing, 0, dual(words as u32 - 1));
        for i in 0..words {
            write(&mut f, &mut backing, i * BANK_WIDTH, i);
        }
        write(&mut f, &mut backing, 0, dual(0xD0));
        assert!(!program_error(&mut f, &mut backing));
        write(&mut f, &mut backing, 0, dual(0xFF));
        assert_eq!(
            f.read(
                &backing,
                RegOffset::new(4096 - BANK_WIDTH),
                AccessWidth::Word
            ),
            words - 1
        );
        write(&mut f, &mut backing, 0, dual(0xE8));
        write(&mut f, &mut backing, 0, dual(words as u32));
        for i in 0..=words {
            write(&mut f, &mut backing, i * BANK_WIDTH, i);
        }
        write(&mut f, &mut backing, 0, dual(0xD0));
        assert!(
            program_error(&mut f, &mut backing),
            "a word past the buffer"
        );
    }

    #[test]
    fn a_read_only_bank_reports_erase_errors_and_keeps_its_data() {
        let backing = [0xABu8; SECTOR_LEN as usize];
        let mut f = CfiFlash::new(backing.len() as u64, FlashIdentity::NONE);
        f.write(
            FlashArray::ReadOnly(&backing),
            RegOffset::new(0),
            AccessWidth::Word,
            dual(0x20),
        );
        assert_ne!(
            f.read(&backing, RegOffset::new(0), AccessWidth::Word) & 0x0020_0020,
            0
        );
        f.write(
            FlashArray::ReadOnly(&backing),
            RegOffset::new(0),
            AccessWidth::Word,
            dual(0xFF),
        );
        assert_eq!(f.read(&backing, RegOffset::new(0), AccessWidth::Byte), 0xAB);
    }

    #[test]
    fn a_command_state_snapshot_round_trips_mid_buffered_write() {
        let mut backing = [0xFFu8; SECTOR_LEN as usize];
        let mut f = CfiFlash::new(backing.len() as u64, FlashIdentity::NONE);
        f.write(
            FlashArray::Writable(&mut backing),
            RegOffset::new(0),
            AccessWidth::Word,
            dual(0xE8),
        );
        f.write(
            FlashArray::Writable(&mut backing),
            RegOffset::new(0),
            AccessWidth::Word,
            dual(1),
        );
        f.write(
            FlashArray::Writable(&mut backing),
            RegOffset::new(0),
            AccessWidth::Word,
            0x1234_5678,
        );
        let mut image = std::vec::Vec::new();
        f.save(&mut image).expect("save");
        let mut restored = CfiFlash::new(backing.len() as u64, FlashIdentity::NONE);
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(source.is_empty());
        // Finishing the write on the restored bank commits what was collected
        // before the snapshot.
        restored.write(
            FlashArray::Writable(&mut backing),
            RegOffset::new(4),
            AccessWidth::Word,
            0x9ABC_DEF0,
        );
        restored.write(
            FlashArray::Writable(&mut backing),
            RegOffset::new(0),
            AccessWidth::Word,
            dual(0xD0),
        );
        restored.write(
            FlashArray::Writable(&mut backing),
            RegOffset::new(0),
            AccessWidth::Word,
            dual(0xFF),
        );
        assert_eq!(
            restored.read(&backing, RegOffset::new(0), AccessWidth::Word),
            0x1234_5678
        );
        assert_eq!(
            restored.read(&backing, RegOffset::new(4), AccessWidth::Word),
            0x9ABC_DEF0
        );
    }
}
