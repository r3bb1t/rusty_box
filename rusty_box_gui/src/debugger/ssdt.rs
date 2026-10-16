//! The System Service Descriptor Table (`KeServiceDescriptorTable`), decoded.
//!
//! `KeServiceDescriptorTable` is an array of `KSERVICE_TABLE_DESCRIPTOR`, each
//! 0x20 bytes on x64: `ServiceTable` (`KiServiceTable`) at +0x00, `CounterTable`
//! at +0x08, `ServiceLimit` (a `ULONG` count) at +0x10, `ParamTableBase`
//! (`KiArgumentTable`) at +0x18. The first descriptor is the kernel's services.
//!
//! Each `KiServiceTable` entry is a 32-bit signed value: an offset from the
//! table's own base, with the low 4 bits an argument hint. Windows 7 and later
//! shift the value right by 4 for the offset (`base + (entry >> 4)`); Windows
//! XP / Server 2003 x64 mask the low nibble instead (`base + (entry & !0xF)`).
//! The right one is the one whose routines land inside the kernel image.
//!
//! Only the first x64 kernel (5.2 SP1) exports `KeServiceDescriptorTable`;
//! every later one keeps it unexported. The system-service dispatcher
//! (`KiSystemServiceRepeat`) loads its address with a RIP-relative
//! `lea r10, [KeServiceDescriptorTable]` (bytes `4C 8D 15` and a 32-bit
//! displacement from the next instruction), so the table is found as the
//! `lea r10` target in the kernel's code that reads as a descriptor whose
//! routines land inside the kernel image most often.

/// `KSERVICE_TABLE_DESCRIPTOR.ServiceTable` (`KiServiceTable`) at +0x00.
const DESCRIPTOR_SERVICE_TABLE: usize = 0x00;
/// `KSERVICE_TABLE_DESCRIPTOR.ServiceLimit` (a `ULONG`) at +0x10.
const DESCRIPTOR_SERVICE_LIMIT: usize = 0x10;
/// How many bytes of the first descriptor are read.
pub(crate) const DESCRIPTOR_BYTES: usize = 0x18;
/// More services than any Windows kernel has: a larger limit is a wrong read.
pub(crate) const MAX_SERVICES: u32 = 1 << 14;
/// `lea r10, [rip + disp32]`: REX.WR, LEA, ModRM mod=00 reg=r10 rm=101.
const LEA_R10_RIP: [u8; 3] = [0x4C, 0x8D, 0x15];
/// The opcode bytes and the 32-bit displacement.
const LEA_R10_LEN: usize = 7;

/// The first `KSERVICE_TABLE_DESCRIPTOR`: the kernel's own services.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Descriptor {
    /// `KiServiceTable`.
    pub(crate) service_table: u64,
    /// `ServiceLimit`: how many entries `KiServiceTable` holds.
    pub(crate) limit: u32,
}

/// The descriptor `bytes` (at least [`DESCRIPTOR_BYTES`] of them) hold, or
/// `None` when its limit is zero or past [`MAX_SERVICES`].
pub(crate) fn parse_descriptor(bytes: &[u8]) -> Option<Descriptor> {
    let table: [u8; 8] = bytes
        .get(DESCRIPTOR_SERVICE_TABLE..DESCRIPTOR_SERVICE_TABLE + 8)?
        .try_into()
        .ok()?;
    let limit: [u8; 4] = bytes
        .get(DESCRIPTOR_SERVICE_LIMIT..DESCRIPTOR_SERVICE_LIMIT + 4)?
        .try_into()
        .ok()?;
    let limit = u32::from_le_bytes(limit);
    (1..=MAX_SERVICES).contains(&limit).then(|| Descriptor {
        service_table: u64::from_le_bytes(table),
        limit,
    })
}

/// Adds to `out` the address every `lea r10, [rip + disp32]` in `code`
/// loads. `code` is read from linear `address`; an instruction cut off at its
/// end is passed by.
fn push_lea_r10_targets(code: &[u8], address: u64, out: &mut Vec<u64>) {
    for (at, bytes) in code.windows(LEA_R10_LEN).enumerate() {
        if bytes.get(..3) != Some(LEA_R10_RIP.as_slice()) {
            continue;
        }
        let Some(displacement) = bytes
            .get(3..LEA_R10_LEN)
            .and_then(|raw| <[u8; 4]>::try_from(raw).ok())
            .map(i32::from_le_bytes)
        else {
            continue;
        };
        // RIP-relative: from the end of the instruction. `at` indexes a
        // window of `code`, so the sum stays far below `usize::MAX`.
        out.push(
            address
                .wrapping_add(at.wrapping_add(LEA_R10_LEN) as u64)
                .wrapping_add_signed(i64::from(displacement)),
        );
    }
}

