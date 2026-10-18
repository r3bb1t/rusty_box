//! A frame's variables as the variables pane shows them (spec §8): each a
//! row with its name, its type's name and its value, a structure or array
//! expandable into rows of its own.

use rusty_box_symbols::tables::{
    BaseKind, BaseRegister, BaseType, Constness, Enumerator, FrameScope, FrameVariable, LiveRange, LocalsError,
    PointerKind, RangedPlace, RecordField, SymbolTables, TypeShape, VariableKind, VariableLocation, Volatility,
};
use rusty_box_symbols::units::{BitRange, CvRegister, Rva, TypeRef};

use super::frames::{InspectRequest, LocalsRead};
use super::guest::{GuestMemory, GuestRead, Root};
use super::unwind::{CodeWidth, Frame, RAX, RBP, RBX, RCX, RDI, RDX, RSI, RSP};

/// The most rows one read lists, the last of them a `Ceiling` row.
pub(crate) const MAX_VARIABLE_ROWS: usize = 5_000;
/// The most characters a pointed-at string is read to.
pub(crate) const STRING_CHARACTERS: usize = 256;
/// The elements an expanded array lists, per page asked for.
pub(crate) const ELEMENTS_PER_PAGE: u64 = 100;
/// The members or elements an aggregate's preview shows.
pub(crate) const MAX_PREVIEW_ITEMS: usize = 6;
/// The most links one walk of a type follows, as the TPI reader's own walks.
const MAX_TYPE_HOPS: u32 = 64;
/// The widest scalar a value reads.
const MAX_SCALAR_BYTES: u64 = 16;
/// The largest variable assembled from register pieces.
const MAX_PIECED_BYTES: u64 = 4096;
const PAGE_BYTES: u64 = 0x1000;
const PAGE_OFFSET_MASK: u64 = PAGE_BYTES - 1;
const R13: usize = 13;
/// CodeView numbers `AX`..`DI` (and their byte and doubleword forms) in this
/// order (LLVM `CodeViewRegisters.def`).
const LEGACY_ORDER: [usize; 8] = [RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI];

/// What a row's path starts from: a variable of the frame, watch `n`, or
/// the value tip.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PathRoot {
    Variable(String),
    Watch(usize),
    Tip,
}

/// One step from a value to a part of it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PathStep {
    Field(String),
    Element(u64),
    Deref,
}

/// Where a row's value sits under its root: what the pane expands by.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ValuePath {
    pub(crate) root: PathRoot,
    pub(crate) steps: Vec<PathStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signedness {
    Signed,
    Unsigned,
}

/// How a string a pointer points at ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StringEnd {
    Terminated,
    /// `STRING_CHARACTERS` read without a terminator.
    Cut,
    /// The bytes after it did not read.
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PointedString {
    pub(crate) text: String,
    pub(crate) end: StringEnd,
}

/// One member (`name` set) or element of an aggregate's preview: a scalar's
/// value, or a nested aggregate with an empty preview.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreviewItem {
    pub(crate) name: Option<String>,
    pub(crate) value: ShownValue,
}

/// Whether a preview shows every member or element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Preview {
    Whole,
    Cut,
}

/// A value as read, for the pane to format (spec §8.2).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ShownValue {
    Integer { raw: u64, size: u8, signedness: Signedness },
    Character { code: u32, size: u8 },
    Boolean { raw: u8 },
    Real32(f32),
    Real64(f64),
    Hresult(u32),
    Pointer { address: u64, string: Option<PointedString> },
    /// One name for an exact value, several for flags, none: the number.
    Enum { raw: i128, names: Vec<String> },
    /// A record (`count` none) or an array (its element count), with a
    /// preview of its first members or elements.
    Aggregate { count: Option<u64>, preview: Vec<PreviewItem>, more: Preview },
    /// "not available here"
    NotAvailable,
    /// "optimized away"
    OptimizedAway,
    /// "memory at 0x… is not readable"
    Unreadable { at: u64 },
    /// "<type 0x1234>" and its bytes.
    UnknownType { index: u32, leaf: u16, bytes: Vec<u8> },
    /// A type or records error, verbatim.
    Problem(String),
}

/// Whether a row opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Expand {
    Leaf,
    Closed,
    Open,
}

/// What a row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    Parameter,
    Local,
    Static,
    Member,
    Element,
    /// What a pointer points at.
    Target,
    /// A watch's own row.
    Watch,
    /// "Show 100 more": `shown` of `of` elements are listed.
    MoreElements { shown: u64, of: u64 },
    /// More rows than a read lists.
    Ceiling,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VariableRow {
    pub(crate) path: ValuePath,
    pub(crate) depth: u8,
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) kind: RowKind,
    pub(crate) value: ShownValue,
    pub(crate) expand: Expand,
    /// The bytes the value is in, when it is in memory.
    pub(crate) memory: Option<MemorySpan>,
}

/// `size` bytes of guest memory from linear `address`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MemorySpan {
    pub(crate) address: u64,
    pub(crate) size: u64,
}

/// Whether a frame is the one the processor stopped in, whose every
/// register is its own, or a caller found by unwinding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Innermost {
    Yes,
    Caller,
}

/// The frame whose variables are read, and what reading them needs.
#[derive(Clone, Copy)]
pub(crate) struct FrameContext<'a> {
    pub(crate) frame: &'a Frame,
    pub(crate) innermost: Innermost,
    /// The processor's XMM registers, when they are known.
    pub(crate) xmm: Option<&'a [[u8; 16]; 16]>,
    pub(crate) root: Root,
    pub(crate) width: CodeWidth,
    pub(crate) module_base: u64,
    pub(crate) tables: &'a SymbolTables,
}

impl FrameContext<'_> {
    /// The frame's code as an RVA of its module.
    fn rva(&self) -> u64 {
        self.frame.code_address().wrapping_sub(self.module_base)
    }

    /// `base + offset` as the frame's code computes an address.
    pub(crate) fn address(&self, base: u64, offset: i64) -> u64 {
        let address = base.wrapping_add_signed(offset);
        match self.width {
            CodeWidth::Long64 => address,
            CodeWidth::Protected32 => address & 0xFFFF_FFFF,
        }
    }

    /// General register `index` as the frame knows it.
    fn general(&self, index: usize) -> Option<u64> {
        if index == RSP {
            return Some(self.frame.registers.rsp);
        }
        self.frame.registers.registers.get(index).copied().flatten()
    }
}

/// Where a value is: in memory, in bytes already read (a register's), or
/// nowhere, with what to show instead. `AddressOf` is a pointer value no
/// type record describes, a watch's `&x`: the address of the located type.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Place {
    Memory(u64),
    Bytes(Vec<u8>),
    Missing(ShownValue),
    AddressOf(u64),
}

impl Place {
    /// The part of the value `offset` bytes in.
    pub(crate) fn part(&self, offset: u64) -> Place {
        match self {
            Place::Memory(address) => Place::Memory(address.wrapping_add(offset)),
            Place::Bytes(bytes) => match usize::try_from(offset).ok().and_then(|offset| bytes.get(offset..)) {
                Some(rest) => Place::Bytes(rest.to_vec()),
                None => Place::Missing(ShownValue::NotAvailable),
            },
            Place::Missing(value) => Place::Missing(value.clone()),
            Place::AddressOf(_) => Place::Missing(ShownValue::NotAvailable),
        }
    }
}

/// A value of type `ty` at `place`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Located {
    pub(crate) ty: TypeRef,
    pub(crate) place: Place,
}

/// What a row is before its value is read: where it sits, and its name.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowHead {
    pub(crate) path: ValuePath,
    pub(crate) depth: u8,
    pub(crate) name: String,
    pub(crate) kind: RowKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fullness {
    Room,
    Full,
}

/// The rows of one read, up to `MAX_VARIABLE_ROWS`, the last of which then
/// says the rest were not listed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowSink {
    rows: Vec<VariableRow>,
    full: Fullness,
}

