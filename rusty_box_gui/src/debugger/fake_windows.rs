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
pub(crate) const DISK_BASE: u64 = 0xFFFF_F800_0300_0000;

/// The kernel's code and read-only data, when the spec gives it services.
const KERNEL_TEXT_RVA: u64 = 0x5000;
const KERNEL_RDATA_RVA: u64 = 0x6000;
/// `KiSystemServiceRepeat`'s `lea r10, [KeServiceDescriptorTable]` and
/// `lea r11, [KeServiceDescriptorTableShadow]`.
const SERVICE_REPEAT_RVA: u64 = KERNEL_TEXT_RVA;
const KERNEL_ROUTINES_RVA: u64 = KERNEL_TEXT_RVA + 0x100;
/// How far apart the fake's service routines lie.
const ROUTINE_STRIDE: u64 = 0x20;
const KI_SERVICE_TABLE_RVA: u64 = KERNEL_RDATA_RVA;
/// The two descriptors, at the RVAs the fixture PDB names.
pub(crate) const SERVICE_DESCRIPTOR_RVA: u64 = DATA_RVA + 0x40;
pub(crate) const SHADOW_DESCRIPTOR_RVA: u64 = DATA_RVA + 0x80;
/// `win32k.sys`, in session space, and its service routines and table.
pub(crate) const WIN32K_BASE: u64 = 0xFFFF_F960_0000_0000;
const WIN32K_ROUTINES_RVA: u64 = 0x1000;
const W32P_SERVICE_TABLE_RVA: u64 = 0x2000;
/// A service routine's first bytes: `mov rax, rsp; ret`.
const ROUTINE_CODE: [u8; 4] = [0x48, 0x8B, 0xC4, 0xC3];
/// Where every user process keeps its PEB, its loader data and its images.
pub(crate) const EPROCESS_PEB: u64 = 0x550;
const PEB: u64 = 0x7FFD_E000;
const PEB_LDR: u64 = 0x7FFD_F000;
const USER_LDR_ENTRIES: u64 = 0x7FFD_0000;
const PROCESS_IMAGE_BASE: u64 = 0x4A00_0000;
pub(crate) const NTDLL_BASE: u64 = 0x7FF9_0000_0000;
pub(crate) const WIN32U_BASE: u64 = 0x7FF9_1000_0000;
/// Where `ntdll.dll` and `win32u.dll` keep their system-call stubs.
const STUBS_RVA: u64 = 0x2000;

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
    /// Its section table; none written when empty.
    pub(crate) sections: Vec<FakeSection>,
}

/// One section header.
#[derive(Clone, Debug)]
pub(crate) struct FakeSection {
    pub(crate) name: &'static str,
    pub(crate) rva: u32,
    pub(crate) size: u32,
    pub(crate) executable: bool,
}

/// The system services the spec gives the kernel and `win32k.sys`, by
/// index, as `ntdll.dll` and `win32u.dll` name them.
#[derive(Clone, Debug)]
pub(crate) struct FakeServices {
    pub(crate) kernel: Vec<&'static str>,
    pub(crate) gui: Vec<&'static str>,
    /// The process (an index into the spec's processes) whose session maps
    /// `win32k.sys` and its service table.
    pub(crate) session_process: usize,
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
    /// System services, with every process but System given a PEB and a
    /// loader list holding its image, `ntdll.dll` and `win32u.dll`.
    pub(crate) services: Option<FakeServices>,
}

