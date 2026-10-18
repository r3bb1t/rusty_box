//! One function's variables, read from its scope in a module stream (spec
//! §8.1): from its `S_GPROC32` to the `S_END` that closes it, the blocks
//! holding the address, an `S_LOCAL`'s def-ranges, and `S_FRAMEPROC`.
//!
//! Tier A: the walk reads records of a stream already in memory, moves
//! forward on every step, and stops at `MAX_SCOPE_RECORDS`.

use super::symbols::{
    decode_block, decode_bp_relative, decode_data, decode_def_range, decode_frame_procedure, decode_local,
    decode_procedure, decode_record_header, decode_register_relative, decode_register_variable, DefPlace, S_BLOCK32,
    S_BPREL32, S_DEFRANGE_FRAMEPOINTER_REL, S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE, S_DEFRANGE_REGISTER,
    S_DEFRANGE_REGISTER_REL, S_DEFRANGE_SUBFIELD_REGISTER, S_END, S_FRAMEPROC, S_GDATA32, S_GPROC32, S_GPROC32_ID,
    S_INLINESITE, S_INLINESITE_END, S_LDATA32, S_LOCAL, S_LPROC32, S_LPROC32_ID, S_PROC_ID_END, S_REGISTER, S_REGREL32,
};
use super::tables::Addresses;
use crate::read::Bytes;
use crate::tables::{
    BaseRegister, FrameBase, FrameScope, FrameVariable, LiveRange, LocalsError, ProcedureSpan, RangeGap, RangedPlace,
    VariableKind, VariableLocation,
};
use crate::units::{CvRegister, Rva, TypeRef};

/// Records past this in one function are not a real PDB's.
const MAX_SCOPE_RECORDS: u32 = 1 << 20;
/// `S_LOCAL` flags (`CV_LVARFLAGS`): a parameter, and optimized out.
const PARAMETER_FLAG: u16 = 0x01;
const OPTIMIZED_OUT_FLAG: u16 = 0x100;
/// `CV_REG_EBP`, the register `S_BPREL32` is relative to.
const CV_REG_EBP: u16 = 22;
/// Other records that open a scope an `S_END` closes: a thunk, a `with`
/// block, separated code. Their variables are not this function's own.
const S_THUNK32: u16 = 0x1102;
const S_WITH32: u16 = 0x1104;
const S_SEPCODE: u16 = 0x1132;

/// A scope the walk is inside: one whose variables count (the function, a
/// block holding the address), or one whose do not (a block elsewhere, an
/// inline site, a nested scope of another kind).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Live,
    Hidden,
}

/// A variable found, the depth of the scope holding it, and its `S_LOCAL`
/// flags when an `S_LOCAL` named it.
struct Found {
    depth: usize,
    variable: FrameVariable,
    local_flags: Option<u16>,
}

