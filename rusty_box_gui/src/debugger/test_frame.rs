//! One function's frame over a fixture PDB, for the tests of what reads its
//! variables: `ReadBlock` at RVA 0x1100, 0x60 bytes, its body 0x1110..0x1150,
//! its stack at `RSP_AT`.

use rusty_box_symbols::tables::SymbolTables;
use rusty_box_symbols_fixtures::pdb::{
    build_pdb, FixtureField, FixtureFrameProc, FixtureGlobal, FixtureLine, FixtureLocal, FixturePlace, FixtureProcedure,
    FixtureScope, FixtureType, FixtureUdt, PdbFixture,
};

use super::frames::{InspectRequest, LocalsRead};
use super::guest::fake::FakeGuest;
use super::guest::Root;
use super::unwind::{CodeWidth, Frame, FrameFound, FrameRegisters};
use super::variables::{frame_scope, locals_of, FrameContext, Innermost, ShownValue, VariableRow};

pub(crate) const CR3: u64 = 0x1000;
pub(crate) const BASE: u64 = 0x1_4000_0000;
pub(crate) const RSP_AT: u64 = 0x7_0000;
pub(crate) const TEXT: u64 = 0x8_0000;
/// CodeView's `RSP`.
pub(crate) const CV_RSP: u16 = 335;
/// In `ReadBlock`'s body.
pub(crate) const IN_BODY: u64 = 0x1120;

/// What the fixture PDB holds besides `ReadBlock`: its variables, the
/// structures they use, and the module's globals.
#[derive(Clone, Debug, Default)]
pub(crate) struct FrameFixture {
    pub(crate) locals: Vec<FixtureLocal>,
    pub(crate) types: Vec<FixtureUdt>,
    pub(crate) globals: Vec<FixtureGlobal>,
}

pub(crate) fn local(name: &str, ty: FixtureType, place: FixturePlace) -> FixtureLocal {
    FixtureLocal { name: name.into(), ty, place }
}

pub(crate) fn on_stack(name: &str, ty: FixtureType, offset: i32) -> FixtureLocal {
    local(name, ty, FixturePlace::RegRel { register: CV_RSP, offset })
}

pub(crate) fn field(name: &str, offset: u64, ty: FixtureType) -> FixtureField {
    FixtureField { name: name.into(), offset, ty }
}

pub(crate) fn tables_with(fixture: FrameFixture) -> SymbolTables {
    let mut pdb = PdbFixture::new([6; 16], 1);
    pdb.types = fixture.types;
    pdb.globals = fixture.globals;
    pdb.procedures.push(FixtureProcedure {
        name: "ReadBlock".into(),
        section: 1,
        offset: 0x100,
        len: 0x60,
        file: r"C:\src\read.c".into(),
        checksum: None,
        lines: vec![FixtureLine { offset: 0, line: 10 }],
    });
    pdb.scopes.push(FixtureScope {
        procedure: "ReadBlock".into(),
        debug_start: 0x10,
        debug_end: 0x50,
        parameters: None,
        frame: Some(FixtureFrameProc { locals: 1, params: 1 }),
        locals: fixture.locals,
        inline_locals: Vec::new(),
        blocks: Vec::new(),
    });
    SymbolTables::parse(&build_pdb(&pdb)).expect("the fixture parses")
}

/// `ReadBlock` with `locals` and the structures `types`.
pub(crate) fn tables(locals: Vec<FixtureLocal>, types: Vec<FixtureUdt>) -> SymbolTables {
    tables_with(FrameFixture { locals, types, globals: Vec::new() })
}

/// A frame whose code is at `rva`: the innermost one stopped there, a
/// caller resumed just past it.
pub(crate) fn frame(rva: u64, innermost: Innermost, registers: [Option<u64>; 16]) -> Frame {
    let how = match innermost {
        Innermost::Yes => FrameFound::Registers,
        Innermost::Caller => FrameFound::Unwind,
    };
    let rip = match innermost {
        Innermost::Yes => BASE + rva,
        Innermost::Caller => BASE + rva + 1,
    };
    Frame { registers: FrameRegisters { rip, rsp: RSP_AT, registers }, how }
}

/// Everything a read needs but the guest: the frame, its tables, and its
/// XMM registers.
pub(crate) struct Setup {
    pub(crate) tables: SymbolTables,
    pub(crate) frame: Frame,
    pub(crate) innermost: Innermost,
    pub(crate) xmm: Option<[[u8; 16]; 16]>,
}

impl Setup {
    pub(crate) fn new(tables: SymbolTables) -> Setup {
        Setup { tables, frame: frame(IN_BODY, Innermost::Yes, [Some(0); 16]), innermost: Innermost::Yes, xmm: None }
    }

    pub(crate) fn context(&self) -> FrameContext<'_> {
        FrameContext {
            frame: &self.frame,
            innermost: self.innermost,
            xmm: self.xmm.as_ref(),
            root: Root::Live,
            width: CodeWidth::Long64,
            module_base: BASE,
            tables: &self.tables,
        }
    }

    pub(crate) fn rows(&self, guest: &mut FakeGuest, request: &InspectRequest) -> Vec<VariableRow> {
        let context = self.context();
        match locals_of(guest, &context, &frame_scope(&context), request) {
            LocalsRead::Rows(rows) => rows,
            other => panic!("no rows: {other:?}"),
        }
    }
}

/// A guest whose stack page reads as zeros.
pub(crate) fn guest() -> FakeGuest {
    let mut guest = FakeGuest::new(CR3);
    guest.write(CR3, RSP_AT, &[0; 0x100]);
    guest
}

pub(crate) fn value<'a>(rows: &'a [VariableRow], name: &str) -> &'a ShownValue {
    &rows.iter().find(|row| row.name == name).unwrap_or_else(|| panic!("a row {name}: {rows:?}")).value
}
