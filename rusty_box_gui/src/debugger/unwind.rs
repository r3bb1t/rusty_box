//! Call stacks of a paused guest (spec §9). x64 code is unwound by the
//! image's own unwind data, read from guest memory: the function's
//! `RUNTIME_FUNCTION` in the exception directory and its `UNWIND_INFO`
//! (Microsoft, "x64 exception handling"), undone as Wine's
//! `RtlVirtualUnwind` does: an epilog recognised by its bytes is run
//! forwards, a prolog paused in undoes only the codes it has run, chained
//! infos are followed, and a machine frame restores RIP and RSP from the
//! trap's frame. A function with no table entry is a leaf. 32-bit code is
//! unwound by its EBP chain.

use super::guest::{GuestMemory, GuestRead, Root};
use super::pe;

/// The most frames a stack is unwound to.
#[cfg_attr(not(test), allow(dead_code, reason = "the call-stack pane (plan 4b) walks whole stacks"))]
pub(crate) const MAX_FRAMES: usize = 256;
/// The most chained infos followed for one frame.
const MAX_CHAIN: u32 = 32;
/// The most instructions an epilog is read through.
const MAX_EPILOG_INSTRUCTIONS: u32 = 64;
/// A `RUNTIME_FUNCTION` table past this many entries is not an image's.
const MAX_FUNCTIONS: u32 = 1 << 20;
const RUNTIME_FUNCTION_BYTES: u32 = 12;

/// x64 general registers by number, as unwind codes and ModRM name them.
pub(crate) const RAX: usize = 0;
pub(crate) const RCX: usize = 1;
pub(crate) const RDX: usize = 2;
#[cfg_attr(not(test), allow(dead_code, reason = "the variables pane (plan 4b) reads registers by number"))]
pub(crate) const RBX: usize = 3;
#[cfg_attr(not(test), allow(dead_code, reason = "the variables pane (plan 4b) reads registers by number"))]
pub(crate) const RSP: usize = 4;
pub(crate) const RBP: usize = 5;
#[cfg_attr(not(test), allow(dead_code, reason = "the variables pane (plan 4b) reads registers by number"))]
pub(crate) const RSI: usize = 6;
#[cfg_attr(not(test), allow(dead_code, reason = "the variables pane (plan 4b) reads registers by number"))]
pub(crate) const RDI: usize = 7;
/// The registers a call may change (the x64 calling convention's volatile
/// ones): not known in a caller's frame.
const VOLATILE: [usize; 7] = [RAX, RCX, RDX, 8, 9, 10, 11];

const UWOP_PUSH_NONVOL: u8 = 0;
const UWOP_ALLOC_LARGE: u8 = 1;
const UWOP_ALLOC_SMALL: u8 = 2;
const UWOP_SET_FPREG: u8 = 3;
const UWOP_SAVE_NONVOL: u8 = 4;
const UWOP_SAVE_NONVOL_FAR: u8 = 5;
const UWOP_EPILOG: u8 = 6;
const UWOP_SPARE_CODE: u8 = 7;
const UWOP_SAVE_XMM128: u8 = 8;
const UWOP_SAVE_XMM128_FAR: u8 = 9;
const UWOP_PUSH_MACHFRAME: u8 = 10;
const UNW_FLAG_CHAININFO: u8 = 4;

/// The code a stack runs: 64-bit, or 32-bit (protected mode or WOW64).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodeWidth {
    Long64,
    Protected32,
}

/// A frame's registers: where it resumes, its stack pointer, and the
/// general registers by number (`RSP`'s slot unused) where they are known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameRegisters {
    pub(crate) rip: u64,
    pub(crate) rsp: u64,
    pub(crate) registers: [Option<u64>; 16],
}

/// How a frame was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameFound {
    /// The paused processor's own registers.
    Registers,
    /// By the function's unwind info.
    Unwind,
    /// A function without unwind info returns through `[RSP]`.
    Leaf,
    /// From the frame an interrupt or exception pushed.
    MachineFrame,
    /// By running the epilog the frame was paused in.
    Epilog,
    /// By the EBP chain.
    FramePointer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) registers: FrameRegisters,
    pub(crate) how: FrameFound,
}

/// Why a stack ends where it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StackEnd {
    /// The outermost frame: it returns to nothing.
    Top,
    /// The code is in no module given, so it has no unwind info.
    NoUnwindInfo { rip: u64 },
    /// The unwind info at `at` is not one this reader can follow.
    BadUnwindInfo { at: u64 },
    /// The frame needs register `register`, which is not known in it.
    RegisterNotKnown { register: usize },
    /// The 32-bit code paused in keeps no frame pointer.
    NoFramePointer,
    /// The next frame would not lie above this one on the stack.
    NotGrowing,
    Unreadable { at: u64 },
    /// [`MAX_FRAMES`] frames.
    #[cfg_attr(not(test), allow(dead_code, reason = "the call-stack pane (plan 4b) walks whole stacks"))]
    Ceiling,
}

#[cfg_attr(not(test), allow(dead_code, reason = "the call-stack pane (plan 4b) walks whole stacks"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stack {
    pub(crate) frames: Vec<Frame>,
    pub(crate) end: StackEnd,
}

/// A loaded image a frame's code may be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UnwindModule {
    pub(crate) base: u64,
    pub(crate) size: u64,
}

/// What a stack is unwound in: the page tables its memory is read through,
/// the modules its code may be in, and its code's width.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Unwinding<'a> {
    pub(crate) root: Root,
    pub(crate) modules: &'a [UnwindModule],
    pub(crate) width: CodeWidth,
}

/// The stack from `start`, the paused processor's registers.
#[cfg_attr(not(test), allow(dead_code, reason = "the call-stack pane (plan 4b) walks whole stacks"))]
pub(crate) fn unwind<G: GuestMemory>(memory: &mut G, unwinding: &Unwinding<'_>, start: FrameRegisters) -> Stack {
    let mut frames = vec![Frame { registers: start, how: FrameFound::Registers }];
    let mut last = frames.first().copied();
    while let Some(frame) = last {
        if frames.len() >= MAX_FRAMES {
            return Stack { frames, end: StackEnd::Ceiling };
        }
        match caller_of(memory, unwinding, &frame) {
            Ok(caller) => {
                frames.push(caller);
                last = Some(caller);
            }
            Err(end) => return Stack { frames, end },
        }
    }
    Stack { frames, end: StackEnd::Top }
}