impl RowSink {
    pub(crate) fn new() -> RowSink {
        RowSink { rows: Vec::new(), full: Fullness::Room }
    }

    pub(crate) fn fullness(&self) -> Fullness {
        self.full
    }

    pub(crate) fn into_rows(self) -> Vec<VariableRow> {
        self.rows
    }

    pub(crate) fn push(&mut self, row: VariableRow) {
        match self.full {
            Fullness::Full => {}
            Fullness::Room if self.rows.len().saturating_add(1) >= MAX_VARIABLE_ROWS => {
                self.rows.push(VariableRow {
                    name: String::new(),
                    type_name: String::new(),
                    kind: RowKind::Ceiling,
                    value: ShownValue::NotAvailable,
                    expand: Expand::Leaf,
                    memory: None,
                    ..row
                });
                self.full = Fullness::Full;
            }
            Fullness::Room => self.rows.push(row),
        }
    }
}

/// How far a value is read: a row's in full, a preview item's without its
/// own preview or string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reading {
    Row,
    PreviewItem,
}

/// What an open row lists beneath it.
#[derive(Clone, Debug, PartialEq)]
enum Opening {
    Nothing,
    Members(Vec<RecordField>),
    Elements { element: TypeRef, count: u64, size: u64 },
    Target { to: TypeRef, address: u64 },
}

/// Which bytes of which general register a CodeView register number names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RegisterPart {
    index: usize,
    offset: usize,
    len: usize,
}

/// The variables in scope at the frame's code: none when no function's
/// records hold it.
pub(crate) fn frame_scope(context: &FrameContext<'_>) -> Result<Option<FrameScope>, LocalsError> {
    context.tables.frame_locals(Rva(context.rva()))
}

/// The variables of `scope` (the frame's, from [`frame_scope`]) as rows:
/// each variable, and beneath each value `request` expands its members,
/// elements or target.
pub(crate) fn locals_of<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    scope: &Result<Option<FrameScope>, LocalsError>,
    request: &InspectRequest,
) -> LocalsRead {
    let scope = match scope {
        Ok(Some(scope)) => scope,
        Ok(None) => return LocalsRead::NoFunction,
        Err(error) => return LocalsRead::Unreadable { why: error.to_string() },
    };
    let mut rows = RowSink::new();
    for variable in &scope.variables {
        if rows.fullness() == Fullness::Full {
            break;
        }
        let head = RowHead {
            path: ValuePath { root: PathRoot::Variable(variable.name.clone()), steps: Vec::new() },
            depth: 0,
            name: variable.name.clone(),
            kind: match variable.kind {
                VariableKind::Parameter => RowKind::Parameter,
                VariableKind::Local => RowKind::Local,
                VariableKind::Static => RowKind::Static,
            },
        };
        rows_of(memory, context, head, locate(context, variable, scope), request, &mut rows);
    }
    LocalsRead::Rows(rows.into_rows())
}

/// Where `variable` of `scope` is with the frame's code where it is
/// (spec §8.1).
pub(crate) fn locate(context: &FrameContext<'_>, variable: &FrameVariable, scope: &FrameScope) -> Located {
    let rva = context.rva();
    let in_body = scope.body_start.0 <= rva && rva < scope.body_end.0;
    let framed = match context.innermost {
        Innermost::Yes => in_body,
        Innermost::Caller => true,
    };
    let place = match &variable.location {
        VariableLocation::OptimizedOut => Place::Missing(ShownValue::OptimizedAway),
        VariableLocation::Static { rva } => Place::Memory(context.module_base.wrapping_add(rva.0)),
        VariableLocation::Register { register } => register_place(context, *register),
        VariableLocation::RegisterRelative { .. } | VariableLocation::FrameRelative { .. } if !framed => {
            Place::Missing(ShownValue::NotAvailable)
        }
        VariableLocation::RegisterRelative { register, offset } => relative(context, register_value(context, *register), *offset),
        VariableLocation::FrameRelative { offset } => relative(context, frame_base(context, scope, variable.kind), *offset),
        VariableLocation::Ranged(ranges) => ranged_place(context, scope, variable, ranges),
    };
    Located { ty: variable.ty, place }
}

/// Memory at `base + offset`, when the base is known.
fn relative(context: &FrameContext<'_>, base: Option<u64>, offset: i64) -> Place {
    match base {
        Some(base) => Place::Memory(context.address(base, offset)),
        None => Place::Missing(ShownValue::NotAvailable),
    }
}

/// The register `S_FRAMEPROC` addresses a variable of `kind` from.
fn frame_base(context: &FrameContext<'_>, scope: &FrameScope, kind: VariableKind) -> Option<u64> {
    let base = match kind {
        VariableKind::Parameter => scope.base.params,
        VariableKind::Local | VariableKind::Static => scope.base.locals,
    };
    match base {
        BaseRegister::None => None,
        BaseRegister::StackPointer => match context.width {
            CodeWidth::Long64 => context.general(RSP),
            // `VFRAME`: computed from the frame's FPO data, which is not read.
            CodeWidth::Protected32 => None,
        },
        BaseRegister::FramePointer => context.general(RBP),
        BaseRegister::BasePointer => match context.width {
            CodeWidth::Long64 => context.general(R13),
            CodeWidth::Protected32 => context.general(RBX),
        },
    }
}

/// Where a variable placed by its live ranges is: the first range holding
/// the frame's code outside its gaps; a variable in register pieces is whole
/// only when every live piece is known and they cover all its bytes.
fn ranged_place(context: &FrameContext<'_>, scope: &FrameScope, variable: &FrameVariable, ranges: &[LiveRange]) -> Place {
    let rva = context.rva();
    let live: Vec<&LiveRange> = ranges
        .iter()
        .filter(|range| {
            range.start.0 <= rva && rva < range.end.0 && !range.gaps.iter().any(|gap| gap.start.0 <= rva && rva < gap.end.0)
        })
        .collect();
    let Some(first) = live.first() else {
        return Place::Missing(ShownValue::NotAvailable);
    };
    match first.place {
        RangedPlace::Register { register } => register_place(context, register),
        RangedPlace::FrameRelative { offset } => relative(context, frame_base(context, scope, variable.kind), offset),
        RangedPlace::RegisterRelative { register, offset } => relative(context, register_value(context, register), offset),
        RangedPlace::Subfield { .. } => pieced_place(context, &live, size_of(context.tables, variable.ty)),
    }
}

/// A variable of `size` bytes assembled from the register pieces among `live`.
fn pieced_place(context: &FrameContext<'_>, live: &[&LiveRange], size: Option<u64>) -> Place {
    let Some(size) = size.filter(|size| *size > 0 && *size <= MAX_PIECED_BYTES).and_then(|size| usize::try_from(size).ok())
    else {
        return Place::Missing(ShownValue::NotAvailable);
    };
    let mut bytes = vec![0u8; size];
    let mut covered = vec![false; size];
    for range in live {
        let RangedPlace::Subfield { register, offset_in_parent } = range.place else {
            continue;
        };
        let Ok(piece) = register_bytes(context, register) else {
            return Place::Missing(ShownValue::NotAvailable);
        };
        let Ok(start) = usize::try_from(offset_in_parent) else {
            return Place::Missing(ShownValue::NotAvailable);
        };
        for (index, byte) in piece.iter().enumerate() {
            let at = start.saturating_add(index);
            if let Some(slot) = bytes.get_mut(at) {
                *slot = *byte;
            }
            if let Some(seen) = covered.get_mut(at) {
                *seen = true;
            }
        }
    }
    if covered.iter().all(|seen| *seen) { Place::Bytes(bytes) } else { Place::Missing(ShownValue::NotAvailable) }
}

/// A register's bytes as a place.
fn register_place(context: &FrameContext<'_>, register: CvRegister) -> Place {
    match register_bytes(context, register) {
        Ok(bytes) => Place::Bytes(bytes),
        Err(missing) => Place::Missing(missing),
    }
}

