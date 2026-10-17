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
//!
//! `KeServiceDescriptorTableShadow`, loaded right after with
//! `lea r11, [rip + disp32]` (`4C 8D 1D`), repeats the kernel's descriptor
//! and then holds the GUI's: `W32pServiceTable`, in `win32k`, encoded as the
//! kernel's table is.
//!
//! A user-mode system-call stub (`ntdll.dll`'s `Nt*`, `win32u.dll`'s
//! `NtUser*`/`NtGdi*`) begins `mov r10, rcx; mov eax, index`; an index with
//! bit 12 set is entry `index & 0xFFF` of the GUI table. A hook that leaves
//! the table alone overwrites a routine's first instruction with a jump.

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
/// `lea r11, [rip + disp32]`: REX.WR, LEA, ModRM mod=00 reg=r11 rm=101.
const LEA_R11_RIP: [u8; 3] = [0x4C, 0x8D, 0x1D];
/// The opcode bytes and the 32-bit displacement.
const LEA_RIP_LEN: usize = 7;
/// Descriptors in `KeServiceDescriptorTableShadow` are this far apart.
const DESCRIPTOR_STRIDE: usize = 0x20;
/// The bytes read of the shadow array: its first descriptor whole, and the
/// second up to its limit.
pub(crate) const SHADOW_BYTES: usize = DESCRIPTOR_STRIDE + DESCRIPTOR_BYTES;
/// `mov r10, rcx; mov eax, imm32`: how every x64 system-call stub begins.
const STUB_PREFIX: [u8; 4] = [0x4C, 0x8B, 0xD1, 0xB8];
/// The stub prefix and its 32-bit index.
pub(crate) const STUB_BYTES: usize = 8;
/// A GUI service's index has bit 12 set; the kernel's are below it.
const GUI_TABLE_BIT: u32 = 0x1000;
/// The bytes a routine's first jump is read from: the longest form,
/// `mov rax, imm64; jmp rax`.
pub(crate) const JUMP_BYTES: usize = 12;

/// The service a system-call stub calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StubService {
    /// Entry `n` of `KiServiceTable`.
    Kernel(u32),
    /// Entry `n` of `W32pServiceTable`.
    Gui(u32),
}

/// The service the system-call stub at the start of `code` loads into
/// `eax`, when `code` begins as one (`ntdll`'s `Nt*`, `win32u`'s exports).
pub(crate) fn stub_service(code: &[u8]) -> Option<StubService> {
    if code.get(..STUB_PREFIX.len())? != STUB_PREFIX.as_slice() {
        return None;
    }
    let index = u32::from_le_bytes(code.get(STUB_PREFIX.len()..STUB_BYTES)?.try_into().ok()?);
    match index {
        0..GUI_TABLE_BIT => Some(StubService::Kernel(index)),
        GUI_TABLE_BIT..0x2000 => Some(StubService::Gui(index & (GUI_TABLE_BIT - 1))),
        _ => None,
    }
}

/// Where a routine's first instruction jumps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FirstJump {
    /// Straight to this address.
    To(u64),
    /// To the address held in the 8-byte slot here.
    Through(u64),
}

/// The jump `code`, read from linear `at`, begins with, in the forms a hook
/// writes over a routine: `jmp rel32` (`E9`), `jmp [rip+disp32]` (`FF 25`),
/// `mov rax, imm64; jmp rax` (`48 B8 … FF E0`), `push imm32; ret`
/// (`68 … C3`, the push sign-extended). `None` for anything else.
pub(crate) fn first_jump(code: &[u8], at: u64) -> Option<FirstJump> {
    let le32 = |from: usize| -> Option<i32> { Some(i32::from_le_bytes(code.get(from..from.checked_add(4)?)?.try_into().ok()?)) };
    match code {
        [0xE9, ..] => Some(FirstJump::To(at.wrapping_add(5).wrapping_add_signed(i64::from(le32(1)?)))),
        [0xFF, 0x25, ..] => Some(FirstJump::Through(at.wrapping_add(6).wrapping_add_signed(i64::from(le32(2)?)))),
        [0x48, 0xB8, ..] => {
            if code.get(10..12)? != [0xFF, 0xE0].as_slice() {
                return None;
            }
            Some(FirstJump::To(u64::from_le_bytes(code.get(2..10)?.try_into().ok()?)))
        }
        [0x68, ..] => {
            if *code.get(5)? != 0xC3 {
                return None;
            }
            Some(FirstJump::To(i64::from(le32(1)?) as u64))
        }
        _ => None,
    }
}

