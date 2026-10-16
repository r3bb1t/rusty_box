//! A 64-bit Windows kernel laid out in a [`FakeGuest`], deep enough for the
//! walks in `windows.rs` to find it the way they find a real one: a KPCR
//! whose IDT's vector 0 handler lies in the kernel image, the image's export
//! table, the System process and its neighbours on `ActiveProcessLinks`, and
//! the loaded-module list. Every image carries a debug directory whose
//! CodeView record names its PDB, on a page of its own so a test can page it
//! out.

use rusty_box::cpu::{CpuDebugView, CpuSetupMode};
use rusty_box::emulator::{Emulator, EmulatorConfig};

use super::guest::fake::FakeGuest;

/// The live processor's address space; the System process runs in it.
pub(crate) const LIVE_CR3: u64 = 0x1000;
pub(crate) const KPCR: u64 = 0xFFFF_F800_0200_0000;
const IDT: u64 = 0xFFFF_F800_0201_0000;
pub(crate) const KERNEL_BASE: u64 = 0xFFFF_F800_0100_0000;
const EPROCESS_BASE: u64 = 0xFFFF_FA80_0000_0000;
const LDR_ENTRY_BASE: u64 = 0xFFFF_FA80_0100_0000;

/// Where each image keeps what the walks read, as RVAs.
pub(crate) const EXPORTS_RVA: u64 = 0x1000;
const HANDLER_RVA: u64 = 0x2000;
const DATA_RVA: u64 = 0x3000;
pub(crate) const DEBUG_DIRECTORY_RVA: u64 = 0x4000;
const CODEVIEW_RVA: u64 = 0x4040;
/// The kernel variables in its data page.
pub(crate) const ACTIVE_PROCESS_HEAD_RVA: u64 = DATA_RVA;
pub(crate) const INITIAL_SYSTEM_PROCESS_RVA: u64 = DATA_RVA + 0x10;
pub(crate) const LOADED_MODULE_LIST_RVA: u64 = DATA_RVA + 0x20;
pub(crate) const NT_BUILD_NUMBER_RVA: u64 = DATA_RVA + 0x30;
/// `NtBuildNumber`'s high nibble on a free (retail) build.
const FREE_BUILD: u32 = 0xF000_0000;

/// The PDB an image's CodeView record names.
#[derive(Clone, Debug)]
pub(crate) struct FakePdb {
    pub(crate) guid: [u8; 16],
    pub(crate) age: u32,
    pub(crate) path: &'static str,
}

/// One loaded image.
#[derive(Clone, Debug)]
pub(crate) struct FakeImage {
    pub(crate) name: &'static str,
    pub(crate) base: u64,
    pub(crate) size: u32,
    pub(crate) time_date_stamp: u32,
    pub(crate) pdb: FakePdb,
}

/// One process on `ActiveProcessLinks`.
#[derive(Clone, Debug)]
pub(crate) struct FakeProcess {
    pub(crate) pid: u64,
    pub(crate) name: &'static str,
    pub(crate) cr3: u64,
}

/// Where this build keeps the `EPROCESS` fields that move between builds;
/// `ActiveProcessLinks` follows `UniqueProcessId`, as in every x64 build.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EprocessFields {
    pub(crate) unique_process_id: u64,
    pub(crate) image_file_name: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct FakeWindowsSpec {
    pub(crate) kernel: FakeImage,
    pub(crate) drivers: Vec<FakeImage>,
    /// The first is the System process.
    pub(crate) processes: Vec<FakeProcess>,
    pub(crate) eprocess: EprocessFields,
    /// The kernel's `NtBuildNumber`, exported when `Some`.
    pub(crate) build_number: Option<u32>,
}

