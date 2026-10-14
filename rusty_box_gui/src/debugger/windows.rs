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
//! (`DebugCpu::dbg_xlate_linear2phy_in`), so every process's modules list,
//! not only the running one's.
//!
//! What the walks discover about a guest's layouts is held in a
//! [`WindowsDiscovery`] for the run and checked where it is used, so a pause
//! pays for the lists it reads and not for finding the structures again.

use std::collections::HashSet;

use rusty_box::cpu::{
    CpuDebugView, Fetched, Instrumentation, LinearRange, MappingWalk, WalkStep,
};
use rusty_box::emulator::{DebugCpu, SliceEngine};

use super::inspect::{ModuleSpace, SymModule, SymbolIndex};
use super::memory_map::{self, MemoryMap, ModuleLayout};
use super::pe::{self, HEADER_BYTES};
use super::threads::{
    decode_trap_frame, frame_is_users, walk_thread_list, SavedRegisters, WinThread, MAX_THREADS,
    TRAP_FRAME_BYTES,
};
use super::vad::{self, VadNode, VadRegion, VadShape, MAX_VAD_NODES};

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
    /// running process when none is selected — either its modules or its
    /// memory map.
    Processes {
        selected: Option<u64>,
        detail: ProcessDetail,
    },
    /// The drivers and the kernel itself, in load order.
    KernelModules,
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
    KernelModules(Vec<WinModule>),
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
}

impl KernelMiss {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::NotLongMode => "the processor is not in long mode",
            Self::NoKpcr => "neither GS nor KERNEL_GS_BASE points at a KPCR",
            Self::KernelUnmapped => {
                "the kernel is not mapped in the page tables this pause runs on; pause \
                 while the guest runs kernel code"
            }
            Self::NoKernelImage => "no kernel image exporting PsInitialSystemProcess was found",
            Self::NoSystemProcess => "PsInitialSystemProcess is not set",
            Self::UnknownProcessLayout => "the System process's EPROCESS was not recognised",
            Self::NoModuleList => "the kernel's loaded-module list was not found",
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

/// Whose page tables a read walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Root {
    /// The shown processor's own CR3.
    Live,
    /// A process's `DirectoryTableBase`.
    Process(u64),
}

/// The kernel image found in the guest.
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
}

struct Kernel {
    image: PeImage,
    /// The loaded-module list's head, when one was found.
    loaded_modules: Option<u64>,
    system: u64,
    layout: ProcessLayout,
}

/// Where a process's VAD tree root sits in its `EPROCESS`, and the node shape
/// that tree uses.
#[derive(Clone, Copy)]
struct VadSite {
    shape: VadShape,
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
}

/// What the walks have learned about one guest's structure layouts. A run
/// keeps one, so a pause reads lists instead of finding the structures
/// again; it is dropped when a new run begins.
#[derive(Default)]
pub(crate) struct WindowsDiscovery {
    state: Option<KernelState>,
}

/// What a guest's Windows structures gave one pause.
pub(crate) struct WindowsRead {
    /// The modules addresses are named against.
    pub(crate) symbols: SymbolIndex,
    /// What the view asked to be listed.
    pub(crate) system: SystemListing,
}

/// The modules the code, stack and memory panes name an address against (the
/// kernel's loaded modules, and the shown processor's process's own modules
/// when it runs one) and the listing `request` asks for, through the
/// processor `view` describes. The symbols are read whatever is requested.
/// A read that comes up short leaves the index short; it never fails the
/// snapshot, so a non-Windows guest simply names nothing.
pub(crate) fn read_windows<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    view: &CpuDebugView,
    request: SystemView,
    discovery: &mut WindowsDiscovery,
) -> WindowsRead {
    let state = match find_kernel(cpu, view, &mut discovery.state) {
        Ok(state) => state,
        Err(miss) => {
            let system = match request {
                SystemView::Off => SystemListing::NotAsked,
                SystemView::Processes { .. } | SystemView::KernelModules => {
                    SystemListing::NoWindowsKernel(miss)
                }
            };
            return WindowsRead {
                symbols: SymbolIndex::default(),
                system,
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
    } = state;
    let kernel_modules = match kernel.loaded_modules {
        Some(head) => list_modules(cpu, Root::Live, head).unwrap_or_default(),
        None => Vec::new(),
    };
    let processes = list_processes(cpu, kernel, view.registers.cr3, parent_offset);
    let current_index = processes.iter().position(|process| process.current);
    let current_read = current_index.map(|index| {
        process_modules(cpu, &processes, &processes[index], peb_offset, wow64_offset)
    });

    let mut named = Vec::new();
    gather_modules(&mut named, &kernel_modules, ModuleSpace::Kernel);
    if let Some(ModuleRead::Listed(user)) = &current_read {
        gather_modules(&mut named, user, ModuleSpace::User);
    }
    let symbols = SymbolIndex { modules: named };

    let system = match request {
        SystemView::Off => SystemListing::NotAsked,
        SystemView::KernelModules => match kernel.loaded_modules {
            Some(_) => SystemListing::KernelModules(kernel_modules),
            None => SystemListing::NoWindowsKernel(KernelMiss::NoModuleList),
        },
        SystemView::Processes { selected, detail } => {
            let chosen_index = selected
                .and_then(|eprocess| processes.iter().position(|p| p.eprocess == eprocess))
                .or(current_index);
            let modules = chosen_index.map(|index| {
                let process = &processes[index];
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
                            read_memory_map(cpu, process.eprocess, process.cr3, modules, vad);
                        memory_map::label_stacks(&mut map, &threads);
                        Some(map)
                    }
                    _ => None,
                };
                ProcessModules {
                    eprocess: process.eprocess,
                    read,
                    map,
                    threads,
                }
            });
            let tree = process_tree(&processes);
            SystemListing::Processes(ProcessListing {
                processes,
                tree,
                modules,
            })
        }
    };
    WindowsRead { symbols, system }
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
        }
    }
}

