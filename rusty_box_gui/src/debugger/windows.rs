//! A 64-bit Windows guest's processes and kernel modules, read from its own
//! kernel structures while the VM is paused.
//!
//! Nothing here is keyed to one Windows build. The few layouts that hold for
//! every x64 Windows are used as they are: the KPCR's head (`Self` at +0x18,
//! `IdtBase` at +0x38), `KPROCESS.DirectoryTableBase` at +0x28, the PE image
//! format, and the loader's `LDR_DATA_TABLE_ENTRY` / `KLDR_DATA_TABLE_ENTRY`
//! head (links at +0x0, `DllBase` +0x30, `SizeOfImage` +0x40, `FullDllName`
//! +0x48, `BaseDllName` +0x58). Everything that moves between builds is found
//! in the guest instead:
//!
//! - the kernel image: the first image at or below interrupt vector 0's
//!   handler that exports `PsInitialSystemProcess` (the HAL, which can lie
//!   below the handler, does not);
//! - the System process: `PsInitialSystemProcess`;
//! - the loaded-module list: the `PsLoadedModuleList` export where the kernel
//!   has one (Windows XP x64 and Server 2003 do not), else the field of that
//!   name in the kernel's debugger data block (`KDDEBUGGER_DATA64`, tagged
//!   `KDBG`, whose `KernBase` is the image's base);
//! - the `EPROCESS` offsets of `UniqueProcessId` and `ActiveProcessLinks`: the
//!   System process's PID 4 followed by a list entry whose neighbours point
//!   back at it; of `ImageFileName`: the System process's name, "System";
//! - the `EPROCESS.Peb` offset: the field that, in several processes at once,
//!   points at a page whose `ImageBaseAddress` (+0x10) holds an `MZ` image.
//!
//! A process's own memory is read through its page tables
//! ([`Root::Process`]), so every process's modules list, not only the
//! running one's. Every read goes through [`GuestMemory`], so the walks run
//! as well over a page map as over a paused machine.
//!
//! What the walks discover about a guest's layouts is held in a
//! [`WindowsDiscovery`] for the run and checked where it is used, so a pause
//! pays for the lists it reads and not for finding the structures again.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use rusty_box::cpu::{CpuDebugView, LinearRange, MappingWalk, PageMapping, WalkStep};
use rusty_box_symbols::units::FieldOffset;

use super::drivers;
use super::guest::{GuestMemory, GuestRead, Root};
use super::kernel_layout::{same_layouts, KernelLayout, LayoutSource};
use super::module_identity::{read_module_identity, GuestIdentity};
use super::inspect::{ModuleSpace, SymModule, SymbolIndex};
use super::ssdt;
use super::memory_map::{self, MemoryMap, ModuleLayout};
use super::pe::{self, ExportRead, ExportTarget, ImportRead, HEADER_BYTES};
use super::threads::{
    decode_trap_frame, frame_is_users, walk_thread_list, SavedRegisters, WinThread, MAX_THREADS,
    TRAP_FRAME_BYTES,
};
use super::vad::{
    self, FieldWidth, VadLayout, VadNode, VadRegion, VadRootKind, VadShape, MAX_VAD_NODES,
};

/// The top of a user address space: canonical low half in long mode, and
/// above every 32-bit user address, so one bound covers native 64-bit, a
/// WOW64 process and a 32-bit kernel's user space alike.
const USER_SPACE_END: u64 = 0x0000_8000_0000_0000;

/// The largest span a single VAD may describe: 64 GiB, far above any real
/// reservation yet far below the whole address space. A descriptor larger than
/// this is a wrong field read as a node, not a VAD.
const MAX_VAD_SPAN: u64 = 0x10_0000_0000;

/// What the page asks to list beside the processor panes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SystemView {
    /// Nothing: the processor panes only.
    #[default]
    Off,
    /// Every process, and — for `selected` (an `EPROCESS` address), or the
    /// running process when none is selected — its modules, its memory map
    /// or its threads; with its modules, `module_detail` of the module based
    /// at `module`.
    Processes {
        selected: Option<u64>,
        detail: ProcessDetail,
        module: Option<u64>,
        module_detail: ModuleDetail,
    },
    /// The drivers and the kernel itself, in load order, and — for
    /// `selected` (a module's base) — its sections, exports or imports.
    KernelModules {
        selected: Option<u64>,
        detail: ModuleDetail,
    },
    /// Every driver object, and — for `selected` (a `DRIVER_OBJECT` address) —
    /// its IRP dispatch table or its devices.
    Drivers {
        selected: Option<u64>,
        detail: DriverDetail,
    },
    /// The kernel's system-service dispatch table.
    Ssdt,
}

/// What the Drivers view shows for the chosen driver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DriverDetail {
    /// The 28 IRP handlers, `StartIo`, `Unload`, `Init` and `FastIoDispatch`,
    /// each classified against the loaded modules.
    #[default]
    Dispatch,
    /// Its device objects, with their types, filter stacks and `\??\` links.
    Devices,
}

/// What is shown of a picked module: a driver in the Kernel modules view, or
/// a module of a process in the Processes view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ModuleDetail {
    /// Its PE sections, with how much of each is resident and its access.
    #[default]
    Sections,
    /// The functions it exports.
    Exports,
    /// The functions it imports, by the module they come from.
    Imports,
    /// Its identity, and what the symbol engine has for it.
    Symbols,
}

/// The kernel's modules, and the picked one's detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KernelListing {
    pub(crate) modules: Vec<WinModule>,
    pub(crate) picked: Option<PickedModule>,
}

/// The picked module's detail, as the view asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PickedModule {
    pub(crate) base: u64,
    pub(crate) shown: PickedDetail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickedDetail {
    Sections(SectionsRead),
    Exports(ExportRead),
    Imports(ImportRead),
    /// The identity its image gives, which the engine's state is looked up by.
    Symbols(GuestIdentity),
}

impl PickedDetail {
    /// Whether this is the detail `asked` names.
    pub(crate) const fn answers(&self, asked: ModuleDetail) -> bool {
        matches!(
            (self, asked),
            (Self::Sections(_), ModuleDetail::Sections)
                | (Self::Exports(_), ModuleDetail::Exports)
                | (Self::Imports(_), ModuleDetail::Imports)
                | (Self::Symbols(_), ModuleDetail::Symbols)
        )
    }
}

/// A module's sections as its headers and the page tables give them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SectionsRead {
    /// Its header page and each section, with residency and access.
    Mapped(MemoryMap),
    /// Its header page is paged out, so its sections are unknown.
    HeaderUnreadable,
}

/// What the Processes view shows for the chosen process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ProcessDetail {
    /// Its loaded modules, in load order.
    #[default]
    Modules,
    /// Its address space, region by region.
    MemoryMap,
    /// Its threads, with their start addresses and stacks.
    Threads,
}

/// What was listed for a [`SystemView`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SystemListing {
    NotAsked,
    /// The guest is not running a 64-bit Windows kernel this walk can find
    /// (another OS, a 32-bit Windows, or a kernel not mapped in the running
    /// address space), and the step that showed it.
    NoWindowsKernel(KernelMiss),
    Processes(ProcessListing),
    KernelModules(KernelListing),
    Drivers(DriverListing),
    Ssdt(SsdtListing),
}

/// The kernel's two system-service tables: its own (`KiServiceTable`) and
/// the GUI's (`W32pServiceTable`, in `win32k`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SsdtListing {
    pub(crate) kernel: ServiceTableRead,
    pub(crate) gui: GuiServices,
}

/// The GUI services, or why they were not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuiServices {
    Listed(ServiceTableRead),
    Missing(GuiMiss),
}

/// Why the GUI table was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuiMiss {
    /// No `KeServiceDescriptorTableShadow` was found.
    NoShadowDescriptor,
    /// No `win32k*.sys` is loaded.
    NoWin32k,
    /// No process but System maps the table: its session space is in none
    /// read.
    NoSessionProcess,
}

impl GuiMiss {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::NoShadowDescriptor => "The GUI services were not read: KeServiceDescriptorTableShadow was not found.",
            Self::NoWin32k => "The GUI services were not read: no win32k module is loaded.",
            Self::NoSessionProcess => {
                "The GUI services were not read: no process maps them (they live in a session's address space)."
            }
        }
    }
}

/// How a table's descriptor was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableSource {
    /// The kernel PDB's symbol.
    Pdb,
    /// The kernel's export (the first x64 kernel only).
    Export,
    /// The `lea` that loads it in `KiSystemServiceRepeat`.
    Code,
}

impl TableSource {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pdb => "found by: PDB",
            Self::Export => "found by: export",
            Self::Code => "found by: code",
        }
    }
}

/// Where a service table itself lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TablePlace {
    /// In the image that owns it.
    InOwner,
    /// Outside it: the whole table was swapped.
    Outside,
}

/// One system-service table, one row per service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceTableRead {
    pub(crate) source: TableSource,
    /// The table's address.
    pub(crate) base: u64,
    pub(crate) place: TablePlace,
    pub(crate) services: Vec<SsdtService>,
    /// Whether any entry leads outside the table's owner, or any routine's
    /// first instruction jumps outside it — a hook.
    pub(crate) any_hooked: bool,
}

/// One system service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SsdtService {
    pub(crate) index: u32,
    pub(crate) routine: u64,
    pub(crate) site: PointerSite,
    /// The name of the module the routine lies in, when it lies in one.
    pub(crate) module_name: Option<String>,
    /// The name of the system-call stub that calls this service (`ntdll`'s
    /// or `win32u`'s), when one does.
    pub(crate) stub_name: Option<String>,
    /// Where the routine's first instruction jumps, when it jumps outside the
    /// table's owner: an inline hook.
    pub(crate) inline: Option<InlineJump>,
}

/// An inline hook: the routine's first instruction leaves its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InlineJump {
    pub(crate) target: u64,
    /// The module the jump lands in, when it lands in one.
    pub(crate) module_name: Option<String>,
}

/// Every driver object, and the picked one's detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DriverListing {
    pub(crate) drivers: Vec<WinDriver>,
    pub(crate) picked: Option<PickedDriver>,
}

/// One driver object in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WinDriver {
    /// The kernel address of its `DRIVER_OBJECT`.
    pub(crate) driver_object: u64,
    /// Its namespace name, e.g. `\Driver\Disk`.
    pub(crate) name: String,
    /// The base of the loaded module its `DriverStart`/`DriverSize` matches,
    /// when one matches.
    pub(crate) module: Option<u64>,
    /// The module's name, when the match is named.
    pub(crate) module_name: Option<String>,
    /// Whether the driver was found in `\Driver` or `\FileSystem`; a driver
    /// reached only through a device reads false ("not in namespace").
    pub(crate) in_namespace: bool,
    pub(crate) device_count: usize,
    /// It has no matching module, or at least one handler points into none.
    pub(crate) flagged: bool,
}

/// The picked driver's detail, as the view asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PickedDriver {
    pub(crate) driver_object: u64,
    pub(crate) shown: DriverShown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DriverShown {
    /// The 28 IRP handlers, then `StartIo`, `Unload`, `Init`, then the
    /// `FastIoDispatch` entries.
    Dispatch(Vec<DispatchEntry>),
    Devices(Vec<DeviceRow>),
}

impl DriverShown {
    /// Whether this is the detail `asked` names.
    pub(crate) const fn answers(&self, asked: DriverDetail) -> bool {
        matches!(
            (self, asked),
            (Self::Dispatch(_), DriverDetail::Dispatch) | (Self::Devices(_), DriverDetail::Devices)
        )
    }
}

/// One entry in a driver's dispatch table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DispatchEntry {
    /// e.g. `READ`, `DEVICE_CONTROL`, `Unload`, `FastIoRead`.
    pub(crate) label: String,
    pub(crate) addr: u64,
    pub(crate) site: PointerSite,
}

/// One of a driver's device objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceRow {
    /// Its object name, e.g. `\Device\Harddisk0`, when it has one.
    pub(crate) name: Option<String>,
    pub(crate) device_type: String,
    pub(crate) characteristics: u32,
    /// The named devices attached above this one, in stack order (filters).
    pub(crate) attached_above: Vec<String>,
    /// The `\??\` symbolic links resolving to this device.
    pub(crate) links: Vec<String>,
}

/// Where a kernel code pointer in a dispatch or callback table leads. Reused
/// by the SSDT and callbacks views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerSite {
    /// Inside the owning driver's own image: a normal, unhooked handler.
    OwnImage,
    /// The kernel's shared default handler (`IopInvalidDeviceRequest`): the
    /// value most drivers share that lands inside the kernel image.
    KernelDefault,
    /// Inside another loaded module, at `base`+`off`. Normal for drivers that
    /// route through a framework (Wdf01000, CLASSPNP); shown neutrally.
    InModule { base: u64, off: u64 },
    /// Inside no loaded module — the case worth flagging.
    NoModule,
}

/// The step at which the walk to a Windows x64 kernel gave out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KernelMiss {
    /// EFER.LMA is clear: no 64-bit kernel runs.
    NotLongMode,
    /// Neither GS nor `IA32_KERNEL_GS_BASE` points at a KPCR.
    NoKpcr,
    /// A KPCR-shaped address is held, but it does not read: the kernel is not
    /// mapped in the page tables the processor runs on (kernel page-table
    /// isolation leaves it out of a user-mode pause's).
    KernelUnmapped,
    /// No image below interrupt vector 0's handler exports
    /// `PsInitialSystemProcess`.
    NoKernelImage,
    /// `PsInitialSystemProcess` is not set.
    NoSystemProcess,
    /// The System process holds no PID 4 on a process list, or no "System".
    UnknownProcessLayout,
    /// Neither an export nor the debugger data block gives the
    /// loaded-module list.
    NoModuleList,
    /// The object namespace root was not reached from `IoDriverObjectType`.
    NoObjectNamespace,
    /// `KeServiceDescriptorTable` was not found or did not read.
    NoSsdt,
}

impl KernelMiss {
    /// Whether the kernel itself was found, and only a structure beneath it
    /// was not.
    pub(crate) const fn kernel_found(self) -> bool {
        match self {
            Self::NotLongMode
            | Self::NoKpcr
            | Self::KernelUnmapped
            | Self::NoKernelImage
            | Self::NoSystemProcess
            | Self::UnknownProcessLayout => false,
            Self::NoModuleList | Self::NoObjectNamespace | Self::NoSsdt => true,
        }
    }

    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::NotLongMode => "the processor is not in long mode",
            Self::NoKpcr => "neither GS nor KERNEL_GS_BASE points at a KPCR",
            Self::KernelUnmapped => {
                "the kernel is not mapped in the page tables this pause runs on, and the \
                 KPCR's mapped pages hold none of this process's kernel tables; pause while \
                 the guest runs kernel code"
            }
            Self::NoKernelImage => "no kernel image exporting PsInitialSystemProcess was found",
            Self::NoSystemProcess => "PsInitialSystemProcess is not set",
            Self::UnknownProcessLayout => "the System process's EPROCESS was not recognised",
            Self::NoModuleList => "the kernel's loaded-module list was not found",
            Self::NoObjectNamespace => "the object namespace was not reached",
            Self::NoSsdt => "the system-service table (KeServiceDescriptorTable) was not found",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessListing {
    pub(crate) processes: Vec<WinProcess>,
    /// `processes` as a tree, depth first: one row per process.
    pub(crate) tree: Vec<TreeRow>,
    /// The modules of the selected process (or the running one).
    pub(crate) modules: Option<ProcessModules>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WinProcess {
    /// The kernel address of its `EPROCESS`.
    pub(crate) eprocess: u64,
    pub(crate) pid: u64,
    /// The PID of the process that created it, when that field was found; a
    /// parent that has since exited is no longer listed.
    pub(crate) parent: Option<u64>,
    /// `EPROCESS.ImageFileName`: the image's base name, cut at 15 bytes.
    pub(crate) name: String,
    /// Its address space: `KPROCESS.DirectoryTableBase`.
    pub(crate) cr3: u64,
    /// The shown processor runs in this process's address space.
    pub(crate) current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessModules {
    pub(crate) eprocess: u64,
    pub(crate) read: ModuleRead,
    /// The memory map, when the view asked for it and the modules read.
    pub(crate) map: Option<MemoryMap>,
    /// The process's threads, from its `ETHREAD` list.
    pub(crate) threads: Vec<WinThread>,
    /// The picked module's detail, when the view lists modules and one of
    /// them is picked.
    pub(crate) picked: Option<PickedModule>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModuleRead {
    Listed(Vec<WinModule>),
    /// A process with no user half (System, Idle): nothing loaded for it.
    NoUserSpace,
    /// Its PEB or loader data is paged out, or no PEB field was found.
    Unreadable,
}

/// A loaded image's bitness: a 64-bit module, or a 32-bit one a WOW64 process
/// loaded through its 32-bit loader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModuleBits {
    Bits64,
    Bits32,
}

/// One loaded image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WinModule {
    pub(crate) base: u64,
    pub(crate) size: u32,
    /// `BaseDllName`, when its buffer is resident.
    pub(crate) name: Option<String>,
    /// `FullDllName`, when its buffer is resident.
    pub(crate) path: Option<String>,
    pub(crate) bits: ModuleBits,
}

/// The System process's ID on every x64 Windows.
const SYSTEM_PID: u64 = 4;
/// `KPROCESS.DirectoryTableBase`.
const DIRECTORY_TABLE_BASE: u64 = 0x28;
/// How much of an `EPROCESS` is searched for a field.
const EPROCESS_SPAN: usize = 0x800;
/// `EPROCESS.ImageFileName` is 15 bytes.
const IMAGE_FILE_NAME_BYTES: usize = 15;
/// How far below vector 0's handler the kernel's header is looked for.
const KERNEL_SCAN_PAGES: u64 = 0x2000;
/// Walks stop here, so a corrupt list cannot spin the run thread.
const MAX_PROCESSES: usize = 1024;
const MAX_MODULES: usize = 2048;
/// The longest module name read, in bytes.
const MAX_NAME_BYTES: u16 = 2048;
/// How many processes must agree on the PEB offset.
const PEB_WITNESSES: usize = 2;
const PAGE_BYTES: u64 = 0x1000;
const MZ: u16 = 0x5A4D;
const PE_SIGNATURE: u32 = 0x4550;
const PE32_PLUS_MAGIC: u16 = 0x20B;

/// The kernel image found in the guest.
#[derive(Clone, Copy)]
struct PeImage {
    base: u64,
    /// The `IMAGE_NT_HEADERS64`.
    nt: u64,
    size: u32,
}

/// The kernel's image, and the address of its `PsInitialSystemProcess`.
struct KernelImage {
    image: PeImage,
    system_variable: u64,
}

/// Where the moving `EPROCESS` fields sit in this build.
struct ProcessLayout {
    pid: u64,
    links: u64,
    name: u64,
    /// `KPROCESS.DirectoryTableBase`.
    directory_table_base: u64,
}

/// Whose layouts the walks read a kernel's structures by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KernelLayouts {
    /// The kernel PDB's, which agree with memory.
    Pdb,
    /// Found in memory: no kernel PDB is loaded.
    Discovered,
    /// Found in memory: the System process does not read as System where the
    /// kernel PDB puts its fields.
    PdbDisagrees,
    /// A PDB of another build the owner loaded by hand, which agrees with
    /// memory: its layouts may still differ where memory is not checked.
    ForcedPdb,
}

impl KernelLayouts {
    /// What each listing says of its layouts.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pdb => "layouts: PDB",
            Self::ForcedPdb => "layouts: PDB of another build, loaded by hand",
            Self::Discovered => "layouts: discovered",
            Self::PdbDisagrees => "layouts: discovered (the kernel PDB disagrees with memory)",
        }
    }
}

struct Kernel {
    image: PeImage,
    /// The loaded-module list's head, when one was found.
    loaded_modules: Option<u64>,
    system: u64,
    layout: ProcessLayout,
    /// The kernel's build, when its `NtBuildNumber` export reads.
    build: Option<u32>,
}

/// The kernel's build, from its `NtBuildNumber` export: the low 16 bits, the
/// high nibble marking a free or checked build.
fn nt_build_number<G: GuestMemory>(cpu: &mut G, image: &PeImage) -> Option<u32> {
    let variable = export(cpu, image, "NtBuildNumber")?;
    read_u32(cpu, Root::Live, variable).map(|value| value & 0xFFFF)
}

/// Where a process's VAD tree root sits in its `EPROCESS`, and the layout
/// that tree uses.
#[derive(Clone, Copy)]
struct VadSite {
    layout: VadLayout,
    offset: u64,
}

/// A kernel found in the guest, and the field offsets learned beneath it.
/// They hold for as long as the kernel does: a different kernel (the guest
/// rebooted inside the VM) is a new state.
struct KernelState {
    kernel: Kernel,
    /// `EPROCESS.Peb`.
    peb_offset: Option<u64>,
    /// `EPROCESS.InheritedFromUniqueProcessId`.
    parent_offset: Option<u64>,
    /// `EPROCESS.Wow64Process`.
    wow64_offset: Option<u64>,
    threads: Option<ThreadLayout>,
    vad: Option<VadSite>,
    /// The object-header name-lookup era, once the object namespace was read.
    object_era: Option<drivers::ObjectHeaderEra>,
    /// The object namespace root directory (`\`), once reached.
    object_root: Option<u64>,
    /// The service descriptors and stub names, once found.
    ssdt: SsdtCache,
    /// Whose layouts the offsets above were seeded from.
    layouts: KernelLayouts,
}

impl KernelState {
    /// A kernel found by discovery, nothing learned beneath it yet.
    fn discovered(kernel: Kernel, layouts: KernelLayouts) -> KernelState {
        KernelState {
            kernel,
            peb_offset: None,
            parent_offset: None,
            wow64_offset: None,
            threads: None,
            vad: None,
            object_era: None,
            object_root: None,
            ssdt: SsdtCache::default(),
            layouts,
        }
    }

    /// A kernel whose PDB agrees with memory: every member the PDB has is
    /// taken from it, and one it lacks is left for discovery. Each is still
    /// checked where it is used, as a discovered one is.
    fn seeded<G: GuestMemory>(cpu: &mut G, kernel: Kernel, layout: &KernelLayout) -> KernelState {
        let base = kernel.image.base;
        let process = &layout.process;
        let globals = &layout.globals;
        let object_root = globals
            .root_directory_object
            .and_then(|rva| read_u64(cpu, Root::Live, base.wrapping_add(rva.0)))
            .and_then(kernel_pointer);
        KernelState {
            peb_offset: process.peb.map(|offset| offset.0),
            parent_offset: process.inherited_from_unique_process_id.map(|offset| offset.0),
            wow64_offset: process.wow64_process.map(|offset| offset.0),
            threads: seeded_threads(layout),
            vad: layout
                .vad
                .and_then(|tree| process.vad_root.map(|root| VadSite { layout: tree, offset: root.0 })),
            object_era: layout.object_era,
            object_root,
            ssdt: SsdtCache {
                kernel: globals
                    .service_descriptor_table
                    .map(|rva| FoundDescriptor { address: base.wrapping_add(rva.0), source: TableSource::Pdb }),
                shadow: globals
                    .service_descriptor_table_shadow
                    .map(|rva| FoundDescriptor { address: base.wrapping_add(rva.0), source: TableSource::Pdb }),
                stubs: StubNames::default(),
            },
            layouts: match layout.source {
                LayoutSource::Matched => KernelLayouts::Pdb,
                LayoutSource::Forced => KernelLayouts::ForcedPdb,
            },
            kernel,
        }
    }
}