impl FakeWindowsSpec {
    /// A kernel with one driver and three processes, its `EPROCESS` laid out
    /// as Windows 10 1903 through 22H2 lay it out.
    pub(crate) fn typical() -> FakeWindowsSpec {
        FakeWindowsSpec {
            kernel: FakeImage {
                name: "ntoskrnl.exe",
                base: KERNEL_BASE,
                size: 0x10_0000,
                time_date_stamp: 0x5E1D_2A4B,
                pdb: FakePdb {
                    guid: [
                        0xFE, 0xC6, 0x61, 0xB6, 0x99, 0x72, 0x16, 0x1D, 0x0F, 0xE3, 0x7C, 0x48,
                        0xB8, 0xBC, 0x54, 0x70,
                    ],
                    age: 1,
                    path: "ntkrnlmp.pdb",
                },
            },
            drivers: vec![FakeImage {
                name: "disk.sys",
                base: 0xFFFF_F800_0300_0000,
                size: 0x2_0000,
                time_date_stamp: 0x1234_5678,
                pdb: FakePdb {
                    guid: [0x11; 16],
                    age: 2,
                    path: "d:\\build\\disk.pdb",
                },
            }],
            processes: vec![
                FakeProcess { pid: 4, name: "System", cr3: LIVE_CR3 },
                FakeProcess { pid: 0x1F0, name: "smss.exe", cr3: 0x2_0000 },
                FakeProcess { pid: 0x26C, name: "csrss.exe", cr3: 0x3_0000 },
            ],
            eprocess: EprocessFields {
                unique_process_id: 0x440,
                image_file_name: 0x5A8,
            },
            // Windows 10 22H2.
            build_number: Some(19045),
        }
    }
}

/// A guest holding the kernel `spec` describes, and a processor view paused
/// in its kernel.
pub(crate) struct FakeWindows {
    pub(crate) guest: FakeGuest,
    pub(crate) view: CpuDebugView,
}

impl FakeWindows {
    pub(crate) fn build(spec: FakeWindowsSpec) -> FakeWindows {
        let mut guest = FakeGuest::new(LIVE_CR3);
        let mut kernel_exports = vec![
            FakeExport { name: "PsActiveProcessHead", rva: ACTIVE_PROCESS_HEAD_RVA },
            FakeExport { name: "PsInitialSystemProcess", rva: INITIAL_SYSTEM_PROCESS_RVA },
            FakeExport { name: "PsLoadedModuleList", rva: LOADED_MODULE_LIST_RVA },
        ];
        if spec.build_number.is_some() {
            kernel_exports.push(FakeExport { name: "NtBuildNumber", rva: NT_BUILD_NUMBER_RVA });
        }
        write_image(&mut guest, &spec.kernel, &kernel_exports);
        if let Some(build) = spec.build_number {
            guest.write_u32(LIVE_CR3, spec.kernel.base + NT_BUILD_NUMBER_RVA, FREE_BUILD | build);
        }
        for driver in &spec.drivers {
            write_image(&mut guest, driver, &[]);
        }
        write_kpcr(&mut guest, spec.kernel.base + HANDLER_RVA);
        write_processes(&mut guest, &spec);
        let images: Vec<&FakeImage> =
            core::iter::once(&spec.kernel).chain(spec.drivers.iter()).collect();
        write_module_list(&mut guest, spec.kernel.base + LOADED_MODULE_LIST_RVA, &images);
        FakeWindows {
            guest,
            view: kernel_view(),
        }
    }

    /// The `EPROCESS` address of the `index`th process of the spec.
    pub(crate) fn eprocess(&self, index: usize) -> u64 {
        EPROCESS_BASE + index as u64 * 0x1000
    }
}

/// A processor in 64-bit kernel mode whose GS base is the fake KPCR.
fn kernel_view() -> CpuDebugView {
    let mut machine = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
        .expect("a long-mode machine");
    let mut view = machine
        .debug_cpu(0)
        .expect("the boot processor")
        .view();
    view.segment_bases.gs = KPCR;
    view.kernel_gs_base = 0;
    view.registers.cr3 = LIVE_CR3;
    view.registers.cpl = 0;
    view
}

