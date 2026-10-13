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

use rusty_box::cpu::{CpuDebugView, Fetched, Instrumentation, LinearRange, WalkStep};
use rusty_box::emulator::{DebugCpu, SliceEngine};

use super::memory_map::{self, MemoryMap, ModuleLayout};
use super::pe::{self, HEADER_BYTES};
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModuleRead {
    Listed(Vec<WinModule>),
    /// A process with no user half (System, Idle): nothing loaded for it.
    NoUserSpace,
    /// Its PEB or loader data is paged out, or no PEB field was found.
    Unreadable,
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

/// Lists what `request` asks for, through the processor `view` describes.
pub(crate) fn read_system<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    view: &CpuDebugView,
    request: SystemView,
) -> SystemListing {
    if request == SystemView::Off {
        return SystemListing::NotAsked;
    }
    let kernel = match find_kernel(cpu, view) {
        Ok(kernel) => kernel,
        Err(miss) => return SystemListing::NoWindowsKernel(miss),
    };
    match request {
        SystemView::Off => SystemListing::NotAsked,
        SystemView::KernelModules => match kernel.loaded_modules {
            Some(head) => SystemListing::KernelModules(
                list_modules(cpu, Root::Live, head).unwrap_or_default(),
            ),
            None => SystemListing::NoWindowsKernel(KernelMiss::NoModuleList),
        },
        SystemView::Processes { selected, detail } => {
            let processes = list_processes(cpu, &kernel, view.registers.cr3);
            let chosen_index = selected
                .and_then(|eprocess| processes.iter().position(|p| p.eprocess == eprocess))
                .or_else(|| processes.iter().position(|p| p.current));
            let modules = chosen_index.map(|index| {
                let eprocess = processes[index].eprocess;
                let cr3 = processes[index].cr3;
                let read = process_modules(cpu, &processes, &processes[index]);
                let map = match (detail, &read) {
                    (ProcessDetail::MemoryMap, ModuleRead::Listed(modules)) => {
                        Some(read_memory_map(cpu, eprocess, cr3, modules))
                    }
                    _ => None,
                };
                ProcessModules {
                    eprocess,
                    read,
                    map,
                }
            });
            SystemListing::Processes(ProcessListing { processes, modules })
        }
    }
}

fn find_kernel<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    view: &CpuDebugView,
) -> Result<Kernel, KernelMiss> {
    // EFER.LMA: a 64-bit kernel.
    if view.efer & 0x400 == 0 {
        return Err(KernelMiss::NotLongMode);
    }
    let kpcr = find_kpcr(cpu, view).ok_or(KernelMiss::NoKpcr)?;
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
) -> Vec<WinProcess> {
    let layout = &kernel.layout;
    let image = kernel.image.base..kernel.image.base + u64::from(kernel.image.size);
    let first = kernel.system + layout.links;
    let mut processes = Vec::new();
    let mut entry = first;
    for _ in 0..MAX_PROCESSES {
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
    assign_parents(cpu, layout, &mut processes);
    processes
}

/// Fills each process's `parent` from `EPROCESS.InheritedFromUniqueProcessId`,
/// found as the first field past `UniqueProcessId` that reads as a parent ID
/// in every process at once: zero in System, the System process's PID in at
/// least one other (the session manager, smss.exe), never the process's own
/// PID, always a PID-shaped value (a multiple of 4 below 2^24), and the PID of
/// a listed process for at least half the rest (a parent that has exited,
/// such as explorer.exe's userinit.exe, is no longer listed). Left `None`
/// when no field qualifies.
fn assign_parents<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    layout: &ProcessLayout,
    processes: &mut [WinProcess],
) {
    const MAX_PID: u64 = 1 << 24;
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
    let listed = |pid: u64| processes.iter().any(|p| p.pid == pid);
    let parent_of = |process: &WinProcess| {
        process
            .parent
            .filter(|&parent| parent != process.pid && listed(parent))
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
) -> ModuleRead {
    let Some(peb_offset) = find_peb_offset(cpu, processes) else {
        return ModuleRead::Unreadable;
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
        Some(modules) => ModuleRead::Listed(modules),
        None => ModuleRead::Unreadable,
    }
}

/// The page-table memory map of the process whose address space is `cr3`:
/// its mapped user pages, with each module's span split into its header and
/// its sections. The module list is already read by [`process_modules`]; the
/// sections come from each module's PE headers, read through its own tables.
pub(crate) fn read_memory_map<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    eprocess: u64,
    cr3: u64,
    modules: &[WinModule],
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
    cpu.dbg_page_mappings_in(
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
    // Overlay Windows' own records when the VAD tree is found, so reserved
    // regions and Windows' protection show; else the page-table map stands.
    let module_bases: Vec<u64> = modules.iter().map(|module| module.base).collect();
    match read_vads(cpu, eprocess, &module_bases) {
        Some(vads) => memory_map::build_with_vads(&mappings, &layouts, &vads),
        None => memory_map::build(&mappings, &layouts),
    }
}

/// Windows' own records of the process's address space: the VAD tree, read
/// from the `EPROCESS` at `eprocess`. The tree's root and node shape are
/// discovered by trying each 64-bit shape at every `EPROCESS` field and
/// accepting the tree that contains the process's loaded modules. `None`
/// when no shape's tree validates (an unknown build, or the tree is paged
/// out), so the map falls back to the page-table form.
fn read_vads<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    eprocess: u64,
    module_bases: &[u64],
) -> Option<Vec<VadRegion>> {
    // A 64-bit kernel uses one of these two shapes; the x86 shape is added
    // with 32-bit-kernel support.
    for shape in [VadShape::AvlX64, VadShape::BalancedNode] {
        if let Some(vads) = try_vad_shape(cpu, eprocess, shape, module_bases) {
            return Some(vads);
        }
    }
    None
}

