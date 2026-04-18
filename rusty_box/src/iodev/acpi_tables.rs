#![allow(dead_code)]
//! ACPI Table Generation for UEFI/OVMF Support
//!
//! Matches Bochs `iodev/acpi_tables.cc` (540 lines) + `acpi_tables.h` (290 lines).
//!
//! Generates ACPI tables that OVMF firmware reads via fw_cfg to configure the
//! system. Uses pre-compiled DSDT from Bochs BIOS (bios/acpi-dsdt.hex) which
//! includes PCI root bridge, interrupt routing, ISA devices, HPET, and sleep states.
//!
//! ## QEMU Table Loader Protocol
//!
//! The loader blob contains a sequence of 128-byte commands that tell OVMF how to:
//! - **ALLOCATE**: Reserve memory for table blobs in HIGH or FSEG zones
//! - **ADD_POINTER**: Patch offset fields to become physical addresses
//! - **ADD_CHECKSUM**: Recompute checksums after pointer patching

use alloc::vec::Vec;

// ─── QEMU Loader Constants ──────────────────────────────────────────

/// Each loader command is exactly 128 bytes (padded)
const LOADER_ENTRY_SIZE: usize = 128;

/// Maximum file name length in loader entries
const LOADER_FNAME_SIZE: usize = 56;

/// Loader command types (matches QEMU spec)
const LOADER_CMD_ALLOCATE: u32 = 1;
const LOADER_CMD_ADD_POINTER: u32 = 2;
const LOADER_CMD_ADD_CHECKSUM: u32 = 3;

/// Allocation zones
const ALLOC_HIGH: u8 = 1; // Above 1MB
const ALLOC_FSEG: u8 = 2; // 0xE0000-0xFFFFF (real-mode visible)

// ─── ACPI Table Constants ───────────────────────────────────────────

const OEM_ID: &[u8; 6] = b"BOCHS ";
const OEM_TABLE_ID: &[u8; 8] = b"BXPC    ";
const ASL_COMPILER_ID: &[u8; 4] = b"BXPC";

/// fw_cfg file names used by OVMF
const TABLES_FILE: &str = "etc/acpi/tables";
const RSDP_FILE: &str = "etc/acpi/rsdp";

// ─── Struct Sizes (packed, matching C++ pragma pack(1)) ─────────────

const HEADER_SIZE: usize = 36; // acpi_table_header
const FACS_SIZE: usize = 64;
const FADT_SIZE: usize = 276; // acpi_fadt (ACPI 5.0)
const HPET_SIZE: usize = 56; // acpi_hpet
const RSDP_SIZE: usize = 36; // acpi_rsdp (ACPI 2.0+)

// MADT entry sizes
const MADT_HEADER_SIZE: usize = 44; // header(36) + local_apic_address(4) + flags(4)
const MADT_LAPIC_SIZE: usize = 8;
const MADT_IOAPIC_SIZE: usize = 12;
const MADT_ISO_SIZE: usize = 10; // Interrupt Source Override
const MADT_LAPIC_NMI_SIZE: usize = 6;

// ─── Field Offsets within FADT ──────────────────────────────────────

const FADT_OFF_FIRMWARE_CTRL: usize = 36;
const FADT_OFF_DSDT: usize = 40;
const FADT_OFF_X_FIRMWARE_CTRL: usize = 132;
const FADT_OFF_X_DSDT: usize = 140;

// ─── Field Offsets within RSDP ──────────────────────────────────────

const RSDP_OFF_CHECKSUM: usize = 8;
const RSDP_OFF_XSDT_ADDRESS: usize = 24;
const RSDP_OFF_EXT_CHECKSUM: usize = 32;

// ─── Checksum offset in any table header ────────────────────────────

const HEADER_OFF_CHECKSUM: usize = 9;

// ─── Pre-compiled DSDT AML ─────────────────────────────────────────
//
// Generated from Bochs BIOS acpi-dsdt.dsl using iasl compiler.
// Contains PCI Root Bridge (PNP0A03), Interrupt Link Devices (LNKA-LNKD),
// ISA Bridge with RTC/keyboard/mouse, HPET device, S3/S4/S5 sleep states.
// 3782 bytes (0xEC6).

