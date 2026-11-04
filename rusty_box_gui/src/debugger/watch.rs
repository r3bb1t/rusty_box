//! Watches: C expressions over a frame's variables and the module's globals
//! (spec §8.3), read into rows as the variables pane shows them.

use rusty_box_symbols::tables::{FrameScope, TypeShape};
use rusty_box_symbols::units::TypeRef;

use super::frames::{InspectRequest, TipRead, WatchRead};
use super::guest::GuestMemory;
use super::variables::{
    bitfield_place, locate, read_place, rows_of, size_of, type_text, unmodified, unsigned, Expand, FrameContext, Located,
    PathRoot, Place, RowHead, RowKind, RowSink, ShownValue, ValuePath, VariableRow,
};

/// The most unary operators and parentheses one watch nests.
pub(crate) const MAX_WATCH_DEPTH: usize = 64;
/// CodeView's `unsigned __int64` (`T_UQUAD`): the type of an integer term.
const T_UQUAD: TypeRef = TypeRef(0x0023);

/// A watch as written: `expr := unary`, `unary := '*' unary | '&' unary |
/// postfix`, `postfix := primary ('.' name | '->' name | '[' integer ']')*`,
/// `primary := name | integer | '(' expr ')'` — C's precedence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WatchExpr {
    /// `[A-Za-z_][A-Za-z0-9_]*`, with `::` between parts for a C++ global.
    Name(String),
    /// Decimal, or hexadecimal after `0x`.
    Integer(u64),
    Deref(Box<WatchExpr>),
    AddressOf(Box<WatchExpr>),
    Member { of: Box<WatchExpr>, name: String },
    Arrow { of: Box<WatchExpr>, name: String },
    Index { of: Box<WatchExpr>, index: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum WatchSyntaxError {
    #[error("an empty watch")]
    Empty,
    #[error("unexpected '{found}' at {at}")]
    Unexpected { at: usize, found: char },
    #[error("a '(' is not closed")]
    UnclosedParenthesis,
    #[error("not a number at {at}")]
    BadInteger { at: usize },
    #[error("the watch ends early")]
    EndsEarly,
}

/// Why a watch has no value, in the spec's words (§8.3).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum WatchError {
    #[error(transparent)]
    Syntax(#[from] WatchSyntaxError),
    #[error("no variable named {name}")]
    NoVariable { name: String },
    #[error("-> on a non-pointer")]
    ArrowOnNonPointer,
    #[error(". on a non-structure")]
    DotOnNonRecord,
    #[error("[ ] on a non-array or non-pointer")]
    IndexOnNonArray,
    #[error("no member named {name}")]
    NoMember { name: String },
    #[error("& of a value not in memory")]
    AddressOfNonMemory,
    #[error("* of a non-pointer")]
    DerefNonPointer,
    #[error("memory at {at:#x} is not readable")]
    Unreadable { at: u64 },
    #[error("not available here")]
    NotAvailable,
    #[error("optimized away")]
    OptimizedAway,
    #[error("{0}")]
    Type(String),
}

/// Reads watch `text`.
pub(crate) fn parse_watch(text: &str) -> Result<WatchExpr, WatchSyntaxError> {
    let mut parser = Parser { text, at: 0 };
    parser.skip_space();
    if parser.peek().is_none() {
        return Err(WatchSyntaxError::Empty);
    }
    let expr = parser.unary(0)?;
    parser.skip_space();
    match parser.peek() {
        None => Ok(expr),
        Some(found) => Err(WatchSyntaxError::Unexpected { at: parser.at, found }),
    }
}

/// A recursive-descent reader over `text`, `at` its byte offset.
struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl Parser<'_> {
    fn rest(&self) -> &str {
        self.text.get(self.at..).unwrap_or("")
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn advance(&mut self, past: char) {
        self.skip(past.len_utf8());
    }

    fn skip(&mut self, bytes: usize) {
        self.at = self.at.saturating_add(bytes);
    }

    fn skip_space(&mut self) {
        while let Some(space) = self.peek().filter(|next| next.is_whitespace()) {
            self.advance(space);
        }
    }

    /// The error for what stands at `at`.
    fn unexpected(&self) -> WatchSyntaxError {
        match self.peek() {
            Some(found) => WatchSyntaxError::Unexpected { at: self.at, found },
            None => WatchSyntaxError::EndsEarly,
        }
    }

    fn unary(&mut self, depth: usize) -> Result<WatchExpr, WatchSyntaxError> {
        self.skip_space();
        if depth >= MAX_WATCH_DEPTH {
            return Err(self.unexpected());
        }
        match self.peek() {
            Some('*') => {
                self.advance('*');
                Ok(WatchExpr::Deref(Box::new(self.unary(depth.saturating_add(1))?)))
            }
            Some('&') => {
                self.advance('&');
                Ok(WatchExpr::AddressOf(Box::new(self.unary(depth.saturating_add(1))?)))
            }
            Some(_) | None => self.postfix(depth),
        }
    }

    /// A primary and its links; each link nests the expression one level,
    /// counted with `depth` against `MAX_WATCH_DEPTH`.
    fn postfix(&mut self, depth: usize) -> Result<WatchExpr, WatchSyntaxError> {
        let mut expr = self.primary(depth)?;
        let mut nested = depth;
        loop {
            self.skip_space();
            let links = matches!(self.peek(), Some('.' | '[')) || self.rest().starts_with("->");
            if links {
                if nested >= MAX_WATCH_DEPTH {
                    return Err(self.unexpected());
                }
                nested = nested.saturating_add(1);
            }
            match self.peek() {
                Some('.') => {
                    self.advance('.');
                    expr = WatchExpr::Member { of: Box::new(expr), name: self.name()? };
                }
                Some('-') if self.rest().starts_with("->") => {
                    self.skip("->".len());
                    expr = WatchExpr::Arrow { of: Box::new(expr), name: self.name()? };
                }
                Some('[') => {
                    self.advance('[');
                    self.skip_space();
                    let index = self.integer()?;
                    self.skip_space();
                    match self.peek() {
                        Some(']') => self.advance(']'),
                        Some(_) | None => return Err(self.unexpected()),
                    }
                    expr = WatchExpr::Index { of: Box::new(expr), index };
                }
                Some(_) | None => return Ok(expr),
            }
        }
    }

    fn primary(&mut self, depth: usize) -> Result<WatchExpr, WatchSyntaxError> {
        self.skip_space();
        match self.peek() {
            Some('(') => {
                self.advance('(');
                let inner = self.unary(depth.saturating_add(1))?;
                self.skip_space();
                match self.peek() {
                    Some(')') => {
                        self.advance(')');
                        Ok(inner)
                    }
                    Some(found) => Err(WatchSyntaxError::Unexpected { at: self.at, found }),
                    None => Err(WatchSyntaxError::UnclosedParenthesis),
                }
            }
            Some(digit) if digit.is_ascii_digit() => Ok(WatchExpr::Integer(self.integer()?)),
            Some(_) | None => Ok(WatchExpr::Name(self.name()?)),
        }
    }

    /// A name: its parts joined by `::`.
    fn name(&mut self) -> Result<String, WatchSyntaxError> {
        self.skip_space();
        let start = self.at;
        if !self.peek().is_some_and(|first| first == '_' || first.is_ascii_alphabetic()) {
            return Err(self.unexpected());
        }
        loop {
            match self.peek() {
                Some(next) if next == '_' || next.is_ascii_alphanumeric() => self.advance(next),
                Some(':') if self.rest().starts_with("::") => self.skip("::".len()),
                Some(_) | None => break,
            }
        }
        Ok(self.text.get(start..self.at).unwrap_or("").to_owned())
    }

    fn integer(&mut self) -> Result<u64, WatchSyntaxError> {
        let start = self.at;
        let hexadecimal = self.rest().starts_with("0x") || self.rest().starts_with("0X");
        if hexadecimal {
            self.skip("0x".len());
        }
        let digits = self.at;
        while let Some(next) = self.peek().filter(|next| next.is_ascii_alphanumeric() || *next == '_') {
            self.advance(next);
        }
        let radix = if hexadecimal { 16 } else { 10 };
        let text = self.text.get(digits..self.at).unwrap_or("");
        u64::from_str_radix(text, radix).map_err(|_| WatchSyntaxError::BadInteger { at: start })
    }
}

/// Watch `index` (`text`) in the frame of `context`: its rows, or one row
/// saying why it has no value.
pub(crate) fn read_watch<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    scope: Option<&FrameScope>,
    index: usize,
    text: &str,
    request: &InspectRequest,
) -> WatchRead {
    let value = parse_watch(text).map_err(WatchError::from).and_then(|expr| evaluate(memory, context, scope, &expr));
    expression_rows(memory, context, PathRoot::Watch(index), text, value, request)
}

