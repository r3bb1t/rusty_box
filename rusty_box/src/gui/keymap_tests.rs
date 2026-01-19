//! Tests for keyboard scancode mapping
//!
//! These tests verify that the scancode mappings are correct
//! according to PS/2 scancode set 2 standard.

#[cfg(test)]
mod tests {
    use crate::gui::keymap::{ascii_to_scancode, char_to_scancode_sequence, needs_shift};

    /// Test basic letter mappings (verify they return Some)
    #[test]
    fn test_letters_have_scancodes() {
        for ch in 'a'..='z' {
            assert!(
                ascii_to_scancode(ch).is_some(),
                "Letter '{}' should have a scancode",
                ch
            );
        }

        for ch in 'A'..='Z' {
            assert!(
                ascii_to_scancode(ch).is_some(),
                "Letter '{}' should have a scancode",
                ch
            );
        }
    }

    /// Test that uppercase letters need shift
    #[test]
    fn test_uppercase_needs_shift() {
        for ch in 'A'..='Z' {
            assert!(
                needs_shift(ch),
                "Uppercase letter '{}' should need shift",
                ch
            );
        }

        for ch in 'a'..='z' {
            assert!(
                !needs_shift(ch),
                "Lowercase letter '{}' should not need shift",
                ch
            );
        }
    }

    /// Test special keys
    #[test]
    fn test_special_keys() {
        // Enter key
        assert_eq!(ascii_to_scancode('\n'), Some((0x5A, 0xF0)));
        assert_eq!(ascii_to_scancode('\r'), Some((0x5A, 0xF0)));

        // Backspace
        assert_eq!(ascii_to_scancode('\x08'), Some((0x66, 0xF0)));

        // Escape
        assert_eq!(ascii_to_scancode('\x1B'), Some((0x76, 0xF0)));

        // Space
        assert_eq!(ascii_to_scancode(' '), Some((0x29, 0xF0)));

        // Tab
        assert_eq!(ascii_to_scancode('\t'), Some((0x0D, 0xF0)));
    }

    /// Test scancode sequence generation for lowercase
    #[test]
    fn test_lowercase_sequence() {
        // Lowercase 'a' should generate: [make, break_prefix, make]
        let seq = char_to_scancode_sequence('a');
        assert!(!seq.is_empty(), "Lowercase 'a' should generate scancodes");
        // Should start with make code
        assert_eq!(seq[0], 0x1C, "First scancode should be make code for 'a'");
    }

    /// Test scancode sequence generation for uppercase
    #[test]
    fn test_uppercase_sequence() {
        // Uppercase 'A' should generate: [shift_make, char_make, char_break, shift_break]
        let seq = char_to_scancode_sequence('A');
        assert!(!seq.is_empty(), "Uppercase 'A' should generate scancodes");
        // Should start with shift make (0x12)
        assert_eq!(seq[0], 0x12, "First scancode should be shift make");
    }

    /// Test that numbers don't need shift (but shifted versions do)
    #[test]
    fn test_numbers_shift() {
        // Numbers don't need shift
        for ch in '0'..='9' {
            assert!(!needs_shift(ch), "Number '{}' should not need shift", ch);
        }

        // Shifted number symbols need shift
        assert!(needs_shift('!'));
        assert!(needs_shift('@'));
        assert!(needs_shift('#'));
    }

    /// Test that unknown characters return None
    #[test]
    fn test_unknown_characters() {
        // Some unicode characters should return None
        assert_eq!(ascii_to_scancode('€'), None);
        assert_eq!(ascii_to_scancode('α'), None);
    }

    /// Test scancode sequence format
    #[test]
    fn test_scancode_sequence_format() {
        // A lowercase letter should produce: [make, 0xF0, make]
        let seq = char_to_scancode_sequence('a');
        assert!(seq.len() >= 3, "Lowercase should produce at least 3 scancodes");
        assert_eq!(seq[1], 0xF0, "Second should be break prefix");
    }
}