include!("acpi_dsdt_aml.inc");

// ─── Public API ─────────────────────────────────────────────────────

/// Generates ACPI tables for UEFI/OVMF firmware consumption via fw_cfg.
///
/// Produces three blobs:
/// - `tables_blob`: All ACPI tables concatenated (DSDT, FACS, FADT, MADT, HPET, XSDT)
/// - `rsdp_blob`: Root System Description Pointer (placed in FSEG)
/// - `loader_blob`: QEMU loader commands for ALLOCATE/ADD_POINTER/ADD_CHECKSUM
pub struct AcpiTableGenerator {
    tables_blob: Vec<u8>,
    rsdp_blob: Vec<u8>,
    loader_blob: Vec<u8>,
}

impl AcpiTableGenerator {
    /// Generate all ACPI tables for the given system configuration.
    ///
    /// `ram_size` is the total RAM in bytes (unused currently but reserved
    /// for future memory-map tables). `num_cpus` determines the number of
    /// Local APIC entries in the MADT.
    pub fn generate(_ram_size: u64, num_cpus: u32) -> Self {
        let mut tables = Vec::with_capacity(64 * 1024);
        let mut loader = LoaderBuilder::new();

        // 1. DSDT — pre-compiled AML
        let dsdt_offset = tables.len();
        tables.extend_from_slice(&DSDT_AML);
        align8(&mut tables);

        // 2. FACS
        let facs_offset = tables.len();
        build_facs(&mut tables);
        align8(&mut tables);

        // 3. FADT
        let fadt_offset = tables.len();
        let fadt_size = build_fadt(&mut tables, facs_offset as u32, dsdt_offset as u32);
        align8(&mut tables);

        // 4. MADT
        let madt_offset = tables.len();
        build_madt(&mut tables, num_cpus);
        align8(&mut tables);

        // 5. HPET
        let hpet_offset = tables.len();
        build_hpet(&mut tables);
        align8(&mut tables);

        // 6. XSDT (points to FADT, MADT, HPET)
        let xsdt_entries = [fadt_offset, madt_offset, hpet_offset];
        let xsdt_offset = tables.len();
        let xsdt_size = build_xsdt(&mut tables, &xsdt_entries);
        align8(&mut tables);

        // 7. RSDP (separate blob, goes in FSEG)
        let rsdp_blob = build_rsdp(xsdt_offset as u32);

        // 8. Loader commands

        // Allocate blobs
        loader.allocate(TABLES_FILE, 64, ALLOC_HIGH);
        loader.allocate(RSDP_FILE, 16, ALLOC_FSEG);

        // Patch FADT pointers to FACS and DSDT
        loader.add_pointer(TABLES_FILE, TABLES_FILE,
            (fadt_offset + FADT_OFF_FIRMWARE_CTRL) as u32, 4);
        loader.add_pointer(TABLES_FILE, TABLES_FILE,
            (fadt_offset + FADT_OFF_DSDT) as u32, 4);
        loader.add_pointer(TABLES_FILE, TABLES_FILE,
            (fadt_offset + FADT_OFF_X_FIRMWARE_CTRL) as u32, 8);
        loader.add_pointer(TABLES_FILE, TABLES_FILE,
            (fadt_offset + FADT_OFF_X_DSDT) as u32, 8);

        // Patch XSDT entries (64-bit pointers to each table)
        let xsdt_entries_base = xsdt_offset + HEADER_SIZE;
        for i in 0..xsdt_entries.len() {
            loader.add_pointer(TABLES_FILE, TABLES_FILE,
                (xsdt_entries_base + i * 8) as u32, 8);
        }

        // Patch RSDP.xsdt_address to point into tables blob
        loader.add_pointer(RSDP_FILE, TABLES_FILE,
            RSDP_OFF_XSDT_ADDRESS as u32, 8);

        // Checksums (must be last — computed after pointer patching)
        loader.add_checksum(TABLES_FILE,
            (fadt_offset + HEADER_OFF_CHECKSUM) as u32,
            fadt_offset as u32, fadt_size as u32);
        loader.add_checksum(TABLES_FILE,
            (xsdt_offset + HEADER_OFF_CHECKSUM) as u32,
            xsdt_offset as u32, xsdt_size as u32);
        // RSDP has two checksums: legacy (first 20 bytes) and extended (all 36 bytes)
        loader.add_checksum(RSDP_FILE,
            RSDP_OFF_CHECKSUM as u32, 0, 20);
        loader.add_checksum(RSDP_FILE,
            RSDP_OFF_EXT_CHECKSUM as u32, 0, 36);

        Self {
            tables_blob: tables,
            rsdp_blob,
            loader_blob: loader.finish(),
        }
    }