/// The value tip's expression `text`, read as a watch is; a word that
/// names no variable reads as `NoVariable`, which closes the tip.
pub(crate) fn read_tip<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    scope: Option<&FrameScope>,
    text: &str,
    request: &InspectRequest,
) -> TipRead {
    let value = parse_watch(text).map_err(WatchError::from).and_then(|expr| evaluate(memory, context, scope, &expr));
    match value {
        Err(WatchError::NoVariable { .. }) => TipRead::NoVariable { text: text.to_owned() },
        Ok(_) | Err(_) => TipRead::Rows(expression_rows(memory, context, PathRoot::Tip, text, value, request)),
    }
}

/// The rows of expression `text` under `root`: its value's, or one saying
/// why it has none.
fn expression_rows<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    root: PathRoot,
    text: &str,
    value: Result<Located, WatchError>,
    request: &InspectRequest,
) -> WatchRead {
    let head = expression_head(root, text);
    let mut rows = RowSink::new();
    match value {
        Ok(value) => rows_of(memory, context, head, value, request, &mut rows),
        Err(error) => rows.push(problem_row(head, error.to_string())),
    }
    WatchRead { text: text.to_owned(), rows: rows.into_rows() }
}

/// Expression `text` under `root` where no frame's variables can be read,
/// `why`.
pub(crate) fn unread_watch(root: PathRoot, text: &str, why: &str) -> WatchRead {
    WatchRead { text: text.to_owned(), rows: vec![problem_row(expression_head(root, text), why.to_owned())] }
}