/// Tries `shape` at every `EPROCESS` field: the field that reads as a tree
/// root whose regions cover most of the process's modules is the `VadRoot`.
fn try_vad_shape<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    eprocess: u64,
    shape: VadShape,
    module_bases: &[u64],
) -> Option<Vec<VadRegion>> {
    for offset in (0..EPROCESS_SPAN as u64).step_by(8) {
        let Some(root) = vad_root_at(cpu, eprocess + offset, shape) else {
            continue;
        };
        // Probe: the root must read as a plausible node before a full walk.
        if read_vad_node(cpu, root, shape).is_none() {
            continue;
        }
        let vads = vad::walk(root, |pointer| read_vad_node(cpu, pointer, shape), MAX_VAD_NODES);
        if vads_cover_modules(&vads, module_bases) {
            return Some(vads);
        }
    }
    None
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
        VadShape::XpX86 => None,
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
    let start = start_vpn << 12;
    let end = end_vpn.checked_add(1)?.checked_shl(12)?;
    if start >= end || end > USER_SPACE_END {
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
    read_exact(cpu, root, address, 1).map(|bytes| bytes[0])
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
    let mut entry = read_u64(cpu, root, head)?;
    for _ in 0..MAX_MODULES {
        if entry == head || entry == 0 {
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
        });
        let Some(next) = read_u64(cpu, root, entry) else {
            break;
        };
        entry = next;
    }
    Some(modules)
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

/// `len` bytes from linear `address` under `root`, or `None` when any page of
/// them does not map or does not read.
fn read_exact<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    address: u64,
    len: usize,
) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(len);
    let mut at = address;
    while bytes.len() < len {
        let in_page = usize::try_from(PAGE_BYTES - (at % PAGE_BYTES)).unwrap_or(len);
        let take = in_page.min(len - bytes.len());
        let physical = match root {
            Root::Live => cpu.dbg_xlate_linear2phy(at)?,
            Root::Process(cr3) => cpu.dbg_xlate_linear2phy_in(cr3, at)?,
        };
        let mut piece = vec![0u8; take];
        match cpu.dbg_fetch_mem(physical, &mut piece) {
            Ok(Fetched::Valid) => {}
            Ok(Fetched::OutsideMemory) => return None,
            Err(error) => {
                tracing::warn!("debugger read at {physical:#x} failed: {error}");
                return None;
            }
        }
        bytes.extend_from_slice(&piece);
        at = at.wrapping_add(take as u64);
    }
    Some(bytes)
}

fn read_u64<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    address: u64,
) -> Option<u64> {
    let bytes = read_exact(cpu, root, address, 8)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u32<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    address: u64,
) -> Option<u32> {
    let bytes = read_exact(cpu, root, address, 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u16<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    root: Root,
    address: u64,
) -> Option<u16> {
    let bytes = read_exact(cpu, root, address, 2)?;
    Some(u16::from_le_bytes(bytes.try_into().ok()?))
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
        assert_eq!(
            read_system(&mut cpu, &view, SystemView::Off),
            SystemListing::NotAsked
        );
        let processes = SystemView::Processes {
            selected: None,
            detail: ProcessDetail::Modules,
        };
        for asked in [SystemView::KernelModules, processes] {
            // The flat test mode keeps GS at 0: there is no KPCR to find.
            assert_eq!(
                read_system(&mut cpu, &view, asked),
                SystemListing::NoWindowsKernel(KernelMiss::NoKpcr)
            );
        }
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
