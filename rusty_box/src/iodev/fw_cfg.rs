#![allow(dead_code)]
//! QEMU fw_cfg Device Emulation
//!
//! Provides QEMU-compatible firmware configuration interface used by UEFI/OVMF.
//! The device exposes system configuration (RAM size, CPU count, E820 map, ACPI
//! tables, etc.) through a selector/data port pair at 0x510-0x511 and a DMA
//! interface at 0x514-0x51B.
//!
//! Reference: `cpp_orig/bochs/iodev/fw_cfg.cc` (700 lines)

use crate::memory::{BxMemC, CpuTlbPin};
#[cfg(feature = "std")]
use std::io::{self, Error, ErrorKind, Read, Write};

#[cfg(feature = "std")]
use crate::snapshot::{
    bounds, checked_snapshot_len_add, checked_snapshot_len_mul, SnapshotReader, SnapshotWriteExt,
};

// ─── I/O Ports ──────────────────────────────────────────────────────────

const FW_CFG_IO_BASE: u16 = 0x510;
const FW_CFG_DATA_PORT: u16 = FW_CFG_IO_BASE + 1;

// ─── Selector Keys ──────────────────────────────────────────────────────

const FW_CFG_SIGNATURE: u16 = 0x00;
const FW_CFG_ID: u16 = 0x01;
const FW_CFG_UUID: u16 = 0x02;
const FW_CFG_RAM_SIZE: u16 = 0x03;
const FW_CFG_NOGRAPHIC: u16 = 0x04;
const FW_CFG_NB_CPUS: u16 = 0x05;
const FW_CFG_MACHINE_ID: u16 = 0x06;
const FW_CFG_KERNEL_ADDR: u16 = 0x07;
const FW_CFG_KERNEL_SIZE: u16 = 0x08;
const FW_CFG_KERNEL_CMDLINE: u16 = 0x09;
const FW_CFG_INITRD_ADDR: u16 = 0x0A;
const FW_CFG_INITRD_SIZE: u16 = 0x0B;
const FW_CFG_BOOT_DEVICE: u16 = 0x0C;
const FW_CFG_NUMA: u16 = 0x0D;
const FW_CFG_BOOT_MENU: u16 = 0x0E;
const FW_CFG_MAX_CPUS: u16 = 0x0F;
const FW_CFG_KERNEL_ENTRY: u16 = 0x10;
const FW_CFG_KERNEL_DATA: u16 = 0x11;
const FW_CFG_INITRD_DATA: u16 = 0x12;
const FW_CFG_CMDLINE_ADDR: u16 = 0x13;
const FW_CFG_CMDLINE_SIZE: u16 = 0x14;
const FW_CFG_CMDLINE_DATA: u16 = 0x15;
const FW_CFG_SETUP_ADDR: u16 = 0x16;
const FW_CFG_SETUP_SIZE: u16 = 0x17;
const FW_CFG_SETUP_DATA: u16 = 0x18;
const FW_CFG_FILE_DIR: u16 = 0x19;

const FW_CFG_FILE_FIRST: u16 = 0x20;
const FW_CFG_FILE_SLOTS: usize = 0x10;
const FW_CFG_MAX_FILE_PATH: usize = 56;

const FW_CFG_WRITE_CHANNEL: u16 = 0x4000;
const FW_CFG_ARCH_LOCAL: u16 = 0x8000;

// x86-specific entries
const FW_CFG_ACPI_TABLES: u16 = FW_CFG_ARCH_LOCAL;
const FW_CFG_SMBIOS_ENTRIES: u16 = FW_CFG_ARCH_LOCAL + 1;
const FW_CFG_IRQ0_OVERRIDE: u16 = FW_CFG_ARCH_LOCAL + 2;
const FW_CFG_E820_TABLE: u16 = FW_CFG_ARCH_LOCAL + 3;
const FW_CFG_HPET: u16 = FW_CFG_ARCH_LOCAL + 4;

// ─── E820 Types ─────────────────────────────────────────────────────────

const E820_RAM: u32 = 1;

// ─── Entry Mask / Limits ────────────────────────────────────────────────

/// Mask to strip control bits from a key to get the entry index.
const FW_CFG_ENTRY_MASK: u16 = !(FW_CFG_WRITE_CHANNEL | FW_CFG_ARCH_LOCAL);

const FW_CFG_INVALID: u16 = 0xFFFF;

// ─── Sparse Storage Constants ───────────────────────────────────────────

/// Maximum entries that can be stored.
const FW_CFG_MAX_ENTRIES: usize = 64;
/// Maximum total data bytes across all entries.
const FW_CFG_DATA_POOL_SIZE: usize = 32768;
/// Fixed storage for the serialized firmware file directory.
const FW_CFG_FILE_DIRECTORY_SIZE: usize = 4096;

// ─── FW_CFG_ID bits ─────────────────────────────────────────────────────

const FW_CFG_VERSION: u32 = 0x01;
const FW_CFG_VERSION_DMA: u32 = 0x02;

// ─── DMA ────────────────────────────────────────────────────────────────

/// "QEMU CFG" in big-endian
const FW_CFG_DMA_SIGNATURE: u64 = 0x51454d5520434647;

const FW_CFG_DMA_CTL_ERROR: u32 = 0x01;
const FW_CFG_DMA_CTL_READ: u32 = 0x02;
const FW_CFG_DMA_CTL_SKIP: u32 = 0x04;
const FW_CFG_DMA_CTL_SELECT: u32 = 0x08;
const FW_CFG_DMA_CTL_WRITE: u32 = 0x10;

// ─── Packed Structures ──────────────────────────────────────────────────

/// E820 memory map entry (20 bytes, matching QEMU layout).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct E820Entry {
    address: u64,
    length: u64,
    entry_type: u32,
}

/// HPET firmware entry (matching QEMU hpet_fw_entry).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct HpetFwEntry {
    event_timer_block_id: u32,
    address: u64,
    min_tick: u16,
    page_prot: u8,
}

/// HPET firmware config (matching QEMU hpet_fw_config).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct HpetFwConfig {
    count: u8,
    hpet: [HpetFwEntry; 8],
}