/// The thread layout the PDB gives, its `ETHREAD` members made relative to
/// `ThreadListEntry` as the walk reads them; `None` without the list's head,
/// its entry or `Cid`.
fn seeded_threads(layout: &KernelLayout) -> Option<ThreadLayout> {
    let head = layout.process.thread_list_head?.0;
    let fields = &layout.thread;
    let entry = i64::try_from(fields.thread_list_entry?.0).ok()?;
    let delta = |member: Option<FieldOffset>| {
        member
            .and_then(|offset| i64::try_from(offset.0).ok())
            .and_then(|offset| offset.checked_sub(entry))
    };
    Some(ThreadLayout {
        head,
        cid: delta(fields.cid)?,
        teb: delta(fields.teb),
        start: delta(fields.win32_start_address)
            .and_then(|win32| delta(fields.start_address).map(|kernel| StartLayout { win32, kernel })),
        trap_frame: delta(fields.trap_frame),
    })
}

/// What the walks have learned about one guest's structure layouts. A run
/// keeps one, so a pause reads lists instead of finding the structures
/// again; it is dropped when a new run begins.
#[derive(Default)]
pub(crate) struct WindowsDiscovery {
    state: Option<KernelState>,
    /// The kernel PDB's layouts `state` was learned under.
    layout: Option<Arc<KernelLayout>>,
    /// Where, from the KPCR, the KPRCB's `KernelDirectoryTableBase` lies,
    /// once a KVA-shadow pause found it.
    kva_shadow: Option<u64>,
}

impl WindowsDiscovery {
    /// Keeps what was learned only while the layouts stay `layout`: a state
    /// learned under other layouts may hold offsets they do not.
    fn under(&mut self, layout: Option<&Arc<KernelLayout>>) {
        if !same_layouts(self.layout.as_ref(), layout) {
            self.state = None;
            self.layout = layout.cloned();
        }
    }
}

/// What a guest's Windows structures gave one pause.
pub(crate) struct WindowsRead {
    /// The modules addresses are named against.
    pub(crate) symbols: SymbolIndex,
    /// What the view asked to be listed.
    pub(crate) system: SystemListing,
    /// Whose layouts the kernel's structures were read by, once it was found.
    pub(crate) layouts: Option<KernelLayouts>,
    /// Whose page tables the kernel was read through.
    pub(crate) through: ReadThrough,
    /// The modules looked for in the other processes.
    pub(crate) elsewhere: ModulesElsewhere,
}

/// Whose page tables a pause's kernel reads go through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadThrough {
    /// The processor's own: they map the kernel.
    ProcessorTables,
    /// The kernel's, from the KPRCB: the processor runs user mode on KVA-shadow
    /// tables that leave the kernel out.
    KernelTables { cr3: u64 },
}

impl ReadThrough {
    /// The CR3 the walk's live space is rooted at: the running process's.
    fn live_cr3(self, view: &CpuDebugView) -> u64 {
        match self {
            Self::ProcessorTables => view.registers.cr3,
            Self::KernelTables { cr3 } => cr3,
        }
    }

    /// What a listing says of the tables it was read through, when not the
    /// processor's own.
    pub(crate) const fn label(self) -> Option<&'static str> {
        match self {
            Self::ProcessorTables => None,
            Self::KernelTables { .. } => {
                Some("read through the kernel's page tables: this pause runs user mode on KVA-shadow tables")
            }
        }
    }
}

/// A guest read through the kernel's page tables where the processor's
/// leave the kernel out: `Root::Live` walks from `cr3`, and every other read
/// is the guest's own.
struct KernelTablesView<'a, G> {
    cpu: &'a mut G,
    cr3: u64,
}

impl<G: GuestMemory> GuestMemory for KernelTablesView<'_, G> {
    fn translate(&mut self, root: Root, linear: u64) -> Option<u64> {
        let root = match root {
            Root::Live => Root::Process(self.cr3),
            Root::Process(cr3) => Root::Process(cr3),
        };
        self.cpu.translate(root, linear)
    }

    fn read_physical(&mut self, physical: u64, buf: &mut [u8]) -> GuestRead {
        self.cpu.read_physical(physical, buf)
    }

    fn page_mappings<F>(&mut self, cr3: u64, range: LinearRange, visit: F) -> MappingWalk
    where
        F: FnMut(PageMapping) -> WalkStep,
    {
        self.cpu.page_mappings(cr3, range, visit)
    }
}

/// How far past the KPCR the KPRCB's `KernelDirectoryTableBase` is looked
/// for: past the end of the largest KPRCB.
const KPCR_SHADOW_SPAN: u64 = 0x2_0000;
/// The CR3 bits that address the top-level table; the low 12 are flags or
/// a PCID.
const CR3_TABLE: u64 = 0x000F_FFFF_FFFF_F000;
/// The CR3 bits no table address uses: 52–62 (bit 63 is the no-flush hint).
const CR3_RESERVED: u64 = 0x7FF0_0000_0000_0000;

/// Whose page tables this pause's kernel reads go through: the processor's
/// while the kernel reads through them, else the kernel's own tables, found
/// in the KPRCB page a KVA-shadow user space keeps mapped (see
/// [`shadow_kernel_cr3`]).
fn kernel_tables<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
    discovery: &mut WindowsDiscovery,
) -> ReadThrough {
    let mapped = match &discovery.state {
        Some(state) => kernel_stands(cpu, &state.kernel),
        None => find_kpcr(cpu, view).and_then(|kpcr| kernel_image(cpu, kpcr)).is_some(),
    };
    if mapped {
        return ReadThrough::ProcessorTables;
    }
    match shadow_kernel_cr3(cpu, view, &mut discovery.kva_shadow) {
        Some(cr3) => ReadThrough::KernelTables { cr3 },
        None => ReadThrough::ProcessorTables,
    }
}

/// The kernel's CR3 for the running process, under KVA shadow: the entry
/// stubs load it from the KPRCB's `KernelDirectoryTableBase`, on a page the
/// user tables keep mapped, at an offset that differs by build. Each 8-byte
/// value in the KPCR's readable pages is tried (the offset `known` first),
/// and taken when through it the KPCR's `Self` reads back the KPCR, its
/// tables are not the processor's, and the paused RIP maps where it maps
/// under the processor's — the same process. The offset found is kept in
/// `known`.
fn shadow_kernel_cr3<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
    known: &mut Option<u64>,
) -> Option<u64> {
    let kpcrs: Vec<u64> = [view.segment_bases.gs, view.kernel_gs_base]
        .into_iter()
        .filter_map(kernel_pointer)
        .collect();
    let live_table = view.registers.cr3 & CR3_TABLE;
    let rip = view.registers.rip;
    let live_rip = cpu.translate(Root::Live, rip);
    let fits = |cpu: &mut G, kpcr: u64, value: u64| -> bool {
        value & CR3_TABLE != 0
            && value & CR3_RESERVED == 0
            && value & CR3_TABLE != live_table
            && read_u64(cpu, Root::Process(value), kpcr.wrapping_add(0x18)) == Some(kpcr)
            && cpu.translate(Root::Process(value), rip) == live_rip
    };
    for &kpcr in &kpcrs {
        if let Some(offset) = *known {
            if let Some(value) = read_u64(cpu, Root::Live, kpcr.wrapping_add(offset)) {
                if fits(cpu, kpcr, value) {
                    return Some(value);
                }
            }
        }
    }
    for &kpcr in &kpcrs {
        let mut page = vec![0u8; PAGE_BYTES as usize];
        let start = kpcr & !(PAGE_BYTES - 1);
        for offset in (0..KPCR_SHADOW_SPAN).step_by(PAGE_BYTES as usize) {
            let at = start.wrapping_add(offset);
            if !read_into(cpu, Root::Live, at, &mut page) {
                continue;
            }
            for (slot, bytes) in (0u64..).zip(page.chunks_exact(8)) {
                let Ok(raw) = <[u8; 8]>::try_from(bytes) else {
                    continue;
                };
                let value = u64::from_le_bytes(raw);
                if fits(cpu, kpcr, value) {
                    *known = Some(at.wrapping_add(slot.wrapping_mul(8)).wrapping_sub(kpcr));
                    return Some(value);
                }
            }
        }
    }
    None
}

/// [`read_windows_finding`], finding no module in other processes.
#[cfg(test)]
pub(crate) fn read_windows<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
    request: SystemView,
    discovery: &mut WindowsDiscovery,
    layout: Option<&Arc<KernelLayout>>,
) -> WindowsRead {
    read_windows_finding(cpu, view, request, FindElsewhere::NOTHING, discovery, layout)
}

/// The modules the code, stack and memory panes name an address against (the
/// kernel's loaded modules, and the shown processor's process's own modules
/// when it runs one), the listing `request` asks for, and the modules `find`
/// asks for in the other processes, through the processor `view` describes.
/// The symbols are read whatever is requested. A read that comes up short
/// leaves the index short; it never fails the snapshot, so a non-Windows
/// guest simply names nothing. `layout` is the kernel PDB's, when the shell
/// handed one over.
pub(crate) fn read_windows_finding<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
    request: SystemView,
    find: FindElsewhere<'_>,
    discovery: &mut WindowsDiscovery,
    layout: Option<&Arc<KernelLayout>>,
) -> WindowsRead {
    discovery.under(layout);
    let asked = Asked { request, find };
    match kernel_tables(cpu, view, discovery) {
        ReadThrough::ProcessorTables => {
            read_windows_through(cpu, view, asked, discovery, layout, ReadThrough::ProcessorTables)
        }
        ReadThrough::KernelTables { cr3 } => {
            let mut kernel = KernelTablesView { cpu, cr3 };
            read_windows_through(&mut kernel, view, asked, discovery, layout, ReadThrough::KernelTables { cr3 })
        }
    }
}

/// What a Windows read is asked for: a listing, and modules in other
/// processes.
#[derive(Clone, Copy)]
struct Asked<'a> {
    request: SystemView,
    find: FindElsewhere<'a>,
}

/// The modules a pause looks for in the processes its panes do not stand
/// in: the owner's binaries by file name, in every process but the one
/// whose modules the pause already names.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FindElsewhere<'a> {
    /// The file names (any case); none reads no other process.
    pub(crate) names: &'a [String],
    /// The address space (`DirectoryTableBase`) of the process the pause's
    /// modules are: a thread view's; the running process's when `None`.
    pub(crate) shown: Option<u64>,
}

#[cfg(test)]
impl FindElsewhere<'_> {
    /// Nothing looked for.
    pub(crate) const NOTHING: FindElsewhere<'static> = FindElsewhere { names: &[], shown: None };
}

/// A module of a process other than the one the pause's modules are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessModule {
    /// The process's address space: its `DirectoryTableBase`.
    pub(crate) cr3: u64,
    /// `EPROCESS.ImageFileName`.
    pub(crate) process: String,
    pub(crate) module: SymModule,
}

/// The modules of the names asked for, in every process but the one whose
/// modules the pause names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ModulesElsewhere {
    /// The file names looked for: a pause that asks for the same ones while
    /// the guest has not moved finds the same modules.
    pub(crate) names: Vec<String>,
    pub(crate) modules: Vec<ProcessModule>,
}

/// Where a process's PEB and its WOW64 PEB pointer sit in `EPROCESS`, as far
/// as they were found.
struct ProcessOffsets<'a> {
    peb: &'a mut Option<u64>,
    wow64: &'a mut Option<u64>,
}

/// The user modules whose file names are among `find.names` (any case), in
/// every process of `processes` but the one `find.shown` names (the running
/// one when it names none), each read in its own process's address space:
/// where the owner's own programs run while the pause stands elsewhere. The
/// running process's list, read already as `current_read`, is not read
/// again.
fn modules_elsewhere<G: GuestMemory>(
    cpu: &mut G,
    processes: &[WinProcess],
    current_read: Option<&ModuleRead>,
    find: FindElsewhere<'_>,
    offsets: ProcessOffsets<'_>,
) -> ModulesElsewhere {
    if find.names.is_empty() {
        return ModulesElsewhere::default();
    }
    let shown = |process: &WinProcess| match find.shown {
        Some(cr3) => same_address_space(process.cr3, cr3),
        None => process.current,
    };
    let wanted: Vec<String> = find.names.iter().map(|name| name.to_ascii_lowercase()).collect();
    let mut modules = Vec::new();
    for process in processes.iter().filter(|process| !shown(process)) {
        let read = match current_read.filter(|_| process.current) {
            Some(read) => read.clone(),
            None => process_modules(cpu, processes, process, &mut *offsets.peb, &mut *offsets.wow64),
        };
        let ModuleRead::Listed(listed) = read else {
            continue;
        };
        let ours: Vec<WinModule> = listed
            .into_iter()
            .filter(|module| module.name.as_deref().is_some_and(|name| wanted.contains(&name.to_ascii_lowercase())))
            .collect();
        let mut found = Vec::new();
        gather_modules(cpu, Root::Process(process.cr3), &mut found, &ours, ModuleSpace::User);
        modules.extend(found.into_iter().map(|module| ProcessModule {
            cr3: process.cr3,
            process: process.name.clone(),
            module,
        }));
    }
    ModulesElsewhere { names: find.names.to_vec(), modules }
}

/// [`read_windows_finding`] through the page tables `through` names, which
/// `Root::Live` already walks.
fn read_windows_through<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
    asked: Asked<'_>,
    discovery: &mut WindowsDiscovery,
    layout: Option<&Arc<KernelLayout>>,
    through: ReadThrough,
) -> WindowsRead {
    let Asked { request, find } = asked;
    let live_cr3 = through.live_cr3(view);
    let state = match find_kernel(cpu, view, &mut discovery.state, layout) {
        Ok(state) => state,
        Err(miss) => {
            let system = match request {
                SystemView::Off => SystemListing::NotAsked,
                SystemView::Processes { .. }
                | SystemView::KernelModules { .. }
                | SystemView::Drivers { .. }
                | SystemView::Ssdt => SystemListing::NoWindowsKernel(miss),
            };
            return WindowsRead {
                symbols: SymbolIndex::default(),
                system,
                layouts: None,
                through,
                elsewhere: ModulesElsewhere::default(),
            };
        }
    };
    let KernelState {
        kernel,
        peb_offset,
        parent_offset,
        wow64_offset,
        threads: thread_layout,
        vad,
        object_era,
        object_root,
        ssdt,
        layouts,
    } = state;
    let layouts = *layouts;
    let kernel_modules = match kernel.loaded_modules {
        Some(head) => list_modules(cpu, Root::Live, head).unwrap_or_default(),
        None => Vec::new(),
    };
    let processes = list_processes(cpu, kernel, live_cr3, parent_offset);
    let current_index = processes.iter().position(|process| process.current);
    let current_read = current_index
        .and_then(|index| processes.get(index))
        .map(|current| process_modules(cpu, &processes, current, peb_offset, wow64_offset));

    let mut named = Vec::new();
    gather_modules(cpu, Root::Live, &mut named, &kernel_modules, ModuleSpace::Kernel);
    if let Some(ModuleRead::Listed(user)) = &current_read {
        gather_modules(cpu, Root::Live, &mut named, user, ModuleSpace::User);
    }
    let symbols = SymbolIndex {
        modules: named,
        kernel: Some(kernel.image.base),
    };
    let elsewhere = modules_elsewhere(
        cpu,
        &processes,
        current_read.as_ref(),
        find,
        ProcessOffsets { peb: &mut *peb_offset, wow64: &mut *wow64_offset },
    );

    let system = match request {
        SystemView::Off => SystemListing::NotAsked,
        SystemView::KernelModules { selected, detail } => match kernel.loaded_modules {
            Some(_) => {
                // The picked module's detail is read only as asked. The
                // kernel half is the same in every address space.
                let picked = selected
                    .and_then(|base| kernel_modules.iter().find(|module| module.base == base))
                    .cloned()
                    .map(|module| PickedModule {
                        base: module.base,
                        shown: read_module_detail(
                            cpu,
                            ImageSpace {
                                root: Root::Live,
                                cr3: live_cr3,
                            },
                            &module,
                            detail,
                        ),
                    });
                SystemListing::KernelModules(KernelListing {
                    modules: kernel_modules,
                    picked,
                })
            }
            None => SystemListing::NoWindowsKernel(KernelMiss::NoModuleList),
        },
        SystemView::Processes {
            selected,
            detail,
            module,
            module_detail,
        } => {
            let chosen_index = selected
                .and_then(|eprocess| processes.iter().position(|p| p.eprocess == eprocess))
                .or(current_index);
            let modules = chosen_index.and_then(|index| {
                let process = processes.get(index)?;
                let read = match &current_read {
                    Some(read) if Some(index) == current_index => read.clone(),
                    Some(_) | None => {
                        process_modules(cpu, &processes, process, peb_offset, wow64_offset)
                    }
                };
                let (user_space, listed) = match &read {
                    ModuleRead::Listed(modules) => (true, modules.as_slice()),
                    ModuleRead::Unreadable => (true, &[][..]),
                    ModuleRead::NoUserSpace => (false, &[][..]),
                };
                let ranges = code_ranges(&kernel_modules, listed);
                let threads = read_threads(
                    cpu,
                    process.eprocess,
                    process.pid,
                    process.cr3,
                    user_space,
                    thread_layout,
                    &ranges,
                );
                let map = match (detail, &read) {
                    (ProcessDetail::MemoryMap, ModuleRead::Listed(modules)) => {
                        let mut map =
                            read_memory_map(cpu, process.eprocess, process.cr3, modules, kernel.build, vad);
                        memory_map::label_stacks(&mut map, &threads);
                        Some(map)
                    }
                    (ProcessDetail::MemoryMap, ModuleRead::NoUserSpace | ModuleRead::Unreadable)
                    | (ProcessDetail::Modules | ProcessDetail::Threads, _) => None,
                };
                // The picked module's detail, read in the process's own
                // address space: the running one through the processor's
                // page tables, any other through its DirectoryTableBase.
                let space = ImageSpace {
                    root: if process.current {
                        Root::Live
                    } else {
                        Root::Process(process.cr3)
                    },
                    cr3: process.cr3,
                };
                let picked = match (detail, &read) {
                    (ProcessDetail::Modules, ModuleRead::Listed(modules)) => module
                        .and_then(|base| modules.iter().find(|listed| listed.base == base))
                        .map(|listed| PickedModule {
                            base: listed.base,
                            shown: read_module_detail(cpu, space, listed, module_detail),
                        }),
                    (ProcessDetail::Modules, ModuleRead::NoUserSpace | ModuleRead::Unreadable)
                    | (ProcessDetail::MemoryMap | ProcessDetail::Threads, _) => None,
                };
                Some(ProcessModules {
                    eprocess: process.eprocess,
                    read,
                    map,
                    threads,
                    picked,
                })
            });
            let tree = process_tree(&processes);
            SystemListing::Processes(ProcessListing {
                processes,
                tree,
                modules,
            })
        }
        SystemView::Drivers { selected, detail } => {
            match read_drivers(cpu, kernel, &kernel_modules, object_era, object_root) {
                Some((mut drivers, default)) => {
                    let picked = match (*object_era, *object_root) {
                        (Some(era), Some(root)) => selected.and_then(|driver_object| {
                            read_driver_detail(
                                cpu,
                                &kernel_modules,
                                driver_object,
                                detail,
                                era,
                                root,
                                default,
                            )
                        }),
                        (Some(_), None) | (None, _) => None,
                    };
                    drivers.sort_by(|a, b| a.name.cmp(&b.name));
                    SystemListing::Drivers(DriverListing { drivers, picked })
                }
                None => SystemListing::NoWindowsKernel(KernelMiss::NoObjectNamespace),
            }
        }
        SystemView::Ssdt => {
            let scene = SsdtScene {
                kernel,
                modules: &kernel_modules,
                processes: &processes,
            };
            match read_ssdt(cpu, &scene, ssdt, peb_offset, wow64_offset) {
                Some(listing) => SystemListing::Ssdt(listing),
                None => SystemListing::NoWindowsKernel(KernelMiss::NoSsdt),
            }
        }
    };
    WindowsRead { symbols, system, layouts: Some(layouts), through, elsewhere }
}

// ───────────────────────────── drivers ──────────────────────────────

/// The `DRIVER_OBJECT` directories, plus `\Device` for drivers reached only
/// through a device.
const DRIVER_DIRECTORIES: [&str; 2] = ["Driver", "FileSystem"];

/// The `OBJECT_HEADER` of the object whose body is at `body`.
fn object_header(body: u64) -> u64 {
    body.wrapping_sub(drivers::OBJECT_BODY_OFFSET)
}

/// The `OBJECT_HEADER_NAME_INFO` of the object at `body`, when it has a name.
fn name_info_of<G: GuestMemory>(
    cpu: &mut G,
    era: drivers::ObjectHeaderEra,
    body: u64,
) -> Option<u64> {
    let header = object_header(body);
    let byte_at = match era {
        drivers::ObjectHeaderEra::Legacy => header.wrapping_add(drivers::LEGACY_NAME_INFO_OFFSET),
        drivers::ObjectHeaderEra::Modern => header.wrapping_add(drivers::MODERN_INFO_MASK_OFFSET),
    };
    let delta = drivers::name_info_delta(era, read_u8(cpu, Root::Live, byte_at)?)?;
    Some(header.checked_sub(delta)?)
}

/// The name of the object at `body` (its `NAME_INFO.Name`), when it has one.
fn object_name<G: GuestMemory>(
    cpu: &mut G,
    era: drivers::ObjectHeaderEra,
    body: u64,
) -> Option<String> {
    let name_info = name_info_of(cpu, era, body)?;
    read_unicode_string(cpu, Root::Live, name_info.wrapping_add(drivers::NAME_INFO_NAME))
}

/// The directory an object belongs to: its `NAME_INFO.Directory`.
fn object_directory<G: GuestMemory>(
    cpu: &mut G,
    era: drivers::ObjectHeaderEra,
    body: u64,
) -> Option<u64> {
    let name_info = name_info_of(cpu, era, body)?;
    read_u64(cpu, Root::Live, name_info).and_then(kernel_pointer)
}

/// Every object body in an `OBJECT_DIRECTORY` at `directory`, with its name.
fn enumerate_directory<G: GuestMemory>(
    cpu: &mut G,
    era: drivers::ObjectHeaderEra,
    directory: u64,
) -> Vec<(Option<String>, u64)> {
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    for bucket in 0..drivers::OBJECT_DIRECTORY_BUCKETS as u64 {
        // `bucket` stays below OBJECT_DIRECTORY_BUCKETS: the product cannot wrap.
        let bucket_at = directory.wrapping_add(bucket.wrapping_mul(8));
        let Some(mut entry) = read_u64(cpu, Root::Live, bucket_at) else {
            continue;
        };
        for _ in 0..MAX_OBJECTS_PER_BUCKET {
            let Some(entry_addr) = kernel_pointer(entry) else {
                break;
            };
            if !visited.insert(entry_addr) {
                break;
            }
            let object = read_u64(
                cpu,
                Root::Live,
                entry_addr.wrapping_add(drivers::DIRECTORY_ENTRY_OBJECT),
            )
            .and_then(kernel_pointer);
            if let Some(object) = object {
                out.push((object_name(cpu, era, object), object));
            }
            let chain = entry_addr.wrapping_add(drivers::DIRECTORY_ENTRY_CHAIN);
            match read_u64(cpu, Root::Live, chain) {
                Some(0) | None => break,
                Some(next) => entry = next,
            }
        }
    }
    out
}