/// What looking from a process or thread came to.
#[derive(Clone, Debug)]
pub(crate) enum ThreadViewRead {
    /// The process asked for is the one the processor runs: its live state is
    /// the view.
    Live,
    Thread(ThreadView),
    Miss(ViewMiss),
}

/// Finds `target` in the guest's process list and reads the thread to look
/// from: the one asked for, or for a process its first thread with saved user
/// state. A process the shown processor runs reads as `Live`. The walks are
/// the ones `read_windows` makes, through the same `discovery`.
pub(crate) fn read_thread_view<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    view: &CpuDebugView,
    target: ViewTarget,
    discovery: &mut WindowsDiscovery,
) -> ThreadViewRead {
    let state = match find_kernel(cpu, view, &mut discovery.state) {
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
    } = state;
    let kernel_modules = match kernel.loaded_modules {
        Some(head) => list_modules(cpu, Root::Live, head).unwrap_or_default(),
        None => Vec::new(),
    };
    let processes = list_processes(cpu, kernel, view.registers.cr3, parent_offset);
    let eprocess = match target {
        ViewTarget::Process { eprocess } | ViewTarget::Thread { eprocess, .. } => eprocess,
    };
    let Some(process) = processes.iter().find(|process| process.eprocess == eprocess) else {
        return ThreadViewRead::Miss(ViewMiss::NoSuchProcess);
    };
    if process.current && matches!(target, ViewTarget::Process { .. }) {
        return ThreadViewRead::Live;
    }
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
        ViewTarget::Process { .. } => threads.iter().find(|thread| thread.saved.is_some()),
        ViewTarget::Thread { tid, .. } => match threads.iter().find(|thread| thread.tid == tid) {
            Some(thread) => Some(thread),
            None => return ThreadViewRead::Miss(ViewMiss::NoSuchThread),
        },
    };
    let Some((thread, saved)) = chosen.and_then(|thread| thread.saved.map(|saved| (thread, saved)))
    else {
        return ThreadViewRead::Miss(ViewMiss::NoSavedState);
    };
    let mut named = Vec::new();
    gather_modules(&mut named, &kernel_modules, ModuleSpace::Kernel);
    gather_modules(&mut named, listed, ModuleSpace::User);
    ThreadViewRead::Thread(ThreadView {
        process: process.name.clone(),
        pid: process.pid,
        tid: thread.tid,
        cr3: process.cr3,
        saved,
        symbols: SymbolIndex { modules: named },
    })
}

/// Appends `source` modules to `into` as `space` symbols, naming an unnamed
/// module by its base so an address in it still resolves.
fn gather_modules(into: &mut Vec<SymModule>, source: &[WinModule], space: ModuleSpace) {
    for module in source {
        let name = module
            .name
            .clone()
            .unwrap_or_else(|| format!("{:X}", module.base));
        into.push(SymModule {
            base: module.base,
            size: u64::from(module.size),
            name,
            space,
        });
    }
}

/// EFER.LMA: the processor is in long mode.
const EFER_LMA: u64 = 1 << 10;

/// The kernel held in `cache` while it still stands in the guest, else the
/// one found there now.
fn find_kernel<'a, T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    view: &CpuDebugView,
    cache: &'a mut Option<KernelState>,
) -> Result<&'a mut KernelState, KernelMiss> {
    if view.efer & EFER_LMA == 0 {
        return Err(KernelMiss::NotLongMode);
    }
    let standing = cache
        .as_ref()
        .is_some_and(|state| kernel_stands(cpu, &state.kernel));
    if !standing {
        *cache = None;
        let kernel = discover_kernel(cpu, view)?;
        *cache = Some(KernelState {
            kernel,
            peb_offset: None,
            parent_offset: None,
            wow64_offset: None,
            threads: None,
            vad: None,
        });
    }
    cache.as_mut().ok_or(KernelMiss::NoKernelImage)
}

/// Whether `kernel`, found earlier, is still the guest's: its image header
/// reads and its System process still holds PID 4 at the learned offset.
fn kernel_stands<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    kernel: &Kernel,
) -> bool {
    read_u16(cpu, Root::Live, kernel.image.base) == Some(MZ)
        && read_u64(cpu, Root::Live, kernel.system + kernel.layout.pid) == Some(SYSTEM_PID)
}