/// fw_cfg file directory entry (64 bytes: 4+2+2+56). All multi-byte fields big-endian.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct FwCfgFile {
    size: u32,
    select: u16,
    reserved: u16,
    name: [u8; FW_CFG_MAX_FILE_PATH],
}

/// fw_cfg file directory header (count + array of file entries).
#[repr(C, packed)]
struct FwCfgFiles {
    count: u32,
    f: [FwCfgFile; FW_CFG_FILE_SLOTS],
}

// ─── Sparse Slot ──────────────────────────────────────────────────────

/// A single entry in the sparse slot table, mapping a selector key to a
/// region in the shared data pool.
#[derive(Clone, Copy)]
struct FwCfgSlot {
    /// Selector key (with FW_CFG_ENTRY_MASK already applied for base keys,
    /// or the raw key for arch-local keys).
    key: u16,
    /// Byte offset into `data_pool` where this entry's data begins.
    offset: u16,
    /// Length of entry data in `data_pool`.
    len: u16,
}

// ─── BxFwCfg ────────────────────────────────────────────────────────────

/// QEMU-compatible firmware configuration device.
///
/// Uses sparse flat storage: a small slot table maps selector keys to regions
/// in a fixed-size data pool. Linear scan over ≤64 slots replaces the old
/// 16384-element Vec.
pub struct BxFwCfg {
    /// Sparse slot table — only populated entries are meaningful (indices < slot_count).
    slots: [FwCfgSlot; FW_CFG_MAX_ENTRIES],
    /// Number of populated slots.
    slot_count: u16,
    /// Shared backing store for all entry data.
    data_pool: [u8; FW_CFG_DATA_POOL_SIZE],
    /// Next free byte in `data_pool`.
    data_used: usize,
    /// Currently selected entry key.
    cur_entry: u16,
    /// Byte offset within the currently selected entry.
    cur_offset: u32,
    /// DMA descriptor address (accumulated across port writes).
    dma_addr: u64,
    /// Serialized file directory (written into entries[FW_CFG_FILE_DIR]).
    file_dir: [u8; FW_CFG_FILE_DIRECTORY_SIZE],
    /// Valid length of `file_dir`.
    file_dir_len: usize,
    /// Number of files added.
    file_count: u16,
}

impl core::fmt::Debug for BxFwCfg {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BxFwCfg")
            .field("slot_count", &self.slot_count)
            .field("data_used", &self.data_used)
            .field("cur_entry", &self.cur_entry)
            .field("cur_offset", &self.cur_offset)
            .field("dma_addr", &self.dma_addr)
            .field("file_count", &self.file_count)
            .finish()
    }
}

impl BxFwCfg {
    /// Create a new fw_cfg device with empty state.
    pub fn new() -> Self {
        Self {
            slots: [FwCfgSlot {
                key: 0,
                offset: 0,
                len: 0,
            }; FW_CFG_MAX_ENTRIES],
            slot_count: 0,
            data_pool: [0u8; FW_CFG_DATA_POOL_SIZE],
            data_used: 0,
            cur_entry: FW_CFG_INVALID,
            cur_offset: 0,
            dma_addr: 0,
            file_dir: [0u8; FW_CFG_FILE_DIRECTORY_SIZE],
            file_dir_len: 0,
            file_count: 0,
        }
    }

    /// Populate all standard entries. Called once during device initialization.
    ///
    /// This sets up the signature, ID, RAM size, CPU count, E820 map, HPET config,
    /// and all Linux kernel boot entries (zeroed). ACPI tables are added separately
    /// via [`add_acpi_tables`].
    pub fn init(&mut self, ram_size: u64, num_cpus: u32) {
        // Signature: "QEMU"
        self.add_bytes(FW_CFG_SIGNATURE, b"QEMU");

        // ID with DMA support
        self.add_bytes(
            FW_CFG_ID,
            &(FW_CFG_VERSION | FW_CFG_VERSION_DMA).to_le_bytes(),
        );

        // RAM size (little-endian 8 bytes)
        self.add_bytes(FW_CFG_RAM_SIZE, &ram_size.to_le_bytes());

        // CPU count
        self.add_bytes(FW_CFG_NB_CPUS, &(num_cpus as u16).to_le_bytes());
        self.add_bytes(FW_CFG_MAX_CPUS, &(num_cpus as u16).to_le_bytes());

        // x86-specific: IRQ0 connected to IOAPIC pin 2
        self.add_bytes(FW_CFG_IRQ0_OVERRIDE, &1u32.to_le_bytes());

        // NOGRAPHIC: 0 = graphics enabled
        self.add_bytes(FW_CFG_NOGRAPHIC, &0u16.to_le_bytes());

        // BOOT_MENU: 0 = disabled
        self.add_bytes(FW_CFG_BOOT_MENU, &0u16.to_le_bytes());

        // MACHINE_ID: 0 = PC
        self.add_bytes(FW_CFG_MACHINE_ID, &0u32.to_le_bytes());

        // BOOT_DEVICE: 0 = default
        self.add_bytes(FW_CFG_BOOT_DEVICE, &0u16.to_le_bytes());

        // NUMA: empty
        self.add_bytes(FW_CFG_NUMA, &0u64.to_le_bytes());

        // Linux kernel boot entries — zeroed (OVMF probes these)
        for key in [
            FW_CFG_KERNEL_ADDR,
            FW_CFG_KERNEL_SIZE,
            FW_CFG_KERNEL_ENTRY,
            FW_CFG_INITRD_ADDR,
            FW_CFG_INITRD_SIZE,
            FW_CFG_CMDLINE_ADDR,
            FW_CFG_CMDLINE_SIZE,
            FW_CFG_SETUP_ADDR,
            FW_CFG_SETUP_SIZE,
        ] {
            self.add_bytes(key, &0u32.to_le_bytes());
        }

        // Initialize file directory
        self.init_file_dir();

        // E820 memory map (added as file "etc/e820")
        self.generate_e820_map(ram_size);

        // HPET configuration
        self.generate_hpet_config();

        tracing::info!("fw_cfg device initialized (ports 0x510-0x511, DMA 0x514)");
    }