/// A named child directory of `directory`.
fn child_directory<G: GuestMemory>(
    cpu: &mut G,
    era: drivers::ObjectHeaderEra,
    directory: u64,
    name: &str,
) -> Option<u64> {
    enumerate_directory(cpu, era, directory)
        .into_iter()
        .find(|(child_name, _)| child_name.as_deref() == Some(name))
        .map(|(_, body)| body)
}

/// A corrupt bucket chain stops here.
const MAX_OBJECTS_PER_BUCKET: usize = 1 << 16;

/// Finds the object namespace root `\` and the `OBJECT_HEADER` era, from the
/// kernel export `IoDriverObjectType` (a pointer to the Driver type object,
/// whose name is "Driver"). Caches both in `object_era`/`object_root`.
fn object_namespace<G: GuestMemory>(
    cpu: &mut G,
    kernel: &Kernel,
    object_era: &mut Option<drivers::ObjectHeaderEra>,
    object_root: &mut Option<u64>,
) -> Option<(drivers::ObjectHeaderEra, u64)> {
    if let (Some(era), Some(root)) = (*object_era, *object_root) {
        // Still a directory holding a child named "Driver"?
        if child_directory(cpu, era, root, "Driver").is_some() {
            return Some((era, root));
        }
    }
    let type_export = export(cpu, &kernel.image, "IoDriverObjectType")?;
    let type_object = read_u64(cpu, Root::Live, type_export).and_then(kernel_pointer)?;
    for era in [drivers::ObjectHeaderEra::Legacy, drivers::ObjectHeaderEra::Modern] {
        if object_name(cpu, era, type_object).as_deref() != Some("Driver") {
            continue;
        }
        // The type object lives in \ObjectTypes; its directory's directory is
        // the root \.
        let Some(object_types) = object_directory(cpu, era, type_object) else {
            continue;
        };
        let root = object_directory(cpu, era, object_types)?;
        *object_era = Some(era);
        *object_root = Some(root);
        return Some((era, root));
    }
    None
}

/// The loaded module whose image covers `addr`.
fn module_of(addr: u64, modules: &[WinModule]) -> Option<&WinModule> {
    modules
        .iter()
        .find(|m| (m.base..m.base.wrapping_add(u64::from(m.size))).contains(&addr))
}

/// Where a kernel code pointer leads. `own` is the owning driver's module;
/// `default` is the kernel's shared handler value (so it is not called a hook).
fn classify_pointer(
    addr: u64,
    own: Option<&WinModule>,
    modules: &[WinModule],
    default: Option<u64>,
) -> PointerSite {
    if addr == 0 {
        return PointerSite::NoModule;
    }
    if Some(addr) == default {
        return PointerSite::KernelDefault;
    }
    if let Some(module) = own {
        if (module.base..module.base.wrapping_add(u64::from(module.size))).contains(&addr) {
            return PointerSite::OwnImage;
        }
    }
    match module_of(addr, modules) {
        // `module_of` found `addr` inside the module: at or past its base.
        Some(module) => PointerSite::InModule {
            base: module.base,
            off: addr.wrapping_sub(module.base),
        },
        None => PointerSite::NoModule,
    }
}

/// The value most drivers' IRP handlers share that lies inside the kernel
/// image: `IopInvalidDeviceRequest`, found without its symbol.
fn kernel_default_handler(objects: &[drivers::DriverObject], kernel: &Kernel) -> Option<u64> {
    use std::collections::HashMap;
    let range = kernel.image.base..kernel.image.base.wrapping_add(u64::from(kernel.image.size));
    let mut counts: HashMap<u64, usize> = HashMap::new();
    for object in objects {
        for &handler in &object.major {
            if range.contains(&handler) {
                let count = counts.entry(handler).or_default();
                *count = count.saturating_add(1);
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(handler, _)| handler)
}

/// Every driver object in the guest, and the kernel's shared default handler.
/// `None` only when the object namespace cannot be reached.
fn read_drivers<G: GuestMemory>(
    cpu: &mut G,
    kernel: &Kernel,
    modules: &[WinModule],
    object_era: &mut Option<drivers::ObjectHeaderEra>,
    object_root: &mut Option<u64>,
) -> Option<(Vec<WinDriver>, Option<u64>)> {
    let (era, root) = object_namespace(cpu, kernel, object_era, object_root)?;

    // Each driver object, with the directory name it was found under and its
    // decoded body. A driver object is kept once, by its address.
    let mut found: Vec<(u64, bool, drivers::DriverObject)> = Vec::new();
    let mut seen = HashSet::new();
    for directory_name in DRIVER_DIRECTORIES {
        let Some(directory) = child_directory(cpu, era, root, directory_name) else {
            continue;
        };
        for (_, body) in enumerate_directory(cpu, era, directory) {
            if found.len() >= drivers::MAX_DRIVERS {
                break;
            }
            if !seen.insert(body) {
                continue;
            }
            if let Some(object) = read_exact(cpu, Root::Live, body, drivers::DRIVER_OBJECT_SIZE)
                .and_then(|b| drivers::parse_driver(&b))
            {
                found.push((body, true, object));
            }
        }
    }
    // Drivers reached only through a named device in \Device.
    if let Some(device_dir) = child_directory(cpu, era, root, "Device") {
        for (_, device_body) in enumerate_directory(cpu, era, device_dir) {
            if found.len() >= drivers::MAX_DRIVERS {
                break;
            }
            let Some(device) = read_exact(cpu, Root::Live, device_body, drivers::DEVICE_OBJECT_SPAN)
                .and_then(|b| drivers::parse_device(&b))
            else {
                continue;
            };
            let Some(driver_object) = kernel_pointer(device.driver) else {
                continue;
            };
            if !seen.insert(driver_object) {
                continue;
            }
            if let Some(object) = read_exact(cpu, Root::Live, driver_object, drivers::DRIVER_OBJECT_SIZE)
                .and_then(|b| drivers::parse_driver(&b))
            {
                found.push((driver_object, false, object));
            }
        }
    }

    let objects: Vec<drivers::DriverObject> = found.iter().map(|(_, _, o)| o.clone()).collect();
    let default = kernel_default_handler(&objects, kernel);

    let drivers_list = found
        .iter()
        .map(|(driver_object, in_namespace, object)| {
            let name = object_name(cpu, era, *driver_object)
                .map(|leaf| format!("\\{}\\{leaf}", namespace_prefix(*in_namespace)))
                .unwrap_or_else(|| format!("{driver_object:#X}"));
            let module = module_of(object.start, modules);
            let device_count = device_chain(cpu, object.device_object).len();
            let own = module;
            let flagged = module.is_none()
                || object
                    .major
                    .iter()
                    .chain([&object.start_io, &object.unload])
                    .any(|&addr| {
                        addr != 0 && classify_pointer(addr, own, modules, default) == PointerSite::NoModule
                    });
            WinDriver {
                driver_object: *driver_object,
                name,
                module: module.map(|m| m.base),
                module_name: module.and_then(|m| m.name.clone()),
                in_namespace: *in_namespace,
                device_count,
                flagged,
            }
        })
        .collect();
    Some((drivers_list, default))
}

/// A driver's directory for its namespace name: not-in-namespace drivers reach
/// us through \Device, so they are shown as living there.
fn namespace_prefix(in_namespace: bool) -> &'static str {
    if in_namespace {
        "Driver"
    } else {
        "Device"
    }
}

/// The device objects of the driver at `device_object`, following `NextDevice`.
fn device_chain<G: GuestMemory>(
    cpu: &mut G,
    device_object: u64,
) -> Vec<(u64, drivers::DeviceObject)> {
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    let mut at = device_object;
    for _ in 0..drivers::MAX_DEVICES_PER_DRIVER {
        let Some(addr) = kernel_pointer(at) else {
            break;
        };
        if !visited.insert(addr) {
            break;
        }
        let Some(device) = read_exact(cpu, Root::Live, addr, drivers::DEVICE_OBJECT_SPAN)
            .and_then(|b| drivers::parse_device(&b))
        else {
            break;
        };
        let next = device.next;
        out.push((addr, device));
        at = next;
    }
    out
}

/// The picked driver's dispatch table or its devices.
#[allow(clippy::too_many_arguments)]
fn read_driver_detail<G: GuestMemory>(
    cpu: &mut G,
    modules: &[WinModule],
    driver_object: u64,
    detail: DriverDetail,
    era: drivers::ObjectHeaderEra,
    root: u64,
    default: Option<u64>,
) -> Option<PickedDriver> {
    let object =
        read_exact(cpu, Root::Live, driver_object, drivers::DRIVER_OBJECT_SIZE).and_then(|b| drivers::parse_driver(&b))?;
    let own = module_of(object.start, modules);
    let shown = match detail {
        DriverDetail::Dispatch => {
            let mut entries = Vec::new();
            for (index, &addr) in object.major.iter().enumerate() {
                entries.push(DispatchEntry {
                    label: drivers::major_function_name(index).to_owned(),
                    addr,
                    site: classify_pointer(addr, own, modules, default),
                });
            }
            for (label, addr) in [
                ("StartIo", object.start_io),
                ("Unload", object.unload),
                ("Init", object.init),
            ] {
                entries.push(DispatchEntry {
                    label: label.to_owned(),
                    addr,
                    site: classify_pointer(addr, own, modules, default),
                });
            }
            for entry in fast_io_entries(cpu, object.fast_io, own, modules, default) {
                entries.push(entry);
            }
            DriverShown::Dispatch(entries)
        }
        DriverDetail::Devices => {
            let links = symbolic_links(cpu, era, root);
            let rows = device_chain(cpu, object.device_object)
                .into_iter()
                .map(|(addr, device)| {
                    let name = object_name(cpu, era, addr);
                    let attached_above = attached_stack(cpu, era, device.attached);
                    let device_links = name
                        .as_ref()
                        .map(|device_name| {
                            links
                                .iter()
                                .filter(|(_, target)| target == device_name)
                                .map(|(link, _)| link.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    DeviceRow {
                        name,
                        device_type: drivers::device_type_name(device.device_type),
                        characteristics: device.characteristics,
                        attached_above,
                        links: device_links,
                    }
                })
                .collect();
            DriverShown::Devices(rows)
        }
    };
    Some(PickedDriver {
        driver_object,
        shown,
    })
}

/// The `FastIoDispatch` entries, when the table reads: `SizeOfFastIoDispatch`
/// gives the count, then one pointer each.
fn fast_io_entries<G: GuestMemory>(
    cpu: &mut G,
    fast_io: u64,
    own: Option<&WinModule>,
    modules: &[WinModule],
    default: Option<u64>,
) -> Vec<DispatchEntry> {
    const NAMES: [&str; 27] = [
        "FastIoCheckIfPossible",
        "FastIoRead",
        "FastIoWrite",
        "FastIoQueryBasicInfo",
        "FastIoQueryStandardInfo",
        "FastIoLock",
        "FastIoUnlockSingle",
        "FastIoUnlockAll",
        "FastIoUnlockAllByKey",
        "FastIoDeviceControl",
        "AcquireFileForNtCreateSection",
        "ReleaseFileForNtCreateSection",
        "FastIoDetachDevice",
        "FastIoQueryNetworkOpenInfo",
        "AcquireForModWrite",
        "MdlRead",
        "MdlReadComplete",
        "PrepareMdlWrite",
        "MdlWriteComplete",
        "FastIoReadCompressed",
        "FastIoWriteCompressed",
        "MdlReadCompleteCompressed",
        "MdlWriteCompleteCompressed",
        "FastIoQueryOpen",
        "ReleaseForModWrite",
        "AcquireForCcFlush",
        "ReleaseForCcFlush",
    ];
    let Some(fast_io) = kernel_pointer(fast_io) else {
        return Vec::new();
    };
    let Some(size) = read_u32(cpu, Root::Live, fast_io) else {
        return Vec::new();
    };
    // SizeOfFastIoDispatch counts the whole structure, header included.
    let count = (size as usize).saturating_sub(8) / 8;
    NAMES
        .iter()
        .take(count)
        .enumerate()
        .filter_map(|(i, name)| {
            // `i` is below NAMES.len(): the offset cannot wrap.
            let slot = fast_io.wrapping_add(8).wrapping_add((i as u64).wrapping_mul(8));
            let addr = read_u64(cpu, Root::Live, slot)?;
            (addr != 0).then(|| DispatchEntry {
                label: format!("FastIo.{name}"),
                addr,
                site: classify_pointer(addr, own, modules, default),
            })
        })
        .collect()
}

/// The named drivers of the devices attached above a device, bottom-up.
fn attached_stack<G: GuestMemory>(
    cpu: &mut G,
    era: drivers::ObjectHeaderEra,
    attached: u64,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    let mut at = attached;
    for _ in 0..drivers::MAX_ATTACH_DEPTH {
        let Some(addr) = kernel_pointer(at) else {
            break;
        };
        if !visited.insert(addr) {
            break;
        }
        let Some(device) = read_exact(cpu, Root::Live, addr, drivers::DEVICE_OBJECT_SPAN)
            .and_then(|b| drivers::parse_device(&b))
        else {
            break;
        };
        let label = kernel_pointer(device.driver)
            .and_then(|driver| object_name(cpu, era, driver))
            .map(|leaf| format!("\\Driver\\{leaf}"))
            .unwrap_or_else(|| format!("{addr:#X}"));
        out.push(label);
        at = device.attached;
    }
    out
}

/// The symbolic links under `\GLOBAL??` (or `\??`): (link name, target).
fn symbolic_links<G: GuestMemory>(
    cpu: &mut G,
    era: drivers::ObjectHeaderEra,
    root: u64,
) -> Vec<(String, String)> {
    let directory = child_directory(cpu, era, root, "GLOBAL??")
        .or_else(|| child_directory(cpu, era, root, "??"));
    let Some(directory) = directory else {
        return Vec::new();
    };
    enumerate_directory(cpu, era, directory)
        .into_iter()
        .filter_map(|(name, body)| {
            let name = name?;
            let target = read_unicode_string(
                cpu,
                Root::Live,
                body.wrapping_add(drivers::SYMBOLIC_LINK_TARGET),
            )?;
            Some((format!("\\??\\{name}"), target))
        })
        .collect()
}

// ───────────────────────────── SSDT ──────────────────────────────

/// The service descriptors found in one kernel and the services' stub names,
/// kept for as long as the kernel is.
#[derive(Default)]
struct SsdtCache {
    /// `KeServiceDescriptorTable`.
    kernel: Option<FoundDescriptor>,
    /// `KeServiceDescriptorTableShadow`.
    shadow: Option<FoundDescriptor>,
    stubs: StubNames,
}

/// A descriptor's address, and how it was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FoundDescriptor {
    address: u64,
    source: TableSource,
}

/// The services' names by index, from the system-call stubs that call them.
#[derive(Default)]
struct StubNames {
    /// `ntdll.dll`'s.
    kernel: BTreeMap<u32, String>,
    /// `win32u.dll`'s.
    gui: BTreeMap<u32, String>,
}

/// The first build with `win32u.dll` (Windows 10 1607); before it the GUI
/// stubs lie unexported in `user32.dll` and `gdi32.dll`.
const WIN32U_FIRST_BUILD: u32 = 14393;

/// The processes a session's GUI table is read through first, after the
/// running one.
const SESSION_PROCESSES: [&str; 3] = ["csrss.exe", "winlogon.exe", "explorer.exe"];

/// What an SSDT read looks across: the kernel, its loaded modules and the
/// processes.
struct SsdtScene<'a> {
    kernel: &'a Kernel,
    modules: &'a [WinModule],
    processes: &'a [WinProcess],
}

/// A descriptor found, and what it holds.
#[derive(Clone, Copy)]
struct LocatedDescriptor {
    found: FoundDescriptor,
    descriptor: ssdt::Descriptor,
}

/// A descriptor found, and the table it describes.
struct LocatedTable {
    located: LocatedDescriptor,
    table: ServiceTable,
}

/// The kernel's system-service table and the GUI's, each service classified
/// against the loaded modules, named by its stub and checked for an inline
/// hook. `None` when no `KeServiceDescriptorTable` is found. The descriptors
/// found and the stub names read are kept in `cache`.
fn read_ssdt<G: GuestMemory>(
    cpu: &mut G,
    scene: &SsdtScene<'_>,
    cache: &mut SsdtCache,
    peb_offset: &mut Option<u64>,
    wow64_offset: &mut Option<u64>,
) -> Option<SsdtListing> {
    let image = &scene.kernel.image;
    // The kernel's code is scanned at most once a read, for both tables.
    let mut code = None;
    let owners = [kernel_module(image, scene.modules)];
    let kernel_table = kernel_service_table(cpu, image, &owners, &mut cache.kernel, &mut code)?;
    read_stub_names(cpu, scene, &mut cache.stubs, peb_offset, wow64_offset);
    let kernel = list_services(
        cpu,
        Root::Live,
        &kernel_table,
        &owners,
        scene.modules,
        &cache.stubs.kernel,
    );
    let gui = gui_services(
        cpu,
        scene,
        kernel_table.located.descriptor,
        &mut cache.shadow,
        &mut code,
        &cache.stubs.gui,
    );
    Some(SsdtListing { kernel, gui })
}

/// The kernel's own entry in `modules`, or one made from its image when the
/// list does not hold it.
fn kernel_module(image: &PeImage, modules: &[WinModule]) -> WinModule {
    module_of(image.base, modules).cloned().unwrap_or(WinModule {
        base: image.base,
        size: image.size,
        name: None,
        path: None,
        bits: ModuleBits::Bits64,
    })
}

/// Whether `module` is one of the GUI's kernel images: `win32k.sys`, or
/// `win32kbase.sys` and `win32kfull.sys` since Windows 10.
fn is_win32k(module: &WinModule) -> bool {
    module
        .name
        .as_deref()
        .and_then(|name| name.get(..6))
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("win32k"))
}

/// The processes the GUI table is read through, in the order tried: the
/// running one, then `csrss.exe`, `winlogon.exe` and `explorer.exe`, then
/// the rest. Never System, whose space maps no session.
fn session_candidates(processes: &[WinProcess]) -> Vec<&WinProcess> {
    let user: Vec<&WinProcess> = processes.iter().filter(|p| p.pid != SYSTEM_PID).collect();
    let named = |process: &WinProcess, name: &str| process.name.eq_ignore_ascii_case(name);
    let mut order: Vec<&WinProcess> = user.iter().copied().filter(|p| p.current).collect();
    for name in SESSION_PROCESSES {
        order.extend(user.iter().copied().filter(|p| !p.current && named(p, name)));
    }
    order.extend(
        user.iter()
            .copied()
            .filter(|p| !p.current && !SESSION_PROCESSES.iter().any(|name| named(p, name))),
    );
    order
}

/// The page tables a process's own space is read through: the processor's
/// for the running process, its `DirectoryTableBase` for any other.
fn process_root(process: &WinProcess) -> Root {
    if process.current {
        Root::Live
    } else {
        Root::Process(process.cr3)
    }
}

/// Fills whichever of `names`' tables is empty from the stubs of the first
/// processes, in session order, that load `ntdll.dll` (the kernel's) and
/// `win32u.dll` (the GUI's, on a build that has it). A table no process
/// gave yet (early in boot) is looked for again on the next read.
fn read_stub_names<G: GuestMemory>(
    cpu: &mut G,
    scene: &SsdtScene<'_>,
    names: &mut StubNames,
    peb_offset: &mut Option<u64>,
    wow64_offset: &mut Option<u64>,
) {
    let gui_stubs_exported = scene.kernel.build.is_none_or(|build| build >= WIN32U_FIRST_BUILD);
    for process in session_candidates(scene.processes) {
        let gui_wanted = gui_stubs_exported && names.gui.is_empty();
        if !names.kernel.is_empty() && !gui_wanted {
            return;
        }
        let ModuleRead::Listed(modules) =
            process_modules(cpu, scene.processes, process, peb_offset, wow64_offset)
        else {
            continue;
        };
        let root = Root::Process(process.cr3);
        let native = |name: &str| {
            modules
                .iter()
                .find(|m| m.bits == ModuleBits::Bits64 && m.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        };
        if names.kernel.is_empty() {
            if let Some(ntdll) = native("ntdll.dll") {
                names.kernel = stubs_of(cpu, root, ntdll).kernel;
            }
        }
        if gui_wanted {
            if let Some(win32u) = native("win32u.dll") {
                names.gui = stubs_of(cpu, root, win32u).gui;
            }
        }
    }
}

/// The services `module`'s `Nt*` exports call, by the system-call stub each
/// leads to; an export that is no stub is passed by.
fn stubs_of<G: GuestMemory>(cpu: &mut G, root: Root, module: &WinModule) -> StubNames {
    let mut names = StubNames::default();
    let ExportRead::Listed(exports) = read_module_exports(cpu, root, module) else {
        return names;
    };
    for export in exports {
        let Some(name) = export.name else {
            continue;
        };
        let ExportTarget::Rva(rva) = export.target else {
            continue;
        };
        if !name.starts_with("Nt") {
            continue;
        }
        let at = module.base.wrapping_add(u64::from(rva));
        let Some(code) = read_exact(cpu, root, at, ssdt::STUB_BYTES) else {
            continue;
        };
        let table = match ssdt::stub_service(&code) {
            Some(ssdt::StubService::Kernel(index)) => names.kernel.entry(index),
            Some(ssdt::StubService::Gui(index)) => names.gui.entry(index),
            None => continue,
        };
        table.or_insert(name);
    }
    names
}

/// One table's services, read through `root`: each routine classified
/// against `owners` (the images the table belongs to) and `modules`, named
/// from `names`, and checked for a first instruction that leaves `owners`.
fn list_services<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    located: &LocatedTable,
    owners: &[WinModule],
    modules: &[WinModule],
    names: &BTreeMap<u32, String>,
) -> ServiceTableRead {
    let table = &located.table;
    let mut any_hooked = false;
    let services = table
        .entries
        .iter()
        .enumerate()
        .map(|(index, &entry)| {
            // `parse_descriptor` caps the entries at MAX_SERVICES: `index`
            // fits a u32.
            let index = index as u32;
            let routine = ssdt::routine_of(table.base, entry, table.decoding.encoding);
            let site = classify_pointer(routine, module_of(routine, owners), modules, None);
            let inline = match site {
                PointerSite::OwnImage => inline_jump(cpu, root, routine, owners, modules),
                PointerSite::InModule { .. } | PointerSite::KernelDefault | PointerSite::NoModule => None,
            };
            if site != PointerSite::OwnImage || inline.is_some() {
                any_hooked = true;
            }
            SsdtService {
                index,
                routine,
                site,
                module_name: module_of(routine, modules).and_then(|m| m.name.clone()),
                stub_name: names.get(&index).cloned(),
                inline,
            }
        })
        .collect();
    let place = match module_of(table.base, owners) {
        Some(_) => TablePlace::InOwner,
        None => TablePlace::Outside,
    };
    ServiceTableRead {
        source: located.located.found.source,
        base: table.base,
        place,
        services,
        any_hooked,
    }
}

/// Where the routine at `routine` first jumps, read through `root`, when a
/// jump there leaves `owners`: an inline hook.
fn inline_jump<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    routine: u64,
    owners: &[WinModule],
    modules: &[WinModule],
) -> Option<InlineJump> {
    let code = read_exact(cpu, root, routine, ssdt::JUMP_BYTES)?;
    let target = match ssdt::first_jump(&code, routine)? {
        ssdt::FirstJump::To(target) => target,
        ssdt::FirstJump::Through(slot) => read_u64(cpu, root, slot)?,
    };
    if module_of(target, owners).is_some() {
        return None;
    }
    Some(InlineJump {
        target,
        module_name: module_of(target, modules).and_then(|m| m.name.clone()),
    })
}