/// The GUI table's descriptor in the shadow array `bytes` (at least
/// [`SHADOW_BYTES`] of them), when the array's first descriptor is `kernel`:
/// the shadow repeats the kernel table's descriptor before its own.
pub(crate) fn shadow_gui_descriptor(bytes: &[u8], kernel: Descriptor) -> Option<Descriptor> {
    if parse_descriptor(bytes)? != kernel {
        return None;
    }
    known_shadow_gui_descriptor(bytes)
}

/// The GUI table's descriptor in `bytes` (at least [`SHADOW_BYTES`] of
/// them), an array already known to be the shadow: its second descriptor.
pub(crate) fn known_shadow_gui_descriptor(bytes: &[u8]) -> Option<Descriptor> {
    parse_descriptor(bytes.get(DESCRIPTOR_STRIDE..SHADOW_BYTES)?)
}

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

/// Adds to `r10` and `r11` the address every `lea r10/r11, [rip + disp32]`
/// in `code` loads. `code` is read from linear `address`; an instruction cut
/// off at its end is passed by.
fn push_lea_rip_targets(code: &[u8], address: u64, r10: &mut Vec<u64>, r11: &mut Vec<u64>) {
    for (at, bytes) in code.windows(LEA_RIP_LEN).enumerate() {
        let out = match bytes.get(..3) {
            Some(opcode) if opcode == LEA_R10_RIP.as_slice() => &mut *r10,
            Some(opcode) if opcode == LEA_R11_RIP.as_slice() => &mut *r11,
            Some(_) | None => continue,
        };
        let Some(displacement) = bytes
            .get(3..LEA_RIP_LEN)
            .and_then(|raw| <[u8; 4]>::try_from(raw).ok())
            .map(i32::from_le_bytes)
        else {
            continue;
        };
        // RIP-relative: from the end of the instruction. `at` indexes a
        // window of `code`, so the sum stays far below `usize::MAX`.
        out.push(
            address
                .wrapping_add(at.wrapping_add(LEA_RIP_LEN) as u64)
                .wrapping_add_signed(i64::from(displacement)),
        );
    }
}

/// What a [`LeaRipScan`] found, each list ascending and without repeats.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LeaTargets {
    /// `lea r10` targets: `KiSystemServiceRepeat` loads
    /// `KeServiceDescriptorTable` so.
    pub(crate) r10: Vec<u64>,
    /// `lea r11` targets: it loads `KeServiceDescriptorTableShadow` so.
    pub(crate) r11: Vec<u64>,
}

/// The `lea r10` and `lea r11` targets in code read piece by piece. A piece
/// that follows the one before it is scanned behind that piece's last bytes,
/// so an instruction that straddles two pieces (a page boundary) is still
/// seen; a piece that does not (past a page that did not read, or in another
/// section) starts afresh.
#[derive(Default)]
pub(crate) struct LeaRipScan {
    /// The last bytes fed (fewer than one instruction), then the piece.
    window: Vec<u8>,
    /// Where the bytes after `window` would start.
    next: Option<u64>,
    found: LeaTargets,
}

impl LeaRipScan {
    /// Scans `piece`, read from linear `address`.
    pub(crate) fn feed(&mut self, piece: &[u8], address: u64) {
        if self.next != Some(address) {
            self.window.clear();
        }
        let start = address.wrapping_sub(self.window.len() as u64);
        self.window.extend_from_slice(piece);
        push_lea_rip_targets(&self.window, start, &mut self.found.r10, &mut self.found.r11);
        // Fewer bytes than one instruction: none found twice.
        let keep = self.window.len().min(LEA_RIP_LEN - 1);
        self.window.drain(..self.window.len().saturating_sub(keep));
        self.next = Some(address.wrapping_add(piece.len() as u64));
    }