fn expression_head(root: PathRoot, text: &str) -> RowHead {
    RowHead { path: ValuePath { root, steps: Vec::new() }, depth: 0, name: text.to_owned(), kind: RowKind::Watch }
}

/// C keywords: a click on one names no variable. `this` is not among
/// them: in a method it is a parameter.
const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum", "extern", "float",
    "for", "goto", "if", "inline", "int", "long", "register", "restrict", "return", "short", "signed", "sizeof",
    "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile", "while", "bool", "true", "false",
    "nullptr", "class", "public", "private", "protected", "template", "typename", "namespace", "using", "new", "delete",
    "operator", "virtual",
];

fn is_identifier(character: char) -> bool {
    character == '_' || character.is_ascii_alphanumeric()
}

/// The expression a click at `column` (a char index) of `line` names: the
/// identifier under it, with the `.`, `->` and `::` chain of identifiers in
/// front of it (`p->count` when `count` is clicked; `p` when `p` is). None
/// off an identifier, on a C keyword, or on a number.
pub(crate) fn expression_at(line: &str, column: usize) -> Option<String> {
    let characters: Vec<char> = line.chars().collect();
    if !characters.get(column).copied().is_some_and(is_identifier) {
        return None;
    }
    let mut start = identifier_start(&characters, column)?;
    let after = characters.get(column..)?.iter().take_while(|character| is_identifier(**character)).count();
    let end = column.checked_add(after)?;
    let word: String = characters.get(start..end)?.iter().collect();
    if word.starts_with(|first: char| first.is_ascii_digit()) || C_KEYWORDS.contains(&word.as_str()) {
        return None;
    }
    let mut expression = word;
    loop {
        let before = characters.get(..start)?;
        let joint = if before.ends_with(&['-', '>']) {
            "->"
        } else if before.ends_with(&[':', ':']) {
            "::"
        } else if before.ends_with(&['.']) {
            "."
        } else {
            break;
        };
        let joint_end = start.checked_sub(joint.len())?;
        let part_start = identifier_start(&characters, joint_end)?;
        let part: String = characters.get(part_start..joint_end)?.iter().collect();
        if part.is_empty() || part.starts_with(|first: char| first.is_ascii_digit()) || C_KEYWORDS.contains(&part.as_str()) {
            break;
        }
        expression = format!("{part}{joint}{expression}");
        start = part_start;
    }
    Some(expression)
}

/// Where the identifier ending at `end` (exclusive) starts.
fn identifier_start(characters: &[char], end: usize) -> Option<usize> {
    let before = characters.get(..end)?.iter().rev().take_while(|character| is_identifier(**character)).count();
    end.checked_sub(before)
}