    pub fn tables_blob(&self) -> &[u8] {
        &self.tables_blob
    }

    pub fn rsdp_blob(&self) -> &[u8] {
        &self.rsdp_blob
    }

    pub fn loader_blob(&self) -> &[u8] {
        &self.loader_blob
    }
}

// ─── Table Builders ─────────────────────────────────────────────────

fn build_facs(buf: &mut Vec<u8>) {
    let start = buf.len();
    buf.resize(start + FACS_SIZE, 0);
    let facs = &mut buf[start..];

    facs[0..4].copy_from_slice(b"FACS");
    write_u32(facs, 4, 64); // length
    // hardware_signature, firmware_waking_vector, global_lock, flags: all 0
    // x_firmware_waking_vector: 0
    facs[32] = 2; // version (ACPI 2.0)
    // ospm_flags, reserved: all 0
}

fn build_fadt(buf: &mut Vec<u8>, facs_offset: u32, dsdt_offset: u32) -> usize {
    let start = buf.len();
    buf.resize(start + FADT_SIZE, 0);
    let fadt = &mut buf[start..];

    // Header
    write_header(fadt, b"FACP", FADT_SIZE as u32, 5);

    // FACS and DSDT pointers (offsets now, patched to physical addresses by loader)
    write_u32(fadt, 36, facs_offset); // firmware_ctrl
    write_u32(fadt, 40, dsdt_offset); // dsdt

    // reserved1 (was INT_MODEL): 0
    // preferred_pm_profile: 0 (Unspecified)

    // SCI interrupt
    write_u16(fadt, 46, 9); // sci_int = IRQ 9

    // SMI command port (PIIX4 ACPI)
    write_u32(fadt, 48, 0xB2); // smi_cmd
    fadt[52] = 0xF1; // acpi_enable
    fadt[53] = 0xF0; // acpi_disable
    // s4bios_req, pstate_cnt: 0

    // PM register blocks (PIIX4 at base 0xB000)
    write_u32(fadt, 56, 0xB000); // pm1a_evt_blk
    // pm1b_evt_blk: 0
    write_u32(fadt, 64, 0xB004); // pm1a_cnt_blk
    // pm1b_cnt_blk, pm2_cnt_blk: 0
    write_u32(fadt, 76, 0xB008); // pm_tmr_blk
    write_u32(fadt, 80, 0xB020); // gpe0_blk
    // gpe1_blk: 0
    fadt[88] = 4; // pm1_evt_len
    fadt[89] = 2; // pm1_cnt_len
    // pm2_cnt_len: 0
    fadt[91] = 4; // pm_tmr_len
    fadt[92] = 8; // gpe0_blk_len (8 bytes = 64 bits)
    // gpe1_blk_len, gpe1_base, cst_cnt: 0
    write_u16(fadt, 96, 0xFFFF); // p_lvl2_lat (not supported)
    write_u16(fadt, 98, 0xFFFF); // p_lvl3_lat (not supported)
    // flush_size, flush_stride, duty_offset, duty_width: 0
    // day_alrm, mon_alrm, century: 0

    // IA-PC boot architecture flags
    write_u16(fadt, 109, (1 << 0) | (1 << 1)); // Legacy devices + 8042

    // reserved2: 0

    // FADT flags — CRITICAL: bit 20 (HW_REDUCED_ACPI) must NOT be set for PIIX4
    let flags: u32 = (1 << 0)   // WBINVD supported
                   | (1 << 4)   // Proc C1 supported
                   | (1 << 5)   // P_LVL2_UP (uniprocessor only)
                   | (1 << 8)   // RTC_S4 supported
                   | (1 << 10); // 32-bit PM timer (TMR_VAL_EXT)
    write_u32(fadt, 112, flags);

    // reset_reg (GAS): all 0 (not used)
    // reset_value, arm_boot_arch, fadt_minor_version: 0

    // Extended pointers (ACPI 2.0+)
    write_u64(fadt, 132, facs_offset as u64); // x_firmware_ctrl
    write_u64(fadt, 140, dsdt_offset as u64); // x_dsdt

    // Extended PM register addresses (Generic Address Structures)
    // PM1a Event Block
    write_gas(fadt, 148, 1, 32, 0, 2, 0xB000); // SystemIO, 32-bit, word access
    // x_pm1b_evt_blk (160): 0 (not used)
    // PM1a Control Block
    write_gas(fadt, 172, 1, 16, 0, 2, 0xB004); // SystemIO, 16-bit, word access
    // x_pm1b_cnt_blk (184), x_pm2_cnt_blk (196): 0 (not used)
    // PM Timer Block
    write_gas(fadt, 208, 1, 32, 0, 3, 0xB008); // SystemIO, 32-bit, dword access
    // GPE0 Block
    write_gas(fadt, 220, 1, 64, 0, 1, 0xB020); // SystemIO, 64-bit, byte access

    // Checksum placeholder — loader will recompute after pointer patching
    // (leave at 0)

    FADT_SIZE
}