/// The `lea r10` targets in code read piece by piece. A piece that follows
/// the one before it is scanned behind that piece's last bytes, so an
/// instruction that straddles two pieces (a page boundary) is still seen; a
/// piece that does not (past a page that did not read, or in another
/// section) starts afresh.
#[derive(Default)]
pub(crate) struct LeaR10Scan {
    /// The last bytes fed (fewer than one instruction), then the piece.
    window: Vec<u8>,
    /// Where the bytes after `window` would start.
    next: Option<u64>,
    targets: Vec<u64>,
}

impl LeaR10Scan {
    /// Scans `piece`, read from linear `address`.
    pub(crate) fn feed(&mut self, piece: &[u8], address: u64) {
        if self.next != Some(address) {
            self.window.clear();
        }
        let start = address.wrapping_sub(self.window.len() as u64);
        self.window.extend_from_slice(piece);
        push_lea_r10_targets(&self.window, start, &mut self.targets);
        // Fewer bytes than one instruction: none found twice.
        let keep = self.window.len().min(LEA_R10_LEN - 1);
        self.window.drain(..self.window.len().saturating_sub(keep));
        self.next = Some(address.wrapping_add(piece.len() as u64));
    }

    /// Every target found, ascending, each once.
    pub(crate) fn targets(mut self) -> Vec<u64> {
        self.targets.sort_unstable();
        self.targets.dedup();
        self.targets
    }
}

/// The encoding `entries` are read with, and how many of their routines land
/// where `inside` holds under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Decoding {
    pub(crate) encoding: Encoding,
    pub(crate) inside: usize,
}

/// The encoding under which the most of `entries` (read from the table at
/// `base`) decode to a routine `inside` accepts; [`Encoding::Shift`] on a tie.
pub(crate) fn decode<F: Fn(u64) -> bool>(base: u64, entries: &[i32], inside: F) -> Decoding {
    let count = |encoding| {
        entries
            .iter()
            .filter(|&&entry| inside(routine_of(base, entry, encoding)))
            .count()
    };
    let shift = count(Encoding::Shift);
    let mask = count(Encoding::Mask);
    if shift >= mask {
        Decoding {
            encoding: Encoding::Shift,
            inside: shift,
        }
    } else {
        Decoding {
            encoding: Encoding::Mask,
            inside: mask,
        }
    }
}

/// How a build encodes a `KiServiceTable` entry into an offset from the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoding {
    /// Windows 7 and later: the offset is the entry shifted right by 4.
    Shift,
    /// Windows XP / Server 2003 x64: the offset is the entry's low nibble
    /// masked off.
    Mask,
}