fn text(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

fn base_register(encoded: u32) -> BaseRegister {
    match encoded & 3 {
        1 => BaseRegister::StackPointer,
        2 => BaseRegister::FramePointer,
        3 => BaseRegister::BasePointer,
        _ => BaseRegister::None,
    }
}

/// A def-range's live range, its section offsets made RVAs.
fn live_range(place: DefPlace, span_rva: u64, len: u16, gaps: Vec<RangeGap>) -> Option<LiveRange> {
    let place = match place {
        DefPlace::Register { register } => RangedPlace::Register { register: CvRegister(register) },
        DefPlace::FramePointerRel { offset } => RangedPlace::FrameRelative { offset: i64::from(offset) },
        DefPlace::Subfield { register, offset_in_parent } => {
            RangedPlace::Subfield { register: CvRegister(register), offset_in_parent }
        }
        DefPlace::RegisterRel { register, offset } => {
            RangedPlace::RegisterRelative { register: CvRegister(register), offset: i64::from(offset) }
        }
        DefPlace::FullScope { .. } => return None,
    };
    Some(LiveRange { start: Rva(span_rva), end: Rva(span_rva.saturating_add(u64::from(len))), gaps, place })
}

/// Applies the def-range record `kind` to the `S_LOCAL` variable `variable`.
fn apply_def_range(variable: &mut FrameVariable, kind: u16, data: Bytes<'_>, addresses: &Addresses) -> Result<(), LocalsError> {
    let def = decode_def_range(kind, data).map_err(|_| LocalsError::Unreadable { what: "def-range" })?;
    if let DefPlace::FullScope { offset } = def.place {
        variable.location = VariableLocation::FrameRelative { offset: i64::from(offset) };
        return Ok(());
    }
    let Some(span) = def.range else {
        return Ok(());
    };
    // A range whose section does not map lies in no code of the image.
    let Some(start) = addresses.rva(span.segment, span.offset) else {
        return Ok(());
    };
    let mut gaps = Vec::new();
    for index in 0..def.gap_count() {
        let Some(gap) = def.gap(index) else {
            break;
        };
        let gap_start = start.saturating_add(u64::from(gap.start));
        let gap = RangeGap { start: Rva(gap_start), end: Rva(gap_start.saturating_add(u64::from(gap.len))) };
        gaps.try_reserve(1).map_err(|_| LocalsError::Unreadable { what: "def-range gaps (no memory)" })?;
        gaps.push(gap);
    }
    let Some(range) = live_range(def.place, start, span.len, gaps) else {
        return Ok(());
    };
    // A full-scope place already given wins over ranges.
    if let VariableLocation::Ranged(ranges) = &mut variable.location {
        ranges.try_reserve(1).map_err(|_| LocalsError::Unreadable { what: "def-ranges (no memory)" })?;
        ranges.push(range);
    }
    Ok(())
}

/// The variables in scope at `rva` of the function whose `S_GPROC32` (or
/// `S_LPROC32`, or an `_ID` form) starts at `at` of `records`. The first
/// `parameters` non-static variables of its own scope are its parameters,
/// as its type counts them; an `S_LOCAL` flagged a parameter is one too.
pub(crate) fn read_scope(
    records: Bytes<'_>,
    at: usize,
    rva: Rva,
    addresses: &Addresses,
    parameters: u16,
) -> Result<FrameScope, LocalsError> {
    let unreadable = |what: &'static str| LocalsError::Unreadable { what };
    let head = decode_record_header(records, at).map_err(|_| unreadable("procedure record"))?;
    if !matches!(head.kind, S_GPROC32 | S_LPROC32 | S_GPROC32_ID | S_LPROC32_ID) {
        return Err(unreadable("procedure record"));
    }
    let procedure = decode_procedure(head.data).map_err(|_| unreadable("procedure record"))?;
    let start = addresses.rva(procedure.segment, procedure.offset).ok_or(unreadable("procedure address"))?;
    let function = ProcedureSpan {
        name: text(procedure.name),
        start: Rva(start),
        end: Rva(start.saturating_add(u64::from(procedure.len))),
    };
    let body_start = Rva(start.saturating_add(u64::from(procedure.debug_start)));
    let body_end = Rva(start.saturating_add(u64::from(procedure.debug_end)));
    let mut base = FrameBase { locals: BaseRegister::None, params: BaseRegister::None };
    let mut levels: Vec<Level> = vec![Level::Live];
    let mut found: Vec<Found> = Vec::new();
    // The `S_LOCAL` the def-ranges that follow it belong to.
    let mut open_local: Option<usize> = None;
    let mut position = head.next;
    let mut count = 0u32;
    while !levels.is_empty() {
        count = count.saturating_add(1);
        if count > MAX_SCOPE_RECORDS {
            return Err(unreadable("scope (too many records)"));
        }
        let record = decode_record_header(records, position).map_err(|_| unreadable("scope record"))?;
        if record.next <= position {
            return Err(unreadable("scope record"));
        }
        position = record.next;
        let live = levels.iter().all(|level| *level == Level::Live);
        let depth = levels.len();
        let def_range = matches!(
            record.kind,
            S_DEFRANGE_REGISTER
                | S_DEFRANGE_FRAMEPOINTER_REL
                | S_DEFRANGE_SUBFIELD_REGISTER
                | S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE
                | S_DEFRANGE_REGISTER_REL
        );
        if !def_range {
            open_local = None;
        }
        let variable = match record.kind {
            S_END | S_PROC_ID_END | S_INLINESITE_END => {
                levels.pop();
                None
            }
            S_BLOCK32 => {
                let block = decode_block(record.data).map_err(|_| unreadable("block record"))?;
                let holds = addresses
                    .rva(block.segment, block.offset)
                    .is_some_and(|block_start| block_start <= rva.0 && rva.0 < block_start.saturating_add(u64::from(block.len)));
                levels.push(if holds { Level::Live } else { Level::Hidden });
                None
            }
            S_INLINESITE | S_GPROC32 | S_LPROC32 | S_GPROC32_ID | S_LPROC32_ID | S_THUNK32 | S_WITH32 | S_SEPCODE => {
                levels.push(Level::Hidden);
                None
            }
            S_FRAMEPROC if depth == 1 => {
                let flags = decode_frame_procedure(record.data).map_err(|_| unreadable("frame procedure"))?;
                base = FrameBase { locals: base_register(flags.wrapping_shr(14)), params: base_register(flags.wrapping_shr(16)) };
                None
            }
            S_REGREL32 if live => {
                let named = decode_register_relative(record.data).map_err(|_| unreadable("register-relative variable"))?;
                let location =
                    VariableLocation::RegisterRelative { register: CvRegister(named.register), offset: i64::from(named.offset) };
                Some(Found {
                    depth,
                    variable: FrameVariable { name: text(named.name), ty: TypeRef(named.ty), kind: VariableKind::Local, location },
                    local_flags: None,
                })
            }
            S_BPREL32 if live => {
                let named = decode_bp_relative(record.data).map_err(|_| unreadable("frame-pointer variable"))?;
                let location = VariableLocation::RegisterRelative { register: CvRegister(CV_REG_EBP), offset: i64::from(named.offset) };
                Some(Found {
                    depth,
                    variable: FrameVariable { name: text(named.name), ty: TypeRef(named.ty), kind: VariableKind::Local, location },
                    local_flags: None,
                })
            }
            S_REGISTER if live => {
                let named = decode_register_variable(record.data).map_err(|_| unreadable("register variable"))?;
                let location = VariableLocation::Register { register: CvRegister(named.register) };
                Some(Found {
                    depth,
                    variable: FrameVariable { name: text(named.name), ty: TypeRef(named.ty), kind: VariableKind::Local, location },
                    local_flags: None,
                })
            }
            S_LDATA32 | S_GDATA32 if live => {
                let data = decode_data(record.data).map_err(|_| unreadable("static variable"))?;
                addresses.rva(data.segment, data.offset).map(|at| Found {
                    depth,
                    variable: FrameVariable {
                        name: text(data.name),
                        ty: TypeRef(data.ty),
                        kind: VariableKind::Static,
                        location: VariableLocation::Static { rva: Rva(at) },
                    },
                    local_flags: None,
                })
            }
            S_LOCAL if live => {
                let local = decode_local(record.data).map_err(|_| unreadable("local variable"))?;
                let kind = if local.flags & PARAMETER_FLAG != 0 { VariableKind::Parameter } else { VariableKind::Local };
                open_local = Some(found.len());
                Some(Found {
                    depth,
                    variable: FrameVariable {
                        name: text(local.name),
                        ty: TypeRef(local.ty),
                        kind,
                        location: VariableLocation::Ranged(Vec::new()),
                    },
                    local_flags: Some(local.flags),
                })
            }
            kind if def_range && live => {
                if let Some(variable) = open_local.and_then(|index| found.get_mut(index)) {
                    apply_def_range(&mut variable.variable, kind, record.data, addresses)?;
                }
                None
            }
            _ => None,
        };
        if let Some(variable) = variable {
            found.try_reserve(1).map_err(|_| unreadable("variables (no memory)"))?;
            found.push(variable);
        }
    }
    Ok(FrameScope { function, body_start, body_end, base, variables: settle(found, parameters) })
}

/// The variables as an answer lists them: an `S_LOCAL` with no place, or
/// flagged optimized out, is optimized out; the first `parameters`
/// non-static variables of the function's own scope are parameters; an
/// outer variable an inner one of its name hides is left out.
fn settle(found: Vec<Found>, parameters: u16) -> Vec<FrameVariable> {
    let mut counted = 0u16;
    let mut settled: Vec<Found> = Vec::new();
    for mut item in found {
        if let Some(flags) = item.local_flags {
            let nowhere = matches!(&item.variable.location, VariableLocation::Ranged(ranges) if ranges.is_empty());
            if flags & OPTIMIZED_OUT_FLAG != 0 || nowhere {
                item.variable.location = VariableLocation::OptimizedOut;
            }
        }
        if item.depth == 1 && item.variable.kind != VariableKind::Static {
            if counted < parameters {
                item.variable.kind = VariableKind::Parameter;
            }
            counted = counted.saturating_add(1);
        }
        settled.push(item);
    }
    let mut variables = Vec::new();
    for outer in &settled {
        let hidden = settled.iter().any(|inner| inner.depth > outer.depth && inner.variable.name == outer.variable.name);
        if !hidden {
            variables.push(outer.variable.clone());
        }
    }
    variables
}

#[cfg(test)]
mod tests {
    use crate::tables::{
        BaseRegister, FrameBase, FrameVariable, LiveRange, LocalsError, RangeGap, RangedPlace, SymbolTables, VariableKind,
        VariableLocation,
    };
    use crate::units::{CvRegister, Rva, TypeRef};
    use rusty_box_symbols_fixtures::dwarf::{build_dwarf, DwarfFormat, DwarfSpec};
    use rusty_box_symbols_fixtures::elf::{build_elf, ElfSpec, TEXT_RVA};
    use rusty_box_symbols_fixtures::pdb::{
        build_pdb, FixtureBlock, FixtureDefRange, FixtureFrameProc, FixtureGap, FixtureLine, FixtureLocal, FixturePlace,
        FixtureProcedure, FixtureRange, FixtureScope, FixtureType, PdbFixture,
    };

    const RSP: u16 = 335;
    const RCX: u16 = 330;
    const RDX: u16 = 331;
    /// `T_ULONG` and `T_64PVOID`, as the fixture writes `U32` and `Pointer`.
    const U32: TypeRef = TypeRef(0x0022);
    const VOID_POINTER: TypeRef = TypeRef(0x0603);

    fn local(name: &str, ty: FixtureType, place: FixturePlace) -> FixtureLocal {
        FixtureLocal { name: name.into(), ty, place }
    }

    fn range(offset: u32, len: u16, gaps: Vec<FixtureGap>) -> FixtureRange {
        FixtureRange { offset, len, gaps }
    }

    fn scope(locals: Vec<FixtureLocal>) -> FixtureScope {
        FixtureScope {
            procedure: "ReadBlock".into(),
            debug_start: 0x10,
            debug_end: 0x50,
            parameters: None,
            frame: Some(FixtureFrameProc { locals: 1, params: 1 }),
            locals,
            inline_locals: Vec::new(),
            blocks: Vec::new(),
        }
    }

    /// `ReadBlock` at `.text` + 0x100 (RVA 0x1100), 0x60 bytes, with `scope`.
    fn tables(scope: FixtureScope) -> SymbolTables {
        let mut fixture = PdbFixture::new([9; 16], 1);
        fixture.procedures.push(FixtureProcedure {
            name: "ReadBlock".into(),
            section: 1,
            offset: 0x100,
            len: 0x60,
            file: r"C:\src\read.c".into(),
            checksum: None,
            lines: vec![FixtureLine { offset: 0, line: 10 }],
        });
        fixture.scopes.push(scope);
        SymbolTables::parse(&build_pdb(&fixture)).expect("the fixture parses")
    }

    fn variable(name: &str, ty: TypeRef, kind: VariableKind, location: VariableLocation) -> FrameVariable {
        FrameVariable { name: name.into(), ty, kind, location }
    }

    fn names(tables: &SymbolTables, rva: u64) -> Vec<String> {
        let scope = tables.frame_locals(Rva(rva)).expect("reads").expect("a function");
        scope.variables.into_iter().map(|variable| variable.name).collect()
    }

    #[test]
    fn the_variables_of_a_function_read_with_their_places() {
        let retries = vec![FixtureDefRange::Register { register: RDX, range: range(0x18, 0x20, vec![FixtureGap { start: 4, len: 2 }]) }];
        let tables = tables(scope(vec![
            local("count", FixtureType::U32, FixturePlace::RegRel { register: RSP, offset: 0x20 }),
            local("buffer", FixtureType::Pointer, FixturePlace::Register { register: RCX }),
            local("limit", FixtureType::U32, FixturePlace::Static { section: 2, offset: 8 }),
            local("retries", FixtureType::U32, FixturePlace::Ranged { flags: 0, ranges: retries }),
            local("flags", FixtureType::U32, FixturePlace::Ranged { flags: 0x01, ranges: vec![FixtureDefRange::FullScope { offset: 0x30 }] }),
        ]));
        let scope = tables.frame_locals(Rva(0x1120)).expect("reads").expect("a function");
        assert_eq!(scope.function.name, "ReadBlock");
        assert_eq!(scope.body_start, Rva(0x1110));
        assert_eq!(scope.body_end, Rva(0x1150));
        assert_eq!(scope.base, FrameBase { locals: BaseRegister::StackPointer, params: BaseRegister::StackPointer });
        assert_eq!(
            scope.variables,
            [
                variable("count", U32, VariableKind::Local, VariableLocation::RegisterRelative { register: CvRegister(RSP), offset: 0x20 }),
                variable("buffer", VOID_POINTER, VariableKind::Local, VariableLocation::Register { register: CvRegister(RCX) }),
                variable("limit", U32, VariableKind::Static, VariableLocation::Static { rva: Rva(0x3008) }),
                variable(
                    "retries",
                    U32,
                    VariableKind::Local,
                    VariableLocation::Ranged(vec![LiveRange {
                        start: Rva(0x1118),
                        end: Rva(0x1138),
                        gaps: vec![RangeGap { start: Rva(0x111C), end: Rva(0x111E) }],
                        place: RangedPlace::Register { register: CvRegister(RDX) },
                    }]),
                ),
                variable("flags", U32, VariableKind::Parameter, VariableLocation::FrameRelative { offset: 0x30 }),
            ]
        );
    }

    #[test]
    fn every_def_range_reads_its_place() {
        let one = |def: FixtureDefRange| local("v", FixtureType::U32, FixturePlace::Ranged { flags: 0, ranges: vec![def] });
        let span = || range(0, 0x60, Vec::new());
        let read = |def: FixtureDefRange| -> VariableLocation {
            let tables = tables(scope(vec![one(def)]));
            tables.frame_locals(Rva(0x1120)).expect("reads").expect("a function").variables.remove(0).location
        };
        let live = |place: RangedPlace| VariableLocation::Ranged(vec![LiveRange { start: Rva(0x1100), end: Rva(0x1160), gaps: Vec::new(), place }]);
        assert_eq!(read(FixtureDefRange::FramePointerRel { offset: -8, range: span() }), live(RangedPlace::FrameRelative { offset: -8 }));
        assert_eq!(
            read(FixtureDefRange::Subfield { register: RCX, offset_in_parent: 4, range: span() }),
            live(RangedPlace::Subfield { register: CvRegister(RCX), offset_in_parent: 4 })
        );
        assert_eq!(
            read(FixtureDefRange::RegisterRel { register: RSP, offset: 0x28, range: span() }),
            live(RangedPlace::RegisterRelative { register: CvRegister(RSP), offset: 0x28 })
        );
    }

    #[test]
    fn a_block_adds_its_variables_only_inside_it() {
        let mut with_block = scope(vec![local("n", FixtureType::U32, FixturePlace::RegRel { register: RSP, offset: 0 })]);
        with_block.blocks.push(FixtureBlock {
            offset: 0x20,
            len: 0x10,
            locals: vec![local("i", FixtureType::U32, FixturePlace::RegRel { register: RSP, offset: 8 })],
            blocks: Vec::new(),
        });
        let tables = tables(with_block);
        assert_eq!(names(&tables, 0x1124), ["n", "i"]);
        assert_eq!(names(&tables, 0x1140), ["n"]);
    }

    #[test]
    fn an_inner_variable_hides_an_outer_one_of_its_name() {
        let mut shadowed = scope(vec![local("n", FixtureType::U32, FixturePlace::RegRel { register: RSP, offset: 0 })]);
        shadowed.blocks.push(FixtureBlock {
            offset: 0x20,
            len: 0x10,
            locals: vec![local("n", FixtureType::U32, FixturePlace::Register { register: RDX })],
            blocks: Vec::new(),
        });
        let tables = tables(shadowed);
        let scope = tables.frame_locals(Rva(0x1124)).expect("reads").expect("a function");
        assert_eq!(
            scope.variables,
            [variable("n", U32, VariableKind::Local, VariableLocation::Register { register: CvRegister(RDX) })]
        );
    }

    #[test]
    fn a_local_without_a_location_is_optimized_away() {
        let tables = tables(scope(vec![
            local("gone", FixtureType::U32, FixturePlace::Ranged { flags: 0, ranges: Vec::new() }),
            local(
                "dropped",
                FixtureType::U32,
                FixturePlace::Ranged { flags: 0x100, ranges: vec![FixtureDefRange::Register { register: RDX, range: range(0, 0x60, Vec::new()) }] },
            ),
        ]));
        let scope = tables.frame_locals(Rva(0x1120)).expect("reads").expect("a function");
        assert!(scope.variables.iter().all(|variable| variable.location == VariableLocation::OptimizedOut));
        assert_eq!(scope.variables.len(), 2);
    }

    #[test]
    fn an_inline_site_adds_no_variables() {
        let mut inlined = scope(vec![local("n", FixtureType::U32, FixturePlace::RegRel { register: RSP, offset: 0 })]);
        inlined.inline_locals = vec![local("inner", FixtureType::U32, FixturePlace::Register { register: RDX })];
        assert_eq!(names(&tables(inlined), 0x1120), ["n"]);
    }

    #[test]
    fn parameters_come_first_as_the_procedure_type_counts_them() {
        let mut counted = scope(vec![
            local("a", FixtureType::U32, FixturePlace::RegRel { register: RSP, offset: 8 }),
            local("b", FixtureType::U32, FixturePlace::RegRel { register: RSP, offset: 16 }),
        ]);
        counted.parameters = Some(1);
        let tables = tables(counted);
        let kinds: Vec<VariableKind> =
            tables.frame_locals(Rva(0x1120)).expect("reads").expect("a function").variables.into_iter().map(|variable| variable.kind).collect();
        assert_eq!(kinds, [VariableKind::Parameter, VariableKind::Local]);
    }

    #[test]
    fn an_address_in_no_function_has_no_scope() {
        let tables = tables(scope(Vec::new()));
        assert_eq!(tables.frame_locals(Rva(0x1FF0)), Ok(None));
    }

    #[test]
    fn dwarf_tables_answer_that_their_variables_are_not_read() {
        let elf = build_elf(ElfSpec::pie(), &build_dwarf(DwarfSpec { version: 5, format: DwarfFormat::Dwarf32, text_va: TEXT_RVA }));
        let tables = SymbolTables::parse(&elf).expect("an ELF");
        assert_eq!(tables.frame_locals(Rva(TEXT_RVA)), Err(LocalsError::NotInThisFormat));
    }
}
