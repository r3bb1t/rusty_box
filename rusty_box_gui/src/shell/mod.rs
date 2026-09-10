//! The desktop shell's design vocabulary.
//!
//! `theme` fixes the palette and the type and spacing scale every pane draws
//! from, and applies them to the egui context and to a card frame. `widgets`
//! is the set of pieces a pane is assembled from on that scale: the page
//! header, the field row, the selection row the sidebar tree and the Hardware
//! device list share, the status dot and badge, the hairlines that join
//! stacked panels, and the action tiles. `destination` is where the shell is
//! pointed — which VM profile and which of its pages, held as one value so
//! the pair cannot disagree — and the actions its two chrome surfaces report.
//! `sidebar` is the desktop's navigation: the tree of VM profiles, the
//! selected one open to its pages, and the entry each row is drawn from.
//! `vm_bar` is the one bar above a page: the selected VM's name, its state
//! badge, and the verbs that change that state, with the console's own
//! controls shown only while the console page is.

pub(crate) mod destination;
pub(crate) mod sidebar;
pub(crate) mod theme;
pub(crate) mod vm_bar;
pub(crate) mod widgets;

/// Every character a shell string puts on screen is one egui's default
/// proportional font can draw. The font has no glyph for much of the symbol
/// blocks and paints each such character as a hollow box, so every caption is
/// checked here against the font itself rather than by eye.
#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    /// The sources whose string literals reach the screen, each with the path
    /// a failure names.
    const SOURCES: [(&str, &str); 7] = [
        ("rusty_box_gui/src/app.rs", include_str!("../app.rs")),
        (
            "rusty_box_gui/src/shell/destination.rs",
            include_str!("destination.rs"),
        ),
        ("rusty_box_gui/src/shell/mod.rs", include_str!("mod.rs")),
        ("rusty_box_gui/src/shell/sidebar.rs", include_str!("sidebar.rs")),
        ("rusty_box_gui/src/shell/theme.rs", include_str!("theme.rs")),
        ("rusty_box_gui/src/shell/vm_bar.rs", include_str!("vm_bar.rs")),
        ("rusty_box_gui/src/shell/widgets.rs", include_str!("widgets.rs")),
    ];

    /// Where the scanner is in the source: outside any literal, inside one of
    /// the two comment forms, or inside a string of one of the two kinds.
    #[derive(Clone, Copy)]
    enum Lexeme {
        Code,
        LineComment,
        BlockComment { depth: usize },
        Str,
        RawStr { hashes: usize },
    }

    /// Decodes the `\u{XXXX}` escape whose backslash sits at `start`; returns
    /// the character and the index just past the closing brace.
    fn unicode_escape(chars: &[char], start: usize) -> Option<(char, usize)> {
        if chars.get(start + 1) != Some(&'u') || chars.get(start + 2) != Some(&'{') {
            return None;
        }
        let digits_start = start + 3;
        let digits_end = chars
            .get(digits_start..)?
            .iter()
            .position(|c| *c == '}')
            .map(|offset| digits_start + offset)?;
        let digits: String = chars[digits_start..digits_end].iter().collect();
        let code_point = u32::from_str_radix(&digits, 16).ok()?;
        Some((char::from_u32(code_point)?, digits_end + 1))
    }

    /// The non-ASCII characters `source` puts inside a string or character
    /// literal — spelled directly or as a `\u{XXXX}` escape, in ordinary and
    /// raw strings, across the lines a literal spans — with every comment
    /// skipped, so that the prose of a doc comment never counts and a `'"'`
    /// never opens a string.
    fn non_ascii_in_literals(source: &str) -> BTreeSet<char> {
        let chars: Vec<char> = source.chars().collect();
        let mut found = BTreeSet::new();
        let mut lexeme = Lexeme::Code;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            match lexeme {
                Lexeme::Code => {
                    if c == '/' && next == Some('/') {
                        lexeme = Lexeme::LineComment;
                        i += 2;
                    } else if c == '/' && next == Some('*') {
                        lexeme = Lexeme::BlockComment { depth: 1 };
                        i += 2;
                    } else if c == '"' {
                        lexeme = Lexeme::Str;
                        i += 1;
                    } else if c == 'r' && raw_string_opens(&chars, i) {
                        let mut quote = i + 1;
                        while chars.get(quote) == Some(&'#') {
                            quote += 1;
                        }
                        lexeme = Lexeme::RawStr {
                            hashes: quote - (i + 1),
                        };
                        i = quote + 1;
                    } else if c == '\'' {
                        if next == Some('\\') {
                            // An escaped character literal runs to its closing
                            // quote; only a `\u{XXXX}` escape can name a
                            // non-ASCII character.
                            let close = chars
                                .get(i + 2..)
                                .and_then(|rest| rest.iter().position(|c| *c == '\''))
                                .map_or(chars.len(), |offset| i + 2 + offset);
                            if let Some((decoded, _)) = unicode_escape(&chars, i + 1) {
                                if !decoded.is_ascii() {
                                    found.insert(decoded);
                                }
                            }
                            i = close + 1;
                        } else if chars.get(i + 2) == Some(&'\'') {
                            // A plain character literal, `'"'` included.
                            if let Some(literal) = next {
                                if !literal.is_ascii() {
                                    found.insert(literal);
                                }
                            }
                            i += 3;
                        } else {
                            // A lifetime.
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
                Lexeme::LineComment => {
                    if c == '\n' {
                        lexeme = Lexeme::Code;
                    }
                    i += 1;
                }
                Lexeme::BlockComment { depth } => {
                    if c == '/' && next == Some('*') {
                        lexeme = Lexeme::BlockComment { depth: depth + 1 };
                        i += 2;
                    } else if c == '*' && next == Some('/') {
                        lexeme = if depth == 1 {
                            Lexeme::Code
                        } else {
                            Lexeme::BlockComment { depth: depth - 1 }
                        };
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                Lexeme::Str => {
                    if c == '\\' {
                        if let Some((decoded, after)) = unicode_escape(&chars, i) {
                            if !decoded.is_ascii() {
                                found.insert(decoded);
                            }
                            i = after;
                        } else {
                            i += 2;
                        }
                    } else if c == '"' {
                        lexeme = Lexeme::Code;
                        i += 1;
                    } else {
                        if !c.is_ascii() {
                            found.insert(c);
                        }
                        i += 1;
                    }
                }
                Lexeme::RawStr { hashes } => {
                    let closes = c == '"'
                        && chars[i + 1..]
                            .iter()
                            .take(hashes)
                            .filter(|c| **c == '#')
                            .count()
                            == hashes;
                    if closes {
                        lexeme = Lexeme::Code;
                        i += 1 + hashes;
                    } else {
                        if !c.is_ascii() {
                            found.insert(c);
                        }
                        i += 1;
                    }
                }
            }
        }
        found
    }

    /// Whether the `r` at `at` begins a raw string: it is not the tail of an
    /// identifier, and only `#`s stand between it and a quote.
    fn raw_string_opens(chars: &[char], at: usize) -> bool {
        let tail_of_identifier =
            at > 0 && (chars[at - 1].is_alphanumeric() || chars[at - 1] == '_');
        if tail_of_identifier {
            return false;
        }
        let mut quote = at + 1;
        while chars.get(quote) == Some(&'#') {
            quote += 1;
        }
        chars.get(quote) == Some(&'"')
    }

    /// The characters a fixture source places, one in each spelling the
    /// scanner must tell apart.
    struct Placed {
        in_comments: char,
        in_string: char,
        in_raw: char,
        in_char: char,
        in_multiline: char,
        in_escape: char,
    }

    /// A source built at run time from code points, so that this file's own
    /// literals stay ASCII and the glyph test below reads it as clean.
    fn fixture(placed: &Placed) -> String {
        let Placed {
            in_comments,
            in_string,
            in_raw,
            in_char,
            in_multiline,
            in_escape,
        } = placed;
        let escape = u32::from(*in_escape);
        format!(
            "// a line comment may say {in_comments}\n\
             /* a block comment /* nested */ may say {in_comments} */\n\
             let quote = '\"'; let c = '{in_char}'; let lifetime: &'a str = \"\";\n\
             let label = \"a {in_string} b\";\n\
             let raw = r##\"{in_raw} \"# quoted \"##;\n\
             let multi = \"line one\n\
                 line two {in_multiline}\";\n\
             let escaped = \"\\u{{{escape:04X}}}\";\n"
        )
    }

    #[test]
    fn the_scanner_reads_literals_and_skips_comments() {
        let [in_comments, in_string, in_raw, in_char, in_multiline, in_escape] =
            [0x2192, 0x22EF, 0x00D7, 0x2014, 0x2026, 0x25A3].map(|code_point| {
                char::from_u32(code_point)
                    .expect("the fixture's code points are assigned characters")
            });
        let source = fixture(&Placed {
            in_comments,
            in_string,
            in_raw,
            in_char,
            in_multiline,
            in_escape,
        });
        assert_eq!(
            non_ascii_in_literals(&source),
            BTreeSet::from([in_string, in_raw, in_char, in_multiline, in_escape]),
        );
    }

    /// The characters egui's default proportional family can draw, each with
    /// the faces that supply it: the union of the family's character maps,
    /// which is the table `resolve_face` consults when the shaper cannot map
    /// a character (epaint `text_layout.rs`). `Fonts::has_glyph` is not the
    /// oracle here: it compares the owning face against the face that holds
    /// the replacement glyph, so every symbol NotoEmoji-Regular supplies comes
    /// back from it as missing.
    fn drawable_characters() -> BTreeMap<char, Vec<String>> {
        let ctx = egui::Context::default();
        // Fonts exist only once a pass has begun; the output of an empty pass
        // says nothing about glyph coverage.
        drop(ctx.run_ui(egui::RawInput::default(), |_ui| {}));
        ctx.fonts_mut(|fonts| {
            fonts
                .fonts
                .font(&egui::FontFamily::Proportional)
                .characters()
                .clone()
        })
    }

    #[test]
    fn every_non_ascii_character_in_a_shell_literal_has_a_glyph_in_the_default_font() {
        let drawable = drawable_characters();
        let mut undrawable = Vec::new();
        for (path, source) in SOURCES {
            for c in non_ascii_in_literals(source) {
                if !drawable.contains_key(&c) {
                    undrawable.push(format!("{path}: '{c}' (U+{:04X})", u32::from(c)));
                }
            }
        }
        assert!(
            undrawable.is_empty(),
            "egui's default proportional font has no glyph for these characters and draws each as a hollow box:\n{}",
            undrawable.join("\n"),
        );
    }
}