/// The routine a `KiServiceTable` entry points to: `base` plus the offset the
/// `encoding` reads from `entry`. The offset is signed: a routine may lie
/// before the table.
pub(crate) fn routine_of(base: u64, entry: i32, encoding: Encoding) -> u64 {
    let offset = match encoding {
        Encoding::Shift => i64::from(entry >> 4),
        Encoding::Mask => i64::from(entry & !0xF),
    };
    base.wrapping_add_signed(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shift_encoding_matches_a_known_x64_entry() {
        // From a real x64 kernel: KiServiceTable at 0x1403FE450, entry 4 is
        // 0x02A07C00, which decodes to NtWaitForSingleObject at 0x14069EC10.
        assert_eq!(
            routine_of(0x1_403F_E450, 0x02A0_7C00u32 as i32, Encoding::Shift),
            0x1_4069_EC10
        );
    }

    #[test]
    fn the_mask_encoding_keeps_the_offset_unshifted() {
        assert_eq!(routine_of(0x1000, 0x0000_0024, Encoding::Mask), 0x1020);
        assert_eq!(routine_of(0x1000, 0x0000_0024, Encoding::Shift), 0x1002);
    }

    #[test]
    fn an_offset_can_point_before_the_table() {
        // A negative 32-bit entry decodes to a routine below the base.
        let entry = -0x100i32 << 4; // shifts back to -0x100
        assert_eq!(routine_of(0x1_0000, entry, Encoding::Shift), 0xFF00);
    }

    #[test]
    fn a_lea_r10_target_is_relative_to_the_next_instruction() {
        // From a real x64 kernel's KiSystemServiceRepeat: at FFFFF800`03C73FF2,
        // `4C 8D 15 47 78 23 00` is lea r10, [FFFFF800`03EAB840], followed by
        // the lea r11 of KeServiceDescriptorTableShadow.
        let code = [
            0x90, 0x4C, 0x8D, 0x15, 0x47, 0x78, 0x23, 0x00, 0x4C, 0x8D, 0x1D, 0x80, 0x78, 0x23, 0x00,
        ];
        let mut scan = LeaR10Scan::default();
        scan.feed(&code, 0xFFFF_F800_03C7_3FF1);
        assert_eq!(scan.targets(), [0xFFFF_F800_03EA_B840]);
    }

    #[test]
    fn a_lea_r10_cut_off_at_the_end_is_passed_by() {
        let mut scan = LeaR10Scan::default();
        scan.feed(&[0x4C, 0x8D, 0x15, 0x47, 0x78, 0x23], 0x1000);
        assert!(scan.targets().is_empty());
    }

    #[test]
    fn a_lea_r10_straddling_two_pieces_is_found_once() {
        // lea r10, [rip + 0x10] at 0x1FFD: it ends at 0x2004, loads 0x2014.
        let mut scan = LeaR10Scan::default();
        scan.feed(&[0x90, 0x90, 0x90, 0x4C, 0x8D, 0x15], 0x1FFA);
        scan.feed(&[0x10, 0x00, 0x00, 0x00, 0x90, 0x90, 0x90], 0x2000);
        scan.feed(&[0x90; 8], 0x2007);
        assert_eq!(scan.targets(), [0x2014]);
    }

    #[test]
    fn a_piece_that_does_not_follow_the_last_starts_afresh() {
        // The second piece is not at 0x2000: the first one's tail is not its.
        let mut scan = LeaR10Scan::default();
        scan.feed(&[0x90, 0x90, 0x90, 0x4C, 0x8D, 0x15], 0x1FFA);
        scan.feed(&[0x10, 0x00, 0x00, 0x00, 0x90, 0x90, 0x90], 0x3000);
        assert!(scan.targets().is_empty());
    }

    #[test]
    fn a_descriptor_with_no_services_or_too_many_is_no_descriptor() {
        let mut bytes = [0u8; DESCRIPTOR_BYTES];
        bytes[..8].copy_from_slice(&0xFFFF_F800_0100_0000u64.to_le_bytes());
        assert_eq!(parse_descriptor(&bytes), None);
        bytes[0x10..0x14].copy_from_slice(&0x128u32.to_le_bytes());
        assert_eq!(
            parse_descriptor(&bytes),
            Some(Descriptor {
                service_table: 0xFFFF_F800_0100_0000,
                limit: 0x128,
            })
        );
        bytes[0x10..0x14].copy_from_slice(&(MAX_SERVICES + 1).to_le_bytes());
        assert_eq!(parse_descriptor(&bytes), None);
    }

    #[test]
    fn the_decoding_is_the_encoding_that_lands_most_routines_inside() {
        let base = 0x10_0000;
        let inside = |routine: u64| (0x10_0800..0x10_1000).contains(&routine);
        // XP x64 entries: offset plus an argument hint in the low nibble.
        // Masked they are 0x900, 0xA00, 0xB00 (inside); shifted, 0x90, 0xA0,
        // 0xB0 (outside); the last lands outside either way.
        let entries = [0x903, 0xA04, 0xB05, 0x0001_0005];
        assert_eq!(
            decode(base, &entries, inside),
            Decoding {
                encoding: Encoding::Mask,
                inside: 3,
            }
        );
        // Win7+ entries: offset << 4 plus an argument hint.
        let entries = [0x9003, 0xA004, 0xB005];
        assert_eq!(
            decode(base, &entries, inside),
            Decoding {
                encoding: Encoding::Shift,
                inside: 3,
            }
        );
    }

    /// A descriptor cut short reads as none, at every length; a table at
    /// the top of memory names its routines by wrapping, as the processor
    /// adds.
    #[test]
    fn a_descriptor_cut_short_or_a_table_at_the_top_of_memory_reads_safely() {
        let mut bytes = vec![0u8; DESCRIPTOR_SERVICE_LIMIT + 4];
        bytes[DESCRIPTOR_SERVICE_LIMIT..].copy_from_slice(&10u32.to_le_bytes());
        assert!(parse_descriptor(&bytes).is_some());
        for len in 0..bytes.len() {
            assert!(parse_descriptor(&bytes[..len]).is_none(), "{len} bytes");
        }
        assert_eq!(routine_of(u64::MAX, 0x10, Encoding::Shift), 0);
        assert_eq!(routine_of(0, -0x10, Encoding::Mask), u64::MAX - 0xF);
    }
}
