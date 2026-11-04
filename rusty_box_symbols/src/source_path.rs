//! Source paths as a build recorded them, compared by whole trailing
//! components: `\` and `/` are one separator, letters compare without case
//! (a Windows build's paths), and an empty component (`a\\b`, a leading or
//! trailing separator) is not one.

use core::str::RSplit;

/// A path's components from the last, empty ones included.
fn from_the_end(path: &str) -> RSplit<'_, [char; 2]> {
    path.rsplit(['\\', '/'])
}

fn not_empty(part: &&str) -> bool {
    !part.is_empty()
}

/// The components of a path, from the first.
pub type Components<'a> = core::iter::Filter<core::str::Split<'a, [char; 2]>, fn(&&str) -> bool>;

/// A path's components from the first, empty ones left out.
pub fn components(path: &str) -> Components<'_> {
    path.split(['\\', '/']).filter(not_empty as fn(&&str) -> bool)
}

/// Whether two components are one, letters compared without case.
#[must_use]
pub fn same_component(a: &str, b: &str) -> bool {
    a.chars().flat_map(char::to_lowercase).eq(b.chars().flat_map(char::to_lowercase))
}

/// How many trailing components `a` and `b` share.
#[must_use]
pub fn trailing_components_in_common(a: &str, b: &str) -> usize {
    from_the_end(a)
        .filter(|part| !part.is_empty())
        .zip(from_the_end(b).filter(|part| !part.is_empty()))
        .take_while(|pair| same_component(pair.0, pair.1))
        .count()
}

/// Whether `path` ends with every component of `query`; a query with no
/// components matches nothing.
#[must_use]
pub fn ends_with(path: &str, query: &str) -> bool {
    let wanted = from_the_end(query).filter(|part| !part.is_empty()).count();
    wanted > 0 && trailing_components_in_common(path, query) == wanted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_ends_with_its_trailing_components() {
        assert!(ends_with(r"D:\b\src\io\read.c", r"src\io\read.c"));
        assert!(ends_with(r"D:\b\src\io\read.c", "read.c"));
        assert!(ends_with(r"D:\b\src\io\read.c", r"D:\b\src\io\read.c"));
    }

    #[test]
    fn components_match_whole() {
        assert!(!ends_with(r"D:\b\src\io\xread.c", "read.c"));
        assert!(!ends_with(r"D:\b\src\io\read.c", r"o\read.c"));
        assert!(!ends_with("read.c", r"io\read.c"), "a query longer than the path");
    }

    #[test]
    fn separators_and_case_do_not_matter() {
        assert!(ends_with(r"D:\B\SRC\IO\READ.C", "io/read.c"));
        assert!(ends_with("/build/widget/src/main.c", r"SRC\Main.C"));
        assert!(ends_with(r"D:\b\\src\io\read.c\", r"\src\io\read.c"), "empty components are not components");
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        assert!(!ends_with(r"D:\b\read.c", ""));
        assert!(!ends_with(r"D:\b\read.c", r"\/"));
        assert!(!ends_with("", ""));
    }

    #[test]
    fn components_skip_empty_parts() {
        assert_eq!(components(r"\\server\share//a\").collect::<Vec<&str>>(), ["server", "share", "a"]);
        assert!(same_component("READ.c", "read.C"));
        assert!(!same_component("read.c", "read.h"));
    }

    #[test]
    fn trailing_components_in_common_are_counted() {
        assert_eq!(trailing_components_in_common(r"D:\b\src\io\read.c", r"E:\work\src\io\read.c"), 3);
        assert_eq!(trailing_components_in_common(r"D:\b\src\io\read.c", r"E:\io\READ.C"), 2);
        assert_eq!(trailing_components_in_common(r"D:\b\src\io\read.c", r"E:\io\write.c"), 0);
        assert_eq!(trailing_components_in_common("", r"E:\io\read.c"), 0);
    }
}