/// A register's value, as an address is computed from it.
fn register_value(context: &FrameContext<'_>, register: CvRegister) -> Option<u64> {
    register_bytes(context, register).ok().map(|bytes| unsigned(&bytes))
}

/// The bytes of register `register` in the frame: `NotAvailable` for one the
/// frame does not know (a caller's volatile registers and XMM registers) or
/// a number not mapped (LLVM `CodeViewRegisters.def`, Microsoft `cvconst.h`).
fn register_bytes(context: &FrameContext<'_>, register: CvRegister) -> Result<Vec<u8>, ShownValue> {
    if let Some(part) = general_part(register.0) {
        let value = context.general(part.index).ok_or(ShownValue::NotAvailable)?;
        let bytes = value.to_le_bytes();
        return bytes
            .get(part.offset..part.offset.saturating_add(part.len))
            .map(<[u8]>::to_vec)
            .ok_or(ShownValue::NotAvailable);
    }
    let part = xmm_part(register.0).ok_or(ShownValue::NotAvailable)?;
    let Innermost::Yes = context.innermost else {
        return Err(ShownValue::NotAvailable);
    };
    let xmm = context.xmm.ok_or(ShownValue::NotAvailable)?;
    xmm.get(part.index)
        .and_then(|lanes| lanes.get(part.offset..part.offset.saturating_add(part.len)))
        .map(<[u8]>::to_vec)
        .ok_or(ShownValue::NotAvailable)
}

fn register_part(index: usize, offset: usize, len: usize) -> RegisterPart {
    RegisterPart { index, offset, len }
}

/// Register `number - first` of `LEGACY_ORDER`.
fn legacy(number: u16, first: u16, offset: usize, len: usize) -> Option<RegisterPart> {
    let index = LEGACY_ORDER.get(usize::from(number.checked_sub(first)?))?;
    Some(register_part(*index, offset, len))
}

/// Register `base + (number - first)`, `len` bytes from its start.
fn numbered(number: u16, first: u16, base: usize, len: usize) -> Option<RegisterPart> {
    let index = base.checked_add(usize::from(number.checked_sub(first)?))?;
    Some(register_part(index, 0, len))
}

/// Register `8 + (number - first)`: one of R8..R15.
fn extended(number: u16, first: u16, len: usize) -> Option<RegisterPart> {
    numbered(number, first, 8, len)
}

/// The 4-byte lane `(number - first) % 4` of XMM register
/// `base + (number - first) / 4`.
fn xmm_lane(number: u16, first: u16, base: usize) -> Option<RegisterPart> {
    let at = usize::from(number.checked_sub(first)?);
    let index = base.checked_add(at.checked_div(4)?)?;
    let offset = at.checked_rem(4)?.checked_mul(4)?;
    Some(register_part(index, offset, 4))
}

/// The general-register bytes a CodeView number names.
fn general_part(number: u16) -> Option<RegisterPart> {
    match number {
        1..=4 => legacy(number, 1, 0, 1),
        5..=8 => legacy(number, 5, 1, 1),
        9..=16 => legacy(number, 9, 0, 2),
        17..=24 => legacy(number, 17, 0, 4),
        324 => Some(register_part(RSI, 0, 1)),
        325 => Some(register_part(RDI, 0, 1)),
        326 => Some(register_part(RBP, 0, 1)),
        327 => Some(register_part(RSP, 0, 1)),
        328 => Some(register_part(RAX, 0, 8)),
        329 => Some(register_part(RBX, 0, 8)),
        330 => Some(register_part(RCX, 0, 8)),
        331 => Some(register_part(RDX, 0, 8)),
        332 => Some(register_part(RSI, 0, 8)),
        333 => Some(register_part(RDI, 0, 8)),
        334 => Some(register_part(RBP, 0, 8)),
        335 => Some(register_part(RSP, 0, 8)),
        336..=343 => extended(number, 336, 8),
        344..=351 => extended(number, 344, 1),
        352..=359 => extended(number, 352, 2),
        360..=367 => extended(number, 360, 4),
        _ => None,
    }
}

/// The XMM-register bytes a CodeView number names: a whole register, a
/// 4-byte lane, or the low 8 bytes.
fn xmm_part(number: u16) -> Option<RegisterPart> {
    match number {
        154..=161 => numbered(number, 154, 0, 16),
        162..=193 => xmm_lane(number, 162, 0),
        194..=201 => numbered(number, 194, 0, 8),
        252..=259 => numbered(number, 252, 8, 16),
        260..=291 => xmm_lane(number, 260, 8),
        292..=299 => numbered(number, 292, 8, 8),
        _ => None,
    }
}

/// The row for `value` under `head`, and beneath it, when `request` expands
/// it, its members, elements or target.
pub(crate) fn rows_of<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    head: RowHead,
    value: Located,
    request: &InspectRequest,
    rows: &mut RowSink,
) {
    if rows.fullness() == Fullness::Full {
        return;
    }
    let shown = value_of(memory, context, &value.place, value.ty, Reading::Row);
    let address_of = match value.place {
        Place::AddressOf(address) => Some(address),
        Place::Memory(_) | Place::Bytes(_) | Place::Missing(_) => None,
    };
    let opening = match address_of {
        Some(address) if address != 0 && points_at_data(context.tables, value.ty) => {
            Opening::Target { to: value.ty, address }
        }
        Some(_) => Opening::Nothing,
        None => opening(context.tables, value.ty, &shown),
    };
    let type_name = match address_of {
        Some(_) => spell_pointer(context.tables, value.ty, PointerKind::Pointer, "", String::new(), 0),
        None => type_text(context.tables, value.ty),
    };
    let open = request.expanded.contains(&head.path);
    let expand = match opening {
        Opening::Nothing => Expand::Leaf,
        Opening::Members(_) | Opening::Elements { .. } | Opening::Target { .. } if open => Expand::Open,
        Opening::Members(_) | Opening::Elements { .. } | Opening::Target { .. } => Expand::Closed,
    };
    rows.push(VariableRow {
        path: head.path.clone(),
        depth: head.depth,
        name: head.name.clone(),
        type_name,
        kind: head.kind,
        value: shown,
        expand,
        memory: match value.place {
            Place::Memory(address) => {
                Some(MemorySpan { address, size: size_of(context.tables, value.ty).filter(|size| *size > 0).unwrap_or(1) })
            }
            Place::Bytes(_) | Place::Missing(_) | Place::AddressOf(_) => None,
        },
    });
    if expand != Expand::Open {
        return;
    }
    let Some(depth) = head.depth.checked_add(1) else {
        return;
    };
    match opening {
        Opening::Nothing => {}
        Opening::Members(fields) => {
            for field in &fields {
                if rows.fullness() == Fullness::Full {
                    return;
                }
                let child = RowHead {
                    path: step(&head.path, PathStep::Field(field.name.clone())),
                    depth,
                    name: field.name.clone(),
                    kind: RowKind::Member,
                };
                let place = match field.bits {
                    None => value.place.part(field.offset.0),
                    Some(bits) => bitfield_place(memory, context, &value.place.part(field.offset.0), field.ty, bits),
                };
                rows_of(memory, context, child, Located { ty: field.ty, place }, request, rows);
            }
        }
        Opening::Elements { element, count, size } => {
            let pages = request.pages.get(&head.path).copied().unwrap_or(0);
            let listed = count.min(ELEMENTS_PER_PAGE.saturating_mul(u64::from(pages).saturating_add(1)));
            for index in 0..listed {
                if rows.fullness() == Fullness::Full {
                    return;
                }
                let child = RowHead {
                    path: step(&head.path, PathStep::Element(index)),
                    depth,
                    name: format!("[{index}]"),
                    kind: RowKind::Element,
                };
                let place = value.place.part(index.saturating_mul(size));
                rows_of(memory, context, child, Located { ty: element, place }, request, rows);
            }
            if listed < count {
                rows.push(VariableRow {
                    path: head.path,
                    depth,
                    name: String::new(),
                    type_name: String::new(),
                    kind: RowKind::MoreElements { shown: listed, of: count },
                    value: ShownValue::NotAvailable,
                    expand: Expand::Leaf,
                    memory: None,
                });
            }
        }
        Opening::Target { to, address } => {
            let child = RowHead {
                path: step(&head.path, PathStep::Deref),
                depth,
                name: format!("*{}", head.name),
                kind: RowKind::Target,
            };
            rows_of(memory, context, child, Located { ty: to, place: Place::Memory(address) }, request, rows);
        }
    }
}