    /// Reset selector, offset, and DMA address.
    pub fn reset(&mut self) {
        self.cur_entry = FW_CFG_INVALID;
        self.cur_offset = 0;
        self.dma_addr = 0;
    }
    /// Number of bytes emitted by the PLATFORM fw_cfg component body.
    ///
    /// The parent PLATFORM section owns the section-version prefix.  This
    /// component intentionally writes only its state body.
    #[cfg(feature = "std")]
    pub(crate) fn snapshot_v3_body_len(&self) -> io::Result<u64> {
        self.validate_snapshot_v3_state()?;

        let slot_bytes = checked_snapshot_len_mul(
            u64::try_from(usize::from(self.slot_count))
                .map_err(|_| invalid_fw_cfg_snapshot("slot count does not fit u64"))?,
            6,
        )?;
        let with_slots = checked_snapshot_len_add(38, slot_bytes)?;
        let with_pool = checked_snapshot_len_add(
            with_slots,
            u64::try_from(self.data_used)
                .map_err(|_| invalid_fw_cfg_snapshot("data-pool length does not fit u64"))?,
        )?;
        checked_snapshot_len_add(
            with_pool,
            u64::try_from(FW_CFG_FILE_DIRECTORY_SIZE)
                .map_err(|_| invalid_fw_cfg_snapshot("file-directory length does not fit u64"))?,
        )
    }

    /// Streams the PLATFORM fw_cfg component body without staging a payload.
    #[cfg(feature = "std")]
    pub(crate) fn save_snapshot_v3_body<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.validate_snapshot_v3_state()?;

        writer.write_u32(u32::from(self.slot_count))?;
        writer.write_u64(
            u64::try_from(self.data_used)
                .map_err(|_| invalid_fw_cfg_snapshot("data-pool length does not fit u64"))?,
        )?;
        writer.write_u16(self.cur_entry)?;
        writer.write_u32(self.cur_offset)?;
        writer.write_u64(self.dma_addr)?;
        writer.write_u64(
            u64::try_from(self.file_dir_len)
                .map_err(|_| invalid_fw_cfg_snapshot("file-directory length does not fit u64"))?,
        )?;
        writer.write_u32(u32::from(self.file_count))?;