fn build_madt(buf: &mut Vec<u8>, num_cpus: u32) {
    let start = buf.len();

    // Reserve space for MADT header
    let total_size = MADT_HEADER_SIZE
        + num_cpus as usize * MADT_LAPIC_SIZE
        + MADT_IOAPIC_SIZE
        + MADT_ISO_SIZE * 2   // IRQ0→GSI2 + IRQ9 (SCI)
        + MADT_LAPIC_NMI_SIZE;

    buf.resize(start + total_size, 0);
    let madt = &mut buf[start..];

    // Header
    write_header(madt, b"APIC", total_size as u32, 4);

    // Local APIC address
    write_u32(madt, 36, 0xFEE0_0000);
    // Flags: PCAT_COMPAT (dual 8259 setup)
    write_u32(madt, 40, 1);

    let mut off = MADT_HEADER_SIZE;

    // Local APIC entries (one per CPU)
    for i in 0..num_cpus {
        madt[off] = 0;     // type: Local APIC
        madt[off + 1] = 8; // length
        madt[off + 2] = i as u8; // processor_id
        madt[off + 3] = i as u8; // apic_id
        write_u32(madt, off + 4, 1); // flags: enabled
        off += MADT_LAPIC_SIZE;
    }

    // I/O APIC
    madt[off] = 1;      // type: I/O APIC
    madt[off + 1] = 12; // length
    madt[off + 2] = num_cpus as u8; // io_apic_id (after CPUs)
    // reserved: 0
    write_u32(madt, off + 4, 0xFEC0_0000); // address
    write_u32(madt, off + 8, 0);           // global_irq_base
    off += MADT_IOAPIC_SIZE;

    // Interrupt Source Override: IRQ0 → GSI 2 (timer)
    madt[off] = 2;      // type: Interrupt Override
    madt[off + 1] = 10; // length
    madt[off + 2] = 0;  // bus: ISA
    madt[off + 3] = 0;  // source: IRQ0
    write_u32(madt, off + 4, 2); // global_irq: GSI 2
    write_u16(madt, off + 8, 0); // flags: conforms to bus spec
    off += MADT_ISO_SIZE;

    // Interrupt Source Override: IRQ9 (SCI) — level triggered, active low
    madt[off] = 2;      // type: Interrupt Override
    madt[off + 1] = 10; // length
    madt[off + 2] = 0;  // bus: ISA
    madt[off + 3] = 9;  // source: IRQ9
    write_u32(madt, off + 4, 9);      // global_irq: 9
    write_u16(madt, off + 8, 0x000D); // flags: level triggered, active low
    off += MADT_ISO_SIZE;

    // Local APIC NMI (LINT1, all processors)
    madt[off] = 4;      // type: Local APIC NMI
    madt[off + 1] = 6;  // length
    madt[off + 2] = 0xFF; // processor_id: all
    write_u16(madt, off + 3, 0); // flags: conforms
    madt[off + 5] = 1;  // lint: LINT1

    // Compute checksum (MADT is not patched by loader, so we do it here)
    let madt = &mut buf[start..start + total_size];
    let sum = acpi_checksum(madt);
    madt[HEADER_OFF_CHECKSUM] = sum;
}