fn step(path: &ValuePath, next: PathStep) -> ValuePath {
    let mut steps = path.steps.clone();
    steps.push(next);
    ValuePath { root: path.root.clone(), steps }
}

/// What a row showing `shown` of type `ty` opens into: a read record's
/// members, a read array's elements, a non-null data pointer's target.
fn opening(tables: &SymbolTables, ty: TypeRef, shown: &ShownValue) -> Opening {
    let Ok(shape) = unmodified(tables, ty) else {
        return Opening::Nothing;
    };
    let read = matches!(shown, ShownValue::Aggregate { .. });
    match shape {
        TypeShape::Record { fields, .. } if read && !fields.is_empty() => Opening::Members(fields),
        TypeShape::Array { element, bytes } if read => match size_of(tables, element) {
            Some(size) => match bytes.checked_div(size) {
                Some(count) if count > 0 => Opening::Elements { element, count, size },
                Some(_) | None => Opening::Nothing,
            },
            None => Opening::Nothing,
        },
        TypeShape::Pointer { to, .. } => match shown {
            ShownValue::Pointer { address, .. } if *address != 0 && points_at_data(tables, to) => {
                Opening::Target { to, address: *address }
            }
            _ => Opening::Nothing,
        },
        TypeShape::Base(_)
        | TypeShape::Record { .. }
        | TypeShape::Array { .. }
        | TypeShape::Modified { .. }
        | TypeShape::Incomplete { .. }
        | TypeShape::Enum { .. }
        | TypeShape::Function { .. }
        | TypeShape::Unknown { .. } => Opening::Nothing,
    }
}

/// Whether a pointer to `to` points at a value: not `void`, a function, or a
/// type these symbols do not describe.
fn points_at_data(tables: &SymbolTables, to: TypeRef) -> bool {
    match unmodified(tables, to) {
        Ok(TypeShape::Base(BaseType { kind: BaseKind::Void, .. })) => false,
        Ok(TypeShape::Function { .. } | TypeShape::Incomplete { .. } | TypeShape::Unknown { .. }) => false,
        Ok(
            TypeShape::Base(_)
            | TypeShape::Pointer { .. }
            | TypeShape::Modified { .. }
            | TypeShape::Array { .. }
            | TypeShape::Record { .. }
            | TypeShape::Enum { .. },
        ) => true,
        Err(_) => false,
    }
}

/// The value of type `ty` at `place` (spec §8.2).
fn value_of<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    place: &Place,
    ty: TypeRef,
    reading: Reading,
) -> ShownValue {
    match place {
        Place::Missing(value) => return value.clone(),
        Place::AddressOf(address) => {
            let string = match reading {
                Reading::Row => pointed_string(memory, context, ty, *address),
                Reading::PreviewItem => None,
            };
            return ShownValue::Pointer { address: *address, string };
        }
        Place::Memory(_) | Place::Bytes(_) => {}
    }
    let shape = match unmodified(context.tables, ty) {
        Ok(shape) => shape,
        Err(problem) => return problem,
    };
    match shape {
        TypeShape::Base(base) => base_value(memory, context.root, place, base),
        TypeShape::Pointer { to, size, .. } => {
            let bytes = match read_place(memory, context.root, place, u64::from(size)) {
                Ok(bytes) => bytes,
                Err(missing) => return missing,
            };
            let address = unsigned(&bytes);
            let string = match reading {
                Reading::Row => pointed_string(memory, context, to, address),
                Reading::PreviewItem => None,
            };
            ShownValue::Pointer { address, string }
        }
        TypeShape::Enum { underlying, values, .. } => enum_value(memory, context, place, underlying, &values),
        TypeShape::Record { fields, .. } => {
            let mut preview = Vec::new();
            if reading == Reading::Row {
                for field in fields.iter().take(MAX_PREVIEW_ITEMS) {
                    let value = field_value(memory, context, place, field);
                    preview.push(PreviewItem { name: Some(field.name.clone()), value });
                }
            }
            let more = if fields.len() > preview.len() { Preview::Cut } else { Preview::Whole };
            ShownValue::Aggregate { count: None, preview, more }
        }
        TypeShape::Array { element, bytes } => {
            let size = size_of(context.tables, element).filter(|size| *size > 0);
            let count = size.and_then(|size| bytes.checked_div(size)).unwrap_or(0);
            let mut preview = Vec::new();
            let previewed = match reading {
                Reading::Row => size,
                Reading::PreviewItem => None,
            };
            if let Some(size) = previewed {
                for index in 0..count.min(MAX_PREVIEW_ITEMS as u64) {
                    let value =
                        value_of(memory, context, &place.part(index.saturating_mul(size)), element, Reading::PreviewItem);
                    preview.push(PreviewItem { name: None, value });
                }
            }
            let more = if count > preview.len() as u64 { Preview::Cut } else { Preview::Whole };
            ShownValue::Aggregate { count: Some(count), preview, more }
        }
        TypeShape::Incomplete { name } => ShownValue::Problem(format!("{name} is not defined in these symbols")),
        TypeShape::Function { .. } => ShownValue::Problem("a function has no value".to_owned()),
        TypeShape::Unknown { index, leaf, bytes } => ShownValue::UnknownType { index: index.0, leaf, bytes },
        TypeShape::Modified { .. } => ShownValue::Problem(endless(ty)),
    }
}

/// A record member's value for a preview: a bitfield's bits.
fn field_value<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    record: &Place,
    field: &RecordField,
) -> ShownValue {
    let at = record.part(field.offset.0);
    let place = match field.bits {
        None => at,
        Some(bits) => bitfield_place(memory, context, &at, field.ty, bits),
    };
    value_of(memory, context, &place, field.ty, Reading::PreviewItem)
}

/// A bitfield's value as the bytes of its storage type: the storage unit at
/// `unit` read, its bits shifted down, and sign-extended for a signed type.
pub(crate) fn bitfield_place<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    unit: &Place,
    ty: TypeRef,
    bits: BitRange,
) -> Place {
    let Some(size) = size_of(context.tables, ty).filter(|size| *size > 0 && *size <= 8) else {
        return Place::Missing(ShownValue::Problem(format!("bitfield storage {} is not an integer", type_text(context.tables, ty))));
    };
    let bytes = match read_place(memory, context.root, unit, size) {
        Ok(bytes) => bytes,
        Err(missing) => return Place::Missing(missing),
    };
    let mut raw = bits.extract(unsigned(&bytes));
    let sign_bit = bits.length.checked_sub(1).and_then(|top| raw.checked_shr(top)).is_some_and(|top| top & 1 == 1);
    if is_signed(context.tables, ty) && sign_bit {
        raw |= u64::MAX.checked_shl(bits.length).unwrap_or(0);
    }
    let width = usize::try_from(size).unwrap_or(8);
    Place::Bytes(raw.to_le_bytes().get(..width).map(<[u8]>::to_vec).unwrap_or_default())
}

