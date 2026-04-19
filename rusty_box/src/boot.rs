//! No-alloc direct Linux kernel boot.
//!
//! Loads a bzImage kernel and optional initramfs into guest memory,
//! sets up the Linux boot protocol zero page, and configures the CPU
//! for 32-bit protected mode entry. No heap allocation required.
//!
//! This is the no-alloc equivalent of `Emulator::setup_direct_linux_boot`.

use crate::config::BxPhyAddress;
use crate::cpu::{cpu::BxCpuC, cpuid::BxCpuIdTrait, instrumentation::Instrumentation};
use crate::memory::BxMemC;

/// Error from boot setup (no alloc — uses static strings).
#[derive(Debug)]
pub enum BootError {
    BzImageTooSmall,
    InvalidBootSignature,
    InvalidHeaderMagic,
    BootProtocolTooOld,
    MemoryLoadFailed,
}

/// Set up direct Linux kernel boot, bypassing BIOS entirely.
///
/// Loads a bzImage kernel and optional initramfs into memory, sets up
/// the Linux boot protocol "zero page" (boot_params), configures CPU
/// for 32-bit protected mode, and points EIP at the kernel entry.
///
/// No heap allocation. All temporary buffers are on the stack.
///
/// # Arguments
/// * `cpu` - CPU to configure for protected mode
/// * `memory` - Guest memory to load kernel and boot params into
/// * `bzimage` - Raw bzImage kernel file contents
/// * `initramfs` - Optional initramfs/initrd file contents
/// * `cmdline` - Kernel command line (ASCII, max 2047 bytes)
/// * `ram_size` - Total guest RAM in bytes
pub fn setup_direct_linux_boot<I: BxCpuIdTrait, T: Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    memory: &mut BxMemC<'_>,
    bzimage: &[u8],
    initramfs: Option<&[u8]>,
    cmdline: &[u8],
    ram_size: u64,
) -> Result<(), BootError> {
    // Validate bzImage header
    if bzimage.len() < 0x264 {
        return Err(BootError::BzImageTooSmall);
    }
    if bzimage[0x1FE] != 0x55 || bzimage[0x1FF] != 0xAA {
        return Err(BootError::InvalidBootSignature);
    }
    let header_magic = u32::from_le_bytes([
        bzimage[0x202], bzimage[0x203], bzimage[0x204], bzimage[0x205],
    ]);
    if header_magic != 0x53726448 {
        return Err(BootError::InvalidHeaderMagic);
    }
    let boot_version = u16::from_le_bytes([bzimage[0x206], bzimage[0x207]]);
    if boot_version < 0x0204 {
        return Err(BootError::BootProtocolTooOld);
    }

    // Parse bzImage header
    let setup_sects = if bzimage[0x1F1] == 0 { 4 } else { bzimage[0x1F1] as usize };
    let setup_size = (setup_sects + 1) * 512;
    let pm_kernel = &bzimage[setup_size..];

    let code32_start = u32::from_le_bytes([
        bzimage[0x214], bzimage[0x215], bzimage[0x216], bzimage[0x217],
    ]);

    // Write GDT at 0x1000
    const GDT_ADDR: u64 = 0x1000;
    let gdt: [u64; 4] = [
        0x0000000000000000, // null
        0x0000000000000000, // reserved
        0x00CF9A000000FFFF, // 32-bit code
        0x00CF92000000FFFF, // 32-bit data
    ];
    let mut gdt_bytes = [0u8; 32];
    for (i, &entry) in gdt.iter().enumerate() {
        gdt_bytes[i * 8..(i + 1) * 8].copy_from_slice(&entry.to_le_bytes());
    }
    memory.load_RAM(&gdt_bytes, GDT_ADDR).map_err(|_| BootError::MemoryLoadFailed)?;

    // Write boot_params (zero page)
    let boot_params_addr: u64 = 0x10000;
    let cmdline_addr: u64 = 0x20000;
    let mut boot_params = [0u8; 4096];

    // Copy setup header from bzImage (offsets 0x1F1 to 0x268)
    let hdr_start = 0x1F1;
    let hdr_end = core::cmp::min(0x268, bzimage.len());
    boot_params[hdr_start..hdr_end].copy_from_slice(&bzimage[hdr_start..hdr_end]);

    boot_params[0x210] = 0xFF; // type_of_loader = unknown
    boot_params[0x211] |= 0x01; // LOADED_HIGH
    boot_params[0x228..0x22C].copy_from_slice(&(cmdline_addr as u32).to_le_bytes());
    boot_params[0x224..0x226].copy_from_slice(&0xFE00u16.to_le_bytes());

    // screen_info
    boot_params[0x06] = 0x03; // mode 3 (80x25)
    boot_params[0x07] = 80;
    boot_params[0x0E] = 25;
    boot_params[0x0F] = 0x01; // VGA
    boot_params[0x10..0x12].copy_from_slice(&16u16.to_le_bytes());
    boot_params[0x1FA..0x1FC].copy_from_slice(&0xFFFFu16.to_le_bytes());

    // acpi_rsdp_addr
    boot_params[0x070..0x078].copy_from_slice(&0x40000u64.to_le_bytes());

    // Set up initramfs
    if let Some(initrd_data) = initramfs {
        let initrd_addr_max = if boot_version >= 0x0203 {
            u32::from_le_bytes([
                bzimage[0x22C], bzimage[0x22D], bzimage[0x22E], bzimage[0x22F],
            ]) as u64
        } else {
            0x37FFFFFF
        };
        let max_addr = core::cmp::min(ram_size, initrd_addr_max + 1);
        let initrd_load_addr = (max_addr - initrd_data.len() as u64) & !0xFFF;

        memory.load_RAM(initrd_data, initrd_load_addr).map_err(|_| BootError::MemoryLoadFailed)?;

        boot_params[0x218..0x21C].copy_from_slice(&(initrd_load_addr as u32).to_le_bytes());
        boot_params[0x21C..0x220].copy_from_slice(&(initrd_data.len() as u32).to_le_bytes());
    }

    // E820 memory map
    let e820_base = 0x2D0;
    let mut e820_idx = 0usize;
    let mut write_e820 = |bp: &mut [u8], addr: u64, size: u64, etype: u32| {
        let off = e820_base + e820_idx * 20;
        bp[off..off + 8].copy_from_slice(&addr.to_le_bytes());
        bp[off + 8..off + 16].copy_from_slice(&size.to_le_bytes());
        bp[off + 16..off + 20].copy_from_slice(&etype.to_le_bytes());
        e820_idx += 1;
    };
    write_e820(&mut boot_params, 0, 0x9FC00, 1);
    write_e820(&mut boot_params, 0x9FC00, 0x400, 2);
    write_e820(&mut boot_params, 0xF0000, 0x10000, 2);
    if ram_size > 0x100000 {
        write_e820(&mut boot_params, 0x100000, ram_size - 0x100000, 1);
    }
    boot_params[0x1E8] = e820_idx as u8;

    memory.load_RAM(&boot_params, boot_params_addr).map_err(|_| BootError::MemoryLoadFailed)?;

    // Write command line (stack buffer, max 2048 bytes)
    let mut cmdline_buf = [0u8; 2048];
    let cmdline_len = core::cmp::min(cmdline.len(), 2047);
    cmdline_buf[..cmdline_len].copy_from_slice(&cmdline[..cmdline_len]);
    memory.load_RAM(&cmdline_buf[..cmdline_len + 1], cmdline_addr)
        .map_err(|_| BootError::MemoryLoadFailed)?;

    // ACPI tables (all stack-allocated)
    write_acpi_tables(memory)?;

    // Load protected-mode kernel
    memory.load_RAM(pm_kernel, code32_start as u64).map_err(|_| BootError::MemoryLoadFailed)?;

    // Configure CPU for protected mode
    cpu.setup_for_direct_boot(GDT_ADDR);
    cpu.set_rip(code32_start as u64);
    cpu.set_rsp(0x20000);
    cpu.set_rsi(boot_params_addr);

    Ok(())
}