fn problem_row(head: RowHead, problem: String) -> VariableRow {
    VariableRow {
        path: head.path,
        depth: head.depth,
        name: head.name,
        type_name: String::new(),
        kind: head.kind,
        value: ShownValue::Problem(problem),
        expand: Expand::Leaf,
        memory: None,
    }
}

/// Where `expr`'s value is in the frame of `context`: a name is the
/// innermost variable of `scope` named so, else the module's global.
pub(crate) fn evaluate<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    scope: Option<&FrameScope>,
    expr: &WatchExpr,
) -> Result<Located, WatchError> {
    match expr {
        WatchExpr::Name(name) => lookup(context, scope, name),
        WatchExpr::Integer(value) => Ok(Located { ty: T_UQUAD, place: Place::Bytes(value.to_le_bytes().to_vec()) }),
        WatchExpr::Deref(of) => {
            let pointer = evaluate(memory, context, scope, of)?;
            deref(memory, context, pointer, WatchError::DerefNonPointer)
        }
        WatchExpr::AddressOf(of) => {
            let value = evaluate(memory, context, scope, of)?;
            match value.place {
                Place::Memory(address) => Ok(Located { ty: value.ty, place: Place::AddressOf(address) }),
                Place::Missing(shown) => Err(watch_error(shown)),
                Place::Bytes(_) | Place::AddressOf(_) => Err(WatchError::AddressOfNonMemory),
            }
        }
        WatchExpr::Member { of, name } => {
            let record = evaluate(memory, context, scope, of)?;
            member(memory, context, record, name)
        }
        WatchExpr::Arrow { of, name } => {
            let pointer = evaluate(memory, context, scope, of)?;
            let record = deref(memory, context, pointer, WatchError::ArrowOnNonPointer)?;
            member(memory, context, record, name)
        }
        WatchExpr::Index { of, index } => {
            let base = evaluate(memory, context, scope, of)?;
            element(memory, context, base, *index)
        }
    }
}

fn lookup(context: &FrameContext<'_>, scope: Option<&FrameScope>, name: &str) -> Result<Located, WatchError> {
    let variable = scope.and_then(|scope| scope.variables.iter().find(|variable| variable.name == name).map(|variable| {
        locate(context, variable, scope)
    }));
    if let Some(located) = variable {
        return Ok(located);
    }
    match context.tables.global(name) {
        Some(global) => Ok(Located { ty: global.ty, place: Place::Memory(context.module_base.wrapping_add(global.rva.0)) }),
        None => Err(WatchError::NoVariable { name: name.to_owned() }),
    }
}

/// What `value` points at: a pointer's target, an array's first element.
fn deref<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    value: Located,
    not_a_pointer: WatchError,
) -> Result<Located, WatchError> {
    if let Place::AddressOf(address) = value.place {
        return Ok(Located { ty: value.ty, place: Place::Memory(address) });
    }
    match unmodified(context.tables, value.ty).map_err(watch_error)? {
        TypeShape::Pointer { to, size, .. } => {
            let address = pointer_value(memory, context, &value.place, size)?;
            Ok(Located { ty: to, place: Place::Memory(address) })
        }
        TypeShape::Array { element, .. } => Ok(Located { ty: element, place: value.place }),
        TypeShape::Base(_)
        | TypeShape::Modified { .. }
        | TypeShape::Record { .. }
        | TypeShape::Incomplete { .. }
        | TypeShape::Enum { .. }
        | TypeShape::Function { .. }
        | TypeShape::Unknown { .. } => Err(not_a_pointer),
    }
}

fn member<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    record: Located,
    name: &str,
) -> Result<Located, WatchError> {
    if let Place::AddressOf(_) = record.place {
        return Err(WatchError::DotOnNonRecord);
    }
    let TypeShape::Record { fields, .. } = unmodified(context.tables, record.ty).map_err(watch_error)? else {
        return Err(WatchError::DotOnNonRecord);
    };
    let field = fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| WatchError::NoMember { name: name.to_owned() })?;
    let at = record.place.part(field.offset.0);
    let place = match field.bits {
        None => at,
        Some(bits) => bitfield_place(memory, context, &at, field.ty, bits),
    };
    Ok(Located { ty: field.ty, place })
}