/// Reads `len` bytes (at most `MAX_SCALAR_BYTES`) at `place`.
pub(crate) fn read_place<G: GuestMemory>(memory: &mut G, root: Root, place: &Place, len: u64) -> Result<Vec<u8>, ShownValue> {
    let count = usize::try_from(len.min(MAX_SCALAR_BYTES)).map_err(|_| ShownValue::NotAvailable)?;
    match place {
        Place::Memory(address) => {
            let mut bytes = vec![0; count];
            match memory.read_linear(root, *address, &mut bytes) {
                GuestRead::Read => Ok(bytes),
                GuestRead::Unreadable => Err(ShownValue::Unreadable { at: *address }),
            }
        }
        Place::Bytes(bytes) => bytes.get(..count).map(<[u8]>::to_vec).ok_or(ShownValue::NotAvailable),
        Place::Missing(value) => Err(value.clone()),
        Place::AddressOf(address) => address.to_le_bytes().get(..count).map(<[u8]>::to_vec).ok_or(ShownValue::NotAvailable),
    }
}

/// Little-endian bytes (the first 8) as a number.
pub(crate) fn unsigned(bytes: &[u8]) -> u64 {
    bytes.iter().take(8).rev().fold(0, |value, byte| (value << 8) | u64::from(*byte))
}

/// `raw`, `size` bytes wide, as a signed number.
fn sign_extend(raw: u64, size: u8) -> i128 {
    let unused = 64u32.saturating_sub(u32::from(size).saturating_mul(8));
    raw.checked_shl(unused).map_or(0, |high| i128::from((high as i64) >> unused))
}

/// A primitive type's value.
fn base_value<G: GuestMemory>(memory: &mut G, root: Root, place: &Place, base: BaseType) -> ShownValue {
    let bytes = match read_place(memory, root, place, u64::from(base.size)) {
        Ok(bytes) => bytes,
        Err(missing) => return missing,
    };
    let raw = unsigned(&bytes);
    let size = base.size;
    match base.kind {
        BaseKind::Void => ShownValue::Problem("void has no value".to_owned()),
        BaseKind::Bool => ShownValue::Boolean { raw: bytes.first().copied().unwrap_or(0) },
        BaseKind::SignedChar
        | BaseKind::UnsignedChar
        | BaseKind::Char
        | BaseKind::WideChar
        | BaseKind::Char8
        | BaseKind::Char16
        | BaseKind::Char32 => ShownValue::Character { code: u32::try_from(raw).unwrap_or(u32::MAX), size },
        BaseKind::Signed | BaseKind::Unsigned if size > 8 => {
            ShownValue::Problem(format!("{}-bit integers are not shown", u32::from(size) * 8))
        }
        BaseKind::Signed => ShownValue::Integer { raw, size, signedness: Signedness::Signed },
        BaseKind::Unsigned => ShownValue::Integer { raw, size, signedness: Signedness::Unsigned },
        BaseKind::Float => match size {
            4 => ShownValue::Real32(f32::from_bits(raw as u32)),
            8 => ShownValue::Real64(f64::from_bits(raw)),
            other => ShownValue::Problem(format!("{other}-byte floating point is not shown")),
        },
        BaseKind::Hresult => ShownValue::Hresult(raw as u32),
    }
}

/// An enum's value and its names: the enumerator it equals; else the
/// single-bit enumerators that together make it, ascending; else none.
fn enum_value<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    place: &Place,
    underlying: TypeRef,
    values: &[Enumerator],
) -> ShownValue {
    let base = match unmodified(context.tables, underlying) {
        Ok(TypeShape::Base(base)) => base,
        Ok(_) => return ShownValue::Problem(format!("enum over {}", type_text(context.tables, underlying))),
        Err(problem) => return problem,
    };
    let bytes = match read_place(memory, context.root, place, u64::from(base.size)) {
        Ok(bytes) => bytes,
        Err(missing) => return missing,
    };
    let raw = unsigned(&bytes);
    let value = if signed_kind(base.kind) { sign_extend(raw, base.size) } else { i128::from(raw) };
    ShownValue::Enum { raw: value, names: enum_names(value, values) }
}

fn enum_names(value: i128, values: &[Enumerator]) -> Vec<String> {
    if let Some(exact) = values.iter().find(|enumerator| enumerator.value == value) {
        return vec![exact.name.clone()];
    }
    if value <= 0 {
        return Vec::new();
    }
    let mut flags: Vec<&Enumerator> = values
        .iter()
        .filter(|enumerator| {
            enumerator.value > 0 && enumerator.value.count_ones() == 1 && value & enumerator.value != 0
        })
        .collect();
    flags.sort_by_key(|enumerator| enumerator.value);
    flags.dedup_by_key(|enumerator| enumerator.value);
    let covered = flags.iter().fold(0i128, |bits, enumerator| bits | enumerator.value);
    if covered == value { flags.iter().map(|enumerator| enumerator.name.clone()).collect() } else { Vec::new() }
}

fn signed_kind(kind: BaseKind) -> bool {
    match kind {
        BaseKind::Signed | BaseKind::SignedChar | BaseKind::Char => true,
        BaseKind::Void
        | BaseKind::Bool
        | BaseKind::UnsignedChar
        | BaseKind::WideChar
        | BaseKind::Char8
        | BaseKind::Char16
        | BaseKind::Char32
        | BaseKind::Unsigned
        | BaseKind::Float
        | BaseKind::Hresult => false,
    }
}

fn is_signed(tables: &SymbolTables, ty: TypeRef) -> bool {
    match unmodified(tables, ty) {
        Ok(TypeShape::Base(base)) => signed_kind(base.kind),
        Ok(TypeShape::Enum { underlying, .. }) => is_signed(tables, underlying),
        Ok(_) | Err(_) => false,
    }
}

/// The string a pointer to `to` at `address` points at, when `to` is a
/// character type text is written in (`char`, `wchar_t`, `char16_t`).
fn pointed_string<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    to: TypeRef,
    address: u64,
) -> Option<PointedString> {
    let unit = match unmodified(context.tables, to).ok()? {
        TypeShape::Base(BaseType { kind: BaseKind::Char, .. }) => 1,
        TypeShape::Base(BaseType { kind: BaseKind::WideChar | BaseKind::Char16, .. }) => 2,
        TypeShape::Base(_)
        | TypeShape::Pointer { .. }
        | TypeShape::Modified { .. }
        | TypeShape::Array { .. }
        | TypeShape::Record { .. }
        | TypeShape::Incomplete { .. }
        | TypeShape::Enum { .. }
        | TypeShape::Function { .. }
        | TypeShape::Unknown { .. } => return None,
    };
    if address == 0 {
        return None;
    }
    Some(read_string(memory, context.root, address, unit))
}

