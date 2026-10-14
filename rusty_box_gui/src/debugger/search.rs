//! Searching a paused VM's memory for bytes: a hex pattern, text, or text as
//! Windows stores it (UTF-16LE). The search reads RAM only, never a device
//! window, since a device read changes the device.

use rusty_box::cpu::Instrumentation;
use rusty_box::emulator::{Emulator, RamOffset, SliceEngine};

use super::inspect::{AddressSpace, GuestAddress};

/// How a search's text becomes the bytes it looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchEncoding {
    /// Pairs of hex digits, spaces between them optional: `48 8B 05`.
    Hex,
    /// The text's UTF-8 bytes, which for ASCII are the characters.
    Text,
    /// The text as UTF-16LE, the form Windows keeps most strings in.
    Utf16,
}

/// What a search's text could not be read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatternError {
    Empty,
    NotHex,
    OddHex,
}

impl PatternError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Empty => "Type what to find.",
            Self::NotHex => "Hex is pairs of hexadecimal digits, like 48 8B 05.",
            Self::OddHex => "Hex needs an even number of digits.",
        }
    }
}

/// The bytes `text` stands for under `encoding`.
pub(crate) fn pattern_of(encoding: SearchEncoding, text: &str) -> Result<Vec<u8>, PatternError> {
    let pattern = match encoding {
        SearchEncoding::Hex => {
            let digits: Vec<u8> = text
                .bytes()
                .filter(|byte| !byte.is_ascii_whitespace())
                .collect();
            if !digits.iter().all(u8::is_ascii_hexdigit) {
                return Err(PatternError::NotHex);
            }
            if digits.len() % 2 != 0 {
                return Err(PatternError::OddHex);
            }
            digits
                .chunks(2)
                .map(|pair| {
                    let high = char::from(pair[0]).to_digit(16).unwrap_or(0);
                    let low = char::from(pair[1]).to_digit(16).unwrap_or(0);
                    (high * 16 + low) as u8
                })
                .collect()
        }
        SearchEncoding::Text => text.as_bytes().to_vec(),
        SearchEncoding::Utf16 => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
    };
    if pattern.is_empty() {
        return Err(PatternError::Empty);
    }
    Ok(pattern)
}

/// A search: `pattern`, looked for from `from` onwards in `from`'s space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FindRequest {
    pub(crate) from: GuestAddress,
    pub(crate) pattern: Vec<u8>,
}

/// Where a search hit, and how many bytes the hit covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FoundAt {
    pub(crate) at: GuestAddress,
    pub(crate) len: u64,
}

/// What a search came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindOutcome {
    Found(FoundAt),
    /// Nothing between the start and `searched_to`.
    NotFound { searched_to: GuestAddress },
    /// The VM was told to stop while the search ran.
    Abandoned,
}

/// How far past its start a search of linear memory looks: linear address
/// spaces are sparse, so the walk is bounded rather than run to 2^64.
pub(crate) const LINEAR_SEARCH_SPAN: u64 = 1 << 30;

/// How much RAM is read at once.
const SEARCH_CHUNK: usize = 1 << 20;

const PAGE_BYTES: u64 = 0x1000;

/// Looks for `request.pattern` from `request.from` onwards, giving up once
/// `abandoned` answers true. Linear addresses are walked through processor
/// `cpu`'s page tables.
pub(crate) fn find<T: Instrumentation, E: SliceEngine<T>, F: Fn() -> bool>(
    emu: &mut Emulator<T, E>,
    cpu: usize,
    request: &FindRequest,
    abandoned: F,
) -> FindOutcome {
    match request.from.space {
        AddressSpace::Physical => find_physical(emu, request, abandoned),
        AddressSpace::Linear | AddressSpace::Process { .. } => {
            find_linear(emu, cpu, request.from.space, request, abandoned)
        }
    }
}

/// Where `address` maps in `space`: the processor's own tables for `Linear`,
/// a given CR3's for `Process`.
fn translate_in<T: Instrumentation, E: SliceEngine<T>>(
    emu: &mut Emulator<T, E>,
    cpu: usize,
    space: AddressSpace,
    address: u64,
) -> Option<u64> {
    let mut debug = emu.debug_cpu(cpu)?;
    match space {
        AddressSpace::Linear | AddressSpace::Physical => debug.dbg_xlate_linear2phy(address),
        AddressSpace::Process { cr3 } => debug.dbg_xlate_linear2phy_in(cr3, address),
    }
}

