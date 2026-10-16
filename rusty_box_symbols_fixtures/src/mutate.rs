//! Deterministic corruption for the corruption test (spec §12): every
//! variant of a fixture, numbered, so a failing case is reproduced by its
//! fixture and number alone. A variant is a bit flip at a seeded position,
//! a truncation, or an 8-, 16-, 32- or 64-bit little-endian field set to 0,
//! to its width's maximum, or to one past its own value: a count or length
//! that sits at its bound goes just past it.

use std::fmt;

/// The seed every variant list is drawn from (mixed with the file's length).
pub const SEED: u64 = 0x5EED_C022_0F7E_0001;

/// Bytes at the start of a file whose every truncation and every aligned
/// field is varied: the headers and directories each format keeps there.
pub const HEADER_SPAN: usize = 512;

/// How many variants of each kind a fixture gets past its header span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub flips: usize,
    pub truncations: usize,
    /// Seeded positions past `HEADER_SPAN`, per field width.
    pub field_positions: usize,
}

impl Budget {
    /// For a file a reader parses in microseconds.
    pub const STANDARD: Budget = Budget { flips: 2_000, truncations: 1_000, field_positions: 1_000 };
    /// For a file each of whose variants decodes up to a quarter megabyte
    /// (the LZX cabinets).
    pub const LARGE_OUTPUT: Budget = Budget { flips: 500, truncations: 256, field_positions: 128 };
}

/// SplitMix64 (Steele, Lea and Flood, 2014): fixed by its definition, so a
/// seed draws the same positions on every machine and toolchain.
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..bound`; `bound` is not 0.
    pub fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

/// A field's width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    U8,
    U16,
    U32,
    U64,
}

impl Width {
    pub const ALL: [Width; 4] = [Width::U8, Width::U16, Width::U32, Width::U64];

    pub fn bytes(self) -> usize {
        match self {
            Width::U8 => 1,
            Width::U16 => 2,
            Width::U32 => 4,
            Width::U64 => 8,
        }
    }

    pub fn max(self) -> u64 {
        match self {
            Width::U8 => 0xFF,
            Width::U16 => 0xFFFF,
            Width::U32 => 0xFFFF_FFFF,
            Width::U64 => u64::MAX,
        }
    }
}

/// What a field is set to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Zero,
    Max,
    /// One past the field's own value, wrapping within its width.
    PastOwn,
}

impl FieldValue {
    pub const ALL: [FieldValue; 3] = [FieldValue::Zero, FieldValue::Max, FieldValue::PastOwn];
}

/// One change to a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    FlipBit { at: usize, bit: u8 },
    Truncate { len: usize },
    SetField { at: usize, width: Width, value: FieldValue },
}

impl Mutation {
    /// `original` with this change made.
    pub fn apply(self, original: &[u8]) -> Vec<u8> {
        let mut bytes = original.to_vec();
        match self {
            Mutation::FlipBit { at, bit } => bytes[at] ^= 1 << bit,
            Mutation::Truncate { len } => bytes.truncate(len),
            Mutation::SetField { at, width, value } => {
                let field = &mut bytes[at..at + width.bytes()];
                let mut own = [0u8; 8];
                own[..field.len()].copy_from_slice(field);
                let new = match value {
                    FieldValue::Zero => 0,
                    FieldValue::Max => width.max(),
                    FieldValue::PastOwn => u64::from_le_bytes(own).wrapping_add(1) & width.max(),
                };
                field.copy_from_slice(&new.to_le_bytes()[..width.bytes()]);
            }
        }
        bytes
    }
}

impl fmt::Display for Mutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mutation::FlipBit { at, bit } => write!(f, "flip bit {bit} of byte {at:#x}"),
            Mutation::Truncate { len } => write!(f, "truncate to {len} bytes"),
            Mutation::SetField { at, width, value } => write!(f, "set the {width:?} at {at:#x} to {value:?}"),
        }
    }
}

/// Every variant of a file of `len` bytes, in a fixed order: the seeded
/// flips; every truncation inside the header span, then `truncations`
/// evenly spaced ones and the last byte cut; then, per width, every aligned
/// field inside the header span and `field_positions` seeded aligned fields
/// past it, each set to every `FieldValue`.
pub fn variants(len: usize, budget: Budget) -> Vec<Mutation> {
    let mut out = Vec::new();
    if len == 0 {
        return out;
    }
    let mut random = SplitMix64::new(SEED ^ len as u64);
    for _ in 0..budget.flips {
        let at = random.below(len);
        let bit = random.below(8) as u8;
        out.push(Mutation::FlipBit { at, bit });
    }
    let header = len.min(HEADER_SPAN);
    for cut in 0..header {
        out.push(Mutation::Truncate { len: cut });
    }
    if len > header {
        let spaced = budget.truncations.max(1);
        for step in 0..spaced {
            out.push(Mutation::Truncate { len: header + (len - header) * step / spaced });
        }
        out.push(Mutation::Truncate { len: len - 1 });
    }
    for width in Width::ALL {
        let size = width.bytes();
        if len < size {
            continue;
        }
        // The last offset a field of this width can start at.
        let last = len - size;
        let mut positions: Vec<usize> = (0..header.min(last + 1)).step_by(size).collect();
        if last >= header {
            let aligned_past_header = (last - header) / size + 1;
            for _ in 0..budget.field_positions.min(aligned_past_header) {
                positions.push(header + random.below(aligned_past_header) * size);
            }
        }
        for at in positions {
            for value in FieldValue::ALL {
                out.push(Mutation::SetField { at, width, value });
            }
        }
    }
    out
}