/// The GUI's table, read through the first process in session order whose
/// read of it decodes with more than half its routines inside a `win32k`
/// image; or why it was not read.
fn gui_services<G: GuestMemory>(
    cpu: &mut G,
    scene: &SsdtScene<'_>,
    kernel: ssdt::Descriptor,
    kept: &mut Option<FoundDescriptor>,
    code: &mut Option<ssdt::LeaTargets>,
    names: &BTreeMap<u32, String>,
) -> GuiServices {
    let owners: Vec<WinModule> = scene.modules.iter().filter(|m| is_win32k(m)).cloned().collect();
    if owners.is_empty() {
        return GuiServices::Missing(GuiMiss::NoWin32k);
    }
    let Some(located) = shadow_descriptor(cpu, &scene.kernel.image, kernel, kept, code) else {
        return GuiServices::Missing(GuiMiss::NoShadowDescriptor);
    };
    for process in session_candidates(scene.processes) {
        let root = process_root(process);
        let Some(table) = read_table(cpu, root, located.descriptor, &owners) else {
            continue;
        };
        if table.decoding.inside.saturating_mul(2) > table.entries.len() {
            let located = LocatedTable { located, table };
            return GuiServices::Listed(list_services(cpu, root, &located, &owners, scene.modules, names));
        }
    }
    GuiServices::Missing(GuiMiss::NoSessionProcess)
}

/// The GUI table's descriptor in `KeServiceDescriptorTableShadow`: through
/// the shadow kept while its second descriptor still reads (a kernel PDB
/// seeds it), else through the `lea r11` target in the kernel's code whose
/// first descriptor is the kernel table's `kernel` and whose second reads.
/// The shadow found is kept in `kept`.
fn shadow_descriptor<G: GuestMemory>(
    cpu: &mut G,
    image: &PeImage,
    kernel: ssdt::Descriptor,
    kept: &mut Option<FoundDescriptor>,
    code: &mut Option<ssdt::LeaTargets>,
) -> Option<LocatedDescriptor> {
    let known = (*kept).and_then(|found| {
        let bytes = read_exact(cpu, Root::Live, found.address, ssdt::SHADOW_BYTES)?;
        let descriptor = ssdt::known_shadow_gui_descriptor(&bytes)?;
        Some(LocatedDescriptor { found, descriptor })
    });
    let located = match known {
        Some(located) => located,
        None => lea_targets(cpu, image, code)
            .r11
            .iter()
            .copied()
            .filter(|&address| image_holds(image, address))
            .find_map(|address| {
                let bytes = read_exact(cpu, Root::Live, address, ssdt::SHADOW_BYTES)?;
                let descriptor = ssdt::shadow_gui_descriptor(&bytes, kernel)?;
                Some(LocatedDescriptor {
                    found: FoundDescriptor {
                        address,
                        source: TableSource::Code,
                    },
                    descriptor,
                })
            })?,
    };
    *kept = Some(located.found);
    Some(located)
}

/// A service table, read.
struct ServiceTable {
    /// `KiServiceTable` or `W32pServiceTable` itself.
    base: u64,
    entries: Vec<i32>,
    /// The encoding its entries decode with, and how many of their routines
    /// land inside the table's owners under it.
    decoding: ssdt::Decoding,
}

/// Whether `addr` lies inside `image`.
fn image_holds(image: &PeImage, addr: u64) -> bool {
    (image.base..image.base.wrapping_add(u64::from(image.size))).contains(&addr)
}

/// The table `descriptor` describes, read through `root`, its entries decoded
/// by the encoding that puts the most routines inside `owners`: the wrong
/// one scatters them.
fn read_table<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    descriptor: ssdt::Descriptor,
    owners: &[WinModule],
) -> Option<ServiceTable> {
    let base = kernel_pointer(descriptor.service_table)?;
    // `parse_descriptor` caps the limit at MAX_SERVICES: the length cannot wrap.
    let raw = read_exact(cpu, root, base, (descriptor.limit as usize).wrapping_mul(4))?;
    let entries: Vec<i32> = raw
        .chunks_exact(4)
        .filter_map(|chunk| <[u8; 4]>::try_from(chunk).ok())
        .map(i32::from_le_bytes)
        .collect();
    let decoding = ssdt::decode(base, &entries, |routine| module_of(routine, owners).is_some());
    Some(ServiceTable {
        base,
        entries,
        decoding,
    })
}

/// The kernel descriptor `found` names, and its table.
fn kernel_table_at<G: GuestMemory>(
    cpu: &mut G,
    found: FoundDescriptor,
    owners: &[WinModule],
) -> Option<LocatedTable> {
    let bytes = read_exact(cpu, Root::Live, found.address, ssdt::DESCRIPTOR_BYTES)?;
    let descriptor = ssdt::parse_descriptor(&bytes)?;
    let table = read_table(cpu, Root::Live, descriptor, owners)?;
    Some(LocatedTable {
        located: LocatedDescriptor { found, descriptor },
        table,
    })
}

/// `KeServiceDescriptorTable` and its table: the descriptor kept while it
/// still reads (a kernel PDB seeds it), else the kernel's export (5.2 SP1 is
/// the only x64 kernel with one), else the `lea r10` target in the kernel's
/// code that reads as the kernel's own service table. The descriptor found is
/// kept in `kept`. A descriptor kept or exported is taken whatever its
/// routines point at, so a table hooked wholesale is still listed (and
/// flagged).
fn kernel_service_table<G: GuestMemory>(
    cpu: &mut G,
    image: &PeImage,
    owners: &[WinModule],
    kept: &mut Option<FoundDescriptor>,
    code: &mut Option<ssdt::LeaTargets>,
) -> Option<LocatedTable> {
    let known = (*kept).and_then(|found| kernel_table_at(cpu, found, owners));
    let located = match known {
        Some(located) => located,
        None => {
            let exported = export(cpu, image, "KeServiceDescriptorTable").and_then(|address| {
                let found = FoundDescriptor {
                    address,
                    source: TableSource::Export,
                };
                kernel_table_at(cpu, found, owners)
            });
            match exported {
                Some(located) => located,
                None => kernel_table_from_code(cpu, image, owners, code)?,
            }
        }
    };
    *kept = Some(located.located.found);
    Some(located)
}

/// The descriptor a `lea r10, [rip + disp32]` in the kernel's executable
/// sections loads (`KiSystemServiceRepeat` loads `KeServiceDescriptorTable`
/// so) whose `KiServiceTable` lies in the kernel image and whose routines
/// land in the kernel image most often — at least for half its services,
/// which no other datum a `lea r10` loads does.
fn kernel_table_from_code<G: GuestMemory>(
    cpu: &mut G,
    image: &PeImage,
    owners: &[WinModule],
    code: &mut Option<ssdt::LeaTargets>,
) -> Option<LocatedTable> {
    lea_targets(cpu, image, code)
        .r10
        .iter()
        .copied()
        .filter(|&address| image_holds(image, address))
        .filter_map(|address| {
            let found = FoundDescriptor {
                address,
                source: TableSource::Code,
            };
            kernel_table_at(cpu, found, owners)
        })
        .filter(|located| {
            let table = &located.table;
            image_holds(image, table.base) && table.decoding.inside.saturating_mul(2) > table.entries.len()
        })
        .max_by_key(|located| located.table.decoding.inside)
}

/// The `lea r10` and `lea r11` targets in the kernel's code, scanned into
/// `code` the first time they are asked for.
fn lea_targets<'a, G: GuestMemory>(
    cpu: &mut G,
    image: &PeImage,
    code: &'a mut Option<ssdt::LeaTargets>,
) -> &'a ssdt::LeaTargets {
    code.get_or_insert_with(|| scan_kernel_code(cpu, image))
}

/// Every `lea r10/r11, [rip + disp32]` target in the kernel's executable
/// sections; none when its headers do not read.
fn scan_kernel_code<G: GuestMemory>(cpu: &mut G, image: &PeImage) -> ssdt::LeaTargets {
    let Some(sections) =
        read_exact(cpu, Root::Live, image.base, HEADER_BYTES).and_then(|header| pe::parse_sections(&header))
    else {
        return ssdt::LeaTargets::default();
    };
    let mut scan = ssdt::LeaRipScan::default();
    let mut page = vec![0u8; PAGE_BYTES as usize];
    for section in sections.iter().filter(|section| section.executable) {
        let size = match section.virtual_size {
            0 => section.raw_size,
            size => size,
        };
        let start = image.base.wrapping_add(u64::from(section.rva));
        // A section that would run past the top of memory is not scanned.
        let Some(end) = start.checked_add(u64::from(size)) else {
            continue;
        };
        let mut at = start;
        while at < end {
            // At most the rest of `at`'s page, and no further than `end`.
            let take = PAGE_BYTES
                .wrapping_sub(at & (PAGE_BYTES - 1))
                .min(end.wrapping_sub(at));
            let Some(chunk) = page.get_mut(..take as usize) else {
                break;
            };
            // A paged-out or discarded page is passed by.
            if read_into(cpu, Root::Live, at, chunk) {
                scan.feed(chunk, at);
            }
            // `take <= end - at`: the sum cannot wrap.
            at = at.wrapping_add(take);
        }
    }
    scan.targets()
}

/// Whom the CPU tab is asked to look from: a process (one of its threads with
/// saved user state), or one thread of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewTarget {
    Process { eprocess: u64 },
    Thread { eprocess: u64, tid: u64 },
}

/// A thread the CPU tab looks from: its process, and its user registers as
/// the kernel saved them.
#[derive(Clone, Debug)]
pub(crate) struct ThreadView {
    /// `EPROCESS.ImageFileName`.
    pub(crate) process: String,
    pub(crate) pid: u64,
    pub(crate) tid: u64,
    /// Its process's address space.
    pub(crate) cr3: u64,
    pub(crate) saved: SavedRegisters,
    /// The kernel's modules and its process's, for naming addresses.
    pub(crate) symbols: SymbolIndex,
}

/// Why a process or thread cannot be shown from inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewMiss {
    NoWindowsKernel(KernelMiss),
    /// The process has exited since it was listed.
    NoSuchProcess,
    /// The thread has exited since it was listed.
    NoSuchThread,
    /// No thread of it (or not this thread) has user state saved in a frame
    /// that reads as its own: a kernel process, a thread only ever in the
    /// kernel, or a frame paged out.
    NoSavedState,
    /// The thread runs in user mode on another processor: its trap frame is
    /// an old kernel entry, and its live state is that processor's.
    RunningElsewhere { cpu: usize },
}

/// A processor running a thread in user mode, by the TEB its GS base holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LiveTeb {
    pub(crate) cpu: usize,
    pub(crate) teb: u64,
}

/// The processor running the thread whose TEB is `teb` in user mode, if one
/// does.
pub(crate) fn cpu_running(teb: Option<u64>, live: &[LiveTeb]) -> Option<usize> {
    let teb = teb?;
    live.iter().find(|running| running.teb == teb).map(|running| running.cpu)
}

impl ViewMiss {
    /// What the banner says.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::NoWindowsKernel(miss) => {
                format!("No 64-bit Windows kernel was found ({}).", miss.message())
            }
            Self::NoSuchProcess => "That process has exited.".to_owned(),
            Self::NoSuchThread => "That thread has exited.".to_owned(),
            Self::NoSavedState => {
                "No user state is saved for it: a kernel process or thread, or its trap \
                 frame is paged out."
                    .to_owned()
            }
            Self::RunningElsewhere { cpu } => {
                format!("It is running on CPU {cpu}: pick CPU {cpu} to see it live.")
            }
        }
    }
}

/// What looking from a process or thread came to.
#[derive(Clone, Debug)]
pub(crate) enum ThreadViewRead {
    /// The process (or thread) asked for is the one the shown processor
    /// runs: its live state is the view.
    Running { process: String },
    Thread(ThreadView),
    Miss(ViewMiss),
}

/// Finds `target` in the guest's process list and reads the thread to look
/// from: the one asked for, or for a process its first thread with saved user
/// state that no processor runs. A process the shown processor runs, or a
/// thread it runs in user mode, reads as `Running`: an old trap frame is never
/// shown for a thread whose state is live. `live` names the threads the
/// processors run in user mode. The walks are the ones `read_windows` makes,
/// through the same `discovery` and `layout`.
pub(crate) fn read_thread_view<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
    target: ViewTarget,
    live: &[LiveTeb],
    discovery: &mut WindowsDiscovery,
    layout: Option<&Arc<KernelLayout>>,
) -> ThreadViewRead {
    discovery.under(layout);
    match kernel_tables(cpu, view, discovery) {
        ReadThrough::ProcessorTables => {
            read_thread_view_through(cpu, view, target, live, discovery, layout, ReadThrough::ProcessorTables)
        }
        ReadThrough::KernelTables { cr3 } => {
            let mut kernel = KernelTablesView { cpu, cr3 };
            read_thread_view_through(&mut kernel, view, target, live, discovery, layout, ReadThrough::KernelTables { cr3 })
        }
    }
}

/// [`read_thread_view`] through the page tables `through` names, which
/// `Root::Live` already walks.
fn read_thread_view_through<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
    target: ViewTarget,
    live: &[LiveTeb],
    discovery: &mut WindowsDiscovery,
    layout: Option<&Arc<KernelLayout>>,
    through: ReadThrough,
) -> ThreadViewRead {
    let state = match find_kernel(cpu, view, &mut discovery.state, layout) {
        Ok(state) => state,
        Err(miss) => return ThreadViewRead::Miss(ViewMiss::NoWindowsKernel(miss)),
    };
    let KernelState {
        kernel,
        peb_offset,
        parent_offset,
        wow64_offset,
        threads: thread_layout,
        vad: _,
        object_era: _,
        object_root: _,
        ssdt: _,
        layouts: _,
    } = state;
    let kernel_modules = match kernel.loaded_modules {
        Some(head) => list_modules(cpu, Root::Live, head).unwrap_or_default(),
        None => Vec::new(),
    };
    let processes = list_processes(cpu, kernel, through.live_cr3(view), parent_offset);
    let eprocess = match target {
        ViewTarget::Process { eprocess } | ViewTarget::Thread { eprocess, .. } => eprocess,
    };
    let Some(process) = processes.iter().find(|process| process.eprocess == eprocess) else {
        return ThreadViewRead::Miss(ViewMiss::NoSuchProcess);
    };
    if process.current && matches!(target, ViewTarget::Process { .. }) {
        return ThreadViewRead::Running {
            process: process.name.clone(),
        };
    }
    // The thread the shown processor runs in user mode: its GS base is its
    // TEB.
    let shown_teb = (view.registers.cpl == 3).then_some(view.segment_bases.gs);
    let read = process_modules(cpu, &processes, process, peb_offset, wow64_offset);
    let (user_space, listed) = match &read {
        ModuleRead::Listed(modules) => (true, modules.as_slice()),
        ModuleRead::Unreadable => (true, &[][..]),
        ModuleRead::NoUserSpace => (false, &[][..]),
    };
    let ranges = code_ranges(&kernel_modules, listed);
    let threads = read_threads(
        cpu,
        process.eprocess,
        process.pid,
        process.cr3,
        user_space,
        thread_layout,
        &ranges,
    );
    let chosen = match target {
        // A thread a processor runs shows no live state through its frame.
        ViewTarget::Process { .. } => threads
            .iter()
            .find(|thread| thread.saved.is_some() && cpu_running(thread.teb, live).is_none()),
        ViewTarget::Thread { tid, .. } => match threads.iter().find(|thread| thread.tid == tid) {
            Some(thread) => match cpu_running(thread.teb, live) {
                None => Some(thread),
                Some(_) if thread.teb.is_some() && thread.teb == shown_teb => {
                    return ThreadViewRead::Running {
                        process: process.name.clone(),
                    };
                }
                Some(cpu) => return ThreadViewRead::Miss(ViewMiss::RunningElsewhere { cpu }),
            },
            None => return ThreadViewRead::Miss(ViewMiss::NoSuchThread),
        },
    };
    let Some((thread, saved)) = chosen.and_then(|thread| thread.saved.map(|saved| (thread, saved)))
    else {
        return ThreadViewRead::Miss(ViewMiss::NoSavedState);
    };
    let mut named = Vec::new();
    gather_modules(cpu, Root::Live, &mut named, &kernel_modules, ModuleSpace::Kernel);
    gather_modules(cpu, Root::Process(process.cr3), &mut named, listed, ModuleSpace::User);
    ThreadViewRead::Thread(ThreadView {
        process: process.name.clone(),
        pid: process.pid,
        tid: thread.tid,
        cr3: process.cr3,
        saved,
        symbols: SymbolIndex {
            modules: named,
            kernel: Some(kernel.image.base),
        },
    })
}

/// Appends `source` modules, read under `root`, to `into` as `space`
/// symbols with their identities, naming an unnamed module by its base so an
/// address in it still resolves.
fn gather_modules<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    into: &mut Vec<SymModule>,
    source: &[WinModule],
    space: ModuleSpace,
) {
    for module in source {
        let identity = read_module_identity(cpu, root, module.base, module.name.as_deref());
        let name = module
            .name
            .clone()
            .unwrap_or_else(|| format!("{:X}", module.base));
        into.push(SymModule {
            base: module.base,
            size: u64::from(module.size),
            name,
            space,
            identity,
        });
    }
}

/// EFER.LMA: the processor is in long mode.
const EFER_LMA: u64 = 1 << 10;

/// The kernel held in `cache` while it still stands in the guest, else the
/// one found there now, read by the kernel PDB's `layout` when it agrees
/// with memory.
fn find_kernel<'a, G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
    cache: &'a mut Option<KernelState>,
    layout: Option<&Arc<KernelLayout>>,
) -> Result<&'a mut KernelState, KernelMiss> {
    if view.efer & EFER_LMA == 0 {
        return Err(KernelMiss::NotLongMode);
    }
    let standing = cache
        .as_ref()
        .is_some_and(|state| kernel_stands(cpu, &state.kernel));
    if !standing {
        *cache = None;
        *cache = Some(discover_kernel(cpu, view, layout.map(Arc::as_ref))?);
    }
    cache.as_mut().ok_or(KernelMiss::NoKernelImage)
}

/// Whether `kernel`, found earlier, is still the guest's: its image header
/// reads and its System process still holds PID 4 at the learned offset.
fn kernel_stands<G: GuestMemory>(
    cpu: &mut G,
    kernel: &Kernel,
) -> bool {
    read_u16(cpu, Root::Live, kernel.image.base) == Some(MZ)
        && read_u64(cpu, Root::Live, kernel.system.wrapping_add(kernel.layout.pid))
            == Some(SYSTEM_PID)
}

/// Finds the kernel through the shown processor: its KPCR, the image below
/// interrupt vector 0's handler, and the System process. With the kernel
/// PDB's `layout` the System process is reached through the PDB's
/// `PsInitialSystemProcess`, and the PDB's layouts are used only when that
/// process holds PID 4 and reads "System" where the PDB puts those fields;
/// otherwise every layout is discovered.
fn discover_kernel<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
    layout: Option<&KernelLayout>,
) -> Result<KernelState, KernelMiss> {
    let kpcr = match find_kpcr(cpu, view) {
        Some(kpcr) => kpcr,
        None if kpcr_unreadable(cpu, view) => return Err(KernelMiss::KernelUnmapped),
        None => return Err(KernelMiss::NoKpcr),
    };
    let KernelImage {
        image,
        system_variable,
    } = kernel_image(cpu, kpcr).ok_or(KernelMiss::NoKernelImage)?;
    let layouts = match layout {
        Some(layout) => match pdb_kernel(cpu, &image, system_variable, layout) {
            Some(kernel) => return Ok(KernelState::seeded(cpu, kernel, layout)),
            None => KernelLayouts::PdbDisagrees,
        },
        None => KernelLayouts::Discovered,
    };
    let system = read_u64(cpu, Root::Live, system_variable)
        .and_then(kernel_pointer)
        .ok_or(KernelMiss::NoSystemProcess)?;
    let process = process_layout(cpu, system).ok_or(KernelMiss::UnknownProcessLayout)?;
    let loaded_modules = loaded_module_list(cpu, &image);
    let build = nt_build_number(cpu, &image);
    let kernel = Kernel {
        image,
        loaded_modules,
        system,
        layout: process,
        build,
    };
    Ok(KernelState::discovered(kernel, layouts))
}

/// The kernel as its PDB lays it out, when the System process the PDB's
/// `PsInitialSystemProcess` names holds PID 4 at the PDB's `UniqueProcessId`
/// and "System" at its `ImageFileName`. A symbol or member the PDB lacks
/// falls back alone to what discovery finds (spec §8.1): the export
/// `exported_system` for `PsInitialSystemProcess`, the System process's own
/// fields for the members; a lack is no disagreement.
fn pdb_kernel<G: GuestMemory>(
    cpu: &mut G,
    image: &PeImage,
    exported_system: u64,
    layout: &KernelLayout,
) -> Option<Kernel> {
    let process = &layout.process;
    let system_variable = layout
        .globals
        .initial_system_process
        .map_or(exported_system, |rva| image.base.wrapping_add(rva.0));
    let system = read_u64(cpu, Root::Live, system_variable).and_then(kernel_pointer)?;
    let lacks_a_member = process.unique_process_id.is_none()
        || process.image_file_name.is_none()
        || process.active_process_links.is_none();
    let discovered = if lacks_a_member { process_layout(cpu, system) } else { None };
    let pid = process.unique_process_id.map(|offset| offset.0).or(discovered.as_ref().map(|found| found.pid))?;
    let name = process.image_file_name.map(|offset| offset.0).or(discovered.as_ref().map(|found| found.name))?;
    let links = process.active_process_links.map(|offset| offset.0).or(discovered.as_ref().map(|found| found.links))?;
    if read_u64(cpu, Root::Live, system.wrapping_add(pid)) != Some(SYSTEM_PID)
        || read_array::<7, G>(cpu, Root::Live, system.wrapping_add(name)) != Some(*b"System\0")
    {
        return None;
    }
    let loaded_modules = layout
        .globals
        .loaded_module_list
        .map(|rva| image.base.wrapping_add(rva.0))
        .filter(|&head| heads_a_list(cpu, head))
        .or_else(|| loaded_module_list(cpu, image));
    let build = nt_build_number(cpu, image);
    Some(Kernel {
        image: *image,
        loaded_modules,
        system,
        build,
        layout: ProcessLayout {
            pid,
            links,
            name,
            directory_table_base: process.directory_table_base.map_or(DIRECTORY_TABLE_BASE, |offset| offset.0),
        },
    })
}

/// The KPCR: GS in the kernel, `IA32_KERNEL_GS_BASE` in user mode (and in
/// the kernel between its entry and `SWAPGS`) — whichever points at itself.
fn find_kpcr<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
) -> Option<u64> {
    [view.segment_bases.gs, view.kernel_gs_base]
        .into_iter()
        .filter_map(kernel_pointer)
        .find(|&kpcr| read_u64(cpu, Root::Live, kpcr.wrapping_add(0x18)) == Some(kpcr))
}