impl FakeWindowsSpec {
    /// [`Self::typical`], with system services: a kernel `.text` holding
    /// `KiSystemServiceRepeat` and the kernel's routines, `KiServiceTable` in
    /// `.rdata`, both descriptors, a `win32k.sys` whose routines and
    /// `W32pServiceTable` only `csrss.exe`'s session maps, and user processes
    /// with `ntdll.dll` and `win32u.dll` stubs.
    pub(crate) fn with_services() -> FakeWindowsSpec {
        let mut spec = Self::typical();
        spec.kernel.sections = vec![
            FakeSection { name: ".text", rva: KERNEL_TEXT_RVA as u32, size: 0x1000, executable: true },
            FakeSection { name: ".rdata", rva: KERNEL_RDATA_RVA as u32, size: 0x1000, executable: false },
        ];
        spec.drivers.push(FakeImage {
            name: "win32k.sys",
            base: WIN32K_BASE,
            size: 0x10_0000,
            time_date_stamp: 0x4549_B07D,
            pdb: FakePdb { guid: [0x32; 16], age: 1, path: "win32k.pdb" },
            sections: Vec::new(),
        });
        spec.services = Some(FakeServices {
            kernel: vec!["NtClose", "NtCreateFile", "NtOpenProcess"],
            gui: vec!["NtUserGetMessage", "NtGdiBitBlt"],
            session_process: 2,
        });
        spec
    }

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
                sections: Vec::new(),
            },
            drivers: vec![FakeImage {
                name: "disk.sys",
                base: DISK_BASE,
                size: 0x2_0000,
                time_date_stamp: 0x1234_5678,
                pdb: FakePdb {
                    guid: [0x11; 16],
                    age: 2,
                    path: "d:\\build\\disk.pdb",
                },
                sections: Vec::new(),
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
            services: None,
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
        write_image(&mut guest, LIVE_CR3, &spec.kernel, &kernel_exports);
        if let Some(build) = spec.build_number {
            guest.write_u32(LIVE_CR3, spec.kernel.base + NT_BUILD_NUMBER_RVA, FREE_BUILD | build);
        }
        for driver in &spec.drivers {
            write_image(&mut guest, LIVE_CR3, driver, &[]);
        }
        write_kpcr(&mut guest, spec.kernel.base + HANDLER_RVA);
        write_processes(&mut guest, &spec);
        let images: Vec<&FakeImage> =
            core::iter::once(&spec.kernel).chain(spec.drivers.iter()).collect();
        write_module_list(&mut guest, LIVE_CR3, spec.kernel.base + LOADED_MODULE_LIST_RVA, LDR_ENTRY_BASE, &images);
        if let Some(services) = &spec.services {
            write_services(&mut guest, &spec, services);
            write_user_processes(&mut guest, &spec, services);
        }
        FakeWindows {
            guest,
            view: kernel_view(),
        }
    }

    /// The `EPROCESS` address of the `index`th process of the spec.
    pub(crate) fn eprocess(&self, index: usize) -> u64 {
        EPROCESS_BASE + index as u64 * 0x1000
    }

    /// Kernel service `index`'s routine.
    pub(crate) fn kernel_routine(index: u32) -> u64 {
        KERNEL_BASE + KERNEL_ROUTINES_RVA + u64::from(index) * ROUTINE_STRIDE
    }

    /// GUI service `index`'s routine.
    pub(crate) fn gui_routine(index: u32) -> u64 {
        WIN32K_BASE + WIN32K_ROUTINES_RVA + u64::from(index) * ROUTINE_STRIDE
    }

    /// Points kernel service `index` at `routine`: a hooked entry.
    pub(crate) fn set_kernel_entry(&mut self, index: u32, routine: u64) {
        let table = KERNEL_BASE + KI_SERVICE_TABLE_RVA;
        self.guest.write_u32(LIVE_CR3, table + 4 * u64::from(index), shift_entry(table, routine));
    }

    /// Writes `code` over kernel service `index`'s routine: an inline hook.
    pub(crate) fn patch_kernel_routine(&mut self, index: u32, code: &[u8]) {
        self.guest.write(LIVE_CR3, Self::kernel_routine(index), code);
    }

    /// Pauses the live processor in user mode, at the image base of the
    /// process whose space is `process_cr3`, on that process's KVA-shadow
    /// tables: its user half, and of the kernel only the KPRCB page the
    /// entry stubs read, whose `KernelDirectoryTableBase` holds
    /// `kernel_directory_table_base`.
    pub(crate) fn pause_in_user_mode_under_kva_shadow(&mut self, process_cr3: u64, kernel_directory_table_base: u64) {
        self.guest.alias_user_half(process_cr3, USER_SHADOW_CR3);
        self.guest.isolate(USER_SHADOW_CR3);
        self.guest.write_session(
            USER_SHADOW_CR3,
            KPCR + KPRCB_SHADOW_OFFSET,
            &kernel_directory_table_base.to_le_bytes(),
        );
        self.guest.set_live_cr3(USER_SHADOW_CR3);
        self.view.registers.cr3 = USER_SHADOW_CR3;
        self.view.registers.cpl = 3;
        self.view.registers.rip = PROCESS_IMAGE_BASE;
        self.view.segment_bases.gs = USER_TEB;
        self.view.kernel_gs_base = KPCR;
    }
}

/// A process's KVA-shadow user tables, apart from its kernel ones.
pub(crate) const USER_SHADOW_CR3: u64 = 0x9_0000;
/// Where the fake's KPRCB `KernelDirectoryTableBase` lies from the KPCR, on
/// a page of its own; a real kernel's offset differs by build, which is why
/// the walker searches for it.
pub(crate) const KPRCB_SHADOW_OFFSET: u64 = 0x9000;
/// A user-mode GS base: the running thread's TEB.
const USER_TEB: u64 = 0x7FFD_A000;