fn position(haystack: &[u8], pattern: &[u8]) -> Option<usize> {
    haystack
        .windows(pattern.len())
        .position(|window| window == pattern)
}

/// Bytes seen so far that a hit may still start in: the newest bytes, and
/// where the oldest of them sits.
struct Window {
    bytes: Vec<u8>,
    start: u64,
}

impl Window {
    /// Keeps only the bytes a hit not yet found could start in.
    fn keep_tail(&mut self, pattern_len: usize) {
        let drop = self.bytes.len().saturating_sub(pattern_len.saturating_sub(1));
        drop_front(&mut self.bytes, drop);
        self.start = self.start.wrapping_add(drop as u64);
    }
}

fn drop_front(bytes: &mut Vec<u8>, count: usize) {
    let kept = bytes.split_off(count.min(bytes.len()));
    *bytes = kept;
}

fn find_physical<T: Instrumentation, E: SliceEngine<T>, F: Fn() -> bool>(
    emu: &mut Emulator<T, E>,
    request: &FindRequest,
    abandoned: F,
) -> FindOutcome {
    let total = emu.mem_size() as u64;
    let RamOffset(mut offset) = RamOffset::at_or_after_physical(request.from.value);
    let mut window = Window {
        bytes: Vec::new(),
        start: offset,
    };
    let mut chunk = vec![0u8; SEARCH_CHUNK];
    while offset < total {
        if abandoned() {
            return FindOutcome::Abandoned;
        }
        let len = usize::try_from(total - offset).map_or(SEARCH_CHUNK, |left| left.min(SEARCH_CHUNK));
        let buf = &mut chunk[..len];
        if let Err(error) = emu.read_ram_image(RamOffset(offset), buf) {
            tracing::warn!("memory search stopped at RAM offset {offset:#x}: {error}");
            break;
        }
        window.bytes.extend_from_slice(buf);
        if let Some(hit) = position(&window.bytes, &request.pattern) {
            return FindOutcome::Found(FoundAt {
                at: GuestAddress {
                    space: AddressSpace::Physical,
                    value: RamOffset(window.start + hit as u64).physical(),
                },
                len: request.pattern.len() as u64,
            });
        }
        window.keep_tail(request.pattern.len());
        offset += len as u64;
    }
    FindOutcome::NotFound {
        searched_to: GuestAddress {
            space: AddressSpace::Physical,
            value: RamOffset(offset).physical(),
        },
    }
}