/// Whether a KPCR-shaped address is held but does not read, as when the
/// processor runs on page tables that leave the kernel out.
fn kpcr_unreadable<G: GuestMemory>(
    cpu: &mut G,
    view: &CpuDebugView,
) -> bool {
    [view.segment_bases.gs, view.kernel_gs_base]
        .into_iter()
        .filter_map(kernel_pointer)
        .any(|kpcr| read_u64(cpu, Root::Live, kpcr.wrapping_add(0x18)).is_none())
}

/// The kernel image and its `PsInitialSystemProcess`: the first image at or
/// below interrupt vector 0's handler that exports that variable. Images that
/// do not (the HAL, which the kernel's handlers may lie above) are passed by.
fn kernel_image<G: GuestMemory>(
    cpu: &mut G,
    kpcr: u64,
) -> Option<KernelImage> {
    let idt = kernel_pointer(read_u64(cpu, Root::Live, kpcr.wrapping_add(0x38))?)?;
    let gate: [u8; 16] = read_array(cpu, Root::Live, idt)?;
    // Intel SDM Vol. 3A, "64-bit IDT gate descriptor": offset bits 0–15 at
    // bytes 0–1, 16–31 at bytes 6–7, 32–63 at bytes 8–11.
    let handler = u64::from_le_bytes([
        gate[0], gate[1], gate[6], gate[7], gate[8], gate[9], gate[10], gate[11],
    ]);
    let mut page = kernel_pointer(handler)? & !(PAGE_BYTES - 1);
    for _ in 0..KERNEL_SCAN_PAGES {
        if let Some(image) = pe_image(cpu, Root::Live, page) {
            if let Some(system_variable) = export(cpu, &image, "PsInitialSystemProcess") {
                return Some(KernelImage {
                    image,
                    system_variable,
                });
            }
        }
        page = kernel_pointer(page.checked_sub(PAGE_BYTES)?)?;
    }
    None
}

/// The head of the kernel's loaded-module list: its `PsLoadedModuleList`
/// export, or the debugger data block's copy where it exports none. Either
/// is taken only when it heads a list whose first entry points back at it.
fn loaded_module_list<G: GuestMemory>(
    cpu: &mut G,
    image: &PeImage,
) -> Option<u64> {
    if let Some(head) = export(cpu, image, "PsLoadedModuleList") {
        if heads_a_list(cpu, head) {
            return Some(head);
        }
    }
    let head = debugger_data_module_list(cpu, image)?;
    heads_a_list(cpu, head).then_some(head)
}

/// Whether `head` heads a list: its first entry's `Blink` points back at it.
fn heads_a_list<G: GuestMemory>(cpu: &mut G, head: u64) -> bool {
    read_u64(cpu, Root::Live, head)
        .and_then(kernel_pointer)
        .and_then(|first| read_u64(cpu, Root::Live, first.wrapping_add(8)))
        == Some(head)
}

/// `KDDEBUGGER_DATA64.PsLoadedModuleList`, from the block in `image` whose
/// header tag is `KDBG` (at +0x10, the block 8-aligned) and whose `KernBase`
/// (+0x18) is the image's base. Windows 8 and later keep the block encoded
/// unless a kernel debugger is attached; their kernels export the list.
fn debugger_data_module_list<G: GuestMemory>(
    cpu: &mut G,
    image: &PeImage,
) -> Option<u64> {
    const TAG: &[u8; 4] = b"KDBG";
    const TAG_OFFSET: u64 = 0x10;
    const KERN_BASE: u64 = 0x18;
    const PS_LOADED_MODULE_LIST: u64 = 0x48;
    let mut page = image.base;
    let end = image.base.checked_add(u64::from(image.size))?;
    while page < end {
        // Paged-out and discarded pages are passed by.
        if let Some(bytes) = read_exact(cpu, Root::Live, page, PAGE_BYTES as usize) {
            // The tag sits 0x10 into an 8-aligned block: 8-aligned too.
            for (index, chunk) in bytes.chunks_exact(8).enumerate() {
                if chunk.get(..TAG.len()) != Some(TAG.as_slice()) {
                    continue;
                }
                // At most a page's worth of 8-byte steps: no wrap.
                let block = page
                    .wrapping_add((index as u64).wrapping_mul(8))
                    .wrapping_sub(TAG_OFFSET);
                if read_u64(cpu, Root::Live, block.wrapping_add(KERN_BASE)) == Some(image.base) {
                    return read_u64(cpu, Root::Live, block.wrapping_add(PS_LOADED_MODULE_LIST))
                        .and_then(kernel_pointer);
                }
            }
        }
        page = page.checked_add(PAGE_BYTES)?;
    }
    None
}

/// A PE32+ image whose headers start at `base`.
fn pe_image<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    base: u64,
) -> Option<PeImage> {
    if read_u16(cpu, root, base)? != MZ {
        return None;
    }
    let lfanew = u64::from(read_u32(cpu, root, base.wrapping_add(0x3C))?);
    if !(0x40..PAGE_BYTES).contains(&lfanew) {
        return None;
    }
    let nt = base.wrapping_add(lfanew);
    if read_u32(cpu, root, nt)? != PE_SIGNATURE
        || read_u16(cpu, root, nt.wrapping_add(0x18))? != PE32_PLUS_MAGIC
    {
        return None;
    }
    // OptionalHeader at +0x18; SizeOfImage at +0x38 of it.
    let size = read_u32(cpu, root, nt.wrapping_add(0x18 + 0x38))?;
    Some(PeImage { base, nt, size })
}

/// The address `image` exports under `name`, found by a binary search of its
/// export name table (sorted, as the PE format requires).
fn export<G: GuestMemory>(
    cpu: &mut G,
    image: &PeImage,
    name: &str,
) -> Option<u64> {
    let base = image.base;
    // OptionalHeader at +0x18; DataDirectory[EXPORT] at +0x70 of it.
    let directory = u64::from(read_u32(cpu, Root::Live, image.nt.wrapping_add(0x18 + 0x70))?);
    if directory == 0 {
        return None;
    }
    let directory = base.wrapping_add(directory);
    let count = read_u32(cpu, Root::Live, directory.wrapping_add(0x18))?;
    let functions = base.wrapping_add(u64::from(read_u32(
        cpu,
        Root::Live,
        directory.wrapping_add(0x1C),
    )?));
    let names = base.wrapping_add(u64::from(read_u32(
        cpu,
        Root::Live,
        directory.wrapping_add(0x20),
    )?));
    let ordinals = base.wrapping_add(u64::from(read_u32(
        cpu,
        Root::Live,
        directory.wrapping_add(0x24),
    )?));
    // The name and its terminator: enough to order any export against it.
    let mut wanted = name.as_bytes().to_vec();
    wanted.push(0);
    let mut low = 0u32;
    let mut high = count;
    // At most 32 halvings of a `u32` range; `middle` stays below `high`, so
    // its sums cannot wrap.
    while low < high {
        let middle = low.wrapping_add(high.wrapping_sub(low) / 2);
        let entry = names.wrapping_add(u64::from(middle).wrapping_mul(4));
        let at = base.wrapping_add(u64::from(read_u32(cpu, Root::Live, entry)?));
        let candidate = read_exact(cpu, Root::Live, at, wanted.len())?;
        // Bytes after a shorter name's terminator do not order it.
        let candidate = match candidate.iter().position(|&byte| byte == 0) {
            Some(end) => candidate.get(..=end)?,
            None => candidate.as_slice(),
        };
        match candidate.cmp(wanted.as_slice()) {
            core::cmp::Ordering::Less => low = middle.wrapping_add(1),
            core::cmp::Ordering::Greater => high = middle,
            core::cmp::Ordering::Equal => {
                let ordinal = read_u16(
                    cpu,
                    Root::Live,
                    ordinals.wrapping_add(u64::from(middle).wrapping_mul(2)),
                )?;
                let rva = read_u32(
                    cpu,
                    Root::Live,
                    functions.wrapping_add(u64::from(ordinal).wrapping_mul(4)),
                )?;
                return Some(base.wrapping_add(u64::from(rva)));
            }
        }
    }
    None
}

/// Finds the moving `EPROCESS` fields in the System process.
fn process_layout<G: GuestMemory>(
    cpu: &mut G,
    system: u64,
) -> Option<ProcessLayout> {
    let block = read_exact(cpu, Root::Live, system, EPROCESS_SPAN)?;
    let qword = |offset: usize| {
        block
            .get(offset..offset.checked_add(8)?)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
    };
    let mut pid = None;
    for offset in (0..EPROCESS_SPAN - 24).step_by(8) {
        if qword(offset) != Some(SYSTEM_PID) {
            continue;
        }
        // `offset` is below `EPROCESS_SPAN`: these sums do not wrap.
        let links = system.wrapping_add(offset as u64).wrapping_add(8);
        let (Some(flink), Some(blink)) = (
            qword(offset.wrapping_add(8)).and_then(kernel_pointer),
            qword(offset.wrapping_add(16)).and_then(kernel_pointer),
        ) else {
            continue;
        };
        if read_u64(cpu, Root::Live, flink.wrapping_add(8)) == Some(links)
            && read_u64(cpu, Root::Live, blink) == Some(links)
        {
            pid = Some(offset as u64);
            break;
        }
    }
    let pid = pid?;
    let name = block
        .windows(7)
        .position(|window| window == b"System\0")? as u64;
    Some(ProcessLayout {
        pid,
        links: pid.wrapping_add(8),
        name,
        directory_table_base: DIRECTORY_TABLE_BASE,
    })
}

/// Every process on `ActiveProcessLinks`, starting at System. The list head
/// (`PsActiveProcessHead`) lies inside the kernel image and is no process.
fn list_processes<G: GuestMemory>(
    cpu: &mut G,
    kernel: &Kernel,
    live_cr3: u64,
    parent_offset: &mut Option<u64>,
) -> Vec<WinProcess> {
    let layout = &kernel.layout;
    // Only bounds a `contains`: an image that would run past the top of
    // memory reaches to it.
    let image = kernel.image.base..kernel.image.base.saturating_add(u64::from(kernel.image.size));
    let first = kernel.system.wrapping_add(layout.links);
    let mut processes = Vec::new();
    let mut visited = HashSet::new();
    let mut entry = first;
    for _ in 0..MAX_PROCESSES {
        // A ring that loops without returning to `first` ends at the repeat.
        if !visited.insert(entry) {
            break;
        }
        if !image.contains(&entry) {
            let eprocess = entry.wrapping_sub(layout.links);
            let (Some(pid), Some(cr3)) = (
                read_u64(cpu, Root::Live, eprocess.wrapping_add(layout.pid)),
                read_u64(cpu, Root::Live, eprocess.wrapping_add(layout.directory_table_base)),
            ) else {
                break;
            };
            let name = read_exact(
                cpu,
                Root::Live,
                eprocess.wrapping_add(layout.name),
                IMAGE_FILE_NAME_BYTES,
            )
            .map(|bytes| {
                let name = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
                String::from_utf8_lossy(name).into_owned()
            })
            .unwrap_or_default();
            processes.push(WinProcess {
                eprocess,
                pid,
                parent: None,
                name,
                cr3,
                current: same_address_space(cr3, live_cr3),
            });
        }
        let Some(next) = read_u64(cpu, Root::Live, entry).and_then(kernel_pointer) else {
            break;
        };
        if next == first {
            break;
        }
        entry = next;
    }
    assign_parents(cpu, layout, &mut processes, parent_offset);
    processes
}

/// Where a process's thread fields sit, found by validation (never fixed
/// offsets): `head` is the `EPROCESS`-relative `ThreadListHead`, and the rest
/// are offsets from a `ThreadListEntry` node to the `ETHREAD`'s fields.
#[derive(Clone, Copy)]
struct ThreadLayout {
    head: u64,
    cid: i64,
    /// `KTHREAD.Teb`; found once a process with user threads is read.
    teb: Option<i64>,
    /// The two start addresses; found once the threads' entry points can be
    /// checked against loaded modules.
    start: Option<StartLayout>,
    /// `KTHREAD.TrapFrame`; found once a process whose threads have user
    /// stacks is read.
    trap_frame: Option<i64>,
}

/// Where a thread's two start addresses sit, as node-relative offsets.
#[derive(Clone, Copy)]
struct StartLayout {
    /// `ETHREAD.Win32StartAddress`: the thread's own entry point.
    win32: i64,
    /// `ETHREAD.StartAddress`: the routine the kernel started it at.
    kernel: i64,
}

/// A loaded image's address range, for telling a code pointer from data.
#[derive(Clone, Copy)]
struct CodeRange {
    start: u64,
    end: u64,
}

/// The ranges of the kernel's modules and of one process's, in address order.
fn code_ranges(kernel: &[WinModule], user: &[WinModule]) -> Vec<CodeRange> {
    let mut ranges: Vec<CodeRange> = kernel
        .iter()
        .chain(user)
        .map(|module| CodeRange {
            start: module.base,
            end: module.base.saturating_add(u64::from(module.size)),
        })
        .collect();
    ranges.sort_by_key(|range| range.start);
    ranges
}

/// Whether `address` lies in one of `ranges` (in address order).
fn in_code_range(ranges: &[CodeRange], address: u64) -> bool {
    let after = ranges.partition_point(|range| range.start <= address);
    after
        .checked_sub(1)
        .and_then(|at| ranges.get(at))
        .is_some_and(|range| address < range.end)
}

/// The widest span from a `ThreadListEntry` to one of its `ETHREAD` fields:
/// `Teb` sits in the `KTHREAD` before the entry (a negative delta), `Cid` and
/// `StartAddress` after it.
const THREAD_DELTA_MIN: i64 = -0x600;
const THREAD_DELTA_MAX: i64 = 0x400;
/// How far past `Cid` the start addresses are looked for.
const START_DELTA_AFTER: i64 = 0x200;

fn read_at<G: GuestMemory>(
    cpu: &mut G,
    node: u64,
    delta: i64,
) -> Option<u64> {
    read_u64(cpu, Root::Live, node.wrapping_add(delta as u64))
}

/// A thread ID shape: a non-zero multiple of four below 2^24, as Windows hands
/// out client IDs.
fn is_client_id(value: u64) -> bool {
    value != 0 && value & 3 == 0 && value < 0x0100_0000
}

/// The threads of the process at `eprocess` (PID `pid`), from its
/// `EPROCESS.ThreadListHead`. Every field offset is located by validation —
/// the head is the field reading as a `LIST_ENTRY` whose nodes all sit in an
/// `ETHREAD` whose `Cid.UniqueProcess` is this PID; the `Teb` is the
/// node-relative field whose `NtTib.Self` points back at it (found only for a
/// process with a user half, `user_space`); the two start addresses are the
/// fields that point into a loaded module (`ranges`) for nearly every thread —
/// and then kept in `cache`, to be checked against the next process. A field
/// that was not found leaves that part of a thread blank; a corrupt list
/// stops at `MAX_THREADS`.
fn read_threads<G: GuestMemory>(
    cpu: &mut G,
    eprocess: u64,
    pid: u64,
    cr3: u64,
    user_space: bool,
    cache: &mut Option<ThreadLayout>,
    ranges: &[CodeRange],
) -> Vec<WinThread> {
    let held = cache.filter(|layout| thread_layout_holds(cpu, eprocess, pid, layout));
    let mut layout = match held {
        Some(layout) => layout,
        None => match discover_thread_layout(cpu, eprocess, pid) {
            Some(layout) => layout,
            None => return Vec::new(),
        },
    };
    let head = eprocess.wrapping_add(layout.head);
    let nodes = walk_thread_list(
        head,
        |node| read_u64(cpu, Root::Live, node).and_then(kernel_pointer),
        MAX_THREADS,
    );
    if layout.teb.is_none() && user_space {
        layout.teb = find_teb_delta(cpu, &nodes, cr3);
    }
    // One holder (a process whose threads all left `Win32StartAddress` zero)
    // is looked at again for the pair when a process shows one.
    if layout.start.is_none_or(|fields| fields.win32 == fields.kernel) {
        if let Some(found) = find_start_layout(cpu, &nodes, layout.cid, ranges) {
            layout.start = Some(found);
        }
    }
    let mut threads: Vec<WinThread> = nodes
        .iter()
        .map(|&node| read_thread(cpu, node, &layout, cr3))
        .collect();
    // The trap-frame field is found against the threads' own stacks, so it
    // is looked for once they are read, and the threads take their saved
    // registers from it.
    if layout.trap_frame.is_none() && user_space {
        if let Some(delta) = find_trap_frame_delta(cpu, &nodes, &threads) {
            layout.trap_frame = Some(delta);
            for (thread, &node) in threads.iter_mut().zip(&nodes) {
                thread.saved = thread.stack.and_then(|stack| saved_registers(cpu, node, delta, stack));
            }
        }
    }
    *cache = Some(layout);
    threads
}

/// The thread at `node`'s user registers from the `KTRAP_FRAME` its
/// `KTHREAD.TrapFrame` (node-relative `delta`) points at, when that frame is
/// its own user state (`frame_is_users` against its user `stack`). The frame
/// is kernel memory.
fn saved_registers<G: GuestMemory>(
    cpu: &mut G,
    node: u64,
    delta: i64,
    stack: LinearRange,
) -> Option<SavedRegisters> {
    let frame_at = read_at(cpu, node, delta).and_then(kernel_pointer)?;
    let mut frame = [0u8; TRAP_FRAME_BYTES];
    if !read_into(cpu, Root::Live, frame_at, &mut frame) {
        return None;
    }
    let saved = decode_trap_frame(&frame);
    frame_is_users(&saved, stack).then_some(saved)
}

/// The node-relative offset of `KTHREAD.TrapFrame`: the field that, for the
/// most threads, points at a frame holding that thread's own user state — a
/// user CS, a user RIP and an RSP in its own stack. A stray pointer may lead
/// to one plausible frame; only `TrapFrame` does for every waiting thread at
/// once. Ties go to the lower offset.
fn find_trap_frame_delta<G: GuestMemory>(
    cpu: &mut G,
    nodes: &[u64],
    threads: &[WinThread],
) -> Option<i64> {
    (THREAD_DELTA_MIN..THREAD_DELTA_MAX)
        .step_by(8)
        .map(|delta| {
            let matches = nodes
                .iter()
                .zip(threads)
                .filter(|&(&node, thread)| {
                    thread
                        .stack
                        .is_some_and(|stack| saved_registers(cpu, node, delta, stack).is_some())
                })
                .count();
            (delta, matches)
        })
        .filter(|&(_, matches)| matches > 0)
        .max_by(|(a_delta, a_matches), (b_delta, b_matches)| {
            a_matches.cmp(b_matches).then(b_delta.cmp(a_delta))
        })
        .map(|(delta, _)| delta)
}

/// Whether a thread layout learned earlier reads true for this process: its
/// list head is a ring, empty or whose first thread carries this PID.
fn thread_layout_holds<G: GuestMemory>(
    cpu: &mut G,
    eprocess: u64,
    pid: u64,
    layout: &ThreadLayout,
) -> bool {
    let head = eprocess.wrapping_add(layout.head);
    let Some(first) = read_u64(cpu, Root::Live, head).and_then(kernel_pointer) else {
        return false;
    };
    if first == head {
        return true;
    }
    read_u64(cpu, Root::Live, first.wrapping_add(8)) == Some(head)
        && read_at(cpu, first, layout.cid) == Some(pid)
}

/// Finds the thread-list field and the `Cid` offset by validation.
fn discover_thread_layout<G: GuestMemory>(
    cpu: &mut G,
    eprocess: u64,
    pid: u64,
) -> Option<ThreadLayout> {
    for offset in (0..EPROCESS_SPAN as u64).step_by(8) {
        let head = eprocess.wrapping_add(offset);
        let Some(first) = read_u64(cpu, Root::Live, head).and_then(kernel_pointer) else {
            continue;
        };
        // A LIST_ENTRY head: the first node's Blink points back at the head.
        if first == head || read_u64(cpu, Root::Live, first.wrapping_add(8)) != Some(head) {
            continue;
        }
        let Some(cid) = find_cid_delta(cpu, first, pid) else {
            continue;
        };
        let nodes = walk_thread_list(
            head,
            |node| read_u64(cpu, Root::Live, node).and_then(kernel_pointer),
            MAX_THREADS,
        );
        if nodes.is_empty() {
            continue;
        }
        let covered = nodes
            .iter()
            .filter(|&&node| read_at(cpu, node, cid) == Some(pid))
            .count();
        // Nearly every node must carry this PID: the thread list, not another
        // EPROCESS list that happens to read as a ring.
        // Both counts are at most MAX_THREADS: the products cannot wrap.
        if covered.wrapping_mul(10) < nodes.len().wrapping_mul(9) {
            continue;
        }
        return Some(ThreadLayout {
            head: offset,
            cid,
            teb: None,
            start: None,
            trap_frame: None,
        });
    }
    None
}

/// The node-relative offsets of `StartAddress` and `Win32StartAddress`: the
/// `ETHREAD` fields after `Cid` that, where a thread has a value, point into a
/// loaded module for nearly every thread that does. A thread's other pointers
/// (its process, its stack) lie in no module. `Win32StartAddress` is left zero
/// by a thread made with `RtlCreateUserThread`, so zero is not counted
/// against a field, and `Win32StartAddress` follows `StartAddress`, so the
/// last two holders are the pair; one holder serves as both fields. No holder
/// finds nothing, so a start address is blank, never a guess.
fn find_start_layout<G: GuestMemory>(
    cpu: &mut G,
    nodes: &[u64],
    cid: i64,
    ranges: &[CodeRange],
) -> Option<StartLayout> {
    if nodes.is_empty() || ranges.is_empty() {
        return None;
    }
    let mut holders = Vec::new();
    // `Cid` itself holds a PID and a TID, not an address.
    // `cid` is a learned delta inside THREAD_DELTA_MIN..THREAD_DELTA_MAX:
    // these sums cannot wrap.
    for delta in (cid.wrapping_add(0x10)..cid.wrapping_add(START_DELTA_AFTER)).step_by(8) {
        let mut set = 0usize;
        let mut hits = 0usize;
        for &node in nodes {
            match read_at(cpu, node, delta) {
                Some(0) | None => {}
                Some(value) => {
                    set = set.saturating_add(1);
                    hits = hits.saturating_add(usize::from(in_code_range(ranges, value)));
                }
            }
        }
        // Both counts are at most MAX_THREADS: the products cannot wrap.
        if set > 0 && hits.wrapping_mul(10) >= set.wrapping_mul(9) {
            holders.push(delta);
        }
    }
    match holders.as_slice() {
        [] => None,
        [only] => Some(StartLayout {
            win32: *only,
            kernel: *only,
        }),
        [.., kernel, win32] => Some(StartLayout {
            win32: *win32,
            kernel: *kernel,
        }),
    }
}

/// The node-relative offset of `Cid`: `UniqueProcess` equals the PID and
/// `UniqueThread` is a client-ID-shaped value right after it.
fn find_cid_delta<G: GuestMemory>(
    cpu: &mut G,
    node: u64,
    pid: u64,
) -> Option<i64> {
    (THREAD_DELTA_MIN..THREAD_DELTA_MAX).step_by(8).find(|&delta| {
        read_at(cpu, node, delta) == Some(pid)
            && read_at(cpu, node, delta.wrapping_add(8)).is_some_and(is_client_id)
    })
}