struct FakeExport {
    name: &'static str,
    rva: u64,
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// An RSDS CodeView record.
pub(crate) fn rsds(pdb: &FakePdb) -> Vec<u8> {
    let mut record = b"RSDS".to_vec();
    record.extend_from_slice(&pdb.guid);
    record.extend_from_slice(&pdb.age.to_le_bytes());
    record.extend_from_slice(pdb.path.as_bytes());
    record.push(0);
    record
}

/// The headers, export table and debug directory of `image`.
fn write_image(guest: &mut FakeGuest, image: &FakeImage, exports: &[FakeExport]) {
    let base = image.base;
    let cr3 = LIVE_CR3;
    guest.write_u16(cr3, base, 0x5A4D);
    guest.write_u32(cr3, base + 0x3C, 0x80);
    let nt = base + 0x80;
    guest.write(cr3, nt, b"PE\0\0");
    guest.write_u16(cr3, nt + 4, 0x8664);
    guest.write_u16(cr3, nt + 6, 0);
    guest.write_u32(cr3, nt + 8, image.time_date_stamp);
    guest.write_u16(cr3, nt + 20, 0xF0);
    let optional = nt + 24;
    guest.write_u16(cr3, optional, 0x20B);
    guest.write_u32(cr3, optional + 56, image.size);
    guest.write_u32(cr3, optional + 60, 0x1000);
    guest.write_u32(cr3, optional + 108, 16);
    let directories = optional + 112;
    if !exports.is_empty() {
        guest.write_u32(cr3, directories, EXPORTS_RVA as u32);
        guest.write_u32(cr3, directories + 4, 0x800);
        write_exports(guest, image, exports);
    }
    let codeview = rsds(&image.pdb);
    guest.write_u32(cr3, directories + 6 * 8, DEBUG_DIRECTORY_RVA as u32);
    guest.write_u32(cr3, directories + 6 * 8 + 4, 28);
    let entry = base + DEBUG_DIRECTORY_RVA;
    guest.write_u32(cr3, entry + 12, 2);
    guest.write_u32(cr3, entry + 16, codeview.len() as u32);
    guest.write_u32(cr3, entry + 20, CODEVIEW_RVA as u32);
    guest.write(cr3, base + CODEVIEW_RVA, &codeview);
}

/// An export directory naming `exports`, sorted as the PE format requires.
fn write_exports(guest: &mut FakeGuest, image: &FakeImage, exports: &[FakeExport]) {
    let cr3 = LIVE_CR3;
    let mut sorted: Vec<&FakeExport> = exports.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(b.name));
    let directory = image.base + EXPORTS_RVA;
    let functions = EXPORTS_RVA + 0x100;
    let names = EXPORTS_RVA + 0x200;
    let ordinals = EXPORTS_RVA + 0x300;
    let mut strings = EXPORTS_RVA + 0x400;
    guest.write_u32(cr3, directory + 0x0C, strings as u32);
    guest.write(cr3, image.base + strings, image.name.as_bytes());
    strings += image.name.len() as u64 + 1;
    guest.write_u32(cr3, directory + 0x10, 1);
    guest.write_u32(cr3, directory + 0x14, sorted.len() as u32);
    guest.write_u32(cr3, directory + 0x18, sorted.len() as u32);
    guest.write_u32(cr3, directory + 0x1C, functions as u32);
    guest.write_u32(cr3, directory + 0x20, names as u32);
    guest.write_u32(cr3, directory + 0x24, ordinals as u32);
    for (index, export) in sorted.iter().enumerate() {
        let index = index as u64;
        guest.write_u32(cr3, image.base + functions + 4 * index, export.rva as u32);
        guest.write_u32(cr3, image.base + names + 4 * index, strings as u32);
        guest.write_u16(cr3, image.base + ordinals + 2 * index, index as u16);
        guest.write(cr3, image.base + strings, export.name.as_bytes());
        strings += export.name.len() as u64 + 1;
    }
}