fn find_linear<T: Instrumentation, E: SliceEngine<T>, F: Fn() -> bool>(
    emu: &mut Emulator<T, E>,
    cpu: usize,
    space: AddressSpace,
    request: &FindRequest,
    abandoned: F,
) -> FindOutcome {
    let total = emu.mem_size() as u64;
    let end = request.from.value.saturating_add(LINEAR_SEARCH_SPAN);
    let mut address = request.from.value;
    let mut window = Window {
        bytes: Vec::new(),
        start: address,
    };
    let mut page = vec![0u8; PAGE_BYTES as usize];
    while address < end {
        if abandoned() {
            return FindOutcome::Abandoned;
        }
        let page_end = (address & !(PAGE_BYTES - 1)).saturating_add(PAGE_BYTES).min(end);
        let len = (page_end - address) as usize;
        // RAM only: an unmapped page, or one that maps a device window or
        // lies past RAM, breaks the run of bytes a hit could span.
        let ram = translate_in(emu, cpu, space, address)
            .and_then(RamOffset::of_physical)
            .filter(|RamOffset(offset)| offset.saturating_add(len as u64) <= total);
        let read = match ram {
            Some(offset) => match emu.read_ram_image(offset, &mut page[..len]) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!("memory search skipped {address:#x}: {error}");
                    false
                }
            },
            None => false,
        };
        if read {
            if window.bytes.is_empty() {
                window.start = address;
            }
            window.bytes.extend_from_slice(&page[..len]);
            if let Some(hit) = position(&window.bytes, &request.pattern) {
                return FindOutcome::Found(FoundAt {
                    at: GuestAddress {
                        space,
                        value: window.start.wrapping_add(hit as u64),
                    },
                    len: request.pattern.len() as u64,
                });
            }
            window.keep_tail(request.pattern.len());
        } else {
            window.bytes.clear();
        }
        if page_end == address {
            break;
        }
        address = page_end;
    }
    FindOutcome::NotFound {
        searched_to: GuestAddress {
            space,
            value: address,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box::cpu::{CpuSetupMode, X86Reg};
    use rusty_box::emulator::EmulatorConfig;

    #[test]
    fn a_search_reads_hex_text_and_utf16() {
        assert_eq!(pattern_of(SearchEncoding::Hex, "48 8b 05"), Ok(vec![0x48, 0x8B, 0x05]));
        assert_eq!(pattern_of(SearchEncoding::Hex, "488B05"), Ok(vec![0x48, 0x8B, 0x05]));
        assert_eq!(pattern_of(SearchEncoding::Hex, "48 8"), Err(PatternError::OddHex));
        assert_eq!(pattern_of(SearchEncoding::Hex, "zz"), Err(PatternError::NotHex));
        assert_eq!(pattern_of(SearchEncoding::Hex, "  "), Err(PatternError::Empty));
        assert_eq!(pattern_of(SearchEncoding::Text, "XP"), Ok(b"XP".to_vec()));
        assert_eq!(pattern_of(SearchEncoding::Utf16, "XP"), Ok(vec![b'X', 0, b'P', 0]));
        assert_eq!(pattern_of(SearchEncoding::Text, ""), Err(PatternError::Empty));
    }

    fn long_mode_machine() -> Box<Emulator> {
        Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
            .expect("machine")
    }

    #[test]
    fn a_physical_search_finds_text_across_a_chunk_edge() {
        let mut machine = long_mode_machine();
        // Straddles the first 1 MiB chunk's end.
        let at = (1 << 20) - 3;
        machine.mem_write(at, b"needle").expect("write");
        let outcome = find(
            &mut machine,
            0,
            &FindRequest {
                from: GuestAddress { space: AddressSpace::Physical, value: 0x1000 },
                pattern: b"needle".to_vec(),
            },
            || false,
        );
        assert_eq!(
            outcome,
            FindOutcome::Found(FoundAt {
                at: GuestAddress { space: AddressSpace::Physical, value: at },
                len: 6,
            })
        );
    }

    #[test]
    fn a_linear_search_finds_bytes_through_the_page_tables() {
        let mut machine = long_mode_machine();
        let at = CpuSetupMode::FlatLong64.first_free_physical_address() + 0x2345;
        machine.mem_write(at, &[0xDE, 0xAD, 0xBE, 0xEF]).expect("write");
        machine.reg_write(X86Reg::Rip, at);
        let outcome = find(
            &mut machine,
            0,
            &FindRequest {
                from: GuestAddress { space: AddressSpace::Linear, value: at & !0xFFF },
                pattern: vec![0xDE, 0xAD, 0xBE, 0xEF],
            },
            || false,
        );
        assert_eq!(
            outcome,
            FindOutcome::Found(FoundAt {
                at: GuestAddress { space: AddressSpace::Linear, value: at },
                len: 4,
            })
        );
    }

    #[test]
    fn a_search_past_the_last_hit_says_where_it_stopped() {
        let mut machine = long_mode_machine();
        let outcome = find(
            &mut machine,
            0,
            &FindRequest {
                from: GuestAddress { space: AddressSpace::Physical, value: 0 },
                pattern: b"no such bytes anywhere in a fresh machine".to_vec(),
            },
            || false,
        );
        assert!(matches!(outcome, FindOutcome::NotFound { .. }), "{outcome:?}");
    }

    #[test]
    fn a_search_the_vm_stops_during_gives_up() {
        let mut machine = long_mode_machine();
        let outcome = find(
            &mut machine,
            0,
            &FindRequest {
                from: GuestAddress { space: AddressSpace::Physical, value: 0 },
                pattern: b"x".to_vec(),
            },
            || true,
        );
        assert_eq!(outcome, FindOutcome::Abandoned);
    }
}