/// The node-relative offset of `KTHREAD.Teb`: the field whose value reads as a
/// user pointer whose `NtTib.Self` (TEB +0x30) points back at it. The pointer
/// lives in the `ETHREAD` (kernel), but the `TEB` is user memory, so it is read
/// through the process's own address space. The delta that self-references for
/// the most threads is the field — a stray lower offset might match one thread
/// by chance, but only the real `Teb` matches every user thread at once.
fn find_teb_delta<G: GuestMemory>(
    cpu: &mut G,
    nodes: &[u64],
    cr3: u64,
) -> Option<i64> {
    (THREAD_DELTA_MIN..THREAD_DELTA_MAX)
        .step_by(8)
        .map(|delta| {
            let matches = nodes
                .iter()
                .filter(|&&node| {
                    read_at(cpu, node, delta)
                        .and_then(user_pointer)
                        .is_some_and(|teb| {
                            read_u64(cpu, Root::Process(cr3), teb.wrapping_add(0x30)) == Some(teb)
                        })
                })
                .count();
            (delta, matches)
        })
        .filter(|&(_, matches)| matches > 0)
        .max_by_key(|&(_, matches)| matches)
        .map(|(delta, _)| delta)
}

/// Reads one thread from its `ThreadListEntry` node. The `ETHREAD` fields are
/// kernel memory; the `TEB` and the stack are the process's user memory, read
/// through its address space `cr3`.
fn read_thread<G: GuestMemory>(
    cpu: &mut G,
    node: u64,
    layout: &ThreadLayout,
    cr3: u64,
) -> WinThread {
    let user = Root::Process(cr3);
    let tid = read_at(cpu, node, layout.cid.wrapping_add(8)).unwrap_or(0);
    let teb = layout
        .teb
        .and_then(|delta| read_at(cpu, node, delta))
        .and_then(user_pointer)
        .filter(|&teb| read_u64(cpu, user, teb.wrapping_add(0x30)) == Some(teb));
    // NtTib.StackBase at TEB +0x08, StackLimit at +0x10 (x64): the stack grows
    // down from base to limit.
    let stack = teb.and_then(|teb| {
        let base = read_u64(cpu, user, teb.wrapping_add(0x08))?;
        let limit = read_u64(cpu, user, teb.wrapping_add(0x10))?;
        (limit < base && user_pointer(base).is_some())
            .then_some(LinearRange { start: limit, end: base })
    });
    // `Win32StartAddress` is the thread's own entry, set for user threads; a
    // kernel thread has none, so fall back to the kernel `StartAddress`. Both
    // read from the ETHREAD, at the offsets validated against loaded modules.
    let start = layout
        .start
        .and_then(|fields| {
            let win32 = read_at(cpu, node, fields.win32).filter(|&v| user_pointer(v).is_some());
            win32.or_else(|| {
                read_at(cpu, node, fields.kernel)
                    .filter(|&v| kernel_pointer(v).is_some() || user_pointer(v).is_some())
            })
        })
        .unwrap_or(0);
    // NtTib.FiberData (TEB +0x20) is a union with the TEB version: a thread
    // running fibres holds a heap pointer here, an ordinary thread a small
    // version number. A value above the version range is fibre data.
    let uses_fibers = teb.is_some_and(|teb| {
        read_u64(cpu, user, teb.wrapping_add(0x20))
            .and_then(user_pointer)
            .is_some_and(|data| data >= 0x1_0000)
    });
    let saved = layout
        .trap_frame
        .zip(stack)
        .and_then(|(delta, stack)| saved_registers(cpu, node, delta, stack));
    WinThread {
        tid,
        start,
        teb,
        stack,
        uses_fibers,
        saved,
    }
}

/// Fills each process's `parent` from `EPROCESS.InheritedFromUniqueProcessId`,
/// found as the first field past `UniqueProcessId` that reads as a parent ID
/// in every process at once: zero in System, the System process's PID in at
/// least one other (the session manager, smss.exe), never the process's own
/// PID, always a PID-shaped value (a multiple of 4 below 2^24), and the PID of
/// a listed process for at least half the rest (a parent that has exited,
/// such as explorer.exe's userinit.exe, is no longer listed). Left `None`
/// when no field qualifies. The offset found is kept in `cached`, and later
/// reads take the field there without searching.
fn assign_parents<G: GuestMemory>(
    cpu: &mut G,
    layout: &ProcessLayout,
    processes: &mut [WinProcess],
    cached: &mut Option<u64>,
) {
    const MAX_PID: u64 = 1 << 24;
    if let Some(offset) = *cached {
        for process in processes.iter_mut() {
            process.parent = read_u64(cpu, Root::Live, process.eprocess.wrapping_add(offset))
                .filter(|&parent| parent != 0);
        }
        return;
    }
    let mut blocks = Vec::with_capacity(processes.len());
    for process in processes.iter() {
        match read_exact(cpu, Root::Live, process.eprocess, EPROCESS_SPAN) {
            Some(block) => blocks.push(block),
            None => return,
        }
    }
    let pids: Vec<u64> = processes.iter().map(|p| p.pid).collect();
    let qword = |block: &[u8], offset: usize| {
        block
            .get(offset..offset.checked_add(8)?)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
    };
    let first = usize::try_from(layout.pid)
        .unwrap_or(EPROCESS_SPAN)
        .saturating_add(8);
    let found = (first..EPROCESS_SPAN - 8).step_by(8).find(|&offset| {
        let mut from_system = false;
        let mut listed = 0usize;
        let mut others = 0usize;
        for (process, block) in processes.iter().zip(&blocks) {
            let Some(value) = qword(block, offset) else {
                return false;
            };
            if process.pid == SYSTEM_PID {
                if value != 0 {
                    return false;
                }
                continue;
            }
            if process.pid == 0 {
                continue;
            }
            if value == process.pid || value & 3 != 0 || value >= MAX_PID {
                return false;
            }
            others = others.saturating_add(1);
            from_system |= value == SYSTEM_PID;
            listed = listed.saturating_add(usize::from(pids.contains(&value)));
        }
        from_system && others > 0 && listed.saturating_mul(2) >= others
    });
    let Some(offset) = found else {
        return;
    };
    *cached = Some(offset as u64);
    for (process, block) in processes.iter_mut().zip(&blocks) {
        process.parent = qword(block, offset).filter(|&parent| parent != 0);
    }
}

/// One process's row in the process tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeRow {
    /// Its place in the listing the tree was built from.
    pub(crate) index: usize,
    pub(crate) depth: usize,
    /// For each ancestor level below this row's own: whether a later sibling
    /// follows at that level, so a guide line runs on past this row.
    pub(crate) guides: Vec<bool>,
    /// It is the last child of its parent.
    pub(crate) last: bool,
}

/// `processes` as a tree, depth first, children in listing order. A process
/// whose parent is not listed (exited, or never found) is a root.
pub(crate) fn process_tree(processes: &[WinProcess]) -> Vec<TreeRow> {
    let pids: HashSet<u64> = processes.iter().map(|process| process.pid).collect();
    let parent_of = |process: &WinProcess| {
        process
            .parent
            .filter(|&parent| parent != process.pid && pids.contains(&parent))
    };
    let mut rows = Vec::with_capacity(processes.len());
    let mut placed = vec![false; processes.len()];
    let roots: Vec<usize> = processes
        .iter()
        .enumerate()
        .filter(|&(_, process)| parent_of(process).is_none())
        .map(|(index, _)| index)
        .collect();
    let count = roots.len();
    for (n, root) in roots.into_iter().enumerate() {
        let last = n.saturating_add(1) == count;
        place(processes, root, 0, Vec::new(), last, &mut placed, &mut rows);
    }
    // A cycle of parents has no root: list what it left unplaced at the top.
    for index in 0..processes.len() {
        if placed.get(index) == Some(&false) {
            place(processes, index, 0, Vec::new(), true, &mut placed, &mut rows);
        }
    }
    rows
}

fn place(
    processes: &[WinProcess],
    index: usize,
    depth: usize,
    guides: Vec<bool>,
    last: bool,
    placed: &mut [bool],
    rows: &mut Vec<TreeRow>,
) {
    let (Some(done), Some(process)) = (placed.get_mut(index), processes.get(index)) else {
        return;
    };
    if *done {
        return;
    }
    *done = true;
    let pid = process.pid;
    let children: Vec<usize> = processes
        .iter()
        .enumerate()
        .filter(|&(i, child)| {
            i != index && placed.get(i) == Some(&false) && child.parent == Some(pid)
        })
        .map(|(i, _)| i)
        .collect();
    rows.push(TreeRow {
        index,
        depth,
        guides: guides.clone(),
        last,
    });
    let mut below = guides;
    if depth > 0 {
        below.push(!last);
    }
    let count = children.len();
    for (n, child) in children.into_iter().enumerate() {
        let last = n.saturating_add(1) == count;
        place(processes, child, depth.saturating_add(1), below.clone(), last, placed, rows);
    }
}

/// Two CR3 values name one address space when their table addresses match
/// (the low 12 bits are flags or a PCID).
fn same_address_space(a: u64, b: u64) -> bool {
    (a & !0xFFF) == (b & !0xFFF)
}

/// The modules `process` has loaded: its PEB's loader list, read through its
/// own page tables.
fn process_modules<G: GuestMemory>(
    cpu: &mut G,
    processes: &[WinProcess],
    process: &WinProcess,
    peb_offset: &mut Option<u64>,
    wow64_offset: &mut Option<u64>,
) -> ModuleRead {
    let peb_offset = match *peb_offset {
        Some(offset) => offset,
        None => match find_peb_offset(cpu, processes) {
            Some(offset) => {
                *peb_offset = Some(offset);
                offset
            }
            None => return ModuleRead::Unreadable,
        },
    };
    let Some(peb) = read_u64(cpu, Root::Live, process.eprocess.wrapping_add(peb_offset)) else {
        return ModuleRead::Unreadable;
    };
    if peb == 0 {
        return ModuleRead::NoUserSpace;
    }
    let root = Root::Process(process.cr3);
    // PEB.Ldr at +0x18; PEB_LDR_DATA.InLoadOrderModuleList at +0x10.
    let Some(loader) = read_u64(cpu, root, peb.wrapping_add(0x18)).and_then(user_pointer) else {
        return ModuleRead::Unreadable;
    };
    match list_modules(cpu, root, loader.wrapping_add(0x10)) {
        Some(mut modules) => {
            // A WOW64 process also has a 32-bit loader; its modules sit beside
            // the 64-bit ones, each tagged by bitness.
            modules.extend(read_wow64_modules(
                cpu,
                process.eprocess,
                process.cr3,
                wow64_offset,
            ));
            ModuleRead::Listed(modules)
        }
        None => ModuleRead::Unreadable,
    }
}

/// The page-table memory map of the process whose address space is `cr3`:
/// its mapped user pages, with each module's span split into its header and
/// its sections. The module list is already read by [`process_modules`]; the
/// sections come from each module's PE headers, read through its own tables;
/// `build` picks the VAD flag bits when no PDB named them.
fn read_memory_map<G: GuestMemory>(
    cpu: &mut G,
    eprocess: u64,
    cr3: u64,
    modules: &[WinModule],
    build: Option<u32>,
    vad_site: &mut Option<VadSite>,
) -> MemoryMap {
    let root = Root::Process(cr3);
    let layouts: Vec<ModuleLayout> = modules
        .iter()
        .map(|module| {
            let sections = read_exact(cpu, root, module.base, HEADER_BYTES)
                .and_then(|header| pe::parse_sections(&header))
                .unwrap_or_default();
            let name = module
                .name
                .clone()
                .unwrap_or_else(|| format!("{:X}", module.base));
            ModuleLayout {
                base: module.base,
                name,
                sections,
            }
        })
        .collect();
    let mut mappings = Vec::new();
    let walk = cpu.page_mappings(
        cr3,
        LinearRange {
            start: 0,
            end: USER_SPACE_END,
        },
        |mapping| {
            if mapping.access.user {
                mappings.push(mapping);
            }
            WalkStep::Continue
        },
    );
    match walk {
        MappingWalk::Complete | MappingWalk::Stopped => {}
        // No paging structures: nothing is mapped to map.
        MappingWalk::PagingOff => {
            return MemoryMap {
                origin: memory_map::MapOrigin::PageTables,
                regions: Vec::new(),
            };
        }
    }
    // Overlay Windows' own records when the VAD tree is found, so reserved
    // regions and Windows' protection show; else the page-table map stands.
    let module_bases: Vec<u64> = modules.iter().map(|module| module.base).collect();
    match read_vads(cpu, eprocess, &module_bases, build, vad_site) {
        Some(vads) => memory_map::build_with_vads(&mappings, &layouts, &vads),
        None => memory_map::build(&mappings, &layouts),
    }
}

/// The address space a module's image is read in: the root its bytes are
/// read through, and the page tables its residency is walked in.
#[derive(Clone, Copy, Debug)]
struct ImageSpace {
    root: Root,
    cr3: u64,
}

/// The `detail` of `module`, read in `space`.
fn read_module_detail<G: GuestMemory>(
    cpu: &mut G,
    space: ImageSpace,
    module: &WinModule,
    detail: ModuleDetail,
) -> PickedDetail {
    match detail {
        ModuleDetail::Sections => PickedDetail::Sections(read_module_sections(cpu, space, module)),
        ModuleDetail::Exports => PickedDetail::Exports(read_module_exports(cpu, space.root, module)),
        ModuleDetail::Imports => PickedDetail::Imports(read_module_imports(cpu, space.root, module)),
        ModuleDetail::Symbols => {
            PickedDetail::Symbols(read_module_identity(cpu, space.root, module.base, module.name.as_deref()))
        }
    }
}

/// A module's header page and sections, from its PE headers, with each one's
/// residency and access from the page tables of `space`. A section with no
/// page mapped — one the loader discarded after boot, or paged out — is
/// listed with none resident and the access its PE flags give.
fn read_module_sections<G: GuestMemory>(
    cpu: &mut G,
    space: ImageSpace,
    module: &WinModule,
) -> SectionsRead {
    let Some(sections) = read_exact(cpu, space.root, module.base, HEADER_BYTES)
        .and_then(|header| pe::parse_sections(&header))
    else {
        return SectionsRead::HeaderUnreadable;
    };
    let layout = ModuleLayout {
        base: module.base,
        name: module
            .name
            .clone()
            .unwrap_or_else(|| format!("{:X}", module.base)),
        sections,
    };
    let image = LinearRange {
        start: module.base,
        end: module.base.saturating_add(u64::from(module.size)),
    };
    let mut mappings = Vec::new();
    let walk = cpu.page_mappings(space.cr3, image, |mapping| {
        mappings.push(mapping);
        WalkStep::Continue
    });
    match walk {
        // Without paging nothing is mapped: every section reads as none
        // resident, which is what the empty mappings give.
        MappingWalk::Complete | MappingWalk::Stopped | MappingWalk::PagingOff => {}
    }
    // A kernel image mapped by large pages is reported a page at a time, and
    // the last page runs past the image: only the image's own pages count.
    let mappings = memory_map::clip_mappings(&mappings, image);
    SectionsRead::Mapped(memory_map::build(&mappings, &[layout]))
}

/// A module's exports, read from its export table through `root`.
fn read_module_exports<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    module: &WinModule,
) -> ExportRead {
    let Some(header) = read_exact(cpu, root, module.base, HEADER_BYTES) else {
        return ExportRead::Unreadable;
    };
    pe::parse_exports(&header, |rva, len| {
        read_exact(cpu, root, module.base.wrapping_add(u64::from(rva)), len)
    })
}

/// A module's imports, read from its import table and IAT through `root`.
fn read_module_imports<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    module: &WinModule,
) -> ImportRead {
    let Some(header) = read_exact(cpu, root, module.base, HEADER_BYTES) else {
        return ImportRead::Unreadable;
    };
    pe::parse_imports(&header, |rva, len| {
        read_exact(cpu, root, module.base.wrapping_add(u64::from(rva)), len)
    })
}

/// Windows' own records of the process's address space: the VAD tree, read
/// from the `EPROCESS` at `eprocess`. The tree's root and node shape are
/// discovered by trying each 64-bit shape at every `EPROCESS` field and
/// accepting the tree that contains the process's loaded modules; the site
/// found is kept in `site` and tried first next time. A shape's flag bits are
/// `build`'s era's ([`vad::build_flags`]). `None` when no shape's tree
/// validates (an unknown build, or the tree is paged out), so the map falls
/// back to the page-table form.
fn read_vads<G: GuestMemory>(
    cpu: &mut G,
    eprocess: u64,
    module_bases: &[u64],
    build: Option<u32>,
    site: &mut Option<VadSite>,
) -> Option<Vec<VadRegion>> {
    if let Some(known) = *site {
        if let Some(vads) = vads_at(cpu, eprocess, known, module_bases) {
            return Some(vads);
        }
    }
    for shape in [VadShape::AvlX64, VadShape::BalancedNode] {
        for offset in (0..EPROCESS_SPAN as u64).step_by(8) {
            let candidate = VadSite {
                layout: vad::shape_layout(shape, build),
                offset,
            };
            if let Some(vads) = vads_at(cpu, eprocess, candidate, module_bases) {
                *site = Some(candidate);
                return Some(vads);
            }
        }
    }
    None
}

/// The VAD tree whose root the `EPROCESS` field at `site` holds, when it
/// reads as a root whose regions cover most of the process's modules.
fn vads_at<G: GuestMemory>(
    cpu: &mut G,
    eprocess: u64,
    site: VadSite,
    module_bases: &[u64],
) -> Option<Vec<VadRegion>> {
    let VadSite { layout, offset } = site;
    let root = vad_root_at(cpu, eprocess.wrapping_add(offset), layout.root)?;
    // Probe: the root must read as a plausible node before a full walk.
    read_vad_node(cpu, root, &layout)?;
    let vads = vad::walk(root, |pointer| read_vad_node(cpu, pointer, &layout), MAX_VAD_NODES);
    vads_cover_modules(&vads, module_bases).then_some(vads)
}

/// The VAD tree root held by the candidate field at `field`, or `None` when
/// the field is not a VAD root of `kind`.
///
/// `AvlTable`: the field is an `MM_AVL_TABLE` whose `BalancedRoot` sentinel
/// anchors the tree — its `Parent` points to the sentinel itself and its
/// `LeftChild` is null, a signature a wrong `EPROCESS` field does not share.
/// The real root is the sentinel's `RightChild` (+0x10). `AvlTree`: the field
/// is an `RTL_AVL_TREE` whose `Root` (+0x0) is the root directly.
fn vad_root_at<G: GuestMemory>(
    cpu: &mut G,
    field: u64,
    kind: VadRootKind,
) -> Option<u64> {
    match kind {
        VadRootKind::AvlTable => {
            let parent = read_u64(cpu, Root::Live, field)?;
            let left_child = read_u64(cpu, Root::Live, field.wrapping_add(0x08))?;
            let right_child = read_u64(cpu, Root::Live, field.wrapping_add(0x10))?;
            avl_table_root(parent, left_child, right_child, field)
        }
        VadRootKind::AvlTree => read_u64(cpu, Root::Live, field).and_then(kernel_pointer),
    }
}

/// The root an `MM_AVL_TABLE` at `table` holds, given its `BalancedRoot`
/// sentinel's `Parent`, `LeftChild` and `RightChild`. The sentinel's `Parent`
/// points to the sentinel itself (its low two bits carry the AVL balance), and
/// its `LeftChild` is null; the root is its `RightChild`. `None` when the words
/// do not form that sentinel, so a wrong field is not mistaken for a VAD root.
fn avl_table_root(parent: u64, left_child: u64, right_child: u64, table: u64) -> Option<u64> {
    if parent & !0x3 != table || left_child != 0 {
        return None;
    }
    kernel_pointer(right_child)
}

/// Whether the VAD regions contain nearly every module base — the test that
/// the discovered tree really is this process's address space. The threshold
/// is near-total, not a majority, so a coincidental or partial tree at the
/// wrong `EPROCESS` field is rejected; a few modules may miss (one loading,
/// or a base mid-region), so a small slack is allowed.
fn vads_cover_modules(vads: &[VadRegion], module_bases: &[u64]) -> bool {
    if vads.is_empty() {
        return false;
    }
    if module_bases.is_empty() {
        return true;
    }
    let covered = module_bases
        .iter()
        .filter(|&&base| vads.iter().any(|vad| vad.start <= base && base < vad.end))
        .count();
    // At least 90% of modules must lie in the tree.
    covered.saturating_mul(10) >= module_bases.len().saturating_mul(9)
}

/// A descriptor's first and last page numbers, as its node stores them.
struct VpnSpan {
    first: u64,
    last: u64,
}

/// Reads one VAD node laid out as `layout` at `pointer`, or `None` when it
/// does not read as a plausible descriptor (a sentinel, a leaf's null child,
/// or garbage from a wrong layout).
fn read_vad_node<G: GuestMemory>(
    cpu: &mut G,
    pointer: u64,
    layout: &VadLayout,
) -> Option<VadNode> {
    let node = layout.node;
    let left = read_u64(cpu, Root::Live, pointer.wrapping_add(node.left))?;
    let right = read_u64(cpu, Root::Live, pointer.wrapping_add(node.right))?;
    let vpns = match node.vpn_width {
        FieldWidth::Qword => VpnSpan {
            first: read_u64(cpu, Root::Live, pointer.wrapping_add(node.start_vpn))?,
            last: read_u64(cpu, Root::Live, pointer.wrapping_add(node.end_vpn))?,
        },
        FieldWidth::Dword => {
            let mut first = u64::from(read_u32(cpu, Root::Live, pointer.wrapping_add(node.start_vpn))?);
            let mut last = u64::from(read_u32(cpu, Root::Live, pointer.wrapping_add(node.end_vpn))?);
            // A high byte is bits 32 to 39 of its page number.
            if let Some(high) = node.start_vpn_high {
                first |= u64::from(read_u8(cpu, Root::Live, pointer.wrapping_add(high))?).wrapping_shl(32);
            }
            if let Some(high) = node.end_vpn_high {
                last |= u64::from(read_u8(cpu, Root::Live, pointer.wrapping_add(high))?).wrapping_shl(32);
            }
            VpnSpan { first, last }
        }
    };
    let VpnSpan {
        first: start_vpn,
        last: end_vpn,
    } = vpns;
    if end_vpn < start_vpn {
        return None;
    }
    let flags = match node.flags_width {
        FieldWidth::Qword => read_u64(cpu, Root::Live, pointer.wrapping_add(node.flags))?,
        FieldWidth::Dword => u64::from(read_u32(cpu, Root::Live, pointer.wrapping_add(node.flags))?),
    };
    // No page number reaches past the user half's: rejected before the shift,
    // so a garbage value cannot wrap into a plausible address.
    if start_vpn >= USER_SPACE_END >> 12 || end_vpn >= USER_SPACE_END >> 12 {
        return None;
    }
    let start = start_vpn << 12;
    // `end_vpn` is below the user half's last page number: no wrap.
    let end = end_vpn.wrapping_add(1) << 12;
    if start >= end {
        return None;
    }
    // Defence in depth: no real VAD spans more than a fraction of the address
    // space. A single node claiming the whole of it is a wrong field read as a
    // tree, not a descriptor — reject it so discovery keeps looking.
    if end.wrapping_sub(start) > MAX_VAD_SPAN {
        return None;
    }
    Some(VadNode {
        left: kernel_pointer(left).unwrap_or(0),
        right: kernel_pointer(right).unwrap_or(0),
        region: VadRegion {
            start,
            end,
            info: layout.flags.decode(flags),
        },
    })
}

fn read_u8<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    address: u64,
) -> Option<u8> {
    let mut raw = [0u8; 1];
    read_into(cpu, root, address, &mut raw).then_some(raw[0])
}