/// The frame `frame` returns to.
pub(crate) fn caller_of<G: GuestMemory>(memory: &mut G, unwinding: &Unwinding<'_>, frame: &Frame) -> Result<Frame, StackEnd> {
    let caller = match unwinding.width {
        CodeWidth::Long64 => caller_64(memory, unwinding, frame)?,
        CodeWidth::Protected32 => caller_32(memory, unwinding.root, frame)?,
    };
    if caller.registers.rip == 0 {
        return Err(StackEnd::Top);
    }
    // A machine frame may switch stacks (a user stack under a kernel one).
    if caller.how != FrameFound::MachineFrame && caller.registers.rsp <= frame.registers.rsp {
        return Err(StackEnd::NotGrowing);
    }
    Ok(caller)
}

fn read_array<G: GuestMemory, const N: usize>(memory: &mut G, root: Root, at: u64) -> Result<[u8; N], StackEnd> {
    let mut bytes = [0u8; N];
    match memory.read_linear(root, at, &mut bytes) {
        GuestRead::Read => Ok(bytes),
        GuestRead::Unreadable => Err(StackEnd::Unreadable { at }),
    }
}

fn read_u64<G: GuestMemory>(memory: &mut G, root: Root, at: u64) -> Result<u64, StackEnd> {
    read_array::<G, 8>(memory, root, at).map(u64::from_le_bytes)
}

fn read_u32<G: GuestMemory>(memory: &mut G, root: Root, at: u64) -> Result<u32, StackEnd> {
    read_array::<G, 4>(memory, root, at).map(u32::from_le_bytes)
}

/// A `RUNTIME_FUNCTION`, RVAs in the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeFunction {
    begin: u32,
    end: u32,
    unwind_data: u32,
}

fn read_function<G: GuestMemory>(memory: &mut G, root: Root, at: u64) -> Result<RuntimeFunction, StackEnd> {
    let bytes = read_array::<G, 12>(memory, root, at)?;
    let field = |offset: usize| {
        bytes.get(offset..).and_then(<[u8]>::first_chunk::<4>).copied().map_or(0, u32::from_le_bytes)
    };
    Ok(RuntimeFunction { begin: field(0), end: field(4), unwind_data: field(8) })
}

/// The table entry of the function holding `rva`, by binary search;
/// `None` when no entry holds it (a leaf). An entry whose unwind data has
/// its low bit set names the entry it shares.
fn find_function<G: GuestMemory>(
    memory: &mut G,
    root: Root,
    module: &UnwindModule,
    rva: u32,
) -> Result<Option<RuntimeFunction>, StackEnd> {
    let mut header = vec![0u8; pe::HEADER_BYTES];
    if memory.read_linear(root, module.base, &mut header) == GuestRead::Unreadable {
        return Err(StackEnd::Unreadable { at: module.base });
    }
    let Some(directory) = pe::exception_directory(&header) else {
        return Ok(None);
    };
    let count = directory.size.checked_div(RUNTIME_FUNCTION_BYTES).unwrap_or(0);
    let table = module.base.wrapping_add(u64::from(directory.rva));
    if count > MAX_FUNCTIONS {
        return Err(StackEnd::BadUnwindInfo { at: table });
    }
    let entry_at = |index: u32| table.wrapping_add(u64::from(index).wrapping_mul(u64::from(RUNTIME_FUNCTION_BYTES)));
    let mut low = 0u32;
    let mut high = count;
    while low < high {
        let middle = low.wrapping_add(high.wrapping_sub(low).wrapping_shr(1));
        let function = read_function(memory, root, entry_at(middle))?;
        if rva < function.begin {
            high = middle;
        } else if rva >= function.end {
            low = middle.wrapping_add(1);
        } else if function.unwind_data & 1 == 1 {
            let shared = module.base.wrapping_add(u64::from(function.unwind_data & !1));
            return read_function(memory, root, shared).map(Some);
        } else {
            return Ok(Some(function));
        }
    }
    Ok(None)
}

/// The caller of a 64-bit frame. A frame resumed by a return is looked up
/// one byte back: a call may be its function's last instruction.
fn caller_64<G: GuestMemory>(memory: &mut G, unwinding: &Unwinding<'_>, frame: &Frame) -> Result<Frame, StackEnd> {
    let root = unwinding.root;
    let rip = frame.registers.rip;
    let lookup = match frame.how {
        FrameFound::Registers | FrameFound::MachineFrame => rip,
        FrameFound::Unwind | FrameFound::Leaf | FrameFound::Epilog | FrameFound::FramePointer => rip.wrapping_sub(1),
    };
    let module = unwinding
        .modules
        .iter()
        .find(|module| lookup.wrapping_sub(module.base) < module.size)
        .ok_or(StackEnd::NoUnwindInfo { rip })?;
    let rva = u32::try_from(lookup.wrapping_sub(module.base)).map_err(|_| StackEnd::NoUnwindInfo { rip })?;
    let mut caller = match find_function(memory, root, module, rva)? {
        Some(function) => virtual_unwind(memory, root, module.base, function, frame.registers)?,
        None => {
            let mut registers = frame.registers;
            registers.rip = read_u64(memory, root, registers.rsp)?;
            registers.rsp = registers.rsp.wrapping_add(8);
            Frame { registers, how: FrameFound::Leaf }
        }
    };
    for volatile in VOLATILE {
        if let Some(slot) = caller.registers.registers.get_mut(volatile) {
            *slot = None;
        }
    }
    Ok(caller)
}

/// One unwind code slot.
#[derive(Clone, Copy)]
struct Slot(u16);

impl Slot {
    fn offset(self) -> u8 {
        self.0.to_le_bytes()[0]
    }
    fn op(self) -> u8 {
        self.0.to_le_bytes()[1] & 0x0F
    }
    fn info(self) -> u8 {
        self.0.to_le_bytes()[1] >> 4
    }
    /// How many slots the code takes (Wine `get_opcode_size`).
    fn slots(self) -> usize {
        match self.op() {
            UWOP_ALLOC_LARGE if self.info() != 0 => 3,
            UWOP_ALLOC_LARGE | UWOP_SAVE_NONVOL | UWOP_SAVE_XMM128 | UWOP_EPILOG => 2,
            UWOP_SAVE_NONVOL_FAR | UWOP_SAVE_XMM128_FAR => 3,
            _ => 1,
        }
    }
}