/// Write minimal ACPI tables (RSDP → XSDT → MADT) to guest memory.
/// All buffers are stack-allocated.
fn write_acpi_tables(memory: &mut BxMemC<'_>) -> Result<(), BootError> {
    const RSDP_ADDR: u64 = 0x40000;
    const XSDT_ADDR: u64 = 0x40100;
    const MADT_ADDR: u64 = 0x40200;

    // MADT: 44 header + 8 (local APIC) + 12 (IO APIC) + 10 (ISO) = 74
    const MADT_LEN: usize = 74;
    let mut madt = [0u8; MADT_LEN];
    madt[0..4].copy_from_slice(b"APIC");
    madt[4..8].copy_from_slice(&(MADT_LEN as u32).to_le_bytes());
    madt[8] = 3;
    madt[10..16].copy_from_slice(b"RUSTYB");
    madt[16..24].copy_from_slice(b"BXMADT  ");
    madt[24..28].copy_from_slice(&1u32.to_le_bytes());
    madt[28..32].copy_from_slice(b"RBOX");
    madt[32..36].copy_from_slice(&1u32.to_le_bytes());
    madt[36..40].copy_from_slice(&0xFEE00000u32.to_le_bytes());
    madt[40..44].copy_from_slice(&1u32.to_le_bytes());

    // Local APIC entry
    let e = 44;
    madt[e] = 0; madt[e + 1] = 8; madt[e + 2] = 0; madt[e + 3] = 0;
    madt[e + 4..e + 8].copy_from_slice(&1u32.to_le_bytes());

    // I/O APIC entry
    let e = 52;
    madt[e] = 1; madt[e + 1] = 12; madt[e + 2] = 1; madt[e + 3] = 0;
    madt[e + 4..e + 8].copy_from_slice(&0xFEC00000u32.to_le_bytes());
    madt[e + 8..e + 12].copy_from_slice(&0u32.to_le_bytes());

    // Interrupt Source Override
    let e = 64;
    madt[e] = 2; madt[e + 1] = 10; madt[e + 2] = 0; madt[e + 3] = 0;
    madt[e + 4..e + 8].copy_from_slice(&2u32.to_le_bytes());
    madt[e + 8..e + 10].copy_from_slice(&0u16.to_le_bytes());

    let sum: u8 = madt.iter().fold(0u8, |a, &b| a.wrapping_add(b));
    madt[9] = 0u8.wrapping_sub(sum);
    memory.load_RAM(&madt, MADT_ADDR).map_err(|_| BootError::MemoryLoadFailed)?;

    // XSDT: 36 header + 8 pointer = 44
    let mut xsdt = [0u8; 44];
    xsdt[0..4].copy_from_slice(b"XSDT");
    xsdt[4..8].copy_from_slice(&44u32.to_le_bytes());
    xsdt[8] = 1;
    xsdt[10..16].copy_from_slice(b"RUSTYB");
    xsdt[16..24].copy_from_slice(b"BXXSDT  ");
    xsdt[24..28].copy_from_slice(&1u32.to_le_bytes());
    xsdt[28..32].copy_from_slice(b"RBOX");
    xsdt[32..36].copy_from_slice(&1u32.to_le_bytes());
    xsdt[36..44].copy_from_slice(&MADT_ADDR.to_le_bytes());
    let sum: u8 = xsdt.iter().fold(0u8, |a, &b| a.wrapping_add(b));
    xsdt[9] = 0u8.wrapping_sub(sum);
    memory.load_RAM(&xsdt, XSDT_ADDR).map_err(|_| BootError::MemoryLoadFailed)?;

    // RSDP v2.0 = 36 bytes
    let mut rsdp = [0u8; 36];
    rsdp[0..8].copy_from_slice(b"RSD PTR ");
    rsdp[9..15].copy_from_slice(b"RUSTYB");
    rsdp[15] = 2;
    rsdp[16..20].copy_from_slice(&(XSDT_ADDR as u32).to_le_bytes());
    rsdp[20..24].copy_from_slice(&36u32.to_le_bytes());
    rsdp[24..32].copy_from_slice(&XSDT_ADDR.to_le_bytes());
    let v1_sum: u8 = rsdp[0..20].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    rsdp[8] = 0u8.wrapping_sub(v1_sum);
    let v2_sum: u8 = rsdp.iter().fold(0u8, |a, &b| a.wrapping_add(b));
    rsdp[32] = 0u8.wrapping_sub(v2_sum);
    memory.load_RAM(&rsdp, RSDP_ADDR).map_err(|_| BootError::MemoryLoadFailed)?;

    Ok(())
}