/// The `EPROCESS.Peb` offset: the first field that, in `PEB_WITNESSES`
/// processes (or every process there is, if fewer), holds a page-aligned
/// user address whose `ImageBaseAddress` (+0x10) points at an `MZ` image.
fn find_peb_offset<G: GuestMemory>(
    cpu: &mut G,
    processes: &[WinProcess],
) -> Option<u64> {
    let mut witnesses = Vec::new();
    for process in processes.iter().filter(|p| p.pid != SYSTEM_PID && p.pid != 0) {
        if let Some(block) = read_exact(cpu, Root::Live, process.eprocess, EPROCESS_SPAN) {
            witnesses.push((process.cr3, block));
        }
        if witnesses.len() == PEB_WITNESSES.saturating_mul(2) {
            break;
        }
    }
    let needed = witnesses.len().min(PEB_WITNESSES);
    if needed == 0 {
        return None;
    }
    for offset in (0..EPROCESS_SPAN).step_by(8) {
        let mut agreeing = 0usize;
        for (cr3, block) in &witnesses {
            let Some(value) = offset
                .checked_add(8)
                .and_then(|end| block.get(offset..end))
                .and_then(|bytes| bytes.try_into().ok())
                .map(u64::from_le_bytes)
            else {
                continue;
            };
            if value & (PAGE_BYTES - 1) != 0 || user_pointer(value).is_none() {
                continue;
            }
            let root = Root::Process(*cr3);
            let image = read_u64(cpu, root, value.wrapping_add(0x10)).and_then(user_pointer);
            if image.and_then(|image| read_u16(cpu, root, image)) == Some(MZ) {
                agreeing = agreeing.saturating_add(1);
            }
        }
        if agreeing >= needed {
            return Some(offset as u64);
        }
    }
    None
}

/// The modules on the loader list headed at `head`, in load order.
fn list_modules<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    head: u64,
) -> Option<Vec<WinModule>> {
    let mut modules = Vec::new();
    let mut visited = HashSet::new();
    let mut entry = read_u64(cpu, root, head)?;
    for _ in 0..MAX_MODULES {
        // A ring that loops without returning to the head ends at the repeat.
        if entry == head || entry == 0 || !visited.insert(entry) {
            break;
        }
        let (Some(base), Some(size)) = (
            read_u64(cpu, root, entry.wrapping_add(0x30)),
            read_u32(cpu, root, entry.wrapping_add(0x40)),
        ) else {
            break;
        };
        modules.push(WinModule {
            base,
            size,
            name: read_unicode_string(cpu, root, entry.wrapping_add(0x58)),
            path: read_unicode_string(cpu, root, entry.wrapping_add(0x48)),
            bits: ModuleBits::Bits64,
        });
        let Some(next) = read_u64(cpu, root, entry) else {
            break;
        };
        entry = next;
    }
    Some(modules)
}

/// A WOW64 process's 32-bit modules, from its `PEB32`: the `EPROCESS` field
/// `Wow64Process` leads to the 32-bit PEB, whose `ImageBaseAddress` reads as
/// `MZ`. Its loader list holds 32-bit `LDR_DATA_TABLE_ENTRY`s with 4-byte
/// fields. The field is found by validation, and kept in `cache` once a
/// WOW64 process has shown where it is; until then a process with no 32-bit
/// half is searched for it. Empty for a plain 64-bit process.
fn read_wow64_modules<G: GuestMemory>(
    cpu: &mut G,
    eprocess: u64,
    cr3: u64,
    cache: &mut Option<u64>,
) -> Vec<WinModule> {
    let root = Root::Process(cr3);
    if let Some(offset) = *cache {
        return wow64_modules_at(cpu, root, eprocess, offset).unwrap_or_default();
    }
    for offset in (0..EPROCESS_SPAN as u64).step_by(8) {
        if let Some(modules) = wow64_modules_at(cpu, root, eprocess, offset) {
            *cache = Some(offset);
            return modules;
        }
    }
    Vec::new()
}

/// The 32-bit modules of the `PEB32` that the `EPROCESS` field at `offset`
/// leads to, when it leads to one.
fn wow64_modules_at<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    eprocess: u64,
    offset: u64,
) -> Option<Vec<WinModule>> {
    let value = read_u64(cpu, Root::Live, eprocess.wrapping_add(offset))?;
    let peb32 = peb32_of(cpu, value)?;
    // PEB32.ImageBaseAddress at +0x08; a real PEB32 begins a PE image.
    let image_base = u64::from(read_u32(cpu, root, peb32.wrapping_add(0x08))?);
    if image_base == 0 || read_u16(cpu, root, image_base)? != MZ {
        return None;
    }
    // PEB32.Ldr at +0x0C; PEB_LDR_DATA32.InLoadOrderModuleList at +0x0C.
    let loader = u64::from(read_u32(cpu, root, peb32.wrapping_add(0x0C))?);
    if loader == 0 {
        return None;
    }
    Some(list_modules32(cpu, root, loader.wrapping_add(0x0C)))
}

/// The `PEB32` address an `EPROCESS.Wow64Process` value stands for: a user
/// address below 4 GiB held directly (Windows XP through 8.1), or a kernel
/// pointer to an `EWOW64PROCESS` whose first field is it (Windows 10 on).
fn peb32_of<G: GuestMemory>(
    cpu: &mut G,
    value: u64,
) -> Option<u64> {
    // A 32-bit PEB lives in the 32-bit half of the address space.
    let below_4g = |address: u64| user_pointer(address).filter(|&a| a <= u64::from(u32::MAX));
    match kernel_pointer(value) {
        Some(structure) => read_u64(cpu, Root::Live, structure).and_then(below_4g),
        None => below_4g(value),
    }
}

/// Walks a 32-bit `InLoadOrderModuleList` at `head`: `DllBase` +0x18,
/// `SizeOfImage` +0x20, `FullDllName` +0x24, `BaseDllName` +0x2C, all 4-byte
/// `LDR_DATA_TABLE_ENTRY` fields.
fn list_modules32<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    head: u64,
) -> Vec<WinModule> {
    let mut modules = Vec::new();
    let mut visited = HashSet::new();
    let Some(mut entry) = read_u32(cpu, root, head).map(u64::from) else {
        return modules;
    };
    for _ in 0..MAX_MODULES {
        // A ring that loops without returning to the head ends at the repeat.
        if entry == head || entry == 0 || !visited.insert(entry) {
            break;
        }
        let (Some(base), Some(size)) = (
            read_u32(cpu, root, entry.wrapping_add(0x18)).map(u64::from),
            read_u32(cpu, root, entry.wrapping_add(0x20)),
        ) else {
            break;
        };
        modules.push(WinModule {
            base,
            size,
            name: read_unicode_string32(cpu, root, entry.wrapping_add(0x2C)),
            path: read_unicode_string32(cpu, root, entry.wrapping_add(0x24)),
            bits: ModuleBits::Bits32,
        });
        let Some(next) = read_u32(cpu, root, entry).map(u64::from) else {
            break;
        };
        entry = next;
    }
    modules
}

/// A 32-bit `UNICODE_STRING` at `at`: `Length` (bytes) at +0, `Buffer` (a
/// 4-byte pointer) at +4.
fn read_unicode_string32<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    at: u64,
) -> Option<String> {
    let length = read_u16(cpu, root, at)?;
    if length == 0 || length % 2 != 0 || length > MAX_NAME_BYTES {
        return None;
    }
    let buffer = u64::from(read_u32(cpu, root, at.wrapping_add(4))?);
    let bytes = read_exact(cpu, root, buffer, usize::from(length))?;
    Some(utf16_lossy(&bytes))
}

/// UTF-16LE `bytes` as text; a trailing odd byte is no unit.
fn utf16_lossy(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .filter_map(|pair| <[u8; 2]>::try_from(pair).ok())
        .map(u16::from_le_bytes)
        .collect();
    String::from_utf16_lossy(&units)
}

/// A 64-bit `UNICODE_STRING` at `at`: `Length` (bytes) at +0, `Buffer` at +8.
fn read_unicode_string<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    at: u64,
) -> Option<String> {
    let length = read_u16(cpu, root, at)?;
    if length == 0 || length % 2 != 0 || length > MAX_NAME_BYTES {
        return None;
    }
    let buffer = read_u64(cpu, root, at.wrapping_add(8))?;
    let bytes = read_exact(cpu, root, buffer, usize::from(length))?;
    Some(utf16_lossy(&bytes))
}

/// An address in the kernel half of a 64-bit address space.
fn kernel_pointer(value: u64) -> Option<u64> {
    (value >= 0xFFFF_8000_0000_0000).then_some(value)
}

/// An address in the user half of a 64-bit address space.
fn user_pointer(value: u64) -> Option<u64> {
    (value != 0 && value < 0x0000_8000_0000_0000).then_some(value)
}

/// Fills `buf` from linear `address` under `root`; `false` when any page of
/// it does not map or does not read.
fn read_into<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    address: u64,
    buf: &mut [u8],
) -> bool {
    match cpu.read_linear(root, address, buf) {
        GuestRead::Read => true,
        GuestRead::Unreadable => false,
    }
}

/// `len` bytes from linear `address` under `root`, or `None` when any page of
/// them does not map or does not read.
fn read_exact<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    address: u64,
    len: usize,
) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    read_into(cpu, root, address, &mut bytes).then_some(bytes)
}

/// `N` bytes at linear `address` under `root`, as an array: a fixed-size
/// read whose fields the caller takes by constant index.
fn read_array<const N: usize, G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    address: u64,
) -> Option<[u8; N]> {
    let mut bytes = [0u8; N];
    read_into(cpu, root, address, &mut bytes).then_some(bytes)
}

fn read_u64<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    address: u64,
) -> Option<u64> {
    let mut raw = [0u8; 8];
    read_into(cpu, root, address, &mut raw).then(|| u64::from_le_bytes(raw))
}

fn read_u32<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    address: u64,
) -> Option<u32> {
    let mut raw = [0u8; 4];
    read_into(cpu, root, address, &mut raw).then(|| u32::from_le_bytes(raw))
}