/// Element `index` of an array, or of the elements a pointer points at.
fn element<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    base: Located,
    index: u64,
) -> Result<Located, WatchError> {
    if let Place::AddressOf(address) = base.place {
        let size = sized(context, base.ty)?;
        return Ok(Located { ty: base.ty, place: Place::Memory(stepped(context, address, index, size)) });
    }
    match unmodified(context.tables, base.ty).map_err(watch_error)? {
        TypeShape::Array { element, .. } => {
            let size = sized(context, element)?;
            Ok(Located { ty: element, place: base.place.part(index.wrapping_mul(size)) })
        }
        TypeShape::Pointer { to, size: width, .. } => {
            let address = pointer_value(memory, context, &base.place, width)?;
            let size = sized(context, to)?;
            Ok(Located { ty: to, place: Place::Memory(stepped(context, address, index, size)) })
        }
        TypeShape::Base(_)
        | TypeShape::Modified { .. }
        | TypeShape::Record { .. }
        | TypeShape::Incomplete { .. }
        | TypeShape::Enum { .. }
        | TypeShape::Function { .. }
        | TypeShape::Unknown { .. } => Err(WatchError::IndexOnNonArray),
    }
}

/// `address` stepped `index` elements of `size` bytes, as the frame's code
/// computes addresses.
fn stepped(context: &FrameContext<'_>, address: u64, index: u64, size: u64) -> u64 {
    context.address(address.wrapping_add(index.wrapping_mul(size)), 0)
}

fn sized(context: &FrameContext<'_>, ty: TypeRef) -> Result<u64, WatchError> {
    size_of(context.tables, ty)
        .filter(|size| *size > 0)
        .ok_or_else(|| WatchError::Type(format!("{} has no size", type_text(context.tables, ty))))
}

fn pointer_value<G: GuestMemory>(
    memory: &mut G,
    context: &FrameContext<'_>,
    place: &Place,
    size: u8,
) -> Result<u64, WatchError> {
    read_place(memory, context.root, place, u64::from(size)).map(|bytes| unsigned(&bytes)).map_err(watch_error)
}