fn build_hpet(buf: &mut Vec<u8>) {
    let start = buf.len();
    buf.resize(start + HPET_SIZE, 0);
    let hpet = &mut buf[start..];

    write_header(hpet, b"HPET", HPET_SIZE as u32, 1);

    // Event Timer Block ID (Intel 8086:A201)
    write_u32(hpet, 36, 0x8086_A201);

    // Base address (Generic Address Structure)
    // address_space=0 (memory), bit_width=64, bit_offset=0, access_size=0
    hpet[40] = 0;  // Memory space
    hpet[41] = 64; // Register bit width
    hpet[42] = 0;  // Bit offset
    hpet[43] = 0;  // Access size
    write_u64(hpet, 44, 0xFED0_0000); // 64-bit base address

    hpet[52] = 0;  // hpet_number (sequence)
    write_u16(hpet, 53, 100); // min_tick
    hpet[55] = 0;  // page_prot

    // Checksum (HPET is not patched by loader)
    let hpet = &mut buf[start..start + HPET_SIZE];
    let sum = acpi_checksum(hpet);
    hpet[HEADER_OFF_CHECKSUM] = sum;
}

/// Build XSDT with 64-bit entry pointers. Returns total size.
fn build_xsdt(buf: &mut Vec<u8>, table_offsets: &[usize]) -> usize {
    let total_size = HEADER_SIZE + table_offsets.len() * 8;
    let start = buf.len();
    buf.resize(start + total_size, 0);
    let xsdt = &mut buf[start..];

    write_header(xsdt, b"XSDT", total_size as u32, 1);

    // Write table offsets (will be patched to physical addresses by loader)
    for (i, &offset) in table_offsets.iter().enumerate() {
        write_u64(xsdt, HEADER_SIZE + i * 8, offset as u64);
    }

    // Checksum placeholder — loader will recompute after pointer patching

    total_size
}

fn build_rsdp(xsdt_offset: u32) -> Vec<u8> {
    let mut rsdp = vec![0u8; RSDP_SIZE];

    // Signature "RSD PTR " (8 bytes including trailing space)
    rsdp[0..8].copy_from_slice(b"RSD PTR ");
    // checksum: placeholder (byte 8)
    rsdp[9..15].copy_from_slice(OEM_ID); // oem_id
    rsdp[15] = 2; // revision: ACPI 2.0+
    // rsdt_address (bytes 16-19): 0 (not provided, XSDT only)
    write_u32(&mut rsdp, 20, 36); // length: 36 bytes (ACPI 2.0 RSDP)
    write_u64(&mut rsdp, 24, xsdt_offset as u64); // xsdt_address (patched by loader)
    // extended_checksum (byte 32): placeholder
    // reserved (bytes 33-35): 0

    rsdp
}

// ─── Loader Builder ─────────────────────────────────────────────────

struct LoaderBuilder {
    entries: Vec<u8>,
}

impl LoaderBuilder {
    fn new() -> Self {
        Self {
            entries: Vec::with_capacity(32 * LOADER_ENTRY_SIZE),
        }
    }