    /// Every target found, ascending, each once.
    pub(crate) fn targets(mut self) -> LeaTargets {
        for list in [&mut self.found.r10, &mut self.found.r11] {
            list.sort_unstable();
            list.dedup();
        }
        self.found
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
        let mut scan = LeaRipScan::default();
        scan.feed(&code, 0xFFFF_F800_03C7_3FF1);
        assert_eq!(scan.targets().r10, [0xFFFF_F800_03EA_B840]);
    }

    #[test]
    fn a_lea_r10_cut_off_at_the_end_is_passed_by() {
        let mut scan = LeaRipScan::default();
        scan.feed(&[0x4C, 0x8D, 0x15, 0x47, 0x78, 0x23], 0x1000);
        assert!(scan.targets().r10.is_empty());
    }

    #[test]
    fn a_lea_r10_straddling_two_pieces_is_found_once() {
        // lea r10, [rip + 0x10] at 0x1FFD: it ends at 0x2004, loads 0x2014.
        let mut scan = LeaRipScan::default();
        scan.feed(&[0x90, 0x90, 0x90, 0x4C, 0x8D, 0x15], 0x1FFA);
        scan.feed(&[0x10, 0x00, 0x00, 0x00, 0x90, 0x90, 0x90], 0x2000);
        scan.feed(&[0x90; 8], 0x2007);
        assert_eq!(scan.targets().r10, [0x2014]);
    }

