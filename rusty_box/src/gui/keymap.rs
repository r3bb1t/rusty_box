//! Keyboard Scancode Mapping
//!
//! Maps ASCII characters and special keys to PS/2 scancode set 2.
//! Based on standard PC/AT keyboard scancode set 2.

/// PS/2 Scancode Set 2 mapping for ASCII characters
/// Returns (make_code, break_code) for a given ASCII character
/// Break code is 0xF0 followed by the make code
pub fn ascii_to_scancode(ch: char) -> Option<(u8, u8)> {
    let make_code = match ch {
        // Letters (lowercase and uppercase use same scancode, shift is handled separately)
        // PS/2 Scancode Set 2 mappings
        'a' | 'A' => Some(0x1C),
        'b' | 'B' => Some(0x32),
        'c' | 'C' => Some(0x21),
        'd' | 'D' => Some(0x23),
        'e' | 'E' => Some(0x24),
        'f' | 'F' => Some(0x2B),
        'g' | 'G' => Some(0x34),
        'h' | 'H' => Some(0x33),
        'i' | 'I' => Some(0x43),
        'j' | 'J' => Some(0x3B),
        'k' | 'K' => Some(0x42),
        'l' | 'L' => Some(0x4B),
        'm' | 'M' => Some(0x3A),
        'n' | 'N' => Some(0x31),
        'o' | 'O' => Some(0x44),
        'p' | 'P' => Some(0x4D),
        'q' | 'Q' => Some(0x15),
        'r' | 'R' => Some(0x2D),
        's' | 'S' => Some(0x1B),
        't' | 'T' => Some(0x2C),
        'u' | 'U' => Some(0x3C),
        'v' | 'V' => Some(0x2A),
        'w' | 'W' => Some(0x1D),
        'x' | 'X' => Some(0x22),
        'y' | 'Y' => Some(0x35),
        'z' | 'Z' => Some(0x1A),
        
        // Numbers (top row)
        '1' | '!' => Some(0x16),
        '2' | '@' => Some(0x1E),
        '3' | '#' => Some(0x26),
        '4' | '$' => Some(0x25),
        '5' | '%' => Some(0x2E),
        '6' | '^' => Some(0x36),
        '7' | '&' => Some(0x3D),
        '8' | '*' => Some(0x3E),
        '9' | '(' => Some(0x46),
        '0' | ')' => Some(0x45),
        
        // Special characters
        '-' | '_' => Some(0x4E),
        '=' | '+' => Some(0x55),
        '[' | '{' => Some(0x54),
        ']' | '}' => Some(0x5B),
        '\\' | '|' => Some(0x5D),
        ';' | ':' => Some(0x4C),
        '\'' | '"' => Some(0x52),
        '`' | '~' => Some(0x0E),
        ',' | '<' => Some(0x41),
        '.' | '>' => Some(0x49),
        '/' | '?' => Some(0x4A),
        ' ' => Some(0x29), // Space
        
        // Control characters
        '\n' | '\r' => Some(0x5A), // Enter
        '\t' => Some(0x0D), // Tab
        '\x08' => Some(0x66), // Backspace
        '\x1B' => Some(0x76), // Escape
        
        _ => None,
    };
    
    make_code.map(|make| (make, 0xF0)) // Break code prefix
}

/// Check if a character needs shift modifier
pub fn needs_shift(ch: char) -> bool {
    match ch {
        'A'..='Z' => true,
        '!' | '@' | '#' | '$' | '%' | '^' | '&' | '*' | '(' | ')' => true,
        '_' | '+' | '{' | '}' | '|' | ':' | '"' | '~' | '<' | '>' | '?' => true,
        _ => false,
    }
}

/// Convert a character to scancode sequence (including shift if needed)
/// Returns a vector of scancodes: [shift_make, char_make, char_break, shift_break]
/// or [char_make, char_break] if no shift needed
pub fn char_to_scancode_sequence(ch: char) -> Vec<u8> {
    if let Some((make, break_prefix)) = ascii_to_scancode(ch) {
        let mut sequence = Vec::new();
        
        if needs_shift(ch) {
            // Press shift
            sequence.push(0x12); // Left shift make
        }
        
        // Press key
        sequence.push(make);
        
        // Release key
        sequence.push(break_prefix);
        sequence.push(make);
        
        if needs_shift(ch) {
            // Release shift
            sequence.push(0xF0); // Break prefix
            sequence.push(0x12); // Left shift break
        }
        
        sequence
    } else {
        Vec::new()
    }
}

#[cfg(test)]
#[path = "keymap_tests.rs"]
mod tests;