/// A KPCR that points at itself, and an IDT whose vector 0 gate leads to
/// `handler`.
fn write_kpcr(guest: &mut FakeGuest, handler: u64) {
    let cr3 = LIVE_CR3;
    guest.write_u64(cr3, KPCR + 0x18, KPCR);
    guest.write_u64(cr3, KPCR + 0x38, IDT);
    guest.write_u16(cr3, IDT, handler as u16);
    guest.write_u16(cr3, IDT + 2, 0x10);
    guest.write_u16(cr3, IDT + 4, 0x8E00);
    guest.write_u16(cr3, IDT + 6, (handler >> 16) as u16);
    guest.write_u32(cr3, IDT + 8, (handler >> 32) as u32);
}

/// Each process's `EPROCESS`, ringed on `ActiveProcessLinks` through the
/// kernel's `PsActiveProcessHead`, and `PsInitialSystemProcess`.
fn write_processes(guest: &mut FakeGuest, spec: &FakeWindowsSpec) {
    let cr3 = LIVE_CR3;
    let fields = spec.eprocess;
    let links_of = |index: usize| {
        EPROCESS_BASE + index as u64 * 0x1000 + fields.unique_process_id + 8
    };
    let head = spec.kernel.base + ACTIVE_PROCESS_HEAD_RVA;
    let mut ring = vec![head];
    ring.extend((0..spec.processes.len()).map(links_of));
    for (at, &node) in ring.iter().enumerate() {
        let next = ring[(at + 1) % ring.len()];
        let previous = ring[(at + ring.len() - 1) % ring.len()];
        guest.write_u64(cr3, node, next);
        guest.write_u64(cr3, node + 8, previous);
    }
    for (index, process) in spec.processes.iter().enumerate() {
        let eprocess = EPROCESS_BASE + index as u64 * 0x1000;
        guest.write_u64(cr3, eprocess + 0x28, process.cr3);
        guest.write_u64(cr3, eprocess + fields.unique_process_id, process.pid);
        let mut name = process.name.as_bytes().to_vec();
        name.resize(15, 0);
        guest.write(cr3, eprocess + fields.image_file_name, &name);
    }
    guest.write_u64(
        cr3,
        spec.kernel.base + INITIAL_SYSTEM_PROCESS_RVA,
        EPROCESS_BASE,
    );
}

/// A `KLDR_DATA_TABLE_ENTRY` per image, ringed through `head`.
fn write_module_list(guest: &mut FakeGuest, head: u64, images: &[&FakeImage]) {
    let cr3 = LIVE_CR3;
    let entry_of = |index: usize| LDR_ENTRY_BASE + index as u64 * 0x1000;
    let mut ring = vec![head];
    ring.extend((0..images.len()).map(entry_of));
    for (at, &node) in ring.iter().enumerate() {
        let next = ring[(at + 1) % ring.len()];
        let previous = ring[(at + ring.len() - 1) % ring.len()];
        guest.write_u64(cr3, node, next);
        guest.write_u64(cr3, node + 8, previous);
    }
    for (index, image) in images.iter().enumerate() {
        let entry = entry_of(index);
        guest.write_u64(cr3, entry + 0x30, image.base);
        guest.write_u32(cr3, entry + 0x40, image.size);
        let full = NameField {
            at: entry + 0x48,
            text: utf16(&format!("\\SystemRoot\\system32\\{}", image.name)),
            buffer: entry + 0x300,
        };
        let base = NameField {
            at: entry + 0x58,
            text: utf16(image.name),
            buffer: entry + 0x100,
        };
        for field in [full, base] {
            guest.write_u16(cr3, field.at, field.text.len() as u16);
            guest.write_u16(cr3, field.at + 2, field.text.len() as u16);
            guest.write_u64(cr3, field.at + 8, field.buffer);
            guest.write(cr3, field.buffer, &field.text);
        }
    }
}

/// A `UNICODE_STRING` at `at` whose characters lie at `buffer`.
struct NameField {
    at: u64,
    text: Vec<u8>,
    buffer: u64,
}