/// The registers an unwind runs on.
struct Context {
    registers: FrameRegisters,
}

impl Context {
    fn get(&self, register: usize) -> Result<u64, StackEnd> {
        self.registers
            .registers
            .get(register)
            .copied()
            .flatten()
            .ok_or(StackEnd::RegisterNotKnown { register })
    }

    fn set(&mut self, register: usize, value: u64) {
        if let Some(slot) = self.registers.registers.get_mut(register) {
            *slot = Some(value);
        }
    }
}

/// The unwind info at `at`: its head and its code slots.
struct UnwindInfo {
    flags: u8,
    prolog: u8,
    frame_register: u8,
    frame_offset: u8,
    codes: Vec<Slot>,
}

fn read_info<G: GuestMemory>(memory: &mut G, root: Root, at: u64) -> Result<UnwindInfo, StackEnd> {
    let head = read_array::<G, 4>(memory, root, at)?;
    let [version_flags, prolog, count, frame] = head;
    let version = version_flags & 7;
    if version != 1 && version != 2 {
        return Err(StackEnd::BadUnwindInfo { at });
    }
    let mut bytes = vec![0u8; usize::from(count).wrapping_mul(2)];
    if memory.read_linear(root, at.wrapping_add(4), &mut bytes) == GuestRead::Unreadable {
        return Err(StackEnd::Unreadable { at: at.wrapping_add(4) });
    }
    let codes = bytes
        .chunks_exact(2)
        .filter_map(|pair| pair.first_chunk::<2>().copied())
        .map(|pair| Slot(u16::from_le_bytes(pair)))
        .collect();
    Ok(UnwindInfo { flags: version_flags >> 3, prolog, frame_register: frame & 0x0F, frame_offset: frame >> 4, codes })
}

/// Unwinds through `function`'s info (Wine `RtlVirtualUnwind`).
fn virtual_unwind<G: GuestMemory>(
    memory: &mut G,
    root: Root,
    base: u64,
    function: RuntimeFunction,
    registers: FrameRegisters,
) -> Result<Frame, StackEnd> {
    let pc = registers.rip;
    let mut context = Context { registers };
    let mut frame_base = registers.rsp;
    let mut function = function;
    for depth in 0..MAX_CHAIN {
        let info_at = base.wrapping_add(u64::from(function.unwind_data));
        let info = read_info(memory, root, info_at)?;
        if info.frame_register != 0 {
            frame_base = context
                .get(usize::from(info.frame_register))?
                .wrapping_sub(u64::from(info.frame_offset).wrapping_mul(16));
        }
        let begin = base.wrapping_add(u64::from(function.begin));
        let into = pc.wrapping_sub(begin);
        // How far into the prolog the frame is paused; past it, every code
        // has run.
        let prolog_offset = (into < u64::from(info.prolog)).then_some(into);
        if depth == 0 && prolog_offset.is_none() && !info.codes.is_empty() && in_epilog(memory, root, base, function, pc) {
            return run_epilog(memory, root, context, pc);
        }
        let mut index = 0usize;
        while let Some(slot) = info.codes.get(index).copied() {
            let extra = |at: usize| -> Result<u64, StackEnd> {
                info.codes
                    .get(index.wrapping_add(at))
                    .map(|slot| u64::from(slot.0))
                    .ok_or(StackEnd::BadUnwindInfo { at: info_at })
            };
            let ran = prolog_offset.is_none_or(|offset| offset >= u64::from(slot.offset()));
            if ran {
                let rsp = context.registers.rsp;
                match slot.op() {
                    UWOP_PUSH_NONVOL => {
                        let value = read_u64(memory, root, rsp)?;
                        context.set(usize::from(slot.info()), value);
                        context.registers.rsp = rsp.wrapping_add(8);
                    }
                    UWOP_ALLOC_LARGE => {
                        let size = if slot.info() == 0 { extra(1)?.wrapping_mul(8) } else { extra(1)? | (extra(2)? << 16) };
                        context.registers.rsp = rsp.wrapping_add(size);
                    }
                    UWOP_ALLOC_SMALL => {
                        context.registers.rsp = rsp.wrapping_add(u64::from(slot.info()).wrapping_mul(8).wrapping_add(8));
                    }
                    UWOP_SET_FPREG => context.registers.rsp = frame_base,
                    UWOP_SAVE_NONVOL => {
                        let value = read_u64(memory, root, frame_base.wrapping_add(extra(1)?.wrapping_mul(8)))?;
                        context.set(usize::from(slot.info()), value);
                    }
                    UWOP_SAVE_NONVOL_FAR => {
                        let value = read_u64(memory, root, frame_base.wrapping_add(extra(1)? | (extra(2)? << 16)))?;
                        context.set(usize::from(slot.info()), value);
                    }
                    // An epilog's description and the vector registers'
                    // saves move no general register.
                    UWOP_EPILOG | UWOP_SPARE_CODE | UWOP_SAVE_XMM128 | UWOP_SAVE_XMM128_FAR => {}
                    UWOP_PUSH_MACHFRAME => {
                        let frame = if slot.info() == 0 { rsp } else { rsp.wrapping_add(8) };
                        context.registers.rip = read_u64(memory, root, frame)?;
                        context.registers.rsp = read_u64(memory, root, frame.wrapping_add(24))?;
                        return Ok(Frame { registers: context.registers, how: FrameFound::MachineFrame });
                    }
                    _ => return Err(StackEnd::BadUnwindInfo { at: info_at }),
                }
            }
            index = index.wrapping_add(slot.slots());
        }
        if info.flags & UNW_FLAG_CHAININFO == 0 {
            let rsp = context.registers.rsp;
            context.registers.rip = read_u64(memory, root, rsp)?;
            context.registers.rsp = rsp.wrapping_add(8);
            return Ok(Frame { registers: context.registers, how: FrameFound::Unwind });
        }
        // The chained entry follows the codes, padded to an even count.
        let padded = (info.codes.len().wrapping_add(1) & !1).wrapping_mul(2);
        function = read_function(memory, root, info_at.wrapping_add(4).wrapping_add(padded as u64))?;
    }
    Err(StackEnd::BadUnwindInfo { at: base.wrapping_add(u64::from(function.unwind_data)) })
}

/// The 16 bytes of code at `at`, when they read.
fn code_at<G: GuestMemory>(memory: &mut G, root: Root, at: u64) -> Option<[u8; 16]> {
    read_array::<G, 16>(memory, root, at).ok()
}