        for slot in &self.slots[..usize::from(self.slot_count)] {
            writer.write_u16(slot.key)?;
            writer.write_u16(slot.offset)?;
            writer.write_u16(slot.len)?;
        }
        writer.write_bytes(&self.data_pool[..self.data_used])?;
        writer.write_bytes(&self.file_dir)
    }

    /// Restores the PLATFORM fw_cfg component body.
    ///
    /// Descriptor/count validation happens before the pool or directory are
    /// touched.  The fixed buffers are then read straight from the bounded
    /// stream; a later I/O error leaves the instance non-resumable, matching
    /// the parent snapshot restore contract.
    #[cfg(feature = "std")]
    pub(crate) fn restore_snapshot_v3_body<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<()> {
        let slot_count = reader.read_count(FW_CFG_MAX_ENTRIES)?;
        let data_used = reader.read_len(FW_CFG_DATA_POOL_SIZE)?;
        let cur_entry = reader.read_u16()?;
        let cur_offset = reader.read_u32()?;
        let dma_addr = reader.read_u64()?;
        let file_dir_len = reader.read_len(FW_CFG_FILE_DIRECTORY_SIZE)?;
        let file_count = reader.read_count(FW_CFG_FILE_SLOTS)?;

        let mut saved_slots = [FwCfgSlot {
            key: 0,
            offset: 0,
            len: 0,
        }; FW_CFG_MAX_ENTRIES];
        for slot in &mut saved_slots[..slot_count] {
            *slot = FwCfgSlot {
                key: reader.read_u16()?,
                offset: reader.read_u16()?,
                len: reader.read_u16()?,
            };
        }
        validate_fw_cfg_slots(
            &saved_slots[..slot_count],
            data_used,
            cur_entry,
            cur_offset,
        )?;

        reader.read_bytes(&mut self.data_pool[..data_used])?;
        reader.read_bytes(&mut self.file_dir)?;
        validate_fw_cfg_file_directory(
            &saved_slots[..slot_count],
            file_dir_len,
            file_count,
            &self.data_pool,
            &self.file_dir,
        )?;

        self.slots = [FwCfgSlot {
            key: 0,
            offset: 0,
            len: 0,
        }; FW_CFG_MAX_ENTRIES];
        self.slots[..slot_count].copy_from_slice(&saved_slots[..slot_count]);
        self.slot_count = u16::try_from(slot_count)
            .map_err(|_| invalid_fw_cfg_snapshot("slot count does not fit u16"))?;
        self.data_used = data_used;
        self.cur_entry = cur_entry;
        self.cur_offset = cur_offset;
        self.dma_addr = dma_addr;
        self.file_dir_len = file_dir_len;
        self.file_count = u16::try_from(file_count)
            .map_err(|_| invalid_fw_cfg_snapshot("file count does not fit u16"))?;
        Ok(())
    }

    #[cfg(feature = "std")]
    fn validate_snapshot_v3_state(&self) -> io::Result<()> {
        let slot_count = usize::from(self.slot_count);
        let file_count = usize::from(self.file_count);
        if slot_count > FW_CFG_MAX_ENTRIES
            || file_count > FW_CFG_FILE_SLOTS
            || slot_count > bounds::MAX_SNAPSHOT_COUNT
            || file_count > bounds::MAX_SNAPSHOT_COUNT
        {
            return Err(invalid_fw_cfg_snapshot("fw_cfg count exceeds capacity"));
        }
        validate_fw_cfg_slots(
            &self.slots[..slot_count],
            self.data_used,
            self.cur_entry,
            self.cur_offset,
        )?;
        validate_fw_cfg_file_directory(
            &self.slots[..slot_count],
            self.file_dir_len,
            file_count,
            &self.data_pool,
            &self.file_dir,
        )
    }


    // ─── Slot Lookup ────────────────────────────────────────────────────

    /// Find the slot index for a given raw key, or None.
    fn find_slot(&self, key: u16) -> Option<usize> {
        let count = self.slot_count as usize;
        for i in 0..count {
            if self.slots[i].key == key {
                return Some(i);
            }
        }
        None
    }

    /// Get entry data for a given raw key.
    fn get_entry_data(&self, key: u16) -> Option<&[u8]> {
        self.find_slot(key).map(|i| {
            let slot = &self.slots[i];
            let start = slot.offset as usize;
            let end = start + slot.len as usize;
            &self.data_pool[start..end]
        })
    }

    // ─── I/O Port Handlers ──────────────────────────────────────────────

    /// Handle a read from fw_cfg I/O ports.
    ///
    /// - 0x510: selector port — returns 0
    /// - 0x511: data port — returns next byte from current entry
    /// - 0x514-0x51B: DMA signature bytes
    pub fn read_port(&self, address: u16, _io_len: u8) -> u32 {
        match address {
            FW_CFG_IO_BASE => 0,
            FW_CFG_DATA_PORT => {
                // Reads from the data port. We need &mut self for cur_offset,
                // but the dispatch gives us &self for reads. Use interior
                // mutability would be needed for a strict port; for now this
                // path is rarely used (OVMF uses DMA), so return 0 if we
                // can't mutate. The mutable path is provided via read_port_mut.
                0
            }
            0x514..=0x51B => {
                // DMA signature: return byte at offset within the 8-byte signature
                let offset = (address - 0x514) as u32;
                let sig = FW_CFG_DMA_SIGNATURE;
                ((sig >> (offset * 8)) & 0xFF) as u32
            }
            _ => 0,
        }
    }

    /// Mutable read from data port (0x511). Advances cur_offset.
    /// Called when the dispatch can provide &mut self.
    pub fn read_port_mut(&mut self, address: u16, _io_len: u8) -> u32 {
        match address {
            FW_CFG_IO_BASE => 0,
            FW_CFG_DATA_PORT => {
                if self.cur_entry == FW_CFG_INVALID {
                    return 0;
                }
                let key = self.cur_entry & FW_CFG_ENTRY_MASK;
                if let Some(data) = self.get_entry_data(key) {
                    if (self.cur_offset as usize) < data.len() {
                        let val = data[self.cur_offset as usize] as u32;
                        self.cur_offset += 1;
                        return val;
                    }
                }
                0
            }
            0x514..=0x51B => {
                let offset = (address - 0x514) as u32;
                ((FW_CFG_DMA_SIGNATURE >> (offset * 8)) & 0xFF) as u32
            }
            _ => 0,
        }
    }

    /// Handle a write to fw_cfg I/O ports.
    ///
    /// - 0x510: selector (2-byte write sets cur_entry, resets offset)
    /// - 0x511: data write (ignored)
    /// - 0x514-0x51B: DMA address accumulation; writing low 32 bits triggers DMA
    pub(crate) fn write_port(
        &mut self,
        address: u16,
        value: u32,
        io_len: u8,
        mem: Option<&mut BxMemC<'_>>,
        pins: &[CpuTlbPin],
    ) {
        match address {
            FW_CFG_IO_BASE => {
                if io_len == 2 {
                    self.cur_entry = (value & 0xFFFF) as u16;
                    self.cur_offset = 0;
                    tracing::debug!("fw_cfg: selected entry {:#06x}", self.cur_entry);
                } else {
                    tracing::warn!("fw_cfg: invalid selector write, io_len={}", io_len);
                }
            }
            FW_CFG_DATA_PORT => {
                // Data port write — not used by OVMF
                tracing::debug!("fw_cfg: write to data port ignored");
            }
            0x514..=0x51B => {
                let offset = (address - 0x514) as usize;

                if io_len == 4 {
                    // OVMF sends values big-endian — swap to native
                    let swapped = value.swap_bytes();

                    if offset == 0 {
                        // High 32 bits → port 0x514
                        self.dma_addr = (swapped as u64) << 32;
                    } else if offset == 4 {
                        // Low 32 bits → port 0x518 — triggers DMA
                        self.dma_addr = (self.dma_addr & 0xFFFF_FFFF_0000_0000) | swapped as u64;
                        self.trigger_dma(mem, pins);
                    }
                } else if io_len == 1 {
                    // Byte-by-byte write (big-endian)
                    let shift = (7 - offset) * 8;
                    self.dma_addr =
                        (self.dma_addr & !(0xFFu64 << shift)) | ((value as u64 & 0xFF) << shift);

                    // Trigger when last byte (offset 7) is written
                    if offset == 7 {
                        self.trigger_dma(mem, pins);
                    }
                }
            }
            _ => {}
        }
    }

    /// Trigger DMA processing if memory is available, then clear dma_addr.
    fn trigger_dma(&mut self, mem: Option<&mut BxMemC<'_>>, pins: &[CpuTlbPin]) {
        let addr = self.dma_addr;
        if let Some(m) = mem {
            self.process_dma(addr, m, pins);
        } else {
            tracing::error!(
                "fw_cfg DMA: triggered at {:#x} but no memory available",
                addr
            );
        }
        self.dma_addr = 0;
    }

    // ─── DMA ────────────────────────────────────────────────────────────

    /// Process a DMA descriptor at `dma_addr` in guest physical memory.
    ///
    /// Descriptor layout (16 bytes, big-endian):
    /// - control (4 bytes): SELECT/READ/SKIP/WRITE flags + key in upper 16 bits
    /// - length (4 bytes)
    /// - address (8 bytes): guest physical address for data transfer
    fn process_dma(&mut self, dma_addr: u64, mem: &mut BxMemC<'_>, pins: &[CpuTlbPin]) {
        // A DMA descriptor is indivisible: do not interpret a short/hole
        // prefix, because its control word may not belong to this request.
        let mut desc = [0u8; 16];
        match mem.read_ram(pins, dma_addr, &mut desc) {
            Ok(16) => {}
            Ok(copied) => {
                tracing::error!(
                    "fw_cfg DMA: descriptor at {dma_addr:#x} is short ({copied}/16 bytes)"
                );
                return;
            }
            Err(error) => {
                tracing::error!("fw_cfg DMA: descriptor read at {dma_addr:#x} failed: {error:?}");
                return;
            }
        }

        // Parse big-endian descriptor.
        let mut control = u32::from_be_bytes([desc[0], desc[1], desc[2], desc[3]]);
        let length = u32::from_be_bytes([desc[4], desc[5], desc[6], desc[7]]);
        let address = u64::from_be_bytes([
            desc[8], desc[9], desc[10], desc[11], desc[12], desc[13], desc[14], desc[15],
        ]);

        tracing::debug!(
            "fw_cfg DMA: control={:#010x}, length={}, address={:#x}, cur_entry={:#06x}",
            control,
            length,
            address,
            self.cur_entry
        );

        if control & FW_CFG_DMA_CTL_SELECT != 0 {
            self.cur_entry = (control >> 16) as u16;
            self.cur_offset = 0;
            tracing::debug!("fw_cfg DMA: selected entry {:#06x}", self.cur_entry);
        }

        if control & FW_CFG_DMA_CTL_READ != 0 {
            let key = self.cur_entry & FW_CFG_ENTRY_MASK;
            let entry_offset = self.cur_offset as usize;
            let mut success = false;
            if let Some(data) = self.get_entry_data(key) {
                let available = data.len().saturating_sub(entry_offset);
                let requested = available.min(length as usize);
                let committed = match mem.write_ram(pins, address, &data[entry_offset..entry_offset + requested]) {
                    Ok(copied) => copied,
                    Err(error) => {
                        tracing::error!("fw_cfg DMA READ: write to {address:#x} failed: {error:?}");
                        0
                    }
                };
                self.cur_offset = self.cur_offset.saturating_add(committed as u32);
                success = committed == requested;
                if !success {
                    tracing::error!(
                        "fw_cfg DMA READ: destination {address:#x} accepted {committed}/{requested} bytes"
                    );
                }
            }
            if !success {
                control |= FW_CFG_DMA_CTL_ERROR;
                tracing::error!("fw_cfg DMA: invalid or incomplete entry {:#06x}", self.cur_entry);
            }
            control &= !FW_CFG_DMA_CTL_READ;
        }

        if control & FW_CFG_DMA_CTL_SKIP != 0 {
            self.cur_offset = self.cur_offset.saturating_add(length);
            control &= !FW_CFG_DMA_CTL_SKIP;
            tracing::debug!("fw_cfg DMA: skipped {} bytes", length);
        }

        // QEMU/Bochs does not implement host-directed writes for this device.
        // Complete them with ERROR rather than leaving the DMA command busy.
        if control & FW_CFG_DMA_CTL_WRITE != 0 {
            control |= FW_CFG_DMA_CTL_ERROR;
            control &= !FW_CFG_DMA_CTL_WRITE;
        }

        control &= !FW_CFG_DMA_CTL_SELECT;
        let ctrl_be = control.to_be_bytes();
        match mem.write_ram(pins, dma_addr, &ctrl_be) {
            Ok(4) => {}
            Ok(copied) => tracing::error!(
                "fw_cfg DMA: completion at {dma_addr:#x} is short ({copied}/4 bytes)"
            ),
            Err(error) => {
                tracing::error!("fw_cfg DMA: completion write at {dma_addr:#x} failed: {error:?}")
            }
        }
    }

    // ─── Entry Management ───────────────────────────────────────────────

    /// Store raw bytes for a given key. Overwrites any existing entry.
    ///
    /// If the key already exists, the old data space in the pool is abandoned
    /// (wasted) and new data is appended. The 32KB pool is generous enough for
    /// the ~15-20 entries used during boot.
    pub fn add_bytes(&mut self, key: u16, data: &[u8]) {
        // Check pool capacity
        if self.data_used + data.len() > FW_CFG_DATA_POOL_SIZE {
            tracing::error!(
                "fw_cfg: data pool full (used={}, need={}), cannot add key {:#06x}",
                self.data_used,
                data.len(),
                key
            );
            return;
        }

        let offset = self.data_used as u16;
        let len = data.len() as u16;

        // Copy data into pool
        self.data_pool[self.data_used..self.data_used + data.len()].copy_from_slice(data);
        self.data_used += data.len();

        // Update existing slot or allocate a new one
        if let Some(i) = self.find_slot(key) {
            // Overwrite: old pool space is wasted, point to new data
            self.slots[i].offset = offset;
            self.slots[i].len = len;
        } else {
            if (self.slot_count as usize) >= FW_CFG_MAX_ENTRIES {
                tracing::error!("fw_cfg: slot table full, cannot add key {:#06x}", key);
                return;
            }
            let i = self.slot_count as usize;
            self.slots[i] = FwCfgSlot { key, offset, len };
            self.slot_count += 1;
        }

        tracing::debug!("fw_cfg: added entry {:#06x}, len={}", key, data.len());
    }

    /// Add a named file to the file directory.
    ///
    /// Files are assigned sequential keys starting at `FW_CFG_FILE_FIRST`.
    /// Directory entry fields (size, select) are stored big-endian.
    pub fn add_file(&mut self, name: &str, data: &[u8]) {
        if self.file_count as usize >= FW_CFG_FILE_SLOTS {
            tracing::error!("fw_cfg: file directory full, cannot add '{}'", name);
            return;
        }

        let file_index = FW_CFG_FILE_FIRST + self.file_count;
        let data_len = data.len() as u32;
        self.add_bytes(file_index, data);

        // Build 64-byte file entry (big-endian size + select, zero-padded name)
        let entry_offset = 4 + (self.file_count as usize) * 64; // skip count field
        if entry_offset + 64 > self.file_dir.len() {
            tracing::error!("fw_cfg: file_dir too small");
            return;
        }

        // size (big-endian u32)
        self.file_dir[entry_offset..entry_offset + 4].copy_from_slice(&data_len.to_be_bytes());
        // select (big-endian u16)
        self.file_dir[entry_offset + 4..entry_offset + 6]
            .copy_from_slice(&file_index.to_be_bytes());
        // reserved (2 bytes, already zero)
        // name (56 bytes, zero-padded)
        let name_bytes = name.as_bytes();
        let copy_len = core::cmp::min(name_bytes.len(), FW_CFG_MAX_FILE_PATH - 1);
        self.file_dir[entry_offset + 8..entry_offset + 8 + copy_len]
            .copy_from_slice(&name_bytes[..copy_len]);

        self.file_count += 1;

        // Update count field (big-endian u32) at start of file_dir
        self.file_dir[0..4].copy_from_slice(&(self.file_count as u32).to_be_bytes());

        // Re-register the file directory in entries so reads see updated data
        let dir_len = self.file_dir_len;
        // Safety: we need to borrow file_dir immutably while calling add_bytes.
        // Copy the relevant slice to avoid aliasing issues.
        let mut dir_copy = [0u8; FW_CFG_FILE_DIRECTORY_SIZE];
        dir_copy[..dir_len].copy_from_slice(&self.file_dir[..dir_len]);
        self.add_bytes(FW_CFG_FILE_DIR, &dir_copy[..dir_len]);

        tracing::debug!(
            "fw_cfg: added file '{}' at index {:#06x} ({} bytes)",
            name,
            file_index,
            data_len
        );
    }

    /// Add ACPI tables, RSDP, and loader as fw_cfg files.
    /// Called externally after ACPI table generation.
    pub fn add_acpi_tables(&mut self, tables: &[u8], rsdp: &[u8], loader: &[u8]) {
        self.add_file("etc/acpi/tables", tables);
        self.add_file("etc/acpi/rsdp", rsdp);
        self.add_file("etc/table-loader", loader);
        tracing::debug!("fw_cfg: added ACPI tables");
    }

    // ─── Internal Generators ────────────────────────────────────────────

    /// Initialize the file directory buffer (count + FILE_SLOTS * 64 bytes).
    fn init_file_dir(&mut self) {
        let dir_size = 4 + FW_CFG_FILE_SLOTS * 64;
        self.file_dir = [0u8; FW_CFG_FILE_DIRECTORY_SIZE];
        self.file_dir_len = dir_size;
        self.file_count = 0;
        // Register empty directory — file_dir is all zeros, safe to copy out
        let empty = [0u8; FW_CFG_FILE_DIRECTORY_SIZE];
        self.add_bytes(FW_CFG_FILE_DIR, &empty[..dir_size]);
    }

    /// Generate the E820 memory map and add it as file "etc/e820".
    ///
    /// Layout:
    /// - Entry 0: RAM from 0 to below_4g (up to 3GB if total >= 3.5GB)
    /// - Entry 1 (optional): RAM from 4GB to 4GB + above_4g
    ///
    /// No RESERVED entry for the PCI hole — OVMF expects this.
    fn generate_e820_map(&mut self, ram_size: u64) {
        let (below_4g, above_4g) = if ram_size >= 0xE000_0000 {
            // RAM >= 3.5GB: cap below-4G at 3GB, remainder goes above 4GB
            (0xC000_0000u64, ram_size - 0xC000_0000)
        } else {
            (ram_size, 0u64)
        };

        let mut entries = [E820Entry {
            address: 0,
            length: 0,
            entry_type: 0,
        }; 2];
        let mut count = 0usize;

        // Entry 0: below 4GB RAM
        entries[0] = E820Entry {
            address: 0,
            length: below_4g,
            entry_type: E820_RAM,
        };
        count += 1;

        // Entry 1: above 4GB RAM (only if present)
        if above_4g > 0 {
            entries[1] = E820Entry {
                address: 0x1_0000_0000,
                length: above_4g,
                entry_type: E820_RAM,
            };
            count += 1;
        }

        tracing::debug!("fw_cfg: generated {} e820 entries:", count);
        for i in 0..count {
            let e = &entries[i];
            tracing::debug!(
                "  Entry {}: addr={:#x} len={:#x} type={}",
                i,
                { e.address },
                { e.length },
                { e.entry_type },
            );
        }

        // Serialize to raw bytes
        let entry_size = core::mem::size_of::<E820Entry>();
        let total_size = count * entry_size;
        let mut data = [0u8; 2 * core::mem::size_of::<E820Entry>()];
        unsafe {
            core::ptr::copy_nonoverlapping(
                entries.as_ptr() as *const u8,
                data.as_mut_ptr(),
                total_size,
            );
        }

        self.add_file("etc/e820", &data[..total_size]);
    }

    /// Generate HPET configuration and add it as FW_CFG_HPET key.
    fn generate_hpet_config(&mut self) {
        let mut cfg = HpetFwConfig {
            count: 1,
            hpet: [HpetFwEntry {
                event_timer_block_id: 0,
                address: 0,
                min_tick: 0,
                page_prot: 0,
            }; 8],
        };

        cfg.hpet[0] = HpetFwEntry {
            event_timer_block_id: 0x8086A201, // Intel vendor ID
            address: 0xFED0_0000,             // Standard HPET base
            min_tick: 100,
            page_prot: 0,
        };

        // Serialize: count byte + 1 entry
        let hpet_size = core::mem::size_of::<u8>() + core::mem::size_of::<HpetFwEntry>();
        let mut data = [0u8; core::mem::size_of::<u8>() + core::mem::size_of::<HpetFwEntry>()];
        unsafe {
            core::ptr::copy_nonoverlapping(
                &cfg as *const HpetFwConfig as *const u8,
                data.as_mut_ptr(),
                hpet_size,
            );
        }

        self.add_bytes(FW_CFG_HPET, &data[..hpet_size]);
        tracing::debug!("fw_cfg: added HPET configuration");
    }
}
#[cfg(feature = "std")]
fn invalid_fw_cfg_snapshot(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
fn validate_fw_cfg_slots(
    slots: &[FwCfgSlot],
    data_used: usize,
    cur_entry: u16,
    cur_offset: u32,
) -> io::Result<()> {
    if data_used > FW_CFG_DATA_POOL_SIZE {
        return Err(invalid_fw_cfg_snapshot("fw_cfg data pool exceeds capacity"));
    }

    for (index, slot) in slots.iter().enumerate() {
        if slot.key == FW_CFG_INVALID || slot.key & FW_CFG_WRITE_CHANNEL != 0 {
            return Err(invalid_fw_cfg_snapshot("fw_cfg slot key is invalid"));
        }

        let start = usize::from(slot.offset);
        let end = start
            .checked_add(usize::from(slot.len))
            .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg slot range overflows"))?;
        if end > data_used {
            return Err(invalid_fw_cfg_snapshot("fw_cfg slot exceeds data pool"));
        }

        for prior in &slots[..index] {
            if prior.key == slot.key {
                return Err(invalid_fw_cfg_snapshot("fw_cfg slot key is duplicated"));
            }
            let prior_start = usize::from(prior.offset);
            let prior_end = prior_start
                .checked_add(usize::from(prior.len))
                .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg slot range overflows"))?;
            if start < prior_end && prior_start < end {
                return Err(invalid_fw_cfg_snapshot("fw_cfg slot ranges overlap"));
            }
        }
    }

    if cur_entry == FW_CFG_INVALID {
        if cur_offset != 0 {
            return Err(invalid_fw_cfg_snapshot(
                "fw_cfg invalid selector has a nonzero offset",
            ));
        }
        return Ok(());
    }

    let selected_key = cur_entry & FW_CFG_ENTRY_MASK;
    let selected_slot = slots
        .iter()
        .find(|slot| slot.key == cur_entry || slot.key == selected_key)
        .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg current selector has no slot"))?;
    if cur_offset > u32::from(selected_slot.len) {
        return Err(invalid_fw_cfg_snapshot(
            "fw_cfg current offset exceeds selected entry",
        ));
    }
    Ok(())
}

#[cfg(feature = "std")]
fn validate_fw_cfg_file_directory(
    slots: &[FwCfgSlot],
    file_dir_len: usize,
    file_count: usize,
    data_pool: &[u8; FW_CFG_DATA_POOL_SIZE],
    directory: &[u8; FW_CFG_FILE_DIRECTORY_SIZE],
) -> io::Result<()> {
    if file_dir_len > FW_CFG_FILE_DIRECTORY_SIZE || file_count > FW_CFG_FILE_SLOTS {
        return Err(invalid_fw_cfg_snapshot("fw_cfg file directory exceeds capacity"));
    }
    let directory_slot = slots
        .iter()
        .find(|slot| slot.key == FW_CFG_FILE_DIR)
        .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file directory slot is absent"))?;
    if usize::from(directory_slot.len) != file_dir_len {
        return Err(invalid_fw_cfg_snapshot(
            "fw_cfg file directory slot length disagrees with metadata",
        ));
    }
    let directory_start = usize::from(directory_slot.offset);
    let directory_end = directory_start
        .checked_add(usize::from(directory_slot.len))
        .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file directory slot range overflows"))?;
    let pool_directory = data_pool.get(directory_start..directory_end).ok_or_else(|| {
        invalid_fw_cfg_snapshot("fw_cfg file directory slot exceeds data pool")
    })?;
    if pool_directory != &directory[..file_dir_len] {
        return Err(invalid_fw_cfg_snapshot(
            "fw_cfg file directory data disagrees with pool",
        ));
    }
    if file_dir_len == 0 {
        return if file_count == 0 {
            Ok(())
        } else {
            Err(invalid_fw_cfg_snapshot(
                "fw_cfg empty file directory has files",
            ))
        };
    }

    let required_len = 4usize
        .checked_add(
            file_count
                .checked_mul(64)
                .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file count overflows"))?,
        )
        .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file directory length overflows"))?;
    if file_dir_len < required_len {
        return Err(invalid_fw_cfg_snapshot(
            "fw_cfg file directory is shorter than its entries",
        ));
    }

    let header = directory
        .get(0..4)
        .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file directory header is missing"))?;
    let mut encoded_count = [0u8; 4];
    encoded_count.copy_from_slice(header);
    if usize::try_from(u32::from_be_bytes(encoded_count))
        .map_err(|_| invalid_fw_cfg_snapshot("fw_cfg file count does not fit usize"))?
        != file_count
    {
        return Err(invalid_fw_cfg_snapshot(
            "fw_cfg file directory count disagrees with metadata",
        ));
    }


    for index in 0..file_count {
        let entry_start = 4usize
            .checked_add(
                index
                    .checked_mul(64)
                    .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file index overflows"))?,
            )
            .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file entry offset overflows"))?;
        let entry_end = entry_start
            .checked_add(64)
            .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file entry range overflows"))?;
        let entry = directory
            .get(entry_start..entry_end)
            .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file entry exceeds directory"))?;

        let mut size_bytes = [0u8; 4];
        size_bytes.copy_from_slice(
            entry
                .get(0..4)
                .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file size is missing"))?,
        );
        let mut select_bytes = [0u8; 2];
        select_bytes.copy_from_slice(
            entry
                .get(4..6)
                .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file selector is missing"))?,
        );
        let selected_key = u16::from_be_bytes(select_bytes);
        let expected_key = u16::try_from(
            usize::from(FW_CFG_FILE_FIRST)
                .checked_add(index)
                .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file selector overflows"))?,
        )
        .map_err(|_| invalid_fw_cfg_snapshot("fw_cfg file selector does not fit u16"))?;
        if selected_key != expected_key {
            return Err(invalid_fw_cfg_snapshot("fw_cfg file selector is out of order"));
        }
        let file_slot = slots
            .iter()
            .find(|slot| slot.key == selected_key)
            .ok_or_else(|| invalid_fw_cfg_snapshot("fw_cfg file entry slot is absent"))?;
        if u32::from(file_slot.len) != u32::from_be_bytes(size_bytes) {
            return Err(invalid_fw_cfg_snapshot(
                "fw_cfg file size disagrees with its slot",
            ));
        }
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    const SELECTOR_WRITE_BYTES: u8 = 2;
    const DATA_READ_BYTES: u8 = 1;
    const TEST_RAM_SIZE: u64 = 64 * 1024 * 1024;
    const TEST_CPU_COUNT: u32 = 4;

    use super::*;

    fn read_u16_entry(fw_cfg: &mut BxFwCfg, key: u16) -> u16 {
        fw_cfg.write_port(FW_CFG_IO_BASE, key as u32, SELECTOR_WRITE_BYTES, None, &[]);
        let lo = fw_cfg.read_port_mut(FW_CFG_DATA_PORT, DATA_READ_BYTES) as u16;
        let hi = fw_cfg.read_port_mut(FW_CFG_DATA_PORT, DATA_READ_BYTES) as u16;
        lo | (hi << 8)
    }

    #[test]
    fn fw_cfg_dma_descriptor_and_payload_in_swapped_blocks() {
        use crate::memory::{BxMemC, BxMemoryStubC};

        const MIB: usize = 1024 * 1024;
        const DESCRIPTOR: u64 = 2 * MIB as u64;
        const DESTINATION: u64 = 3 * MIB as u64;
        const KEY: u16 = 0x1234;

        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(4 * MIB, MIB, MIB).expect("swapped memory"),
            false,
        );
        mem.set_a20_mask(u64::MAX);
        let mut fw_cfg = BxFwCfg::new();
        fw_cfg.add_bytes(KEY, &[0x61, 0x62, 0x63, 0x64, 0x65]);

        let control = ((KEY as u32) << 16) | FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_READ;
        let mut descriptor = [0u8; 16];
        descriptor[..4].copy_from_slice(&control.to_be_bytes());
        descriptor[4..8].copy_from_slice(&(5u32).to_be_bytes());
        descriptor[8..].copy_from_slice(&DESTINATION.to_be_bytes());
        assert_eq!(mem.write_ram(&[], DESCRIPTOR, &descriptor).unwrap(), 16);
        mem.smc_mark_icache_mask(DESCRIPTOR, u32::MAX);
        mem.smc_mark_icache_mask(DESTINATION, u32::MAX);
        let before_smc = mem.smc_seq_next();

        fw_cfg.process_dma(DESCRIPTOR, &mut mem, &[]);

        let mut payload = [0; 5];
        assert_eq!(mem.read_ram(&[], DESTINATION, &mut payload).unwrap(), payload.len());
        assert_eq!(payload, [0x61, 0x62, 0x63, 0x64, 0x65]);
        let mut completion = [0; 4];
        assert_eq!(mem.read_ram(&[], DESCRIPTOR, &mut completion).unwrap(), 4);
        assert_eq!(u32::from_be_bytes(completion), (KEY as u32) << 16);
        assert!(mem.smc_seq_next() > before_smc);

        descriptor[..4].copy_from_slice(&FW_CFG_DMA_CTL_WRITE.to_be_bytes());
        assert_eq!(mem.write_ram(&[], DESCRIPTOR, &descriptor).unwrap(), 16);
        fw_cfg.process_dma(DESCRIPTOR, &mut mem, &[]);
        assert_eq!(mem.read_ram(&[], DESCRIPTOR, &mut completion).unwrap(), 4);
        assert_eq!(u32::from_be_bytes(completion), FW_CFG_DMA_CTL_ERROR);
    }

    #[test]
    fn init_exposes_configured_cpu_count() {
        let mut fw_cfg = BxFwCfg::new();
        fw_cfg.init(TEST_RAM_SIZE, TEST_CPU_COUNT);

        assert_eq!(
            read_u16_entry(&mut fw_cfg, FW_CFG_NB_CPUS),
            TEST_CPU_COUNT as u16
        );
        assert_eq!(
            read_u16_entry(&mut fw_cfg, FW_CFG_MAX_CPUS),
            TEST_CPU_COUNT as u16
        );
    }

    #[test]
    fn platform_snapshot_resumes_fw_cfg_pio_and_partial_dma_address() {
        use crate::memory::{BxMemC, BxMemoryStubC};
        use crate::snapshot::SnapshotReader;
        use std::io::{Cursor, ErrorKind};

        const MIB: usize = 1024 * 1024;
        const DESCRIPTOR: u64 = 2 * MIB as u64;
        const DESTINATION: u64 = 3 * MIB as u64;
        const PAYLOAD: &[u8] = b"resume";

        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(4 * MIB, MIB, MIB).expect("snapshot memory"),
            false,
        );
        mem.set_a20_mask(u64::MAX);

        let mut source = BxFwCfg::new();
        source.init(TEST_RAM_SIZE, TEST_CPU_COUNT);
        let payload_key = FW_CFG_FILE_FIRST + source.file_count;
        source.add_file("etc/snapshot-resume", PAYLOAD);
        source.write_port(
            FW_CFG_IO_BASE,
            payload_key as u32,
            SELECTOR_WRITE_BYTES,
            None,
            &[],
        );
        assert_eq!(
            source.read_port_mut(FW_CFG_DATA_PORT, DATA_READ_BYTES),
            u32::from(PAYLOAD[0])
        );

        let control =
            (u32::from(payload_key) << 16) | FW_CFG_DMA_CTL_SELECT | FW_CFG_DMA_CTL_READ;
        let mut descriptor = [0u8; 16];
        descriptor[..4].copy_from_slice(&control.to_be_bytes());
        descriptor[4..8].copy_from_slice(&(PAYLOAD.len() as u32).to_be_bytes());
        descriptor[8..].copy_from_slice(&DESTINATION.to_be_bytes());
        assert_eq!(mem.write_ram(&[], DESCRIPTOR, &descriptor).unwrap(), descriptor.len());

        // Write all but the triggering byte of a bytewise DMA address.  The
        // final byte must resume the descriptor after restoration.
        source.write_port(0x518, 0, 1, None, &[]);
        source.write_port(0x519, 0x20, 1, None, &[]);
        source.write_port(0x51A, 0, 1, None, &[]);
        assert_eq!(source.dma_addr, DESCRIPTOR);

        let mut saved = Vec::new();
        source.save_snapshot_v3_body(&mut saved).unwrap();

        let mut restored = BxFwCfg::new();
        let mut reader = SnapshotReader::new(Cursor::new(saved.clone()), saved.len() as u64).unwrap();
        restored.restore_snapshot_v3_body(&mut reader).unwrap();
        reader.finish_exact().unwrap();

        assert_eq!(
            restored.read_port_mut(FW_CFG_DATA_PORT, DATA_READ_BYTES),
            u32::from(PAYLOAD[1]),
            "the restored selector and PIO offset must resume at the next byte"
        );
        assert_eq!(restored.dma_addr, DESCRIPTOR);
        restored.write_port(0x51B, 0, 1, Some(&mut mem), &[]);

        let mut payload = [0u8; PAYLOAD.len()];
        assert_eq!(
            mem.read_ram(&[], DESTINATION, &mut payload).unwrap(),
            payload.len()
        );
        assert_eq!(payload, PAYLOAD);

        // The first saved slot has nonempty data.  Moving it to the end of
        // the used pool makes its range invalid before any restored buffers
        // can be committed.
        let data_used = u64::from_le_bytes(saved[4..12].try_into().unwrap());
        let mut malformed = saved;
        malformed[40..42].copy_from_slice(&(data_used as u16).to_le_bytes());
        let malformed_len = malformed.len() as u64;
        let mut reader = SnapshotReader::new(Cursor::new(malformed), malformed_len).unwrap();
        let error = restored.restore_snapshot_v3_body(&mut reader).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
    }
}