    fn allocate(&mut self, file: &str, alignment: u32, zone: u8) {
        let mut entry = [0u8; LOADER_ENTRY_SIZE];
        write_u32(&mut entry, 0, LOADER_CMD_ALLOCATE);
        write_fname(&mut entry, 4, file);
        write_u32(&mut entry, 4 + LOADER_FNAME_SIZE, alignment);
        entry[4 + LOADER_FNAME_SIZE + 4] = zone;
        self.entries.extend_from_slice(&entry);
    }

    fn add_pointer(&mut self, pointer_file: &str, pointee_file: &str,
                   pointer_offset: u32, pointer_size: u8) {
        let mut entry = [0u8; LOADER_ENTRY_SIZE];
        write_u32(&mut entry, 0, LOADER_CMD_ADD_POINTER);
        write_fname(&mut entry, 4, pointer_file);
        write_fname(&mut entry, 4 + LOADER_FNAME_SIZE, pointee_file);
        write_u32(&mut entry, 4 + LOADER_FNAME_SIZE * 2, pointer_offset);
        entry[4 + LOADER_FNAME_SIZE * 2 + 4] = pointer_size;
        self.entries.extend_from_slice(&entry);
    }

    fn add_checksum(&mut self, file: &str, result_offset: u32, start: u32, length: u32) {
        let mut entry = [0u8; LOADER_ENTRY_SIZE];
        write_u32(&mut entry, 0, LOADER_CMD_ADD_CHECKSUM);
        write_fname(&mut entry, 4, file);
        write_u32(&mut entry, 4 + LOADER_FNAME_SIZE, result_offset);
        write_u32(&mut entry, 4 + LOADER_FNAME_SIZE + 4, start);
        write_u32(&mut entry, 4 + LOADER_FNAME_SIZE + 8, length);
        self.entries.extend_from_slice(&entry);
    }

    fn finish(self) -> Vec<u8> {
        self.entries
    }
}

// ─── Helper Functions ───────────────────────────────────────────────

/// Write a standard ACPI table header (36 bytes).
fn write_header(buf: &mut [u8], signature: &[u8; 4], length: u32, revision: u8) {
    buf[0..4].copy_from_slice(signature);
    write_u32(buf, 4, length);
    buf[8] = revision;
    // checksum (byte 9): 0 placeholder
    buf[10..16].copy_from_slice(OEM_ID);
    buf[16..24].copy_from_slice(OEM_TABLE_ID);
    write_u32(buf, 24, 1); // oem_revision
    buf[28..32].copy_from_slice(ASL_COMPILER_ID);
    write_u32(buf, 32, 1); // asl_compiler_revision
}

/// Write a Generic Address Structure (12 bytes) at the given offset.
fn write_gas(buf: &mut [u8], off: usize, addr_space: u8, bit_width: u8,
             bit_offset: u8, access_size: u8, address: u64) {
    buf[off] = addr_space;
    buf[off + 1] = bit_width;
    buf[off + 2] = bit_offset;
    buf[off + 3] = access_size;
    write_u64(buf, off + 4, address);
}

/// Compute ACPI checksum: returns the byte that makes the sum of all bytes zero.
fn acpi_checksum(data: &[u8]) -> u8 {
    let sum: u8 = data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    0u8.wrapping_sub(sum)
}

/// Write a file name into a loader entry field (max LOADER_FNAME_SIZE - 1 chars).
fn write_fname(buf: &mut [u8], offset: usize, name: &str) {
    let bytes = name.as_bytes();
    let len = bytes.len().min(LOADER_FNAME_SIZE - 1);
    buf[offset..offset + len].copy_from_slice(&bytes[..len]);
}

/// Align buffer length up to 8-byte boundary.
fn align8(buf: &mut Vec<u8>) {
    let aligned = (buf.len() + 7) & !7;
    buf.resize(aligned, 0);
}

#[inline]
fn write_u16(buf: &mut [u8], offset: usize, val: u16) {
    buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
}

#[inline]
fn write_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
}

#[inline]
fn write_u64(buf: &mut [u8], offset: usize, val: u64) {
    buf[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}