/// What a value that could not be read says, as a watch's error.
fn watch_error(shown: ShownValue) -> WatchError {
    match shown {
        ShownValue::Unreadable { at } => WatchError::Unreadable { at },
        ShownValue::OptimizedAway => WatchError::OptimizedAway,
        ShownValue::Problem(problem) => WatchError::Type(problem),
        ShownValue::UnknownType { index, .. } => WatchError::Type(format!("<type {index:#x}> is not described")),
        ShownValue::NotAvailable
        | ShownValue::Integer { .. }
        | ShownValue::Character { .. }
        | ShownValue::Boolean { .. }
        | ShownValue::Real32(_)
        | ShownValue::Real64(_)
        | ShownValue::Hresult(_)
        | ShownValue::Pointer { .. }
        | ShownValue::Enum { .. }
        | ShownValue::Aggregate { .. } => WatchError::NotAvailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::frames::InspectRequest;
    use crate::debugger::guest::fake::FakeGuest;
    use crate::debugger::test_frame::{
        field, guest, on_stack, tables, tables_with, FrameFixture, Setup, BASE, CR3, IN_BODY, RSP_AT, TEXT,
    };
    use crate::debugger::variables::{ShownValue, Signedness};
    use rusty_box_symbols::tables::FrameScope;
    use rusty_box_symbols::units::Rva;
    use rusty_box_symbols_fixtures::pdb::{FixtureGlobal, FixtureType, FixtureUdt, UdtKind};

    fn name(text: &str) -> Box<WatchExpr> {
        Box::new(WatchExpr::Name(text.into()))
    }

    fn scope(setup: &Setup) -> FrameScope {
        setup.tables.frame_locals(Rva(IN_BODY)).expect("the records read").expect("ReadBlock's scope")
    }

    /// Watch `text`'s first row's value in `setup`'s frame.
    fn watched(setup: &Setup, guest: &mut FakeGuest, text: &str) -> ShownValue {
        let read = read_watch(guest, &setup.context(), Some(&scope(setup)), 0, text, &InspectRequest::default());
        assert_eq!(read.text, text);
        read.rows.first().map(|row| row.value.clone()).unwrap_or_else(|| panic!("no rows for {text}"))
    }

    fn node() -> FixtureUdt {
        FixtureUdt {
            name: "Node".into(),
            kind: UdtKind::Struct,
            size: 16,
            fields: vec![
                field("value", 0, FixtureType::I32),
                field("next", 8, FixtureType::PointerTo(Box::new(FixtureType::Named("Node".into())))),
            ],
        }
    }

    fn int(raw: u64) -> ShownValue {
        ShownValue::Integer { raw, size: 4, signedness: Signedness::Signed }
    }

    #[test]
    fn the_grammar_reads_members_pointers_and_indices() {
        let chain = WatchExpr::Member {
            of: Box::new(WatchExpr::Index {
                of: Box::new(WatchExpr::Arrow { of: Box::new(WatchExpr::Deref(name("p"))), name: "next".into() }),
                index: 3,
            }),
            name: "count".into(),
        };
        assert_eq!(parse_watch("(*p)->next[3].count"), Ok(chain));
        assert_eq!(
            parse_watch("*p->x"),
            Ok(WatchExpr::Deref(Box::new(WatchExpr::Arrow { of: name("p"), name: "x".into() }))),
            "a postfix binds tighter than a unary, as in C"
        );
        assert_eq!(parse_watch("&ns::g_count"), Ok(WatchExpr::AddressOf(name("ns::g_count"))));
        assert_eq!(parse_watch("0x10"), Ok(WatchExpr::Integer(16)));
        assert_eq!(parse_watch(" 42 "), Ok(WatchExpr::Integer(42)));
        assert_eq!(parse_watch(""), Err(WatchSyntaxError::Empty));
        assert_eq!(parse_watch("   "), Err(WatchSyntaxError::Empty));
        assert_eq!(parse_watch("a..b"), Err(WatchSyntaxError::Unexpected { at: 2, found: '.' }));
        assert_eq!(parse_watch("(a"), Err(WatchSyntaxError::UnclosedParenthesis));
        assert_eq!(parse_watch("a."), Err(WatchSyntaxError::EndsEarly));
        assert_eq!(parse_watch("12ab"), Err(WatchSyntaxError::BadInteger { at: 0 }));
        let deep = format!("{}p", "*".repeat(100));
        assert_eq!(parse_watch(&deep), Err(WatchSyntaxError::Unexpected { at: MAX_WATCH_DEPTH, found: '*' }));
    }

    /// A chain of members nests one level per link, as `*` does: a watch
    /// `a.b.b.b…` past the ceiling is refused at the first link over it.
    #[test]
    fn a_member_chain_past_the_ceiling_is_refused() {
        let long = format!("a{}", ".b".repeat(100_000));
        assert_eq!(parse_watch(&long), Err(WatchSyntaxError::Unexpected { at: 1 + 2 * MAX_WATCH_DEPTH, found: '.' }));
        let within = format!("a{}", ".b".repeat(MAX_WATCH_DEPTH - 1));
        assert!(parse_watch(&within).is_ok());
    }

    /// The expression under a click on `word`'s `offset`-th character, the
    /// first `word` of `line`.
    fn clicked(line: &str, word: &str, offset: usize) -> Option<String> {
        let at = line.find(word).unwrap_or_else(|| panic!("{word} in {line}"));
        expression_at(line, line[..at].chars().count() + offset)
    }

    #[test]
    fn the_expression_under_a_click_takes_its_chain() {
        let line = "    total += p->count[i] + node.next->value;";
        assert_eq!(clicked(line, "count", 1).as_deref(), Some("p->count"));
        assert_eq!(clicked(line, "p->", 0).as_deref(), Some("p"));
        assert_eq!(clicked(line, "value", 2).as_deref(), Some("node.next->value"));
        assert_eq!(clicked(line, "next", 0).as_deref(), Some("node.next"));
        assert_eq!(clicked(line, "total", 4).as_deref(), Some("total"));
        assert_eq!(clicked(line, "+=", 0), None);
        assert_eq!(clicked(line, " ", 0), None);
        let global = "return ns::g_count;";
        assert_eq!(clicked(global, "g_count", 3).as_deref(), Some("ns::g_count"));
        assert_eq!(clicked(global, "return", 1), None);
        let declaration = "int x = 42;";
        assert_eq!(clicked(declaration, "42", 0), None);
        assert_eq!(clicked(declaration, "int", 0), None);
        assert_eq!(clicked(declaration, "x", 0).as_deref(), Some("x"));
        assert_eq!(clicked("this->size", "size", 0).as_deref(), Some("this->size"));
        assert_eq!(expression_at(declaration, 400), None);
    }

    #[test]
    fn a_watch_follows_a_pointer_to_a_member() {
        let to_node = FixtureType::PointerTo(Box::new(FixtureType::Named("Node".into())));
        let setup = Setup::new(tables(vec![on_stack("node", to_node, 0)], vec![node()]));
        let mut guest = guest();
        guest.write_u64(CR3, RSP_AT, TEXT);
        guest.write(CR3, TEXT, &[0; 0x20]);
        guest.write_u32(CR3, TEXT, 1);
        guest.write_u64(CR3, TEXT + 8, TEXT + 0x10);
        guest.write_u32(CR3, TEXT + 0x10, 2);
        assert_eq!(watched(&setup, &mut guest, "node->next->value"), int(2));
        assert_eq!(watched(&setup, &mut guest, "(*node).value"), int(1));
        assert_eq!(watched(&setup, &mut guest, "node[1].value"), int(2), "indexing a pointer steps by its target's size");
    }

    #[test]
    fn the_spec_errors_read_as_written() {
        let setup = Setup::new(tables(
            vec![
                on_stack("count", FixtureType::I32, 0),
                on_stack("bad", FixtureType::PointerTo(Box::new(FixtureType::I32)), 8),
            ],
            Vec::new(),
        ));
        let mut guest = guest();
        let unmapped = TEXT + 0x5000;
        guest.write_u64(CR3, RSP_AT + 8, unmapped);
        assert_eq!(watched(&setup, &mut guest, "nothing"), ShownValue::Problem("no variable named nothing".into()));
        assert_eq!(watched(&setup, &mut guest, "count->x"), ShownValue::Problem("-> on a non-pointer".into()));
        assert_eq!(watched(&setup, &mut guest, "count.x"), ShownValue::Problem(". on a non-structure".into()));
        assert_eq!(watched(&setup, &mut guest, "count[1]"), ShownValue::Problem("[ ] on a non-array or non-pointer".into()));
        assert_eq!(watched(&setup, &mut guest, "*count"), ShownValue::Problem("* of a non-pointer".into()));
        assert_eq!(watched(&setup, &mut guest, "&5"), ShownValue::Problem("& of a value not in memory".into()));
        assert_eq!(watched(&setup, &mut guest, "*bad"), ShownValue::Unreadable { at: unmapped });
        assert_eq!(watched(&setup, &mut guest, "a..b"), ShownValue::Problem("unexpected '.' at 2".into()));
    }

    #[test]
    fn a_local_hides_a_global_of_its_name() {
        let global = |name: &str, offset: u32| FixtureGlobal { name: name.into(), section: 2, offset, ty: FixtureType::I32 };
        let setup = Setup::new(tables_with(FrameFixture {
            locals: vec![on_stack("g", FixtureType::I32, 0)],
            types: Vec::new(),
            globals: vec![global("g", 0x10), global("g_only", 0x20)],
        }));
        let mut guest = guest();
        guest.write_u32(CR3, RSP_AT, 5);
        guest.write(CR3, BASE + 0x3000, &[0; 0x40]);
        guest.write_u32(CR3, BASE + 0x3020, 9);
        assert_eq!(watched(&setup, &mut guest, "g"), int(5));
        assert_eq!(watched(&setup, &mut guest, "g_only"), int(9));
        let read = read_watch(&mut guest, &setup.context(), None, 0, "&g_only", &InspectRequest::default());
        assert_eq!(read.rows[0].value, ShownValue::Pointer { address: BASE + 0x3020, string: None });
        assert_eq!(read.rows[0].type_name, "int *");
    }

    #[test]
    fn indexing_a_pointer_reads_the_element_it_points_past() {
        let setup = Setup::new(tables(vec![on_stack("buffer", FixtureType::PointerTo(Box::new(FixtureType::Char)), 0)], Vec::new()));
        let mut guest = guest();
        guest.write_u64(CR3, RSP_AT, TEXT);
        guest.write(CR3, TEXT, b"hello\0");
        assert_eq!(watched(&setup, &mut guest, "buffer[2]"), ShownValue::Character { code: u32::from(b'l'), size: 1 });
        assert_eq!(watched(&setup, &mut guest, "0x10"), ShownValue::Integer { raw: 16, size: 8, signedness: Signedness::Unsigned });
    }
}