/// Up to `STRING_CHARACTERS` characters of `unit` bytes from `address`, a
/// page at a time, to the terminator or to memory that does not read.
fn read_string<G: GuestMemory>(memory: &mut G, root: Root, address: u64, unit: u64) -> PointedString {
    let mut codes: Vec<u64> = Vec::new();
    let mut at = address;
    let end = loop {
        let left = (STRING_CHARACTERS.saturating_sub(codes.len()) as u64).saturating_mul(unit);
        if left == 0 {
            break StringEnd::Cut;
        }
        let in_page = PAGE_BYTES.wrapping_sub(at & PAGE_OFFSET_MASK);
        let whole_units = left.min(in_page).checked_div(unit).unwrap_or(0).saturating_mul(unit);
        // A character that straddles the page's end is read on its own.
        let take = whole_units.max(unit);
        let Ok(count) = usize::try_from(take) else {
            break StringEnd::Unreadable;
        };
        let mut chunk = vec![0u8; count];
        match memory.read_linear(root, at, &mut chunk) {
            GuestRead::Read => {}
            GuestRead::Unreadable => break StringEnd::Unreadable,
        }
        let unit_len = usize::try_from(unit).unwrap_or(1);
        if let Some(position) = chunk.chunks_exact(unit_len).position(|code| unsigned(code) == 0) {
            codes.extend(chunk.chunks_exact(unit_len).take(position).map(unsigned));
            break StringEnd::Terminated;
        }
        codes.extend(chunk.chunks_exact(unit_len).map(unsigned));
        at = at.wrapping_add(take);
    };
    let text = match unit {
        1 => String::from_utf8_lossy(&codes.iter().map(|code| *code as u8).collect::<Vec<u8>>()).into_owned(),
        _ => char::decode_utf16(codes.iter().map(|code| *code as u16))
            .map(|decoded| decoded.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
    };
    PointedString { text, end }
}

/// `ty` past its `const` and `volatile`.
pub(crate) fn unmodified(tables: &SymbolTables, ty: TypeRef) -> Result<TypeShape, ShownValue> {
    let mut at = ty;
    for _ in 0..MAX_TYPE_HOPS {
        match tables.type_of(at) {
            Ok(TypeShape::Modified { of, .. }) => at = of,
            Ok(shape) => return Ok(shape),
            Err(error) => return Err(ShownValue::Problem(error.to_string())),
        }
    }
    Err(ShownValue::Problem(endless(ty)))
}

fn endless(ty: TypeRef) -> String {
    format!("type {:#x} is reached through a chain that does not end", ty.0)
}

/// A type's size in bytes, when it has one.
pub(crate) fn size_of(tables: &SymbolTables, ty: TypeRef) -> Option<u64> {
    let mut at = ty;
    for _ in 0..MAX_TYPE_HOPS {
        match tables.type_of(at).ok()? {
            TypeShape::Base(base) => return Some(u64::from(base.size)),
            TypeShape::Pointer { size, .. } => return Some(u64::from(size)),
            TypeShape::Array { bytes, .. } => return Some(bytes),
            TypeShape::Record { size, .. } => return Some(size),
            TypeShape::Modified { of, .. } => at = of,
            TypeShape::Enum { underlying, .. } => at = underlying,
            TypeShape::Incomplete { .. } | TypeShape::Function { .. } | TypeShape::Unknown { .. } => return None,
        }
    }
    None
}

/// An array's element count: none when its element has no size.
fn element_count(tables: &SymbolTables, element: TypeRef, bytes: u64) -> u64 {
    size_of(tables, element).and_then(|size| bytes.checked_div(size)).unwrap_or(0)
}

/// `ty` as C writes it: "int", "const char *", "Foo[16]", "void (*)()",
/// and "<type 0x1234>" for one these symbols do not describe. Integer and
/// floating-point names follow the size, whatever CodeView spelling made them.
pub(crate) fn type_text(tables: &SymbolTables, ty: TypeRef) -> String {
    spell(tables, ty, String::new(), 0)
}

/// `ty` around `declarator`, the part of the declaration already spelled.
fn spell(tables: &SymbolTables, ty: TypeRef, declarator: String, hops: u32) -> String {
    if hops >= MAX_TYPE_HOPS {
        return join(&unknown_name(ty), &declarator);
    }
    let Ok(shape) = tables.type_of(ty) else {
        return join(&unknown_name(ty), &declarator);
    };
    match shape {
        TypeShape::Base(base) => join(&base_name(base), &declarator),
        TypeShape::Record { name, .. } | TypeShape::Incomplete { name } | TypeShape::Enum { name, .. } => {
            join(&name, &declarator)
        }
        TypeShape::Unknown { index, .. } => join(&unknown_name(index), &declarator),
        TypeShape::Pointer { to, kind, .. } => spell_pointer(tables, to, kind, "", declarator, hops),
        TypeShape::Modified { of, constant, volatile } => {
            let qualifiers = qualifiers(constant, volatile);
            match tables.type_of(of) {
                Ok(TypeShape::Pointer { to, kind, .. }) => spell_pointer(tables, to, kind, qualifiers, declarator, hops.saturating_add(1)),
                Ok(_) | Err(_) => {
                    let inner = spell(tables, of, declarator, hops.saturating_add(1));
                    if qualifiers.is_empty() { inner } else { format!("{qualifiers} {inner}") }
                }
            }
        }
        TypeShape::Array { element, bytes } => {
            let count = element_count(tables, element, bytes);
            spell(tables, element, format!("{declarator}[{count}]"), hops.saturating_add(1))
        }
        TypeShape::Function { returns, parameters } => {
            let arguments = if parameters == 0 { "()" } else { "(...)" };
            spell(tables, returns, format!("{declarator}{arguments}"), hops.saturating_add(1))
        }
    }
}

/// A pointer (`qualifiers` applying to the pointer itself) to `to`, around
/// `declarator`; a pointer to an array or function is parenthesized.
fn spell_pointer(
    tables: &SymbolTables,
    to: TypeRef,
    kind: PointerKind,
    qualifiers: &str,
    declarator: String,
    hops: u32,
) -> String {
    let mut inner = match kind {
        PointerKind::Pointer => "*",
        PointerKind::Reference => "&",
        PointerKind::RvalueReference => "&&",
    }
    .to_owned();
    inner.push_str(qualifiers);
    if !declarator.is_empty() {
        if !qualifiers.is_empty() {
            inner.push(' ');
        }
        inner.push_str(&declarator);
    }
    let wrapped = match tables.type_of(to) {
        Ok(TypeShape::Array { .. } | TypeShape::Function { .. }) => format!("({inner})"),
        Ok(_) | Err(_) => inner,
    };
    spell(tables, to, wrapped, hops.saturating_add(1))
}

fn join(name: &str, declarator: &str) -> String {
    if declarator.is_empty() {
        name.to_owned()
    } else if declarator.starts_with('[') {
        format!("{name}{declarator}")
    } else {
        format!("{name} {declarator}")
    }
}

fn unknown_name(ty: TypeRef) -> String {
    format!("<type {:#x}>", ty.0)
}

fn qualifiers(constant: Constness, volatile: Volatility) -> &'static str {
    match constant {
        Constness::Const => match volatile {
            Volatility::Volatile => "const volatile",
            Volatility::Plain => "const",
        },
        Constness::Mutable => match volatile {
            Volatility::Volatile => "volatile",
            Volatility::Plain => "",
        },
    }
}