/// Finds the kernel through the shown processor: its KPCR, the image below
/// interrupt vector 0's handler, and the System process.
fn discover_kernel<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    view: &CpuDebugView,
) -> Result<Kernel, KernelMiss> {
    let kpcr = match find_kpcr(cpu, view) {
        Some(kpcr) => kpcr,
        None if kpcr_unreadable(cpu, view) => return Err(KernelMiss::KernelUnmapped),
        None => return Err(KernelMiss::NoKpcr),
    };
    let KernelImage {
        image,
        system_variable,
    } = kernel_image(cpu, kpcr).ok_or(KernelMiss::NoKernelImage)?;
    let system = read_u64(cpu, Root::Live, system_variable)
        .and_then(kernel_pointer)
        .ok_or(KernelMiss::NoSystemProcess)?;
    let layout = process_layout(cpu, system).ok_or(KernelMiss::UnknownProcessLayout)?;
    let loaded_modules = loaded_module_list(cpu, &image);
    Ok(Kernel {
        image,
        loaded_modules,
        system,
        layout,
    })
}

/// The KPCR: GS in the kernel, `IA32_KERNEL_GS_BASE` in user mode (and in
/// the kernel between its entry and `SWAPGS`) — whichever points at itself.
fn find_kpcr<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    view: &CpuDebugView,
) -> Option<u64> {
    [view.segment_bases.gs, view.kernel_gs_base]
        .into_iter()
        .filter_map(kernel_pointer)
        .find(|&kpcr| read_u64(cpu, Root::Live, kpcr + 0x18) == Some(kpcr))
}

/// Whether a KPCR-shaped address is held but does not read, as when the
/// processor runs on page tables that leave the kernel out.
fn kpcr_unreadable<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    view: &CpuDebugView,
) -> bool {
    [view.segment_bases.gs, view.kernel_gs_base]
        .into_iter()
        .filter_map(kernel_pointer)
        .any(|kpcr| read_u64(cpu, Root::Live, kpcr + 0x18).is_none())
}

/// The kernel image and its `PsInitialSystemProcess`: the first image at or
/// below interrupt vector 0's handler that exports that variable. Images that
/// do not (the HAL, which the kernel's handlers may lie above) are passed by.
fn kernel_image<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    kpcr: u64,
) -> Option<KernelImage> {
    let idt = kernel_pointer(read_u64(cpu, Root::Live, kpcr + 0x38)?)?;
    let gate = read_exact(cpu, Root::Live, idt, 16)?;
    let handler = u64::from(u16::from_le_bytes([gate[0], gate[1]]))
        | u64::from(u16::from_le_bytes([gate[6], gate[7]])) << 16
        | u64::from(u32::from_le_bytes([gate[8], gate[9], gate[10], gate[11]])) << 32;
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
fn loaded_module_list<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    image: &PeImage,
) -> Option<u64> {
    let heads_a_list = |cpu: &mut DebugCpu<'_, T, E>, head: u64| {
        read_u64(cpu, Root::Live, head)
            .and_then(kernel_pointer)
            .and_then(|first| read_u64(cpu, Root::Live, first + 8))
            == Some(head)
    };
    if let Some(head) = export(cpu, image, "PsLoadedModuleList") {
        if heads_a_list(cpu, head) {
            return Some(head);
        }
    }
    let head = debugger_data_module_list(cpu, image)?;
    heads_a_list(cpu, head).then_some(head)
}

/// `KDDEBUGGER_DATA64.PsLoadedModuleList`, from the block in `image` whose
/// header tag is `KDBG` (at +0x10, the block 8-aligned) and whose `KernBase`
/// (+0x18) is the image's base. Windows 8 and later keep the block encoded
/// unless a kernel debugger is attached; their kernels export the list.
fn debugger_data_module_list<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    image: &PeImage,
) -> Option<u64> {
    const TAG: &[u8; 4] = b"KDBG";
    const TAG_OFFSET: u64 = 0x10;
    const KERN_BASE: u64 = 0x18;
    const PS_LOADED_MODULE_LIST: u64 = 0x48;
    let mut page = image.base;
    let end = image.base + u64::from(image.size);
    while page < end {
        // Paged-out and discarded pages are passed by.
        if let Some(bytes) = read_exact(cpu, Root::Live, page, PAGE_BYTES as usize) {
            for at in (0..bytes.len() - TAG.len()).step_by(8) {
                // The tag sits 0x10 into an 8-aligned block: 8-aligned too.
                if &bytes[at..at + TAG.len()] != TAG {
                    continue;
                }
                let block = page + at as u64 - TAG_OFFSET;
                if read_u64(cpu, Root::Live, block + KERN_BASE) == Some(image.base) {
                    return read_u64(cpu, Root::Live, block + PS_LOADED_MODULE_LIST)
                        .and_then(kernel_pointer);
                }
            }
        }
        page += PAGE_BYTES;
    }
    None
}

