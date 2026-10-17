//! A file of debug information's line rows, indexed by address and by
//! file and line, and the rule that resolves a source line to code: a line
//! with rows is those rows; a line without code inside a function's lines
//! is the next line of that function that has code (as debuggers place a
//! breakpoint); any other line is no code.

use std::collections::BTreeMap;

use crate::source_path;
use crate::tables::{push, LineRow, ParseError, ResolvedLine, SourceFile};
use crate::units::Rva;

/// A function's code, `start..end` in RVAs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CodeSpan {
    pub(crate) start: u64,
    pub(crate) end: u64,
}

/// One function's rows in one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FunctionInFile {
    file: usize,
    function: usize,
}

#[derive(Default)]
pub(crate) struct LineIndex {
    /// Rows that cover code, by start.
    rows: Vec<LineRow>,
    /// Places in `rows`, by file, line, then start.
    by_line: Vec<usize>,
    /// By place in `rows`: the function the row's start lies in, an index
    /// into the spans the index was built over.
    function_of: Vec<Option<usize>>,
    /// The least line each function has rows for in each file.
    first_line: BTreeMap<FunctionInFile, u32>,
}

/// A line of a file, ordered by file then line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FileLine {
    file: usize,
    line: u32,
}

/// Where a row sorts in `by_line`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LineKey {
    at: FileLine,
    start: Rva,
}

/// The span holding `at`, of spans sorted by start.
fn containing(functions: &[CodeSpan], at: u64) -> Option<usize> {
    let place = functions.partition_point(|span| span.start <= at).checked_sub(1)?;
    functions.get(place).filter(|span| at < span.end).map(|_| place)
}

fn line_key(rows: &[LineRow], place: usize) -> LineKey {
    rows.get(place).map_or(
        LineKey { at: FileLine { file: usize::MAX, line: u32::MAX }, start: Rva(u64::MAX) },
        |row| LineKey { at: FileLine { file: row.file, line: row.line }, start: row.start },
    )
}

impl LineIndex {
    /// Indexes `rows` (a row that covers no bytes is left out) against
    /// `functions`, sorted by start.
    pub(crate) fn build(mut rows: Vec<LineRow>, functions: &[CodeSpan]) -> Result<LineIndex, ParseError> {
        rows.retain(|row| row.start < row.end);
        rows.sort_unstable_by(|a, b| {
            a.start.cmp(&b.start).then(a.end.cmp(&b.end)).then(a.file.cmp(&b.file)).then(a.line.cmp(&b.line))
        });
        let mut by_line = Vec::new();
        by_line.try_reserve_exact(rows.len()).map_err(|_| ParseError::Allocation { what: "line index" })?;
        by_line.extend(0..rows.len());
        by_line.sort_unstable_by_key(|place| line_key(&rows, *place));
        let mut function_of = Vec::new();
        function_of.try_reserve_exact(rows.len()).map_err(|_| ParseError::Allocation { what: "line index" })?;
        let mut first_line = BTreeMap::new();
        for row in &rows {
            let function = containing(functions, row.start.0);
            function_of.push(function);
            if let Some(function) = function {
                first_line
                    .entry(FunctionInFile { file: row.file, function })
                    .and_modify(|least: &mut u32| *least = (*least).min(row.line))
                    .or_insert(row.line);
            }
        }
        Ok(LineIndex { rows, by_line, function_of, first_line })
    }

    /// The row whose code holds `rva`.
    pub(crate) fn line_at(&self, rva: Rva) -> Option<LineRow> {
        let place = self.rows.partition_point(|row| row.start <= rva).checked_sub(1)?;
        self.rows.get(place).filter(|row| rva < row.end).copied()
    }

    /// Line `line` of each of `files` whose path ends with `file`.
    pub(crate) fn lines_of(&self, files: &[SourceFile], file: &str, line: u32) -> Vec<ResolvedLine> {
        let mut answers = Vec::new();
        for (index, source) in files.iter().enumerate() {
            if !source_path::ends_with(&source.path, file) {
                continue;
            }
            let Some(resolved) = self.resolve(index, line) else {
                continue;
            };
            if push(&mut answers, resolved, "line answers").is_err() {
                break;
            }
        }
        answers
    }

    /// The lines of file `file` that have code, ascending, each once.
    pub(crate) fn code_lines(&self, file: usize) -> Vec<u32> {
        let from = self.by_line.partition_point(|place| line_key(&self.rows, *place).at.file < file);
        let mut lines: Vec<u32> = Vec::new();
        for place in self.by_line.get(from..).unwrap_or_default() {
            let Some(row) = self.rows.get(*place).filter(|row| row.file == file) else {
                break;
            };
            if lines.last() != Some(&row.line) && push(&mut lines, row.line, "code lines").is_err() {
                break;
            }
        }
        lines
    }

    /// Where in `by_line` the rows of `at` begin, and where they end.
    fn places_of(&self, at: FileLine) -> core::ops::Range<usize> {
        let from = self.by_line.partition_point(|place| line_key(&self.rows, *place).at < at);
        let to = self.by_line.partition_point(|place| line_key(&self.rows, *place).at <= at);
        from..to
    }