    #[test]
    fn a_piece_that_does_not_follow_the_last_starts_afresh() {
        // The second piece is not at 0x2000: the first one's tail is not its.
        let mut scan = LeaRipScan::default();
        scan.feed(&[0x90, 0x90, 0x90, 0x4C, 0x8D, 0x15], 0x1FFA);
        scan.feed(&[0x10, 0x00, 0x00, 0x00, 0x90, 0x90, 0x90], 0x3000);
        assert!(scan.targets().r10.is_empty());
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

    /// An x64 system-call stub (`mov r10, rcx; mov eax, index`) gives its
    /// index; an index with bit 12 set is the GUI table's.
    #[test]
    fn a_system_call_stub_gives_its_service() {
        // ntdll!NtClose on Windows 10: 4C 8B D1 B8 0F 00 00 00 F6 04 25 …
        let nt_close = [0x4C, 0x8B, 0xD1, 0xB8, 0x0F, 0x00, 0x00, 0x00, 0xF6, 0x04];
        assert_eq!(stub_service(&nt_close), Some(StubService::Kernel(0x0F)));
        // win32u!NtUserGetMessage: index 0x1001.
        let gui = [0x4C, 0x8B, 0xD1, 0xB8, 0x01, 0x10, 0x00, 0x00];
        assert_eq!(stub_service(&gui), Some(StubService::Gui(1)));
        // A stub cut short, another prologue, or an index past both tables.
        assert_eq!(stub_service(&nt_close[..7]), None);
        assert_eq!(stub_service(&[0x48, 0x8B, 0xC4, 0xB8, 0, 0, 0, 0]), None);
        assert_eq!(stub_service(&[0x4C, 0x8B, 0xD1, 0xB8, 0x00, 0x20, 0x00, 0x00]), None);
    }

    /// Each jump form a hook writes over a routine's first bytes, decoded
    /// from the routine's address.
    #[test]
    fn a_routines_first_jump_is_decoded_in_each_form() {
        let at = 0xFFFF_F800_0100_1000;
        // jmp rel32, from the end of the 5-byte instruction.
        let rel = [0xE9, 0x00, 0x10, 0x00, 0x00];
        assert_eq!(first_jump(&rel, at), Some(FirstJump::To(at + 5 + 0x1000)));
        // jmp [rip+disp32]: through the slot after the 6-byte instruction.
        let indirect = [0xFF, 0x25, 0x10, 0x00, 0x00, 0x00];
        assert_eq!(first_jump(&indirect, at), Some(FirstJump::Through(at + 6 + 0x10)));
        // mov rax, imm64; jmp rax.
        let mut absolute = vec![0x48, 0xB8];
        absolute.extend_from_slice(&0xFFFF_F880_0123_4560u64.to_le_bytes());
        absolute.extend_from_slice(&[0xFF, 0xE0]);
        assert_eq!(first_jump(&absolute, at), Some(FirstJump::To(0xFFFF_F880_0123_4560)));
        // push imm32; ret — the processor sign-extends the push.
        let push = [0x68, 0x00, 0x00, 0x00, 0x80, 0xC3];
        assert_eq!(first_jump(&push, at), Some(FirstJump::To(0xFFFF_FFFF_8000_0000)));
        // An ordinary prologue, and every form cut short, is no jump.
        assert_eq!(first_jump(&[0x48, 0x89, 0x5C, 0x24, 0x08], at), None);
        for form in [&rel[..], &indirect[..], &absolute[..], &push[..]] {
            assert_eq!(first_jump(&form[..form.len() - 1], at), None);
        }
    }

    /// `KiSystemServiceRepeat` loads the kernel table's descriptor with
    /// `lea r10` and the shadow's with `lea r11`; one scan finds both.
    #[test]
    fn one_scan_finds_the_lea_r10_and_lea_r11_targets() {
        let code = [
            0x90, 0x4C, 0x8D, 0x15, 0x47, 0x78, 0x23, 0x00, 0x4C, 0x8D, 0x1D, 0x80, 0x78, 0x23, 0x00,
        ];
        let mut scan = LeaRipScan::default();
        scan.feed(&code, 0xFFFF_F800_03C7_3FF1);
        let targets = scan.targets();
        assert_eq!(targets.r10, [0xFFFF_F800_03EA_B840]);
        // lea r11 at …3FF9 ends at …4000; + 0x237880.
        assert_eq!(targets.r11, [0xFFFF_F800_03EA_B880]);
    }

    /// The shadow array's first descriptor is the kernel table's; its second
    /// is the GUI table's. Anything else is not the shadow.
    #[test]
    fn the_shadow_is_the_array_whose_first_descriptor_is_the_kernel_tables() {
        let kernel = Descriptor { service_table: 0xFFFF_F800_0100_2000, limit: 0x191 };
        let mut bytes = vec![0u8; SHADOW_BYTES];
        bytes[..8].copy_from_slice(&kernel.service_table.to_le_bytes());
        bytes[0x10..0x14].copy_from_slice(&kernel.limit.to_le_bytes());
        bytes[0x20..0x28].copy_from_slice(&0xFFFF_F960_0010_0000u64.to_le_bytes());
        bytes[0x30..0x34].copy_from_slice(&0x33Bu32.to_le_bytes());
        assert_eq!(
            shadow_gui_descriptor(&bytes, kernel),
            Some(Descriptor { service_table: 0xFFFF_F960_0010_0000, limit: 0x33B })
        );
        let other = Descriptor { service_table: 0xFFFF_F800_0100_3000, limit: 0x191 };
        assert_eq!(shadow_gui_descriptor(&bytes, other), None);
        assert_eq!(shadow_gui_descriptor(&bytes[..SHADOW_BYTES - 1], kernel), None);
    }

    /// A shadow array known to be one (a PDB symbol, or found before) gives
    /// its second descriptor whatever its first holds: a kernel table moved
    /// in `KeServiceDescriptorTable` alone leaves the GUI table readable.
    #[test]
    fn a_known_shadow_gives_its_second_descriptor_whatever_its_first() {
        let mut bytes = vec![0u8; SHADOW_BYTES];
        bytes[0x20..0x28].copy_from_slice(&0xFFFF_F960_0010_0000u64.to_le_bytes());
        bytes[0x30..0x34].copy_from_slice(&0x33Bu32.to_le_bytes());
        assert_eq!(
            known_shadow_gui_descriptor(&bytes),
            Some(Descriptor { service_table: 0xFFFF_F960_0010_0000, limit: 0x33B })
        );
        assert_eq!(known_shadow_gui_descriptor(&bytes[..SHADOW_BYTES - 1]), None);
        bytes[0x30..0x34].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(known_shadow_gui_descriptor(&bytes), None);
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