/// A PE32+ image whose headers start at `base`.
fn pe_image<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    base: u64,
) -> Option<PeImage> {
    if read_u16(cpu, root, base)? != MZ {
        return None;
    }
    let lfanew = u64::from(read_u32(cpu, root, base + 0x3C)?);
    if !(0x40..PAGE_BYTES).contains(&lfanew) {
        return None;
    }
    let nt = base + lfanew;
    if read_u32(cpu, root, nt)? != PE_SIGNATURE || read_u16(cpu, root, nt + 0x18)? != PE32_PLUS_MAGIC
    {
        return None;
    }
    let size = read_u32(cpu, root, nt + 0x18 + 0x38)?;
    Some(PeImage { base, nt, size })
}

/// The address `image` exports under `name`, found by a binary search of its
/// export name table (sorted, as the PE format requires).
fn export<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    image: &PeImage,
    name: &str,
) -> Option<u64> {
    let base = image.base;
    // OptionalHeader at +0x18; DataDirectory[EXPORT] at +0x70 of it.
    let directory = u64::from(read_u32(cpu, Root::Live, image.nt + 0x18 + 0x70)?);
    if directory == 0 {
        return None;
    }
    let directory = base + directory;
    let count = read_u32(cpu, Root::Live, directory + 0x18)?;
    let functions = base + u64::from(read_u32(cpu, Root::Live, directory + 0x1C)?);
    let names = base + u64::from(read_u32(cpu, Root::Live, directory + 0x20)?);
    let ordinals = base + u64::from(read_u32(cpu, Root::Live, directory + 0x24)?);
    // The name and its terminator: enough to order any export against it.
    let mut wanted = name.as_bytes().to_vec();
    wanted.push(0);
    let (mut low, mut high) = (0u32, count);
    while low < high {
        let middle = low + (high - low) / 2;
        let at = base + u64::from(read_u32(cpu, Root::Live, names + 4 * u64::from(middle))?);
        let candidate = read_exact(cpu, Root::Live, at, wanted.len())?;
        // Bytes after a shorter name's terminator do not order it.
        let candidate = match candidate.iter().position(|&byte| byte == 0) {
            Some(end) => &candidate[..=end],
            None => &candidate[..],
        };
        match candidate.cmp(&wanted[..]) {
            core::cmp::Ordering::Less => low = middle + 1,
            core::cmp::Ordering::Greater => high = middle,
            core::cmp::Ordering::Equal => {
                let ordinal = read_u16(cpu, Root::Live, ordinals + 2 * u64::from(middle))?;
                let rva = read_u32(cpu, Root::Live, functions + 4 * u64::from(ordinal))?;
                return Some(base + u64::from(rva));
            }
        }
    }
    None
}

/// Finds the moving `EPROCESS` fields in the System process.
fn process_layout<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    system: u64,
) -> Option<ProcessLayout> {
    let block = read_exact(cpu, Root::Live, system, EPROCESS_SPAN)?;
    let qword = |offset: usize| {
        block
            .get(offset..offset + 8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
    };
    let mut pid = None;
    for offset in (0..EPROCESS_SPAN - 24).step_by(8) {
        if qword(offset) != Some(SYSTEM_PID) {
            continue;
        }
        let links = system + offset as u64 + 8;
        let (Some(flink), Some(blink)) = (
            qword(offset + 8).and_then(kernel_pointer),
            qword(offset + 16).and_then(kernel_pointer),
        ) else {
            continue;
        };
        if read_u64(cpu, Root::Live, flink + 8) == Some(links)
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
        links: pid + 8,
        name,
    })
}