fn byte(code: &[u8; 16], at: usize) -> u8 {
    code.get(at).copied().unwrap_or(0)
}

fn i32_at(code: &[u8; 16], at: usize) -> i64 {
    code.get(at..).and_then(<[u8]>::first_chunk::<4>).copied().map_or(0, |bytes| i64::from(i32::from_le_bytes(bytes)))
}

fn i8_at(code: &[u8; 16], at: usize) -> i64 {
    i64::from(byte(code, at) as i8)
}

/// Whether `pc` is in an epilog (Wine `is_inside_epilog`): an optional
/// `add rsp, n` or `lea rsp, [reg + n]`, `pop`s, and a `ret`, `jmp`s inside
/// the function followed.
fn in_epilog<G: GuestMemory>(memory: &mut G, root: Root, base: u64, function: RuntimeFunction, pc: u64) -> bool {
    let inside = |target: u64| {
        let rva = target.wrapping_sub(base);
        rva >= u64::from(function.begin) && rva < u64::from(function.end)
    };
    let Some(code) = code_at(memory, root, pc) else {
        return false;
    };
    let mut at = pc;
    if byte(&code, 0) & 0xF8 == 0x48 {
        let modrm = byte(&code, 2);
        match byte(&code, 1) {
            0x81 if byte(&code, 0) == 0x48 && modrm == 0xC4 => at = at.wrapping_add(7),
            0x83 if byte(&code, 0) == 0x48 && modrm == 0xC4 => at = at.wrapping_add(4),
            0x81 | 0x83 => return false,
            0x8D => {
                if byte(&code, 0) & 0x06 != 0 || (modrm >> 3) & 7 != 4 || modrm & 7 == 4 {
                    return false;
                }
                match modrm >> 6 {
                    1 => at = at.wrapping_add(4),
                    2 => at = at.wrapping_add(7),
                    _ => return false,
                }
            }
            _ => {}
        }
    }
    for _ in 0..MAX_EPILOG_INSTRUCTIONS {
        let Some(code) = code_at(memory, root, at) else {
            return false;
        };
        let rex = usize::from(byte(&code, 0) & 0xF0 == 0x40);
        match byte(&code, rex) {
            0x58..=0x5F => at = at.wrapping_add(rex as u64).wrapping_add(1),
            0xC2 | 0xC3 => return true,
            0xF3 => return byte(&code, rex.wrapping_add(1)) == 0xC3,
            0xE9 => {
                let target = at.wrapping_add(rex as u64).wrapping_add(5).wrapping_add_signed(i32_at(&code, rex.wrapping_add(1)));
                if !inside(target) {
                    return false;
                }
                at = target;
            }
            0xEB => {
                let target = at.wrapping_add(rex as u64).wrapping_add(2).wrapping_add_signed(i8_at(&code, rex.wrapping_add(1)));
                if !inside(target) {
                    return false;
                }
                at = target;
            }
            _ => return false,
        }
    }
    false
}

/// Runs the epilog at `pc` forwards to its `ret` (Wine `interpret_epilog`).
fn run_epilog<G: GuestMemory>(memory: &mut G, root: Root, mut context: Context, pc: u64) -> Result<Frame, StackEnd> {
    let mut at = pc;
    for _ in 0..MAX_EPILOG_INSTRUCTIONS {
        let code = code_at(memory, root, at).ok_or(StackEnd::Unreadable { at })?;
        let has_rex = byte(&code, 0) & 0xF0 == 0x40;
        let rex = if has_rex { byte(&code, 0) & 0x0F } else { 0 };
        let k = usize::from(has_rex);
        let high = usize::from(rex & 1).wrapping_mul(8);
        let rsp = context.registers.rsp;
        match byte(&code, k) {
            pop @ 0x58..=0x5F => {
                let value = read_u64(memory, root, rsp)?;
                context.set(usize::from(pop.wrapping_sub(0x58)).wrapping_add(high), value);
                context.registers.rsp = rsp.wrapping_add(8);
                at = at.wrapping_add(k as u64).wrapping_add(1);
            }
            0x81 => {
                context.registers.rsp = rsp.wrapping_add_signed(i32_at(&code, k.wrapping_add(2)));
                at = at.wrapping_add(k as u64).wrapping_add(6);
            }
            0x83 => {
                context.registers.rsp = rsp.wrapping_add_signed(i8_at(&code, k.wrapping_add(2)));
                at = at.wrapping_add(k as u64).wrapping_add(3);
            }
            0x8D => {
                let modrm = byte(&code, k.wrapping_add(1));
                let from = context.get(usize::from(modrm & 7).wrapping_add(high))?;
                if modrm >> 6 == 1 {
                    context.registers.rsp = from.wrapping_add_signed(i8_at(&code, k.wrapping_add(2)));
                    at = at.wrapping_add(k as u64).wrapping_add(3);
                } else {
                    context.registers.rsp = from.wrapping_add_signed(i32_at(&code, k.wrapping_add(2)));
                    at = at.wrapping_add(k as u64).wrapping_add(6);
                }
            }
            0xC2 => {
                let popped = code.get(k.wrapping_add(1)..).and_then(<[u8]>::first_chunk::<2>).copied().map_or(0, u16::from_le_bytes);
                context.registers.rip = read_u64(memory, root, rsp)?;
                context.registers.rsp = rsp.wrapping_add(8).wrapping_add(u64::from(popped));
                return Ok(Frame { registers: context.registers, how: FrameFound::Epilog });
            }
            0xC3 | 0xF3 => {
                context.registers.rip = read_u64(memory, root, rsp)?;
                context.registers.rsp = rsp.wrapping_add(8);
                return Ok(Frame { registers: context.registers, how: FrameFound::Epilog });
            }
            0xE9 => at = at.wrapping_add(k as u64).wrapping_add(5).wrapping_add_signed(i32_at(&code, k.wrapping_add(1))),
            0xEB => at = at.wrapping_add(k as u64).wrapping_add(2).wrapping_add_signed(i8_at(&code, k.wrapping_add(1))),
            _ => return Err(StackEnd::BadUnwindInfo { at }),
        }
    }
    Err(StackEnd::BadUnwindInfo { at: pc })
}

fn low32(value: u64) -> u64 {
    value & 0xFFFF_FFFF
}