/// A `KiServiceTable` entry for `routine`, in the Windows 7+ encoding: the
/// offset from the table shifted left by 4, an argument count of 2 below.
fn shift_entry(table: u64, routine: u64) -> u32 {
    ((routine.wrapping_sub(table) as i64) << 4) as u32 | 2
}

/// The ModRM bytes of `lea r10, [rip+disp32]` and `lea r11, [rip+disp32]`.
const LEA_R10: u8 = 0x15;
const LEA_R11: u8 = 0x1D;

/// `lea r10/r11, [target]` at `at`, RIP-relative from its end.
fn write_lea_rip(guest: &mut FakeGuest, at: u64, modrm: u8, target: u64) {
    guest.write(LIVE_CR3, at, &[0x4C, 0x8D, modrm]);
    guest.write_u32(LIVE_CR3, at + 3, target.wrapping_sub(at + 7) as u32);
}

/// A `KSERVICE_TABLE_DESCRIPTOR` at `at` for `count` services at `table`.
fn write_descriptor(guest: &mut FakeGuest, at: u64, table: u64, count: usize) {
    guest.write_u64(LIVE_CR3, at, table);
    guest.write_u32(LIVE_CR3, at + 0x10, count as u32);
}

/// The kernel's services and `KiSystemServiceRepeat`, both descriptors, and
/// `win32k.sys`'s routines and table in the session process's space.
fn write_services(guest: &mut FakeGuest, spec: &FakeWindowsSpec, services: &FakeServices) {
    let base = spec.kernel.base;
    let table = base + KI_SERVICE_TABLE_RVA;
    for index in 0..services.kernel.len() {
        let routine = FakeWindows::kernel_routine(index as u32);
        guest.write(LIVE_CR3, routine, &ROUTINE_CODE);
        guest.write_u32(LIVE_CR3, table + 4 * index as u64, shift_entry(table, routine));
    }
    let descriptor = base + SERVICE_DESCRIPTOR_RVA;
    let shadow = base + SHADOW_DESCRIPTOR_RVA;
    write_descriptor(guest, descriptor, table, services.kernel.len());
    write_descriptor(guest, shadow, table, services.kernel.len());
    let gui_table = WIN32K_BASE + W32P_SERVICE_TABLE_RVA;
    write_descriptor(guest, shadow + 0x20, gui_table, services.gui.len());
    // KiSystemServiceRepeat: lea r10, [descriptor]; lea r11, [shadow].
    let repeat = base + SERVICE_REPEAT_RVA;
    write_lea_rip(guest, repeat, LEA_R10, descriptor);
    write_lea_rip(guest, repeat + 7, LEA_R11, shadow);
    let session = spec.processes[services.session_process].cr3;
    for index in 0..services.gui.len() {
        let routine = FakeWindows::gui_routine(index as u32);
        guest.write_session(session, routine, &ROUTINE_CODE);
        guest.write_session(session, gui_table + 4 * index as u64, &shift_entry(gui_table, routine).to_le_bytes());
    }
}