/// Every process on `ActiveProcessLinks`, starting at System. The list head
/// (`PsActiveProcessHead`) lies inside the kernel image and is no process.
fn list_processes<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    kernel: &Kernel,
    live_cr3: u64,
    parent_offset: &mut Option<u64>,
) -> Vec<WinProcess> {
    let layout = &kernel.layout;
    let image = kernel.image.base..kernel.image.base + u64::from(kernel.image.size);
    let first = kernel.system + layout.links;
    let mut processes = Vec::new();
    let mut visited = HashSet::new();
    let mut entry = first;
    for _ in 0..MAX_PROCESSES {
        // A ring that loops without returning to `first` ends at the repeat.
        if !visited.insert(entry) {
            break;
        }
        if !image.contains(&entry) {
            let eprocess = entry - layout.links;
            let (Some(pid), Some(cr3)) = (
                read_u64(cpu, Root::Live, eprocess + layout.pid),
                read_u64(cpu, Root::Live, eprocess + DIRECTORY_TABLE_BASE),
            ) else {
                break;
            };
            let name = read_exact(cpu, Root::Live, eprocess + layout.name, IMAGE_FILE_NAME_BYTES)
                .map(|bytes| {
                    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                    String::from_utf8_lossy(&bytes[..end]).into_owned()
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
        .is_some_and(|at| address < ranges[at].end)
}

/// The widest span from a `ThreadListEntry` to one of its `ETHREAD` fields:
/// `Teb` sits in the `KTHREAD` before the entry (a negative delta), `Cid` and
/// `StartAddress` after it.
const THREAD_DELTA_MIN: i64 = -0x600;
const THREAD_DELTA_MAX: i64 = 0x400;
/// How far past `Cid` the start addresses are looked for.
const START_DELTA_AFTER: i64 = 0x200;

fn read_at<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    node: u64,
    delta: i64,
) -> Option<u64> {
    read_u64(cpu, Root::Live, node.wrapping_add(delta as u64))
}

/// A thread ID shape: a non-zero multiple of four below 2^24, as Windows hands
/// out client IDs.
fn is_client_id(value: u64) -> bool {
    value != 0 && value % 4 == 0 && value < 0x0100_0000
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
fn read_threads<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
    let head = eprocess + layout.head;
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
fn saved_registers<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
fn find_trap_frame_delta<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(delta, _)| delta)
}

/// Whether a thread layout learned earlier reads true for this process: its
/// list head is a ring, empty or whose first thread carries this PID.
fn thread_layout_holds<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    eprocess: u64,
    pid: u64,
    layout: &ThreadLayout,
) -> bool {
    let head = eprocess + layout.head;
    let Some(first) = read_u64(cpu, Root::Live, head).and_then(kernel_pointer) else {
        return false;
    };
    if first == head {
        return true;
    }
    read_u64(cpu, Root::Live, first + 8) == Some(head)
        && read_at(cpu, first, layout.cid) == Some(pid)
}

/// Finds the thread-list field and the `Cid` offset by validation.
fn discover_thread_layout<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    eprocess: u64,
    pid: u64,
) -> Option<ThreadLayout> {
    for offset in (0..EPROCESS_SPAN as u64).step_by(8) {
        let head = eprocess + offset;
        let Some(first) = read_u64(cpu, Root::Live, head).and_then(kernel_pointer) else {
            continue;
        };
        // A LIST_ENTRY head: the first node's Blink points back at the head.
        if first == head || read_u64(cpu, Root::Live, first + 8) != Some(head) {
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
        if covered * 10 < nodes.len() * 9 {
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
fn find_start_layout<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    nodes: &[u64],
    cid: i64,
    ranges: &[CodeRange],
) -> Option<StartLayout> {
    if nodes.is_empty() || ranges.is_empty() {
        return None;
    }
    let mut holders = Vec::new();
    // `Cid` itself holds a PID and a TID, not an address.
    for delta in ((cid + 0x10)..(cid + START_DELTA_AFTER)).step_by(8) {
        let mut set = 0usize;
        let mut hits = 0usize;
        for &node in nodes {
            match read_at(cpu, node, delta) {
                Some(0) | None => {}
                Some(value) => {
                    set += 1;
                    hits += usize::from(in_code_range(ranges, value));
                }
            }
        }
        if set > 0 && hits * 10 >= set * 9 {
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
fn find_cid_delta<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    node: u64,
    pid: u64,
) -> Option<i64> {
    (THREAD_DELTA_MIN..THREAD_DELTA_MAX).step_by(8).find(|&delta| {
        read_at(cpu, node, delta) == Some(pid)
            && read_at(cpu, node, delta + 8).is_some_and(is_client_id)
    })
}

/// The node-relative offset of `KTHREAD.Teb`: the field whose value reads as a
/// user pointer whose `NtTib.Self` (TEB +0x30) points back at it. The pointer
/// lives in the `ETHREAD` (kernel), but the `TEB` is user memory, so it is read
/// through the process's own address space. The delta that self-references for
/// the most threads is the field — a stray lower offset might match one thread
/// by chance, but only the real `Teb` matches every user thread at once.
fn find_teb_delta<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
                            read_u64(cpu, Root::Process(cr3), teb + 0x30) == Some(teb)
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
fn read_thread<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    node: u64,
    layout: &ThreadLayout,
    cr3: u64,
) -> WinThread {
    let user = Root::Process(cr3);
    let tid = read_at(cpu, node, layout.cid + 8).unwrap_or(0);
    let teb = layout
        .teb
        .and_then(|delta| read_at(cpu, node, delta))
        .and_then(user_pointer)
        .filter(|&teb| read_u64(cpu, user, teb + 0x30) == Some(teb));
    // NtTib.StackBase at TEB +0x08, StackLimit at +0x10 (x64): the stack grows
    // down from base to limit.
    let stack = teb.and_then(|teb| {
        let base = read_u64(cpu, user, teb + 0x08)?;
        let limit = read_u64(cpu, user, teb + 0x10)?;
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
        read_u64(cpu, user, teb + 0x20)
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
fn assign_parents<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    layout: &ProcessLayout,
    processes: &mut [WinProcess],
    cached: &mut Option<u64>,
) {
    const MAX_PID: u64 = 1 << 24;
    if let Some(offset) = *cached {
        for process in processes.iter_mut() {
            process.parent = read_u64(cpu, Root::Live, process.eprocess + offset)
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
            .get(offset..offset + 8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
    };
    let first = usize::try_from(layout.pid).unwrap_or(EPROCESS_SPAN) + 8;
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
            if value == process.pid || value % 4 != 0 || value >= MAX_PID {
                return false;
            }
            others += 1;
            from_system |= value == SYSTEM_PID;
            listed += usize::from(pids.contains(&value));
        }
        from_system && others > 0 && 2 * listed >= others
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
    let roots: Vec<usize> = (0..processes.len())
        .filter(|&i| parent_of(&processes[i]).is_none())
        .collect();
    let count = roots.len();
    for (n, root) in roots.into_iter().enumerate() {
        place(processes, root, 0, Vec::new(), n + 1 == count, &mut placed, &mut rows);
    }
    // A cycle of parents has no root: list what it left unplaced at the top.
    for index in 0..processes.len() {
        if !placed[index] {
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
    if placed[index] {
        return;
    }
    placed[index] = true;
    let pid = processes[index].pid;
    let children: Vec<usize> = (0..processes.len())
        .filter(|&i| i != index && !placed[i] && processes[i].parent == Some(pid))
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
        place(processes, child, depth + 1, below.clone(), n + 1 == count, placed, rows);
    }
}

/// Two CR3 values name one address space when their table addresses match
/// (the low 12 bits are flags or a PCID).
fn same_address_space(a: u64, b: u64) -> bool {
    (a & !0xFFF) == (b & !0xFFF)
}

/// The modules `process` has loaded: its PEB's loader list, read through its
/// own page tables.
fn process_modules<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
    let Some(peb) = read_u64(cpu, Root::Live, process.eprocess + peb_offset) else {
        return ModuleRead::Unreadable;
    };
    if peb == 0 {
        return ModuleRead::NoUserSpace;
    }
    let root = Root::Process(process.cr3);
    // PEB.Ldr at +0x18; PEB_LDR_DATA.InLoadOrderModuleList at +0x10.
    let Some(loader) = read_u64(cpu, root, peb + 0x18).and_then(user_pointer) else {
        return ModuleRead::Unreadable;
    };
    match list_modules(cpu, root, loader + 0x10) {
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
/// sections come from each module's PE headers, read through its own tables.
fn read_memory_map<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    eprocess: u64,
    cr3: u64,
    modules: &[WinModule],
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
    let walk = cpu.dbg_page_mappings_in(
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
    match read_vads(cpu, eprocess, &module_bases, vad_site) {
        Some(vads) => memory_map::build_with_vads(&mappings, &layouts, &vads),
        None => memory_map::build(&mappings, &layouts),
    }
}

/// Windows' own records of the process's address space: the VAD tree, read
/// from the `EPROCESS` at `eprocess`. The tree's root and node shape are
/// discovered by trying each 64-bit shape at every `EPROCESS` field and
/// accepting the tree that contains the process's loaded modules; the site
/// found is kept in `site` and tried first next time. `None` when no shape's
/// tree validates (an unknown build, or the tree is paged out), so the map
/// falls back to the page-table form.
fn read_vads<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    eprocess: u64,
    module_bases: &[u64],
    site: &mut Option<VadSite>,
) -> Option<Vec<VadRegion>> {
    if let Some(known) = *site {
        if let Some(vads) = vads_at(cpu, eprocess, known, module_bases) {
            return Some(vads);
        }
    }
    for shape in [VadShape::AvlX64, VadShape::BalancedNode] {
        for offset in (0..EPROCESS_SPAN as u64).step_by(8) {
            let candidate = VadSite { shape, offset };
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
fn vads_at<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    eprocess: u64,
    site: VadSite,
    module_bases: &[u64],
) -> Option<Vec<VadRegion>> {
    let VadSite { shape, offset } = site;
    let root = vad_root_at(cpu, eprocess + offset, shape)?;
    // Probe: the root must read as a plausible node before a full walk.
    read_vad_node(cpu, root, shape)?;
    let vads = vad::walk(root, |pointer| read_vad_node(cpu, pointer, shape), MAX_VAD_NODES);
    vads_cover_modules(&vads, module_bases).then_some(vads)
}

/// The VAD tree root held by the candidate field at `field`, or `None` when
/// the field is not a VAD root of `shape`.
///
/// AvlX64: the field is an `MM_AVL_TABLE` whose `BalancedRoot` sentinel anchors
/// the tree — its `Parent` points to the sentinel itself and its `LeftChild`
/// is null, a signature a wrong `EPROCESS` field does not share. The real root
/// is the sentinel's `RightChild` (+0x10). BalancedNode: the field is an
/// `RTL_AVL_TREE` whose `Root` (+0x0) is the root directly.
fn vad_root_at<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    field: u64,
    shape: VadShape,
) -> Option<u64> {
    match shape {
        VadShape::AvlX64 => {
            let parent = read_u64(cpu, Root::Live, field)?;
            let left_child = read_u64(cpu, Root::Live, field + 0x08)?;
            let right_child = read_u64(cpu, Root::Live, field + 0x10)?;
            avl_table_root(parent, left_child, right_child, field)
        }
        VadShape::BalancedNode => read_u64(cpu, Root::Live, field).and_then(kernel_pointer),
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
    covered * 10 >= module_bases.len() * 9
}

/// Reads one VAD node of `shape` at `pointer`, or `None` when it does not
/// read as a plausible descriptor (a sentinel, a leaf's null child, or
/// garbage from a wrong shape).
fn read_vad_node<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    pointer: u64,
    shape: VadShape,
) -> Option<VadNode> {
    let layout = vad::node_layout(shape);
    let left = read_u64(cpu, Root::Live, pointer + layout.left)?;
    let right = read_u64(cpu, Root::Live, pointer + layout.right)?;
    let (start_vpn, end_vpn) = if layout.vpn_is_u64 {
        (
            read_u64(cpu, Root::Live, pointer + layout.start_vpn)?,
            read_u64(cpu, Root::Live, pointer + layout.end_vpn)?,
        )
    } else {
        let mut start = u64::from(read_u32(cpu, Root::Live, pointer + layout.start_vpn)?);
        let mut end = u64::from(read_u32(cpu, Root::Live, pointer + layout.end_vpn)?);
        if let Some(high) = layout.start_vpn_high {
            start |= u64::from(read_u8(cpu, Root::Live, pointer + high)?) << 32;
        }
        if let Some(high) = layout.end_vpn_high {
            end |= u64::from(read_u8(cpu, Root::Live, pointer + high)?) << 32;
        }
        (start, end)
    };
    if end_vpn < start_vpn {
        return None;
    }
    let flags = if layout.flags_is_u64 {
        read_u64(cpu, Root::Live, pointer + layout.flags)?
    } else {
        u64::from(read_u32(cpu, Root::Live, pointer + layout.flags)?)
    };
    // No page number reaches past the user half's: rejected before the shift,
    // so a garbage value cannot wrap into a plausible address.
    if start_vpn >= USER_SPACE_END >> 12 || end_vpn >= USER_SPACE_END >> 12 {
        return None;
    }
    let start = start_vpn << 12;
    let end = (end_vpn + 1) << 12;
    if start >= end {
        return None;
    }
    // Defence in depth: no real VAD spans more than a fraction of the address
    // space. A single node claiming the whole of it is a wrong field read as a
    // tree, not a descriptor — reject it so discovery keeps looking.
    if end - start > MAX_VAD_SPAN {
        return None;
    }
    Some(VadNode {
        left: kernel_pointer(left).unwrap_or(0),
        right: kernel_pointer(right).unwrap_or(0),
        region: VadRegion {
            start,
            end,
            info: vad::decode_flags(flags, shape),
        },
    })
}

fn read_u8<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    address: u64,
) -> Option<u8> {
    let mut raw = [0u8; 1];
    read_into(cpu, root, address, &mut raw).then_some(raw[0])
}

/// The `EPROCESS.Peb` offset: the first field that, in `PEB_WITNESSES`
/// processes (or every process there is, if fewer), holds a page-aligned
/// user address whose `ImageBaseAddress` (+0x10) points at an `MZ` image.
fn find_peb_offset<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    processes: &[WinProcess],
) -> Option<u64> {
    let mut witnesses = Vec::new();
    for process in processes.iter().filter(|p| p.pid != SYSTEM_PID && p.pid != 0) {
        if let Some(block) = read_exact(cpu, Root::Live, process.eprocess, EPROCESS_SPAN) {
            witnesses.push((process.cr3, block));
        }
        if witnesses.len() == 2 * PEB_WITNESSES {
            break;
        }
    }
    let needed = witnesses.len().min(PEB_WITNESSES);
    if needed == 0 {
        return None;
    }
    for offset in (0..EPROCESS_SPAN).step_by(8) {
        let mut agreeing = 0;
        for (cr3, block) in &witnesses {
            let Some(value) = block
                .get(offset..offset + 8)
                .and_then(|bytes| bytes.try_into().ok())
                .map(u64::from_le_bytes)
            else {
                continue;
            };
            if value & (PAGE_BYTES - 1) != 0 || user_pointer(value).is_none() {
                continue;
            }
            let root = Root::Process(*cr3);
            let image = read_u64(cpu, root, value + 0x10).and_then(user_pointer);
            if image.and_then(|image| read_u16(cpu, root, image)) == Some(MZ) {
                agreeing += 1;
            }
        }
        if agreeing >= needed {
            return Some(offset as u64);
        }
    }
    None
}

/// The modules on the loader list headed at `head`, in load order.
fn list_modules<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
            read_u64(cpu, root, entry + 0x30),
            read_u32(cpu, root, entry + 0x40),
        ) else {
            break;
        };
        modules.push(WinModule {
            base,
            size,
            name: read_unicode_string(cpu, root, entry + 0x58),
            path: read_unicode_string(cpu, root, entry + 0x48),
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
fn read_wow64_modules<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
fn wow64_modules_at<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    eprocess: u64,
    offset: u64,
) -> Option<Vec<WinModule>> {
    let value = read_u64(cpu, Root::Live, eprocess + offset)?;
    let peb32 = peb32_of(cpu, value)?;
    // PEB32.ImageBaseAddress at +0x08; a real PEB32 begins a PE image.
    let image_base = u64::from(read_u32(cpu, root, peb32 + 0x08)?);
    if image_base == 0 || read_u16(cpu, root, image_base)? != MZ {
        return None;
    }
    // PEB32.Ldr at +0x0C; PEB_LDR_DATA32.InLoadOrderModuleList at +0x0C.
    let loader = u64::from(read_u32(cpu, root, peb32 + 0x0C)?);
    if loader == 0 {
        return None;
    }
    Some(list_modules32(cpu, root, loader + 0x0C))
}

/// The `PEB32` address an `EPROCESS.Wow64Process` value stands for: a user
/// address below 4 GiB held directly (Windows XP through 8.1), or a kernel
/// pointer to an `EWOW64PROCESS` whose first field is it (Windows 10 on).
fn peb32_of<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
fn list_modules32<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
            read_u32(cpu, root, entry + 0x18).map(u64::from),
            read_u32(cpu, root, entry + 0x20),
        ) else {
            break;
        };
        modules.push(WinModule {
            base,
            size,
            name: read_unicode_string32(cpu, root, entry + 0x2C),
            path: read_unicode_string32(cpu, root, entry + 0x24),
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
fn read_unicode_string32<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    at: u64,
) -> Option<String> {
    let length = read_u16(cpu, root, at)?;
    if length == 0 || length % 2 != 0 || length > MAX_NAME_BYTES {
        return None;
    }
    let buffer = u64::from(read_u32(cpu, root, at + 4)?);
    let bytes = read_exact(cpu, root, buffer, usize::from(length))?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// A 64-bit `UNICODE_STRING` at `at`: `Length` (bytes) at +0, `Buffer` at +8.
fn read_unicode_string<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    at: u64,
) -> Option<String> {
    let length = read_u16(cpu, root, at)?;
    if length == 0 || length % 2 != 0 || length > MAX_NAME_BYTES {
        return None;
    }
    let buffer = read_u64(cpu, root, at + 8)?;
    let bytes = read_exact(cpu, root, buffer, usize::from(length))?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// An address in the kernel half of a 64-bit address space.
fn kernel_pointer(value: u64) -> Option<u64> {
    (value >= 0xFFFF_8000_0000_0000).then_some(value)
}

/// An address in the user half of a 64-bit address space.
fn user_pointer(value: u64) -> Option<u64> {
    (value != 0 && value < 0x0000_8000_0000_0000).then_some(value)
}

/// Fills `buf` from linear `address` under `root`, one page at a time;
/// `false` when any page of it does not map or does not read.
fn read_into<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    address: u64,
    buf: &mut [u8],
) -> bool {
    let mut done = 0usize;
    let mut at = address;
    while done < buf.len() {
        let in_page = usize::try_from(PAGE_BYTES - (at % PAGE_BYTES)).unwrap_or(buf.len());
        let take = in_page.min(buf.len() - done);
        let physical = match root {
            Root::Live => cpu.dbg_xlate_linear2phy(at),
            Root::Process(cr3) => cpu.dbg_xlate_linear2phy_in(cr3, at),
        };
        let Some(physical) = physical else {
            return false;
        };
        match cpu.dbg_fetch_mem(physical, &mut buf[done..done + take]) {
            Ok(Fetched::Valid) => {}
            Ok(Fetched::OutsideMemory) => return false,
            Err(error) => {
                tracing::warn!("debugger read at {physical:#x} failed: {error}");
                return false;
            }
        }
        done += take;
        at = at.wrapping_add(take as u64);
    }
    true
}

/// `len` bytes from linear `address` under `root`, or `None` when any page of
/// them does not map or does not read.
fn read_exact<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    address: u64,
    len: usize,
) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    read_into(cpu, root, address, &mut bytes).then_some(bytes)
}

fn read_u64<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    address: u64,
) -> Option<u64> {
    let mut raw = [0u8; 8];
    read_into(cpu, root, address, &mut raw).then(|| u64::from_le_bytes(raw))
}

fn read_u32<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    address: u64,
) -> Option<u32> {
    let mut raw = [0u8; 4];
    read_into(cpu, root, address, &mut raw).then(|| u32::from_le_bytes(raw))
}

fn read_u16<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
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
        let off = read_windows(&mut cpu, &view, SystemView::Off, &mut discovery);
        assert_eq!(off.system, SystemListing::NotAsked);
        assert!(off.symbols.modules.is_empty());
        let processes = SystemView::Processes {
            selected: None,
            detail: ProcessDetail::Modules,
        };
        for asked in [SystemView::KernelModules, processes] {
            // The flat test mode keeps GS at 0: there is no KPCR to find.
            assert_eq!(
                read_windows(&mut cpu, &view, asked, &mut discovery).system,
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
            let read = read_thread_view(&mut cpu, &view, target, &mut discovery);
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
        ] {
            assert!(!miss.describe().is_empty(), "{miss:?}");
        }
        assert!(ViewMiss::NoSavedState.describe().contains("saved"));
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
            SystemView::KernelModules,
            &mut WindowsDiscovery::default(),
        );
        assert_eq!(
            read.system,
            SystemListing::NoWindowsKernel(KernelMiss::KernelUnmapped)
        );
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
}