fn base_name(base: BaseType) -> String {
    let bits = u32::from(base.size) * 8;
    let name = match base.kind {
        BaseKind::Void => "void",
        BaseKind::Bool => "bool",
        BaseKind::SignedChar => "signed char",
        BaseKind::UnsignedChar => "unsigned char",
        BaseKind::Char => "char",
        BaseKind::WideChar => "wchar_t",
        BaseKind::Char8 => "char8_t",
        BaseKind::Char16 => "char16_t",
        BaseKind::Char32 => "char32_t",
        BaseKind::Signed => match base.size {
            1 => "__int8",
            2 => "short",
            4 => "int",
            8 => "__int64",
            16 => "__int128",
            _ => return format!("int{bits}"),
        },
        BaseKind::Unsigned => match base.size {
            1 => "unsigned __int8",
            2 => "unsigned short",
            4 => "unsigned int",
            8 => "unsigned __int64",
            16 => "unsigned __int128",
            _ => return format!("uint{bits}"),
        },
        BaseKind::Float => match base.size {
            4 => "float",
            8 => "double",
            10 => "long double",
            _ => return format!("float{bits}"),
        },
        BaseKind::Hresult => "HRESULT",
    };
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::frames::InspectRequest;
    use crate::debugger::test_frame::{
        field, frame, guest, local, on_stack, tables, value, Setup, CR3, IN_BODY, RSP_AT, TEXT,
    };
    use crate::debugger::unwind::{RAX, RCX, RDX};
    use rusty_box_symbols_fixtures::pdb::{
        FixtureDefRange, FixtureEnum, FixtureEnumerator, FixtureGap, FixturePlace, FixtureRange, FixtureType, FixtureUdt,
        UdtKind,
    };

    const CV_RCX: u16 = 330;
    const CV_RDX: u16 = 331;
    const CV_EAX: u16 = 17;
    const CV_XMM0L: u16 = 194;

    fn bits(position: u8, length: u8) -> FixtureType {
        FixtureType::Bitfield { storage: Box::new(FixtureType::U32), position, length }
    }

    fn vad_flags() -> FixtureUdt {
        FixtureUdt {
            name: "_MMVAD_FLAGS".into(),
            kind: UdtKind::Struct,
            size: 4,
            fields: vec![field("Lock", 0, bits(0, 1)), field("VadType", 0, bits(4, 3)), field("Protection", 0, bits(7, 5)), field("PrivateMemory", 0, bits(20, 1))],
        }
    }

    fn path(name: &str, steps: Vec<PathStep>) -> ValuePath {
        ValuePath { root: PathRoot::Variable(name.into()), steps }
    }

    fn expanded(paths: Vec<ValuePath>) -> InspectRequest {
        InspectRequest { expanded: paths.into_iter().collect(), ..InspectRequest::default() }
    }

    #[test]
    fn a_stack_local_and_a_register_parameter_read_their_values() {
        let mut setup = Setup::new(tables(
            vec![
                on_stack("count", FixtureType::I32, 0x20),
                local("buffer", FixtureType::PointerTo(Box::new(FixtureType::Char)), FixturePlace::Register { register: CV_RCX }),
            ],
            Vec::new(),
        ));
        let mut registers = [Some(0); 16];
        registers[RCX] = Some(TEXT);
        setup.frame = frame(IN_BODY, Innermost::Yes, registers);
        let mut guest = guest();
        guest.write_u32(CR3, RSP_AT + 0x20, (-5i32) as u32);
        guest.write(CR3, TEXT, b"hello\0");
        let rows = setup.rows(&mut guest, &InspectRequest::default());
        assert_eq!(value(&rows, "count"), &ShownValue::Integer { raw: 0xFFFF_FFFB, size: 4, signedness: Signedness::Signed });
        assert_eq!(
            value(&rows, "buffer"),
            &ShownValue::Pointer { address: TEXT, string: Some(PointedString { text: "hello".into(), end: StringEnd::Terminated }) }
        );
    }

    #[test]
    fn a_register_subregister_and_an_xmm_lane_read_their_bits() {
        let mut setup = Setup::new(tables(
            vec![
                local("e", FixtureType::U32, FixturePlace::Register { register: CV_EAX }),
                local("d", FixtureType::F64, FixturePlace::Register { register: CV_XMM0L }),
            ],
            Vec::new(),
        ));
        let mut registers = [Some(0); 16];
        registers[RAX] = Some(0x1_2345_6789);
        setup.frame = frame(IN_BODY, Innermost::Yes, registers);
        let mut xmm = [[0u8; 16]; 16];
        xmm[0][..8].copy_from_slice(&1.5f64.to_le_bytes());
        setup.xmm = Some(xmm);
        let rows = setup.rows(&mut guest(), &InspectRequest::default());
        assert_eq!(value(&rows, "e"), &ShownValue::Integer { raw: 0x2345_6789, size: 4, signedness: Signedness::Unsigned });
        assert_eq!(value(&rows, "d"), &ShownValue::Real64(1.5));
    }

    #[test]
    fn a_ranged_local_outside_its_range_or_in_a_gap_is_not_available() {
        let retries = FixturePlace::Ranged {
            flags: 0,
            ranges: vec![FixtureDefRange::Register {
                register: CV_RDX,
                range: FixtureRange { offset: 0x18, len: 0x20, gaps: vec![FixtureGap { start: 4, len: 2 }] },
            }],
        };
        let mut setup = Setup::new(tables(vec![local("retries", FixtureType::U32, retries)], Vec::new()));
        let mut registers = [Some(0); 16];
        registers[RDX] = Some(7);
        let cases = [
            At { rva: IN_BODY, expected: ShownValue::Integer { raw: 7, size: 4, signedness: Signedness::Unsigned } },
            At { rva: 0x111C, expected: ShownValue::NotAvailable },
            At { rva: 0x1140, expected: ShownValue::NotAvailable },
        ];
        for case in cases {
            setup.frame = frame(case.rva, Innermost::Yes, registers);
            let rows = setup.rows(&mut guest(), &InspectRequest::default());
            assert_eq!(value(&rows, "retries"), &case.expected, "at {:#x}", case.rva);
        }
    }

    /// What a variable reads as with the frame's code at `rva`.
    struct At {
        rva: u64,
        expected: ShownValue,
    }

    #[test]
    fn a_stack_local_in_the_prolog_is_not_available() {
        let mut setup = Setup::new(tables(vec![on_stack("count", FixtureType::U32, 0x20)], Vec::new()));
        setup.frame = frame(0x110F, Innermost::Yes, [Some(0); 16]);
        assert_eq!(value(&setup.rows(&mut guest(), &InspectRequest::default()), "count"), &ShownValue::NotAvailable);
        setup.innermost = Innermost::Caller;
        setup.frame = frame(0x110F, Innermost::Caller, [Some(0); 16]);
        assert_eq!(
            value(&setup.rows(&mut guest(), &InspectRequest::default()), "count"),
            &ShownValue::Integer { raw: 0, size: 4, signedness: Signedness::Unsigned }
        );
    }

    #[test]
    fn a_caller_frame_does_not_know_its_volatile_registers() {
        let mut setup = Setup::new(tables(
            vec![
                local("buffer", FixtureType::Pointer, FixturePlace::Register { register: CV_RCX }),
                local("d", FixtureType::F64, FixturePlace::Register { register: CV_XMM0L }),
            ],
            Vec::new(),
        ));
        let mut registers = [Some(0); 16];
        registers[RCX] = None;
        setup.innermost = Innermost::Caller;
        setup.frame = frame(IN_BODY, Innermost::Caller, registers);
        setup.xmm = Some([[0; 16]; 16]);
        let rows = setup.rows(&mut guest(), &InspectRequest::default());
        assert_eq!(value(&rows, "buffer"), &ShownValue::NotAvailable);
        assert_eq!(value(&rows, "d"), &ShownValue::NotAvailable);
    }

    #[test]
    fn a_structure_expands_into_its_members_and_a_bitfield_shifts() {
        let setup = Setup::new(tables(vec![on_stack("flags", FixtureType::Named("_MMVAD_FLAGS".into()), 0x30)], vec![vad_flags()]));
        let mut guest = guest();
        guest.write_u32(CR3, RSP_AT + 0x30, 0x0000_0F80);
        let closed = setup.rows(&mut guest, &InspectRequest::default());
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].expand, Expand::Closed);
        let ShownValue::Aggregate { count: None, preview, more: Preview::Whole } = &closed[0].value else {
            panic!("a structure's preview: {:?}", closed[0].value)
        };
        let names: Vec<Option<String>> = preview.iter().map(|item| item.name.clone()).collect();
        assert_eq!(names, ["Lock", "VadType", "Protection", "PrivateMemory"].map(|name| Some(name.to_owned())));
        assert_eq!(preview[2].value, ShownValue::Integer { raw: 0x1F, size: 4, signedness: Signedness::Unsigned });
        let open = setup.rows(&mut guest, &expanded(vec![path("flags", Vec::new())]));
        assert_eq!(open.len(), 5);
        assert_eq!(open[0].expand, Expand::Open);
        assert_eq!(open[3].name, "Protection");
        assert_eq!(open[3].kind, RowKind::Member);
        assert_eq!(open[3].depth, 1);
        assert_eq!(open[3].path, path("flags", vec![PathStep::Field("Protection".into())]));
        assert_eq!(open[3].value, ShownValue::Integer { raw: 0x1F, size: 4, signedness: Signedness::Unsigned });
    }

    #[test]
    fn a_preview_shows_six_items_and_nests_one_level() {
        let inner = FixtureUdt { name: "Inner".into(), kind: UdtKind::Struct, size: 8, fields: vec![field("x", 0, FixtureType::U64)] };
        let mut fields = vec![field("inner", 0, FixtureType::Named("Inner".into()))];
        for index in 0..8u64 {
            fields.push(field(&format!("m{index}"), 8 + 4 * index, FixtureType::I32));
        }
        let wide = FixtureUdt { name: "Wide".into(), kind: UdtKind::Struct, size: 40, fields };
        let setup = Setup::new(tables(
            vec![
                on_stack("wide", FixtureType::Named("Wide".into()), 0),
                on_stack("table", FixtureType::Array { element: Box::new(FixtureType::I32), count: 250 }, 0x40),
            ],
            vec![inner, wide],
        ));
        let rows = setup.rows(&mut guest(), &InspectRequest::default());
        let ShownValue::Aggregate { preview, more, .. } = value(&rows, "wide") else {
            panic!("a structure")
        };
        assert_eq!(preview.len(), MAX_PREVIEW_ITEMS);
        assert_eq!(*more, Preview::Cut);
        assert!(matches!(&preview[0].value, ShownValue::Aggregate { preview, .. } if preview.is_empty()), "one level only");
        let ShownValue::Aggregate { count, preview, more } = value(&rows, "table") else {
            panic!("an array")
        };
        assert_eq!(*count, Some(250));
        assert_eq!(preview.len(), MAX_PREVIEW_ITEMS);
        assert_eq!(preview[0].name, None);
        assert_eq!(*more, Preview::Cut);
    }

    #[test]
    fn an_array_shows_a_hundred_elements_a_page() {
        let setup = Setup::new(tables(
            vec![on_stack("table", FixtureType::Array { element: Box::new(FixtureType::I32), count: 250 }, 0)],
            Vec::new(),
        ));
        let table = path("table", Vec::new());
        let mut request = expanded(vec![table.clone()]);
        let cases = [
            Paged { pages: 0, shown: 100, more: Some(RowKind::MoreElements { shown: 100, of: 250 }) },
            Paged { pages: 1, shown: 200, more: Some(RowKind::MoreElements { shown: 200, of: 250 }) },
            Paged { pages: 2, shown: 250, more: None },
        ];
        for case in cases {
            request.pages.insert(table.clone(), case.pages);
            let rows = setup.rows(&mut guest(), &request);
            let elements = rows.iter().filter(|row| row.kind == RowKind::Element).count() as u64;
            assert_eq!(elements, case.shown, "{} pages", case.pages);
            let more = rows.iter().map(|row| row.kind).find(|kind| matches!(kind, RowKind::MoreElements { .. }));
            assert_eq!(more, case.more, "{} pages", case.pages);
        }
    }

    /// The elements an array lists with `pages` more pages asked for.
    struct Paged {
        pages: u32,
        shown: u64,
        more: Option<RowKind>,
    }

    fn enumerator(name: &str, value: i64) -> FixtureEnumerator {
        FixtureEnumerator { name: name.into(), value }
    }

    fn enumeration(name: &str, values: Vec<FixtureEnumerator>) -> FixtureType {
        FixtureType::Enum(FixtureEnum { name: name.into(), underlying: Box::new(FixtureType::I32), values })
    }

    #[test]
    fn an_enum_shows_its_name_or_its_flags() {
        let mode = enumeration(
            "Mode",
            vec![enumerator("Off", -1), enumerator("Read", 1), enumerator("Write", 2), enumerator("Both", 3)],
        );
        let access = enumeration("Access", vec![enumerator("A", 1), enumerator("B", 2), enumerator("C", 4)]);
        let setup = Setup::new(tables(
            vec![on_stack("mode", mode, 0), on_stack("five", access.clone(), 4), on_stack("eight", access, 8)],
            Vec::new(),
        ));
        let mut guest = guest();
        guest.write_u32(CR3, RSP_AT, 3);
        guest.write_u32(CR3, RSP_AT + 4, 5);
        guest.write_u32(CR3, RSP_AT + 8, 8);
        let rows = setup.rows(&mut guest, &InspectRequest::default());
        assert_eq!(value(&rows, "mode"), &ShownValue::Enum { raw: 3, names: vec!["Both".into()] });
        assert_eq!(value(&rows, "five"), &ShownValue::Enum { raw: 5, names: vec!["A".into(), "C".into()] });
        assert_eq!(value(&rows, "eight"), &ShownValue::Enum { raw: 8, names: Vec::new() });
    }

    #[test]
    fn a_string_stops_at_unreadable_memory_and_at_256_characters() {
        let pointer = FixtureType::PointerTo(Box::new(FixtureType::Char));
        let setup = Setup::new(tables(vec![on_stack("cut", pointer.clone(), 0), on_stack("long", pointer, 8)], Vec::new()));
        let mut guest = guest();
        let near_end = TEXT + 0x1000 - 3;
        guest.write(CR3, TEXT, &[0; 0x1000]);
        guest.write(CR3, near_end, b"abc");
        guest.write_u64(CR3, RSP_AT, near_end);
        let long = TEXT + 0x2000;
        guest.write(CR3, long, &[b'a'; 300]);
        guest.write_u64(CR3, RSP_AT + 8, long);
        let rows = setup.rows(&mut guest, &InspectRequest::default());
        assert_eq!(
            value(&rows, "cut"),
            &ShownValue::Pointer { address: near_end, string: Some(PointedString { text: "abc".into(), end: StringEnd::Unreadable }) }
        );
        let ShownValue::Pointer { string: Some(text), .. } = value(&rows, "long") else {
            panic!("a string")
        };
        assert_eq!(text.text.chars().count(), STRING_CHARACTERS);
        assert_eq!(text.end, StringEnd::Cut);
    }

    #[test]
    fn a_local_without_a_location_is_optimized_away() {
        let gone = FixturePlace::Ranged { flags: 0, ranges: Vec::new() };
        let setup = Setup::new(tables(vec![local("gone", FixtureType::U32, gone)], Vec::new()));
        assert_eq!(value(&setup.rows(&mut guest(), &InspectRequest::default()), "gone"), &ShownValue::OptimizedAway);
    }

    #[test]
    fn an_unknown_type_shows_its_leaf_and_bytes() {
        let setup = Setup::new(tables(vec![on_stack("odd", FixtureType::Raw { leaf: 0x1234, payload: vec![1, 2, 3] }, 0)], Vec::new()));
        let rows = setup.rows(&mut guest(), &InspectRequest::default());
        assert!(matches!(value(&rows, "odd"), ShownValue::UnknownType { leaf: 0x1234, .. }));
        assert!(rows[0].type_name.starts_with("<type 0x1"), "{}", rows[0].type_name);
    }

    #[test]
    fn rows_stop_at_the_ceiling_with_a_row_saying_so() {
        let setup = Setup::new(tables(
            vec![on_stack("huge", FixtureType::Array { element: Box::new(FixtureType::I32), count: 1_000_000 }, 0)],
            Vec::new(),
        ));
        let huge = path("huge", Vec::new());
        let mut request = expanded(vec![huge.clone()]);
        request.pages.insert(huge, 100);
        let rows = setup.rows(&mut guest(), &request);
        assert_eq!(rows.len(), MAX_VARIABLE_ROWS);
        assert_eq!(rows.last().map(|row| row.kind), Some(RowKind::Ceiling));
    }

    #[test]
    fn type_names_read_as_c_writes_them() {
        let setup = Setup::new(tables(
            vec![
                on_stack("name", FixtureType::PointerTo(Box::new(FixtureType::Const(Box::new(FixtureType::Char)))), 0),
                on_stack("table", FixtureType::Array { element: Box::new(FixtureType::I32), count: 16 }, 8),
                on_stack("callback", FixtureType::PointerTo(Box::new(FixtureType::Function)), 0x48),
            ],
            Vec::new(),
        ));
        let rows = setup.rows(&mut guest(), &InspectRequest::default());
        let type_of = |name: &str| rows.iter().find(|row| row.name == name).map(|row| row.type_name.clone());
        assert_eq!(type_of("name").as_deref(), Some("const char *"));
        assert_eq!(type_of("table").as_deref(), Some("int[16]"));
        assert_eq!(type_of("callback").as_deref(), Some("void (*)()"));
    }
}