fn read_u16<G: GuestMemory>(
    cpu: &mut G,
    root: Root,
    address: u64,
) -> Option<u16> {
    let mut raw = [0u8; 2];
    read_into(cpu, root, address, &mut raw).then(|| u16::from_le_bytes(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box::cpu::CpuSetupMode;
    use rusty_box::emulator::{Emulator, EmulatorConfig};

    #[test]
    fn a_guest_that_is_not_windows_lists_no_kernel() {
        let mut machine =
            Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
                .expect("machine");
        let mut cpu = machine.debug_cpu(0).expect("the boot processor");
        let view = cpu.view();
        let mut discovery = WindowsDiscovery::default();
        let off = read_windows(&mut cpu, &view, SystemView::Off, &mut discovery, None);
        assert_eq!(off.system, SystemListing::NotAsked);
        assert!(off.symbols.modules.is_empty());
        let processes = SystemView::Processes {
            selected: None,
            detail: ProcessDetail::Modules,
            module: None,
            module_detail: ModuleDetail::Sections,
        };
        let kernel = SystemView::KernelModules {
            selected: None,
            detail: ModuleDetail::Sections,
        };
        for asked in [kernel, processes] {
            // The flat test mode keeps GS at 0: there is no KPCR to find.
            assert_eq!(
                read_windows(&mut cpu, &view, asked, &mut discovery, None).system,
                SystemListing::NoWindowsKernel(KernelMiss::NoKpcr)
            );
        }
    }

    /// Viewing a process or thread of a guest that is not Windows says why,
    /// and never falls back to the live processor as if it were that thread.
    #[test]
    fn viewing_a_thread_of_a_guest_that_is_not_windows_says_why() {
        let mut machine =
            Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
                .expect("machine");
        let mut cpu = machine.debug_cpu(0).expect("the boot processor");
        let view = cpu.view();
        let mut discovery = WindowsDiscovery::default();
        for target in [
            ViewTarget::Process { eprocess: 0xFFFF_FA80_0000_1000 },
            ViewTarget::Thread { eprocess: 0xFFFF_FA80_0000_1000, tid: 0x1A4 },
        ] {
            let read = read_thread_view(&mut cpu, &view, target, &[], &mut discovery, None);
            assert!(
                matches!(read, ThreadViewRead::Miss(ViewMiss::NoWindowsKernel(KernelMiss::NoKpcr))),
                "{target:?} read as {read:?}"
            );
        }
    }

    /// Every reason a thread cannot be shown says what it is, so the banner
    /// never reads blank.
    #[test]
    fn every_reason_a_thread_cannot_be_shown_is_said() {
        for miss in [
            ViewMiss::NoWindowsKernel(KernelMiss::NoKpcr),
            ViewMiss::NoSuchProcess,
            ViewMiss::NoSuchThread,
            ViewMiss::NoSavedState,
            ViewMiss::RunningElsewhere { cpu: 1 },
        ] {
            assert!(!miss.describe().is_empty(), "{miss:?}");
        }
        assert!(ViewMiss::NoSavedState.describe().contains("saved"));
        assert!(ViewMiss::RunningElsewhere { cpu: 1 }.describe().contains("CPU 1"));
    }

    /// A thread that a processor runs in user mode is the one whose TEB that
    /// processor's GS base holds: its trap frame is an old entry, not where
    /// it is now.
    #[test]
    fn a_thread_running_on_a_processor_is_found_by_its_teb() {
        let live = [
            LiveTeb { cpu: 0, teb: 0x7FF_FFFD_E000 },
            LiveTeb { cpu: 1, teb: 0x7FF_FFFD_A000 },
        ];
        assert_eq!(cpu_running(Some(0x7FF_FFFD_A000), &live), Some(1));
        assert_eq!(cpu_running(Some(0x7FF_FFFD_C000), &live), None);
        assert_eq!(cpu_running(None, &live), None, "a kernel thread has no TEB");
    }

    /// A module's imports are read from its image in guest memory at its
    /// base, through the processor's page tables or a process's alike.
    #[test]
    fn a_modules_imports_are_read_from_its_image_in_memory() {
        let mut machine =
            Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
                .expect("machine");
        let base = CpuSetupMode::FlatLong64.first_free_physical_address();
        machine
            .mem_write(base, &pe::tests::importing_image(true))
            .expect("write the image");
        let mut cpu = machine.debug_cpu(0).expect("the boot processor");
        let cr3 = cpu.view().registers.cr3;
        let module = WinModule {
            base,
            size: 0x3000,
            name: Some("app.exe".to_owned()),
            path: None,
            bits: ModuleBits::Bits64,
        };
        for root in [Root::Live, Root::Process(cr3)] {
            let read = read_module_detail(
                &mut cpu,
                ImageSpace { root, cr3 },
                &module,
                ModuleDetail::Imports,
            );
            let PickedDetail::Imports(ImportRead::Listed(modules)) = read else {
                panic!("{root:?}: the imports read as {read:?}");
            };
            assert_eq!(modules[0].name.as_deref(), Some("ntdll.dll"), "{root:?}");
            assert_eq!(modules[0].functions[0].bound, Some(0x7FF8_1380), "{root:?}");
        }
    }

    /// A KPCR-shaped address that does not read says the kernel is not
    /// mapped under these page tables, rather than that none was found.
    #[test]
    fn a_kernel_address_that_does_not_read_is_an_unmapped_kernel() {
        let mut machine =
            Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
                .expect("machine");
        let mut cpu = machine.debug_cpu(0).expect("the boot processor");
        let mut view = cpu.view();
        view.kernel_gs_base = 0xFFFF_F780_0000_0000;
        let read = read_windows(
            &mut cpu,
            &view,
            SystemView::KernelModules {
                selected: Some(0xFFFF_F800_0100_0000),
                detail: ModuleDetail::Exports,
            },
            &mut WindowsDiscovery::default(),
            None,
        );
        assert_eq!(
            read.system,
            SystemListing::NoWindowsKernel(KernelMiss::KernelUnmapped)
        );
    }

    /// The walks find a kernel through the KPCR, its processes through
    /// `ActiveProcessLinks` and its modules through `PsLoadedModuleList`,
    /// whatever answers the reads.
    #[test]
    fn the_walks_find_a_kernel_laid_out_in_a_page_map() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};

        let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
        let view = fake.view;
        let read = read_windows(
            &mut fake.guest,
            &view,
            SystemView::Processes {
                selected: None,
                detail: ProcessDetail::Modules,
                module: None,
                module_detail: ModuleDetail::Sections,
            },
            &mut WindowsDiscovery::default(),
            None,
        );
        let SystemListing::Processes(listing) = read.system else {
            panic!("the processes read as {:?}", read.system);
        };
        let processes: Vec<&str> = listing.processes.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(processes, ["System", "smss.exe", "csrss.exe"]);
        let pids: Vec<u64> = listing.processes.iter().map(|p| p.pid).collect();
        assert_eq!(pids, [4, 0x1F0, 0x26C]);
        assert_eq!(listing.processes[1].eprocess, fake.eprocess(1));
        assert!(listing.processes[0].current, "System runs in the live address space");
        let modules: Vec<&str> = read.symbols.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(modules, ["ntoskrnl.exe", "disk.sys"]);
    }

    /// The kernel PDB's layouts, as the shell would hand them over for the
    /// PDB `fixture` describes.
    fn pdb_layout(fixture: &rusty_box_symbols_fixtures::pdb::PdbFixture) -> Arc<KernelLayout> {
        layout_from(fixture, crate::debugger::kernel_layout::LayoutSource::Matched)
    }

    fn layout_from(
        fixture: &rusty_box_symbols_fixtures::pdb::PdbFixture,
        source: crate::debugger::kernel_layout::LayoutSource,
    ) -> Arc<KernelLayout> {
        let bytes = rusty_box_symbols_fixtures::pdb::build_pdb(fixture);
        let tables = rusty_box_symbols::tables::SymbolTables::parse(&bytes).expect("the fixture parses");
        Arc::new(KernelLayout::from_tables(crate::symbols::kernel::tests::kernel_identity(), &tables, source))
    }

    /// A PDB of another build the owner loaded by hand, which agrees with
    /// memory, seeds the walks, and the listings say whose layouts they are.
    #[test]
    fn layouts_from_a_forced_pdb_say_so() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
        let view = fake.view;
        let layout = layout_from(&windows10_kernel(), crate::debugger::kernel_layout::LayoutSource::Forced);
        let read = read_windows(&mut fake.guest, &view, PROCESSES, &mut WindowsDiscovery::default(), Some(&layout));
        assert_eq!(read.layouts, Some(KernelLayouts::ForcedPdb));
        assert!(KernelLayouts::ForcedPdb.label().contains("another build"));
    }

    fn windows10_kernel() -> rusty_box_symbols_fixtures::pdb::PdbFixture {
        crate::debugger::kernel_layout::fixture::windows10_kernel([0x42; 16], 1)
    }

    fn listed_processes(read: &WindowsRead) -> Vec<String> {
        match &read.system {
            SystemListing::Processes(listing) => listing.processes.iter().map(|p| format!("{} {}", p.pid, p.name)).collect(),
            other => panic!("the processes read as {other:?}"),
        }
    }

    /// One guest lists the same processes whether its kernel is read by the
    /// PDB's layouts or by discovery, and each listing says which.
    #[test]
    fn the_same_guest_lists_the_same_processes_under_pdb_and_discovered_layouts() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
        let view = fake.view;
        let mut discovery = WindowsDiscovery::default();
        let discovered = read_windows(&mut fake.guest, &view, PROCESSES, &mut discovery, None);
        assert_eq!(discovered.layouts, Some(KernelLayouts::Discovered));
        let layout = pdb_layout(&windows10_kernel());
        let by_pdb = read_windows(&mut fake.guest, &view, PROCESSES, &mut discovery, Some(&layout));
        assert_eq!(by_pdb.layouts, Some(KernelLayouts::Pdb));
        assert_eq!(listed_processes(&by_pdb), listed_processes(&discovered));
        assert_eq!(listed_processes(&by_pdb), ["4 System", "496 smss.exe", "620 csrss.exe"]);
    }

    /// Review focus 2: a PDB of another build (here, an older layout of
    /// `_EPROCESS`) is caught by the System cross-check, and the walks fall
    /// back to discovery and still list every process.
    #[test]
    fn a_kernel_pdb_that_disagrees_with_memory_falls_back_to_discovery() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
        let view = fake.view;
        let mut wrong = windows10_kernel();
        if let Some(eprocess) = wrong.types.iter_mut().find(|udt| udt.name == "_EPROCESS") {
            for field in &mut eprocess.fields {
                match field.name.as_str() {
                    "UniqueProcessId" => field.offset = 0x2E0,
                    "ActiveProcessLinks" => field.offset = 0x2E8,
                    _ => {}
                }
            }
        }
        let layout = pdb_layout(&wrong);
        let read = read_windows(&mut fake.guest, &view, PROCESSES, &mut WindowsDiscovery::default(), Some(&layout));
        assert_eq!(read.layouts, Some(KernelLayouts::PdbDisagrees));
        assert_eq!(listed_processes(&read), ["4 System", "496 smss.exe", "620 csrss.exe"]);
    }

    /// A PDB lacking one `_EPROCESS` member, or the `PsInitialSystemProcess`
    /// symbol, still seeds the rest: just that one falls back to discovery
    /// (spec §8.1). A lack is no disagreement.
    #[test]
    fn a_kernel_pdb_missing_one_field_falls_back_for_that_field_alone() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        let without_links = {
            let mut pdb = windows10_kernel();
            if let Some(eprocess) = pdb.types.iter_mut().find(|udt| udt.name == "_EPROCESS") {
                eprocess.fields.retain(|field| field.name != "ActiveProcessLinks");
            }
            pdb
        };
        let without_system = {
            let mut pdb = windows10_kernel();
            pdb.publics.retain(|public| public.name != "PsInitialSystemProcess");
            pdb
        };
        for pdb in [without_links, without_system] {
            let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
            let view = fake.view;
            let layout = pdb_layout(&pdb);
            let read = read_windows(&mut fake.guest, &view, PROCESSES, &mut WindowsDiscovery::default(), Some(&layout));
            assert_eq!(read.layouts, Some(KernelLayouts::Pdb));
            assert_eq!(listed_processes(&read), ["4 System", "496 smss.exe", "620 csrss.exe"]);
        }
    }

    /// A PDB that agrees with memory seeds every offset the walks would
    /// otherwise search for, its `ETHREAD` members relative to the thread
    /// list's entry.
    #[test]
    fn a_kernel_pdb_seeds_what_the_walks_would_discover() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec, KERNEL_BASE};
        let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
        let view = fake.view;
        let layout = pdb_layout(&windows10_kernel());
        let state = discover_kernel(&mut fake.guest, &view, Some(&layout)).expect("the kernel");
        assert_eq!(state.layouts, KernelLayouts::Pdb);
        assert_eq!(state.peb_offset, Some(0x550));
        assert_eq!(state.parent_offset, Some(0x540));
        assert_eq!(state.wow64_offset, Some(0x580));
        let threads = state.threads.expect("the thread layout");
        assert_eq!(threads.head, 0x5E0);
        assert_eq!(threads.cid, 0x478 - 0x4E8);
        assert_eq!(threads.teb, Some(0xF0 - 0x4E8));
        assert_eq!(threads.trap_frame, Some(0x90 - 0x4E8));
        let start = threads.start.expect("both start addresses");
        assert_eq!(start.win32, 0x4D0 - 0x4E8);
        assert_eq!(start.kernel, 0x450 - 0x4E8);
        let vad = state.vad.expect("the VAD site");
        assert_eq!(vad.offset, 0x7D8);
        // The PDB fixture is Windows 10 22H2's, build 19045.
        assert_eq!(vad.layout, vad::shape_layout(VadShape::BalancedNode, Some(19045)));
        assert_eq!(state.object_era, Some(drivers::ObjectHeaderEra::Modern));
        let pdb = |address| Some(FoundDescriptor { address, source: TableSource::Pdb });
        assert_eq!(state.ssdt.kernel, pdb(KERNEL_BASE + 0x3040));
        assert_eq!(state.ssdt.shadow, pdb(KERNEL_BASE + 0x3080));
        assert_eq!(state.kernel.layout.directory_table_base, 0x28);
    }

    /// The kernel's build is read from its `NtBuildNumber` export, its free-
    /// build nibble dropped; a kernel without the export has no build, so
    /// its regions' kinds read unknown rather than by a guessed era.
    #[test]
    fn the_kernels_build_comes_from_its_nt_build_number_export() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
        let view = fake.view;
        let state = discover_kernel(&mut fake.guest, &view, None).expect("the kernel");
        assert_eq!(state.kernel.build, Some(19045));
        let by_pdb = discover_kernel(&mut fake.guest, &view, Some(&pdb_layout(&windows10_kernel()))).expect("the kernel");
        assert_eq!(by_pdb.kernel.build, Some(19045));
        let mut unexported = FakeWindows::build(FakeWindowsSpec { build_number: None, ..FakeWindowsSpec::typical() });
        let view = unexported.view;
        let state = discover_kernel(&mut unexported.guest, &view, None).expect("the kernel");
        assert_eq!(state.kernel.build, None);
    }

    fn ssdt_of(read: WindowsRead) -> SsdtListing {
        match read.system {
            SystemListing::Ssdt(listing) => listing,
            other => panic!("the SSDT read as {other:?}"),
        }
    }

    fn stub_names(table: &ServiceTableRead) -> Vec<String> {
        table.services.iter().map(|service| service.stub_name.clone().unwrap_or_default()).collect()
    }

    fn read_services(fake: &mut crate::debugger::fake_windows::FakeWindows, layout: Option<&Arc<KernelLayout>>) -> SsdtListing {
        let view = fake.view;
        ssdt_of(read_windows(&mut fake.guest, &view, SystemView::Ssdt, &mut WindowsDiscovery::default(), layout))
    }

    /// With no PDB and no export, both tables are found through
    /// `KiSystemServiceRepeat`'s `lea r10` and `lea r11`, and named by
    /// `ntdll`'s and `win32u`'s stubs; the GUI table is read through
    /// `csrss.exe`, whose session alone maps it.
    #[test]
    fn both_service_tables_are_found_in_code_and_named_by_their_stubs() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        let mut fake = FakeWindows::build(FakeWindowsSpec::with_services());
        let listing = read_services(&mut fake, None);
        let kernel = &listing.kernel;
        assert_eq!(kernel.source, TableSource::Code);
        assert_eq!(kernel.place, TablePlace::InOwner);
        assert_eq!(stub_names(kernel), ["NtClose", "NtCreateFile", "NtOpenProcess"]);
        assert_eq!(kernel.services[1].routine, FakeWindows::kernel_routine(1));
        assert_eq!(kernel.services[1].site, PointerSite::OwnImage);
        assert!(!kernel.any_hooked);
        let GuiServices::Listed(gui) = &listing.gui else {
            panic!("the GUI table read as {:?}", listing.gui);
        };
        assert_eq!(gui.source, TableSource::Code);
        assert_eq!(stub_names(gui), ["NtUserGetMessage", "NtGdiBitBlt"]);
        assert_eq!(gui.services[0].routine, FakeWindows::gui_routine(0));
        assert_eq!(gui.services[0].site, PointerSite::OwnImage);
    }

    /// The kernel PDB names where both descriptors are.
    #[test]
    fn a_kernel_pdb_places_both_descriptors() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        let mut fake = FakeWindows::build(FakeWindowsSpec::with_services());
        let layout = pdb_layout(&windows10_kernel());
        let listing = read_services(&mut fake, Some(&layout));
        assert_eq!(listing.kernel.source, TableSource::Pdb);
        assert!(matches!(&listing.gui, GuiServices::Listed(gui) if gui.source == TableSource::Pdb), "{:?}", listing.gui);
    }

    /// An entry that leads into another driver is hooked; a routine whose
    /// first instruction jumps into another driver is an inline hook.
    #[test]
    fn an_entry_or_a_first_jump_out_of_the_kernel_is_a_hook() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec, DISK_BASE};
        let mut fake = FakeWindows::build(FakeWindowsSpec::with_services());
        fake.set_kernel_entry(1, DISK_BASE + 0x100);
        let patched = FakeWindows::kernel_routine(2);
        let target = DISK_BASE + 0x200;
        let mut jump = vec![0xE9];
        jump.extend_from_slice(&(target.wrapping_sub(patched + 5) as u32).to_le_bytes());
        fake.patch_kernel_routine(2, &jump);
        let listing = read_services(&mut fake, None);
        let services = &listing.kernel.services;
        assert_eq!(services[1].site, PointerSite::InModule { base: DISK_BASE, off: 0x100 });
        assert_eq!(services[1].module_name.as_deref(), Some("disk.sys"));
        assert!(listing.kernel.any_hooked);
        assert_eq!(services[2].site, PointerSite::OwnImage);
        assert_eq!(services[2].inline, Some(InlineJump { target, module_name: Some("disk.sys".to_owned()) }));
        assert_eq!(services[0].inline, None);
    }

    /// A first instruction that jumps through a slot (`jmp [rip+0]`) is an
    /// inline hook to where the slot points.
    #[test]
    fn a_jump_through_a_slot_out_of_the_kernel_is_an_inline_hook() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec, DISK_BASE};
        let mut fake = FakeWindows::build(FakeWindowsSpec::with_services());
        let target = DISK_BASE + 0x300;
        let mut jump = vec![0xFF, 0x25, 0, 0, 0, 0];
        jump.extend_from_slice(&target.to_le_bytes());
        fake.patch_kernel_routine(0, &jump);
        let listing = read_services(&mut fake, None);
        let service = &listing.kernel.services[0];
        assert_eq!(service.site, PointerSite::OwnImage);
        assert_eq!(service.inline, Some(InlineJump { target, module_name: Some("disk.sys".to_owned()) }));
        assert!(listing.kernel.any_hooked);
    }

    /// A kernel table swapped whole — `KeServiceDescriptorTable` pointed at a
    /// copy in another driver — is found through the PDB, listed and called
    /// moved; the shadow, left alone, still gives the GUI table.
    #[test]
    fn a_table_swapped_whole_is_listed_as_moved() {
        use crate::debugger::fake_windows::{
            FakeWindows, FakeWindowsSpec, DISK_BASE, KERNEL_BASE, LIVE_CR3, SERVICE_DESCRIPTOR_RVA,
        };
        let mut fake = FakeWindows::build(FakeWindowsSpec::with_services());
        let copy = DISK_BASE + 0x1000;
        for index in 0..3u32 {
            let offset = FakeWindows::kernel_routine(index).wrapping_sub(copy) as i64;
            fake.guest.write_u32(LIVE_CR3, copy + 4 * u64::from(index), ((offset << 4) | 2) as u32);
        }
        fake.guest.write_u64(LIVE_CR3, KERNEL_BASE + SERVICE_DESCRIPTOR_RVA, copy);
        let layout = pdb_layout(&windows10_kernel());
        let listing = read_services(&mut fake, Some(&layout));
        assert_eq!(listing.kernel.place, TablePlace::Outside);
        assert_eq!(listing.kernel.base, copy);
        assert_eq!(listing.kernel.services[2].routine, FakeWindows::kernel_routine(2));
        assert_eq!(stub_names(&listing.kernel), ["NtClose", "NtCreateFile", "NtOpenProcess"]);
        assert!(matches!(&listing.gui, GuiServices::Listed(gui) if gui.services.len() == 2), "{:?}", listing.gui);
    }

    /// A guest whose GUI table cannot be read says why: no `win32k`, or no
    /// process but System maps the table.
    #[test]
    fn a_gui_table_that_cannot_be_read_says_why() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        let mut spec = FakeWindowsSpec::with_services();
        spec.drivers.retain(|driver| driver.name != "win32k.sys");
        let listing = read_services(&mut FakeWindows::build(spec), None);
        assert_eq!(listing.gui, GuiServices::Missing(GuiMiss::NoWin32k));
        assert_eq!(listing.kernel.services.len(), 3);
        let mut spec = FakeWindowsSpec::with_services();
        if let Some(services) = spec.services.as_mut() {
            services.session_process = 0;
        }
        let listing = read_services(&mut FakeWindows::build(spec), None);
        assert_eq!(listing.gui, GuiServices::Missing(GuiMiss::NoSessionProcess));
    }

    /// Under KVA shadow a user-mode pause runs on tables that leave the
    /// kernel out; the KPRCB page the entry stubs read gives the kernel's
    /// tables, and the walk reads through them: the processes list, and the
    /// running one is the process those tables belong to.
    #[test]
    fn a_user_mode_pause_under_kva_shadow_reads_through_the_kernels_tables() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec, KPRCB_SHADOW_OFFSET};
        let mut fake = FakeWindows::build(FakeWindowsSpec::with_services());
        let csrss = 0x3_0000;
        // The kernel's CR3 with PCID 2 in its low bits.
        fake.pause_in_user_mode_under_kva_shadow(csrss, csrss | 2);
        let view = fake.view;
        let mut discovery = WindowsDiscovery::default();
        let read = read_windows(&mut fake.guest, &view, PROCESSES, &mut discovery, None);
        assert_eq!(read.through, ReadThrough::KernelTables { cr3: csrss | 2 });
        let SystemListing::Processes(listing) = &read.system else {
            panic!("the processes read as {:?}", read.system);
        };
        assert_eq!(listing.processes.len(), 3);
        let current: Vec<&str> = listing.processes.iter().filter(|p| p.current).map(|p| p.name.as_str()).collect();
        assert_eq!(current, ["csrss.exe"]);
        assert!(matches!(&listing.modules, Some(modules) if matches!(&modules.read, ModuleRead::Listed(listed) if listed.len() == 3)));
        assert_eq!(discovery.kva_shadow, Some(KPRCB_SHADOW_OFFSET));
        let ssdt = ssdt_of(read_windows(&mut fake.guest, &view, SystemView::Ssdt, &mut discovery, None));
        assert_eq!(ssdt.kernel.services.len(), 3);
    }

    /// A value in the KPRCB's shadow page that maps the kernel but belongs
    /// to another process (the paused code does not map there) is not
    /// taken; with no kernel tables found the kernel reads unmapped.
    #[test]
    fn a_shadow_page_without_this_processs_kernel_tables_reads_unmapped() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        for value in [0x2_0000, 0] {
            let mut fake = FakeWindows::build(FakeWindowsSpec::with_services());
            fake.pause_in_user_mode_under_kva_shadow(0x3_0000, value);
            let view = fake.view;
            let read = read_windows(&mut fake.guest, &view, PROCESSES, &mut WindowsDiscovery::default(), None);
            assert!(
                matches!(read.system, SystemListing::NoWindowsKernel(KernelMiss::KernelUnmapped)),
                "{value:#x}: {:?}",
                read.system
            );
        }
    }

    /// A pause in a hypervisor (VMX root operation) shows the VMM, whose
    /// registers lead to no Windows; read as the guest its VMCS holds, the
    /// processes list, and the GUI services read in the session process's
    /// space inside that guest.
    #[test]
    fn a_pause_in_a_hypervisor_reads_windows_as_the_guest_its_vmcs_holds() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        use crate::debugger::guest::{vmx_guest_view, LiveAt, PausedCpu};
        let fake = FakeWindows::build(FakeWindowsSpec::with_services());
        let user_process = fake.eprocess(1);
        let mut pause = fake.pause_in_a_hypervisor();
        let mut own = PausedCpu { cpu: &mut pause.host, vmx_guest: None };
        let read = read_windows(&mut own, &pause.view, PROCESSES, &mut WindowsDiscovery::default(), None);
        assert!(matches!(read.system, SystemListing::NoWindowsKernel(_)), "{:?}", read.system);

        let view = vmx_guest_view(&pause.guest, &pause.view);
        let mut paused = PausedCpu { cpu: &mut pause.host, vmx_guest: Some(pause.guest) };
        let mut guest = LiveAt { memory: &mut paused, live_cr3: Some(pause.guest.cr3) };
        let mut discovery = WindowsDiscovery::default();
        let read = read_windows(&mut guest, &view, PROCESSES, &mut discovery, None);
        let SystemListing::Processes(listing) = &read.system else {
            panic!("the processes read as {:?}", read.system);
        };
        assert_eq!(listing.processes.len(), 3);
        let ssdt = ssdt_of(read_windows(&mut guest, &view, SystemView::Ssdt, &mut discovery, None));
        assert_eq!(ssdt.kernel.services.len(), 3);
        assert!(matches!(ssdt.gui, GuiServices::Listed(_)), "{:?}", ssdt.gui);

        let memory_map = SystemView::Processes {
            selected: Some(user_process),
            detail: ProcessDetail::MemoryMap,
            module: None,
            module_detail: ModuleDetail::Sections,
        };
        let read = read_windows(&mut guest, &view, memory_map, &mut discovery, None);
        let SystemListing::Processes(listing) = &read.system else {
            panic!("the processes read as {:?}", read.system);
        };
        let map = listing.modules.as_ref().and_then(|modules| modules.map.as_ref());
        let resident: u64 = map.map_or(0, |map| map.regions.iter().map(|region| region.resident_pages).sum());
        assert!(resident > 0, "the process's pages are walked in the guest: {:?}", listing.modules);
    }

    /// A kernel-mode pause reads through the processor's own tables.
    #[test]
    fn a_kernel_mode_pause_reads_through_the_processors_tables() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec};
        let mut fake = FakeWindows::build(FakeWindowsSpec::with_services());
        let view = fake.view;
        let read = read_windows(&mut fake.guest, &view, PROCESSES, &mut WindowsDiscovery::default(), None);
        assert_eq!(read.through, ReadThrough::ProcessorTables);
    }

    /// The processes view every listing test asks for.
    pub(crate) const PROCESSES: SystemView = SystemView::Processes {
        selected: None,
        detail: ProcessDetail::Modules,
        module: None,
        module_detail: ModuleDetail::Sections,
    };

    fn process_names(read: &WindowsRead) -> Vec<String> {
        match &read.system {
            SystemListing::Processes(listing) => listing.processes.iter().map(|p| p.name.clone()).collect(),
            other => panic!("the processes read as {other:?}"),
        }
    }

    /// Review focus 1: a process whose `Flink` leaves for the top of the
    /// address space ends the list there; what was read stays listed.
    #[test]
    fn a_process_ring_that_leaves_for_the_top_of_memory_ends_the_list() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec, LIVE_CR3};
        let spec = FakeWindowsSpec::typical();
        let links = spec.eprocess.unique_process_id + 8;
        let mut fake = FakeWindows::build(spec);
        let smss = fake.eprocess(1);
        fake.guest.write_u64(LIVE_CR3, smss + links, 0xFFFF_FFFF_FFFF_FFF8);
        let view = fake.view;
        let read = read_windows(&mut fake.guest, &view, PROCESSES, &mut WindowsDiscovery::default(), None);
        assert_eq!(process_names(&read), ["System", "smss.exe"]);
    }

    /// Review focus 1: an export table that claims four billion names, at an
    /// RVA that wraps past the top of memory, names nothing, so no kernel is
    /// found and nothing is listed.
    #[test]
    fn an_export_table_that_claims_every_name_finds_no_kernel() {
        use crate::debugger::fake_windows::{FakeWindows, FakeWindowsSpec, EXPORTS_RVA, KERNEL_BASE, LIVE_CR3};
        let mut fake = FakeWindows::build(FakeWindowsSpec::typical());
        let directory = KERNEL_BASE + EXPORTS_RVA;
        fake.guest.write_u32(LIVE_CR3, directory + 0x18, u32::MAX);
        fake.guest.write_u32(LIVE_CR3, directory + 0x20, 0xFFFF_FFF0);
        let view = fake.view;
        let read = read_windows(&mut fake.guest, &view, PROCESSES, &mut WindowsDiscovery::default(), None);
        assert!(read.symbols.modules.is_empty(), "{:?}", read.symbols.modules);
    }

    /// Review focus 1: a driver whose image would run past the top of the
    /// address space is still listed, and reading its sections ends cleanly.
    #[test]
    fn a_module_whose_size_runs_past_the_top_of_memory_is_still_listed() {
        use crate::debugger::fake_windows::{FakeImage, FakePdb, FakeWindows, FakeWindowsSpec};
        let edge = 0xFFFF_FFFF_FFF0_0000;
        let mut spec = FakeWindowsSpec::typical();
        spec.drivers.push(FakeImage {
            name: "edge.sys",
            base: edge,
            size: 0x20_0000,
            time_date_stamp: 1,
            pdb: FakePdb { guid: [0x22; 16], age: 1, path: "edge.pdb" },
            sections: Vec::new(),
        });
        let mut fake = FakeWindows::build(spec);
        let view = fake.view;
        let request = SystemView::KernelModules { selected: Some(edge), detail: ModuleDetail::Sections };
        let read = read_windows(&mut fake.guest, &view, request, &mut WindowsDiscovery::default(), None);
        let modules: Vec<&str> = read.symbols.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(modules, ["ntoskrnl.exe", "disk.sys", "edge.sys"]);
    }

    #[test]
    fn a_page_number_that_would_wrap_is_not_a_region() {
        let wrapped = (1u64 << 52) | 0x10;
        assert!(wrapped << 12 < USER_SPACE_END, "the shift alone would wrap into the user half");
        assert!(wrapped >= USER_SPACE_END >> 12, "so the page number itself is what is refused");
    }

    #[test]
    fn code_pointers_are_told_from_data_by_the_ranges_of_loaded_images() {
        let module = |base: u64, size: u32| WinModule {
            base,
            size,
            name: None,
            path: None,
            bits: ModuleBits::Bits64,
        };
        let ranges = code_ranges(
            &[module(0xFFFF_F800_0100_0000, 0x40_0000)],
            &[module(0x7FEF_0000_0000, 0x13_0000), module(0x40_0000, 0x1000)],
        );
        assert!(in_code_range(&ranges, 0x40_0FFF));
        assert!(!in_code_range(&ranges, 0x40_1000));
        assert!(in_code_range(&ranges, 0x7FEF_0000_1A2C));
        assert!(in_code_range(&ranges, 0xFFFF_F800_0100_0100));
        assert!(!in_code_range(&ranges, 0));
        assert!(!in_code_range(&[], 0x40_0000));
    }

    fn process(pid: u64, parent: Option<u64>) -> WinProcess {
        WinProcess {
            eprocess: pid,
            pid,
            parent,
            name: format!("p{pid}"),
            cr3: 0,
            current: false,
        }
    }

    /// System → smss → {csrss, winlogon → services}, and explorer, whose
    /// parent has exited, at the top beside System.
    #[test]
    fn a_process_tree_nests_children_under_their_parent() {
        let processes = [
            process(4, None),
            process(220, Some(4)),
            process(276, Some(220)),
            process(300, Some(220)),
            process(348, Some(300)),
            process(1500, Some(1400)),
        ];
        let rows = process_tree(&processes);
        let shape: Vec<(u64, usize, bool)> = rows
            .iter()
            .map(|row| (processes[row.index].pid, row.depth, row.last))
            .collect();
        assert_eq!(
            shape,
            [
                (4, 0, false),
                (220, 1, true),
                (276, 2, false),
                (300, 2, true),
                (348, 3, true),
                (1500, 0, true),
            ]
        );
        // services sits under winlogon, the last child of smss, the last
        // child of System: no guide runs on past it at either level.
        assert_eq!(rows[4].guides, [false, false]);
        // csrss has a later sibling (winlogon): its children would see a guide.
        assert_eq!(rows[2].guides, [false]);
    }

    #[test]
    fn a_cycle_of_parents_still_lists_every_process() {
        let processes = [process(8, Some(12)), process(12, Some(8))];
        assert_eq!(process_tree(&processes).len(), 2);
    }

    #[test]
    fn an_address_space_is_its_table_address_whatever_its_flags() {
        assert!(same_address_space(0x1234_5000, 0x1234_5018));
        assert!(!same_address_space(0x1234_5000, 0x1234_6000));
    }

    #[test]
    fn an_mm_avl_table_root_is_its_self_referential_sentinel_right_child() {
        let table = 0xFFFF_FADF_F3F2_4398;
        let root = 0xFFFF_FADF_F39E_5880;
        // The real sentinel: Parent points to the table itself (the low two
        // bits carry the balance), LeftChild is null, the root is RightChild.
        assert_eq!(avl_table_root(table, 0, root, table), kernel_pointer(root));
        assert_eq!(avl_table_root(table | 0x2, 0, root, table), kernel_pointer(root));
        // A wrong EPROCESS field: its first word is not a self-pointer, so it
        // is not mistaken for a VAD root even if a later word reads as a node.
        assert_eq!(avl_table_root(0xFFFF_FADF_F3E1_E410, 0, root, table), None);
        // A non-empty balanced tree's left child is set: not a VAD sentinel.
        assert_eq!(avl_table_root(table, root, root, table), None);
        // The right child must be a kernel pointer.
        assert_eq!(avl_table_root(table, 0, 0, table), None);
    }

    /// The owner's program runs in `csrss.exe` here: its module is read in
    /// that process's address space, while the pause stands in System.
    #[test]
    fn the_owners_modules_are_found_in_every_other_process() {
        let mut fake = crate::debugger::fake_windows::FakeWindows::build(
            crate::debugger::fake_windows::FakeWindowsSpec::with_services(),
        );
        let view = fake.view;
        let mut discovery = WindowsDiscovery::default();
        let names = ["CSRSS.EXE".to_owned()];
        let find = FindElsewhere { names: &names, shown: None };
        let read = read_windows_finding(&mut fake.guest, &view, SystemView::Off, find, &mut discovery, None).elsewhere;
        assert_eq!(read.names, names, "what was looked for");
        assert_eq!(read.modules.len(), 1, "{:?}", read.modules);
        let found = &read.modules[0];
        assert_eq!(found.cr3, 0x3_0000);
        assert_eq!(found.process, "csrss.exe");
        assert_eq!(found.module.base, 0x4A00_0000);
        assert_eq!(found.module.space, ModuleSpace::User);
        assert!(
            matches!(&found.module.identity, GuestIdentity::Known(rusty_box_symbols::identity::ModuleIdentity::Pe { file_name, pdb: Some(_), .. }) if file_name == "csrss.exe"),
            "{:?}",
            found.module.identity
        );
        let none = read_windows_finding(&mut fake.guest, &view, SystemView::Off, FindElsewhere::NOTHING, &mut discovery, None);
        assert!(none.elsewhere.modules.is_empty(), "no names, nothing read");
        // A thread view in csrss names csrss's modules already.
        let viewing = FindElsewhere { names: &names, shown: Some(0x3_0000) };
        let read = read_windows_finding(&mut fake.guest, &view, SystemView::Off, viewing, &mut discovery, None);
        assert!(read.elsewhere.modules.is_empty(), "{:?}", read.elsewhere.modules);
    }
}