/// Parse an ISO 9660 image and find a file by path components.
///
/// Returns `(offset, length)` of the file within the ISO data, or `None`.
/// No heap allocation — works with `&[u8]` slices.
///
/// # Example
/// ```ignore
/// let (off, len) = iso9660_find(iso_data, &["BOOT", "VMLINUZ"]).unwrap();
/// let vmlinuz = &iso_data[off..off + len];
/// ```
pub fn iso9660_find(iso_data: &[u8], path: &[&str]) -> Option<(usize, usize)> {
    let pvd_offset = 16 * 2048;
    if iso_data.len() < pvd_offset + 2048 {
        return None;
    }
    let pvd = &iso_data[pvd_offset..pvd_offset + 2048];
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return None;
    }

    let root_record = &pvd[156..156 + 34];
    let mut current_lba = u32::from_le_bytes([
        root_record[2], root_record[3], root_record[4], root_record[5],
    ]);
    let mut current_len = u32::from_le_bytes([
        root_record[10], root_record[11], root_record[12], root_record[13],
    ]);

    for (depth, &name) in path.iter().enumerate() {
        let is_file = depth == path.len() - 1;
        let dir_offset = current_lba as usize * 2048;
        if dir_offset + current_len as usize > iso_data.len() {
            return None;
        }
        let dir_data = &iso_data[dir_offset..dir_offset + current_len as usize];

        let mut pos = 0;
        let mut found = false;
        let name_upper = name.as_bytes();

        while pos < dir_data.len() {
            let record_len = dir_data[pos] as usize;
            if record_len == 0 {
                let next_sector = ((pos / 2048) + 1) * 2048;
                if next_sector >= dir_data.len() { break; }
                pos = next_sector;
                continue;
            }
            if pos + record_len > dir_data.len() { break; }

            let name_len = dir_data[pos + 32] as usize;
            if name_len > 0 && pos + 33 + name_len <= dir_data.len() {
                let entry_name = &dir_data[pos + 33..pos + 33 + name_len];
                // Case-insensitive prefix match (ISO 9660 uses uppercase + version suffix)
                if entry_name_matches(entry_name, name_upper) {
                    let entry_lba = u32::from_le_bytes([
                        dir_data[pos + 2], dir_data[pos + 3],
                        dir_data[pos + 4], dir_data[pos + 5],
                    ]);
                    let entry_len = u32::from_le_bytes([
                        dir_data[pos + 10], dir_data[pos + 11],
                        dir_data[pos + 12], dir_data[pos + 13],
                    ]);

                    if is_file {
                        let file_offset = entry_lba as usize * 2048;
                        if file_offset + entry_len as usize <= iso_data.len() {
                            return Some((file_offset, entry_len as usize));
                        }
                        return None;
                    } else {
                        current_lba = entry_lba;
                        current_len = entry_len;
                        found = true;
                        break;
                    }
                }
            }
            pos += record_len;
        }
        if !found && !is_file {
            return None;
        }
    }
    None
}

/// Case-insensitive prefix match for ISO 9660 directory entries.
/// Handles ";1" version suffix and trailing dots.
fn entry_name_matches(entry: &[u8], target: &[u8]) -> bool {
    // Strip ";1" version suffix from entry
    let entry_trimmed = if entry.len() >= 2 && entry[entry.len() - 2] == b';' {
        &entry[..entry.len() - 2]
    } else {
        entry
    };
    // Strip trailing dot
    let entry_trimmed = if entry_trimmed.last() == Some(&b'.') {
        &entry_trimmed[..entry_trimmed.len() - 1]
    } else {
        entry_trimmed
    };

    if entry_trimmed.len() < target.len() {
        return false;
    }
    // Compare prefix case-insensitively
    for (a, b) in entry_trimmed.iter().zip(target.iter()) {
        if a.to_ascii_uppercase() != b.to_ascii_uppercase() {
            return false;
        }
    }
    // Allow entry to be longer (e.g. "VMLINUZ_VIRT" matches "VMLINUZ")
    // But exact match or underscore/dot continuation
    entry_trimmed.len() == target.len()
        || entry_trimmed[target.len()] == b'_'
        || entry_trimmed[target.len()] == b'.'
}