/// `registers` with EBP `ebp` and no other general register known: a
/// 32-bit caller's other registers are wherever its callee saved them.
fn with_ebp(mut registers: FrameRegisters, ebp: u64) -> FrameRegisters {
    registers.registers = [None; 16];
    if let Some(slot) = registers.registers.get_mut(RBP) {
        *slot = Some(ebp);
    }
    registers
}

/// The caller of a 32-bit frame by its EBP chain. Paused at a function's
/// `push ebp`, after it, or at a `ret`, the paused frame has no frame of its
/// own yet (or any more), and returns through ESP.
fn caller_32<G: GuestMemory>(memory: &mut G, root: Root, frame: &Frame) -> Result<Frame, StackEnd> {
    let esp = low32(frame.registers.rsp);
    let context = Context { registers: frame.registers };
    let ebp = low32(context.get(RBP)?);
    if frame.how == FrameFound::Registers {
        let eip = low32(frame.registers.rip);
        let code = code_at(memory, root, eip);
        let popped = match code.map(|code| byte(&code, 0)) {
            Some(0x55 | 0xC3) => Some(0),
            Some(0xC2) => {
                Some(code.and_then(|code| code.get(1..).and_then(<[u8]>::first_chunk::<2>).copied()).map_or(0, u16::from_le_bytes))
            }
            _ => None,
        };
        if let Some(popped) = popped {
            let mut registers = with_ebp(frame.registers, ebp);
            registers.rip = u64::from(read_u32(memory, root, esp)?);
            registers.rsp = low32(esp.wrapping_add(4).wrapping_add(u64::from(popped)));
            return Ok(Frame { registers, how: FrameFound::Leaf });
        }
        let before = code_at(memory, root, low32(eip.wrapping_sub(1)));
        let after_push = before.is_some_and(|before| byte(&before, 0) == 0x55)
            && code.is_some_and(|code| matches!([byte(&code, 0), byte(&code, 1)], [0x8B, 0xEC] | [0x89, 0xE5]));
        if after_push {
            let mut registers = with_ebp(frame.registers, u64::from(read_u32(memory, root, esp)?));
            registers.rip = u64::from(read_u32(memory, root, low32(esp.wrapping_add(4)))?);
            registers.rsp = low32(esp.wrapping_add(8));
            return Ok(Frame { registers, how: FrameFound::FramePointer });
        }
    }
    if ebp == 0 {
        return Err(StackEnd::Top);
    }
    if ebp < esp {
        return Err(if frame.how == FrameFound::Registers { StackEnd::NoFramePointer } else { StackEnd::NotGrowing });
    }
    let mut registers = with_ebp(frame.registers, u64::from(read_u32(memory, root, ebp)?));
    registers.rip = u64::from(read_u32(memory, root, low32(ebp.wrapping_add(4)))?);
    registers.rsp = low32(ebp.wrapping_add(8));
    Ok(Frame { registers, how: FrameFound::FramePointer })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::guest::fake::FakeGuest;

    const CR3: u64 = 0x1000;
    const BASE: u64 = 0xFFFF_F800_1000_0000;
    const STACK: u64 = 0xFFFF_F800_2000_8000;
    const RETURN: u64 = 0xFFFF_F800_3000_1234;
    const PDATA: u32 = 0x2000;
    const XDATA: u32 = 0x3000;

    const UWOP_PUSH_NONVOL: u8 = 0;
    const UWOP_ALLOC_LARGE: u8 = 1;
    const UWOP_ALLOC_SMALL: u8 = 2;
    const UWOP_SET_FPREG: u8 = 3;
    const UWOP_SAVE_NONVOL: u8 = 4;
    const UWOP_SAVE_NONVOL_FAR: u8 = 5;
    const UWOP_PUSH_MACHFRAME: u8 = 10;
    const UNW_FLAG_CHAININFO: u8 = 4;

    /// One unwind code slot: the prolog offset its instruction ends at, the
    /// operation and its info.
    fn code(offset: u8, op: u8, info: u8) -> u16 {
        u16::from(offset) | (u16::from(op | (info << 4)) << 8)
    }

    /// A `RUNTIME_FUNCTION`.
    #[derive(Clone, Copy)]
    struct Function {
        begin: u32,
        end: u32,
        info: u32,
    }

    /// The fields of an `UNWIND_INFO`.
    struct Info<'a> {
        prolog: u8,
        frame_register: u8,
        frame_offset: u8,
        codes: &'a [u16],
        chain: Option<Function>,
    }

    impl Info<'_> {
        fn plain(prolog: u8, codes: &[u16]) -> Info<'_> {
            Info { prolog, frame_register: 0, frame_offset: 0, codes, chain: None }
        }
    }

    /// A PE32+ image at `BASE` in a fake guest: headers, `.pdata` at
    /// `PDATA`, unwind infos from `XDATA`, code where the tests put it.
    struct Image {
        guest: FakeGuest,
        functions: Vec<Function>,
        next_info: u32,
    }

    impl Image {
        fn new() -> Self {
            Image { guest: FakeGuest::new(CR3), functions: Vec::new(), next_info: XDATA }
        }

        fn info(&mut self, info: &Info<'_>) -> u32 {
            let at = self.next_info;
            let flags = if info.chain.is_some() { UNW_FLAG_CHAININFO } else { 0 };
            let mut bytes = vec![1 | (flags << 3), info.prolog, info.codes.len() as u8, info.frame_register | (info.frame_offset << 4)];
            for slot in info.codes {
                bytes.extend_from_slice(&slot.to_le_bytes());
            }
            if info.codes.len() % 2 == 1 {
                bytes.extend_from_slice(&[0, 0]);
            }
            if let Some(chain) = info.chain {
                for value in [chain.begin, chain.end, chain.info] {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            self.guest.write(CR3, BASE + u64::from(at), &bytes);
            self.next_info = at + ((bytes.len() as u32 + 15) & !15);
            at
        }

        fn function(&mut self, begin: u32, end: u32, info: &Info<'_>) -> Function {
            let info = self.info(info);
            let function = Function { begin, end, info };
            self.functions.push(function);
            function
        }

        fn code_at(&mut self, rva: u32, bytes: &[u8]) {
            self.guest.write(CR3, BASE + u64::from(rva), bytes);
        }

        fn finish(mut self) -> FakeGuest {
            self.functions.sort_by_key(|function| function.begin);
            let mut header = vec![0u8; 0x200];
            header[0..2].copy_from_slice(b"MZ");
            header[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
            header[0x80..0x84].copy_from_slice(b"PE\0\0");
            header[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
            header[0x94..0x96].copy_from_slice(&0xF0u16.to_le_bytes());
            let optional = 0x98;
            header[optional..optional + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
            let exception = optional + 0x88;
            header[exception..exception + 4].copy_from_slice(&PDATA.to_le_bytes());
            header[exception + 4..exception + 8].copy_from_slice(&(self.functions.len() as u32 * 12).to_le_bytes());
            self.guest.write(CR3, BASE, &header);
            let mut table = Vec::new();
            for function in &self.functions {
                for value in [function.begin, function.end, function.info] {
                    table.extend_from_slice(&value.to_le_bytes());
                }
            }
            self.guest.write(CR3, BASE + u64::from(PDATA), &table);
            self.guest
        }
    }

    const MODULES: [UnwindModule; 1] = [UnwindModule { base: BASE, size: 0x10_0000 }];

    fn long64() -> Unwinding<'static> {
        Unwinding { root: Root::Live, modules: &MODULES, width: CodeWidth::Long64 }
    }

    fn at(rip_rva: u32, rsp: u64) -> FrameRegisters {
        let mut registers = [Some(0); 16];
        registers[RSP] = None;
        FrameRegisters { rip: BASE + u64::from(rip_rva), rsp, registers }
    }

    fn top(registers: FrameRegisters) -> Frame {
        Frame { registers, how: FrameFound::Registers }
    }

    /// Where a frame resumes.
    #[derive(Debug, PartialEq, Eq)]
    struct Landing {
        rip: u64,
        rsp: u64,
    }

    fn landing(frame: &Frame) -> Landing {
        Landing { rip: frame.registers.rip, rsp: frame.registers.rsp }
    }

    /// `push rbx` (ends at +1), `sub rsp, 0x20` (ends at +5).
    fn push_and_alloc(image: &mut Image) {
        image.function(0x1000, 0x1040, &Info::plain(5, &[code(5, UWOP_ALLOC_SMALL, 3), code(1, UWOP_PUSH_NONVOL, 3)]));
    }

    #[test]
    fn a_push_and_a_small_allocation_are_undone() {
        let mut image = Image::new();
        push_and_alloc(&mut image);
        let mut guest = image.finish();
        guest.write_u64(CR3, STACK + 0x20, 0xB0B);
        guest.write_u64(CR3, STACK + 0x28, RETURN);
        let caller = caller_of(&mut guest, &long64(), &top(at(0x1010, STACK))).unwrap();
        assert_eq!(caller.how, FrameFound::Unwind);
        assert_eq!(landing(&caller), Landing { rip: RETURN, rsp: STACK + 0x30 });
        assert_eq!(caller.registers.registers[RBX], Some(0xB0B));
        assert_eq!(caller.registers.registers[RAX], None, "a volatile register is not known in a caller");
    }

    /// Review focus 5: paused after `push rbx` and before `sub rsp`.
    #[test]
    fn a_prolog_paused_in_undoes_only_what_ran() {
        let mut image = Image::new();
        push_and_alloc(&mut image);
        let mut guest = image.finish();
        guest.write_u64(CR3, STACK, 0xB0B);
        guest.write_u64(CR3, STACK + 8, RETURN);
        let caller = caller_of(&mut guest, &long64(), &top(at(0x1001, STACK))).unwrap();
        assert_eq!(landing(&caller), Landing { rip: RETURN, rsp: STACK + 0x10 });
        assert_eq!(caller.registers.registers[RBX], Some(0xB0B));
        // At the first byte nothing has run: the return address is at RSP.
        guest.write_u64(CR3, STACK, RETURN);
        let caller = caller_of(&mut guest, &long64(), &top(at(0x1000, STACK))).unwrap();
        assert_eq!(landing(&caller), Landing { rip: RETURN, rsp: STACK + 8 });
        assert_eq!(caller.registers.registers[RBX], Some(0), "rbx was not pushed yet");
    }

    #[test]
    fn a_large_allocation_is_undone_in_both_forms() {
        let mut image = Image::new();
        // sub rsp, 0x1000 as (info 0, size / 8 in one slot).
        image.function(0x1000, 0x1040, &Info::plain(7, &[code(7, UWOP_ALLOC_LARGE, 0), 0x200]));
        // sub rsp, 0x12340 as (info 1, the size in two slots).
        image.function(0x1100, 0x1140, &Info::plain(7, &[code(7, UWOP_ALLOC_LARGE, 1), 0x2340, 0x1]));
        let mut guest = image.finish();
        guest.write_u64(CR3, STACK + 0x1000, RETURN);
        guest.write_u64(CR3, STACK + 0x12340, RETURN + 1);
        let small = caller_of(&mut guest, &long64(), &top(at(0x1010, STACK))).unwrap();
        assert_eq!(landing(&small), Landing { rip: RETURN, rsp: STACK + 0x1008 });
        let large = caller_of(&mut guest, &long64(), &top(at(0x1110, STACK))).unwrap();
        assert_eq!(landing(&large), Landing { rip: RETURN + 1, rsp: STACK + 0x12348 });
    }

    /// `push rbp; sub rsp, 0x40; lea rbp, [rsp + 0x20]`, then the body moved
    /// RSP (an `alloca`): the frame is found through RBP.
    #[test]
    fn a_frame_register_frame_is_found_through_it() {
        let mut image = Image::new();
        image.function(
            0x1000,
            0x1080,
            &Info {
                prolog: 10,
                frame_register: RBP as u8,
                frame_offset: 2,
                codes: &[code(10, UWOP_SET_FPREG, 0), code(5, UWOP_ALLOC_SMALL, 7), code(1, UWOP_PUSH_NONVOL, RBP as u8)],
                chain: None,
            },
        );
        let mut guest = image.finish();
        guest.write_u64(CR3, STACK + 0x40, 0xF00D);
        guest.write_u64(CR3, STACK + 0x48, RETURN);
        let mut paused = at(0x1040, STACK - 0x200);
        paused.registers[RBP] = Some(STACK + 0x20);
        let caller = caller_of(&mut guest, &long64(), &top(paused)).unwrap();
        assert_eq!(landing(&caller), Landing { rip: RETURN, rsp: STACK + 0x50 });
        assert_eq!(caller.registers.registers[RBP], Some(0xF00D));
    }

    #[test]
    fn saved_registers_are_read_near_and_far() {
        let mut image = Image::new();
        image.function(
            0x1000,
            0x1040,
            &Info::plain(
                13,
                &[code(13, UWOP_SAVE_NONVOL_FAR, RDI as u8), 0x18, 0, code(8, UWOP_SAVE_NONVOL, RSI as u8), 2, code(4, UWOP_ALLOC_SMALL, 3)],
            ),
        );
        let mut guest = image.finish();
        guest.write_u64(CR3, STACK + 0x10, 0x5151);
        guest.write_u64(CR3, STACK + 0x18, 0xD1D1);
        guest.write_u64(CR3, STACK + 0x20, RETURN);
        let caller = caller_of(&mut guest, &long64(), &top(at(0x1020, STACK))).unwrap();
        assert_eq!(landing(&caller), Landing { rip: RETURN, rsp: STACK + 0x28 });
        assert_eq!(caller.registers.registers[RSI], Some(0x5151));
        assert_eq!(caller.registers.registers[RDI], Some(0xD1D1));
    }

    /// A function's body moved out of line: its info chains to the
    /// function's own.
    #[test]
    fn chained_infos_are_followed() {
        let mut image = Image::new();
        let primary = image.function(0x1000, 0x1020, &Info::plain(1, &[code(1, UWOP_PUSH_NONVOL, RBX as u8)]));
        image.function(
            0x1800,
            0x1840,
            &Info { prolog: 0, frame_register: 0, frame_offset: 0, codes: &[code(4, UWOP_ALLOC_SMALL, 1)], chain: Some(primary) },
        );
        let mut guest = image.finish();
        guest.write_u64(CR3, STACK + 0x10, 0xB0B);
        guest.write_u64(CR3, STACK + 0x18, RETURN);
        let caller = caller_of(&mut guest, &long64(), &top(at(0x1810, STACK))).unwrap();
        assert_eq!(landing(&caller), Landing { rip: RETURN, rsp: STACK + 0x20 });
        assert_eq!(caller.registers.registers[RBX], Some(0xB0B));
    }

    #[test]
    fn a_machine_frame_restores_the_interrupted_rip_and_rsp() {
        let user_rip = 0x7FF6_0000_1234;
        let user_rsp = 0x0000_00AB_CDEF_0000;
        for error_code in [0u8, 1] {
            let mut image = Image::new();
            image.function(0x1000, 0x1100, &Info::plain(0, &[code(0, UWOP_PUSH_MACHFRAME, error_code)]));
            let mut guest = image.finish();
            let frame = STACK + 8 * u64::from(error_code);
            guest.write_u64(CR3, frame, user_rip);
            guest.write_u64(CR3, frame + 24, user_rsp);
            let caller = caller_of(&mut guest, &long64(), &top(at(0x1050, STACK))).unwrap();
            assert_eq!(caller.how, FrameFound::MachineFrame, "error code {error_code}");
            assert_eq!(landing(&caller), Landing { rip: user_rip, rsp: user_rsp }, "error code {error_code}");
        }
    }

    #[test]
    fn a_function_without_a_table_entry_is_a_leaf() {
        let mut image = Image::new();
        push_and_alloc(&mut image);
        let mut guest = image.finish();
        guest.write_u64(CR3, STACK, RETURN);
        let caller = caller_of(&mut guest, &long64(), &top(at(0x5000, STACK))).unwrap();
        assert_eq!(caller.how, FrameFound::Leaf);
        assert_eq!(landing(&caller), Landing { rip: RETURN, rsp: STACK + 8 });
    }

    /// Paused at the `pop rbx` of `add rsp, 0x20; pop rbx; ret`: the epilog
    /// is run forwards, not the prolog undone.
    #[test]
    fn an_epilog_paused_in_is_run_forwards() {
        let mut image = Image::new();
        push_and_alloc(&mut image);
        image.code_at(0x1030, &[0x48, 0x83, 0xC4, 0x20, 0x5B, 0xC3]);
        let mut guest = image.finish();
        guest.write_u64(CR3, STACK, 0xB0B);
        guest.write_u64(CR3, STACK + 8, RETURN);
        let caller = caller_of(&mut guest, &long64(), &top(at(0x1034, STACK))).unwrap();
        assert_eq!(caller.how, FrameFound::Epilog);
        assert_eq!(landing(&caller), Landing { rip: RETURN, rsp: STACK + 0x10 });
        assert_eq!(caller.registers.registers[RBX], Some(0xB0B));
        // At the `add`: the allocation is given back first.
        guest.write_u64(CR3, STACK + 0x20, 0xB0B);
        guest.write_u64(CR3, STACK + 0x28, RETURN);
        let caller = caller_of(&mut guest, &long64(), &top(at(0x1030, STACK))).unwrap();
        assert_eq!(landing(&caller), Landing { rip: RETURN, rsp: STACK + 0x30 });
    }

    /// The caller's `call` was its function's last instruction: the return
    /// address is that function's end, and the function is found one byte
    /// back.
    #[test]
    fn a_return_address_is_looked_up_one_byte_back() {
        let mut image = Image::new();
        push_and_alloc(&mut image);
        image.function(0x10C0, 0x1100, &Info::plain(4, &[code(4, UWOP_ALLOC_SMALL, 1)]));
        let mut guest = image.finish();
        guest.write_u64(CR3, STACK + 0x28, BASE + 0x1100);
        guest.write_u64(CR3, STACK + 0x30, BASE + 0x5000);
        guest.write_u64(CR3, STACK + 0x40, RETURN);
        let stack = unwind(&mut guest, &long64(), at(0x1010, STACK));
        let caller = stack.frames.get(2).expect("the caller's caller");
        assert_eq!(landing(caller), Landing { rip: RETURN, rsp: STACK + 0x48 });
    }

    #[test]
    fn a_stack_unwinds_to_its_end() {
        let mut image = Image::new();
        push_and_alloc(&mut image);
        let mut guest = image.finish();
        // The caller is a leaf of the same module returning to 0: the end.
        guest.write_u64(CR3, STACK + 0x28, BASE + 0x5000);
        guest.write_u64(CR3, STACK + 0x30, 0);
        let stack = unwind(&mut guest, &long64(), at(0x1010, STACK));
        let rips: Vec<u64> = stack.frames.iter().map(|frame| frame.registers.rip).collect();
        assert_eq!(rips, [BASE + 0x1010, BASE + 0x5000]);
        assert_eq!(stack.end, StackEnd::Top);
    }

    #[test]
    fn code_outside_every_module_ends_the_stack() {
        let mut guest = Image::new().finish();
        let stack = unwind(&mut guest, &long64(), FrameRegisters { rip: 0x1234, rsp: STACK, registers: [None; 16] });
        assert_eq!(stack.frames.len(), 1);
        assert_eq!(stack.end, StackEnd::NoUnwindInfo { rip: 0x1234 });
    }

    #[test]
    fn a_stack_that_does_not_read_ends_where_it_stops() {
        let mut image = Image::new();
        push_and_alloc(&mut image);
        let mut guest = image.finish();
        let stack = unwind(&mut guest, &long64(), at(0x1010, 0xFFFF_F800_7000_0000));
        assert_eq!(stack.frames.len(), 1);
        assert_eq!(stack.end, StackEnd::Unreadable { at: 0xFFFF_F800_7000_0020 });
    }

    /// Unwind infos of every byte value: each ends the stack with a reason,
    /// none panics.
    #[test]
    fn garbage_unwind_info_ends_the_stack_without_a_panic() {
        for seed in 0u32..64 {
            let mut image = Image::new();
            image.function(0x1000, 0x1040, &Info::plain(5, &[code(5, UWOP_ALLOC_SMALL, 3)]));
            let mut guest = image.finish();
            let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
            let garbage: Vec<u8> = (0..300)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect();
            guest.write(CR3, BASE + u64::from(XDATA), &garbage);
            for slot in 0..64u64 {
                guest.write_u64(CR3, STACK + slot * 8, BASE + 0x1010);
            }
            let stack = unwind(&mut guest, &long64(), at(0x1010, STACK));
            assert!(!stack.frames.is_empty() && stack.frames.len() <= MAX_FRAMES, "seed {seed}");
        }
    }

    const STACK32: u64 = 0x0012_0000;

    fn protected32() -> Unwinding<'static> {
        Unwinding { root: Root::Live, modules: &[], width: CodeWidth::Protected32 }
    }

    fn at32(eip: u64, esp: u64, ebp: u64) -> FrameRegisters {
        let mut registers = [Some(0); 16];
        registers[RSP] = None;
        registers[RBP] = Some(ebp);
        FrameRegisters { rip: eip, rsp: esp, registers }
    }

    #[test]
    fn a_32_bit_stack_follows_its_frame_pointers() {
        let mut guest = FakeGuest::new(CR3);
        let outer = STACK32 + 0x100;
        let inner = STACK32 + 0x40;
        guest.write_u32(CR3, inner, outer as u32);
        guest.write_u32(CR3, inner + 4, 0x40_1234);
        guest.write_u32(CR3, outer, 0);
        guest.write_u32(CR3, outer + 4, 0x40_5678);
        let stack = unwind(&mut guest, &protected32(), at32(0x40_1000, STACK32, inner));
        let rips: Vec<u64> = stack.frames.iter().map(|frame| frame.registers.rip).collect();
        assert_eq!(rips, [0x40_1000, 0x40_1234, 0x40_5678]);
        assert_eq!(stack.frames.get(1).map(|frame| frame.how), Some(FrameFound::FramePointer));
        assert_eq!(stack.frames.get(1).map(|frame| frame.registers.rsp), Some(inner + 8));
        assert_eq!(stack.end, StackEnd::Top);
    }

    #[test]
    fn a_frame_pointer_below_the_stack_is_no_frame_pointer() {
        let mut guest = FakeGuest::new(CR3);
        guest.write_u32(CR3, STACK32, 0);
        let stack = unwind(&mut guest, &protected32(), at32(0x40_1000, STACK32, STACK32 - 0x10));
        assert_eq!(stack.frames.len(), 1);
        assert_eq!(stack.end, StackEnd::NoFramePointer);
    }

    #[test]
    fn a_frame_pointer_that_does_not_grow_ends_the_stack() {
        let mut guest = FakeGuest::new(CR3);
        let frame = STACK32 + 0x40;
        guest.write_u32(CR3, frame, frame as u32);
        guest.write_u32(CR3, frame + 4, 0x40_1234);
        let stack = unwind(&mut guest, &protected32(), at32(0x40_1000, STACK32, frame));
        assert_eq!(stack.end, StackEnd::NotGrowing);
    }

    #[test]
    fn a_stack_stops_at_its_ceiling() {
        let mut guest = FakeGuest::new(CR3);
        for index in 0..300u64 {
            let frame = STACK32 + index * 16;
            guest.write_u32(CR3, frame, (frame + 16) as u32);
            guest.write_u32(CR3, frame + 4, 0x40_0000 + index as u32);
        }
        let stack = unwind(&mut guest, &protected32(), at32(0x40_1000, STACK32 - 8, STACK32));
        assert_eq!(stack.frames.len(), MAX_FRAMES);
        assert_eq!(stack.end, StackEnd::Ceiling);
    }

    /// At a 32-bit function's first byte (`push ebp`) the return address is
    /// at ESP and EBP is still the caller's.
    #[test]
    fn a_32_bit_function_paused_at_its_entry_returns_through_esp() {
        let mut guest = FakeGuest::new(CR3);
        guest.write(CR3, 0x40_1000, &[0x55, 0x8B, 0xEC]);
        guest.write_u32(CR3, STACK32, 0x40_2222);
        let caller = caller_of(&mut guest, &protected32(), &top(at32(0x40_1000, STACK32, STACK32 + 0x80))).unwrap();
        assert_eq!(landing(&caller), Landing { rip: 0x40_2222, rsp: STACK32 + 4 });
        assert_eq!(caller.registers.registers[RBP], Some(STACK32 + 0x80));
        // After `push ebp`, before `mov ebp, esp`.
        guest.write_u32(CR3, STACK32, (STACK32 + 0x80) as u32);
        guest.write_u32(CR3, STACK32 + 4, 0x40_2222);
        let caller = caller_of(&mut guest, &protected32(), &top(at32(0x40_1001, STACK32, 0x1234))).unwrap();
        assert_eq!(landing(&caller), Landing { rip: 0x40_2222, rsp: STACK32 + 8 });
        assert_eq!(caller.registers.registers[RBP], Some(STACK32 + 0x80));
    }
}