/// Every process but System: a PEB at `EPROCESS + EPROCESS_PEB` whose loader
/// list holds the process's image, `ntdll.dll` with a stub per kernel
/// service, and `win32u.dll` with a stub per GUI service.
fn write_user_processes(guest: &mut FakeGuest, spec: &FakeWindowsSpec, services: &FakeServices) {
    let dll = |name: &'static str, base: u64| FakeImage {
        name,
        base,
        size: 0x10_0000,
        time_date_stamp: 0x5000_0000,
        pdb: FakePdb { guid: [0x55; 16], age: 1, path: "dll.pdb" },
        sections: Vec::new(),
    };
    let ntdll = dll("ntdll.dll", NTDLL_BASE);
    let win32u = dll("win32u.dll", WIN32U_BASE);
    let stub_exports = |names: &[&'static str]| -> Vec<FakeExport> {
        names
            .iter()
            .enumerate()
            .map(|(index, &name)| FakeExport { name, rva: STUBS_RVA + index as u64 * ROUTINE_STRIDE })
            .collect()
    };
    for (index, process) in spec.processes.iter().enumerate().skip(1) {
        let cr3 = process.cr3;
        guest.write_u64(LIVE_CR3, EPROCESS_BASE + index as u64 * 0x1000 + EPROCESS_PEB, PEB);
        guest.write_u64(cr3, PEB + 0x10, PROCESS_IMAGE_BASE);
        guest.write_u64(cr3, PEB + 0x18, PEB_LDR);
        let image = dll(process.name, PROCESS_IMAGE_BASE);
        write_image(guest, cr3, &image, &[]);
        let mut kernel_exports = stub_exports(&services.kernel);
        // An export that is no stub: passed by.
        kernel_exports.push(FakeExport { name: "RtlAllocateHeap", rva: STUBS_RVA + 0x800 });
        write_image(guest, cr3, &ntdll, &kernel_exports);
        write_image(guest, cr3, &win32u, &stub_exports(&services.gui));
        for number in 0..services.kernel.len() {
            write_stub(guest, cr3, NTDLL_BASE + STUBS_RVA + number as u64 * ROUTINE_STRIDE, number as u32);
        }
        guest.write(cr3, NTDLL_BASE + STUBS_RVA + 0x800, &ROUTINE_CODE);
        for number in 0..services.gui.len() {
            write_stub(guest, cr3, WIN32U_BASE + STUBS_RVA + number as u64 * ROUTINE_STRIDE, 0x1000 + number as u32);
        }
        write_module_list(guest, cr3, PEB_LDR + 0x10, USER_LDR_ENTRIES, &[&image, &ntdll, &win32u]);
    }
}

/// `mov r10, rcx; mov eax, index; syscall; ret` at `at`.
fn write_stub(guest: &mut FakeGuest, cr3: u64, at: u64, index: u32) {
    guest.write(cr3, at, &[0x4C, 0x8B, 0xD1, 0xB8]);
    guest.write_u32(cr3, at + 4, index);
    guest.write(cr3, at + 8, &[0x0F, 0x05, 0xC3]);
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

/// `IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ`.
const CODE_SECTION: u32 = 0x6000_0020;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ`.
const DATA_SECTION: u32 = 0x4000_0040;

/// The headers, section table, export table and debug directory of `image`,
/// in the space at `cr3`.
fn write_image(guest: &mut FakeGuest, cr3: u64, image: &FakeImage, exports: &[FakeExport]) {
    let base = image.base;
    guest.write_u16(cr3, base, 0x5A4D);
    guest.write_u32(cr3, base + 0x3C, 0x80);
    let nt = base + 0x80;
    guest.write(cr3, nt, b"PE\0\0");
    guest.write_u16(cr3, nt + 4, 0x8664);
    guest.write_u16(cr3, nt + 6, image.sections.len() as u16);
    guest.write_u32(cr3, nt + 8, image.time_date_stamp);
    guest.write_u16(cr3, nt + 20, 0xF0);
    let optional = nt + 24;
    guest.write_u16(cr3, optional, 0x20B);
    guest.write_u32(cr3, optional + 56, image.size);
    guest.write_u32(cr3, optional + 60, 0x1000);
    guest.write_u32(cr3, optional + 108, 16);
    let directories = optional + 112;
    // IMAGE_SECTION_HEADER: Name, VirtualSize +8, VirtualAddress +12,
    // SizeOfRawData +16, PointerToRawData +20, Characteristics +36.
    let table = optional + 0xF0;
    for (index, section) in image.sections.iter().enumerate() {
        let header = table + 40 * index as u64;
        let mut name = section.name.as_bytes().to_vec();
        name.resize(8, 0);
        guest.write(cr3, header, &name);
        guest.write_u32(cr3, header + 8, section.size);
        guest.write_u32(cr3, header + 12, section.rva);
        guest.write_u32(cr3, header + 16, section.size);
        guest.write_u32(cr3, header + 20, section.rva);
        guest.write_u32(cr3, header + 36, if section.executable { CODE_SECTION } else { DATA_SECTION });
    }
    if !exports.is_empty() {
        guest.write_u32(cr3, directories, EXPORTS_RVA as u32);
        guest.write_u32(cr3, directories + 4, 0x800);
        write_exports(guest, cr3, image, exports);
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
fn write_exports(guest: &mut FakeGuest, cr3: u64, image: &FakeImage, exports: &[FakeExport]) {
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

/// A loader entry per image (`KLDR_DATA_TABLE_ENTRY` and
/// `LDR_DATA_TABLE_ENTRY` share these offsets), page by page from
/// `entries`, ringed through `head`, in the space at `cr3`.
fn write_module_list(guest: &mut FakeGuest, cr3: u64, head: u64, entries: u64, images: &[&FakeImage]) {
    let entry_of = |index: usize| entries + index as u64 * 0x1000;
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