    /// The function the row at `place` lies in.
    fn function_at(&self, place: usize) -> Option<usize> {
        self.function_of.get(place).copied().flatten()
    }

    /// Line `line` of file `file`.
    fn resolve(&self, file: usize, line: u32) -> Option<ResolvedLine> {
        let places = self.places_of(FileLine { file, line });
        let past = places.end;
        if !places.is_empty() {
            let every = |_: Option<usize>| true;
            return Some(ResolvedLine { line, ranges: self.rows_at(self.by_line.get(places)?, &every) });
        }
        // No code on the line: the next line with code in a function whose
        // rows in this file begin before it.
        let begins_before = |function: Option<usize>| -> bool {
            function
                .and_then(|function| self.first_line.get(&FunctionInFile { file, function }))
                .is_some_and(|first| *first < line)
        };
        let mut next = None;
        for place in self.by_line.get(past..)? {
            let row = self.rows.get(*place)?;
            if row.file != file {
                break;
            }
            if begins_before(self.function_at(*place)) {
                next = Some(row.line);
                break;
            }
        }
        let next = next?;
        let places = self.places_of(FileLine { file, line: next });
        let ranges = self.rows_at(self.by_line.get(places)?, &begins_before);
        Some(ResolvedLine { line: next, ranges })
    }

    /// The rows at `places` (in order) whose function `keep` accepts.
    fn rows_at<K>(&self, places: &[usize], keep: &K) -> Vec<LineRow>
    where
        K: Fn(Option<usize>) -> bool,
    {
        let mut rows = Vec::new();
        for place in places {
            let Some(row) = self.rows.get(*place) else {
                continue;
            };
            if !keep(self.function_at(*place)) {
                continue;
            }
            if push(&mut rows, *row, "line rows").is_err() {
                break;
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(start: u64, end: u64, file: usize, line: u32) -> LineRow {
        LineRow { start: Rva(start), end: Rva(end), file, line }
    }

    fn file(path: &str) -> SourceFile {
        SourceFile { path: path.to_owned(), checksum: None }
    }

    /// `first` holds lines 10 to 20 of a.c, `second` lines 30 to 40.
    fn two_functions() -> LineIndex {
        let rows = vec![
            row(0x100, 0x108, 0, 10),
            row(0x108, 0x110, 0, 20),
            row(0x200, 0x208, 0, 30),
            row(0x208, 0x210, 0, 40),
        ];
        LineIndex::build(rows, &[CodeSpan { start: 0x100, end: 0x110 }, CodeSpan { start: 0x200, end: 0x210 }]).unwrap()
    }

    #[test]
    fn a_line_between_two_functions_resolves_to_nothing() {
        let index = two_functions();
        let files = [file("a.c")];
        assert_eq!(index.lines_of(&files, "a.c", 25), []);
        assert_eq!(
            index.lines_of(&files, "a.c", 35),
            [ResolvedLine { line: 40, ranges: vec![row(0x208, 0x210, 0, 40)] }]
        );
        assert_eq!(index.lines_of(&files, "a.c", 15), [ResolvedLine { line: 20, ranges: vec![row(0x108, 0x110, 0, 20)] }]);
    }

    #[test]
    fn a_row_that_covers_no_code_is_left_out() {
        let index = LineIndex::build(vec![row(0x100, 0x100, 0, 9), row(0x100, 0x108, 0, 10)], &[]).unwrap();
        assert_eq!(index.line_at(Rva(0x100)), Some(row(0x100, 0x108, 0, 10)));
        assert_eq!(index.lines_of(&[file("a.c")], "a.c", 9), [], "no function holds it, so nothing resolves");
    }

    /// Rows outside every function are found by address and by their own
    /// line, but a line without code near them resolves to nothing.
    #[test]
    fn rows_outside_functions_answer_only_their_own_line() {
        let index = LineIndex::build(vec![row(0x100, 0x108, 0, 10), row(0x108, 0x110, 0, 12)], &[]).unwrap();
        let files = [file("a.c")];
        assert_eq!(index.lines_of(&files, "a.c", 12), [ResolvedLine { line: 12, ranges: vec![row(0x108, 0x110, 0, 12)] }]);
        assert_eq!(index.lines_of(&files, "a.c", 11), []);
    }

    #[test]
    fn a_files_lines_with_code_are_listed_once_each() {
        let index = LineIndex::build(
            vec![row(0x100, 0x108, 0, 10), row(0x108, 0x110, 1, 3), row(0x110, 0x118, 0, 12), row(0x200, 0x208, 0, 10)],
            &[],
        )
        .unwrap();
        assert_eq!(index.code_lines(0), [10, 12]);
        assert_eq!(index.code_lines(1), [3]);
        assert_eq!(index.code_lines(2), Vec::<u32>::new());
    }

    #[test]
    fn the_last_line_number_resolves() {
        let index = LineIndex::build(vec![row(0x100, 0x108, 0, u32::MAX)], &[]).unwrap();
        assert_eq!(
            index.lines_of(&[file("a.c")], "a.c", u32::MAX),
            [ResolvedLine { line: u32::MAX, ranges: vec![row(0x100, 0x108, 0, u32::MAX)] }]
        );
    }
}
