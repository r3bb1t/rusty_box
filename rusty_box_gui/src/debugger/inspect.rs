//! What the Debugger page shows of a paused VM, read the way Bochs's debugger
//! reads: through `DebugCpu::dbg_xlate_linear2phy` and `dbg_fetch_mem`.

use rusty_box::cpu::{CodeSize, CpuDebugView, Fetched, Instrumentation};
use rusty_box::emulator::{DebugCpu, Emulator, SliceEngine};

pub(crate) const ROW_BYTES: usize = 16;
pub(crate) const MEMORY_ROWS: usize = 32;
pub(crate) const MEMORY_WINDOW_BYTES: u64 = (ROW_BYTES * MEMORY_ROWS) as u64;
pub(crate) const CODE_LINES: usize = 24;
pub(crate) const STACK_SLOTS: usize = 16;
/// The longest x86 instruction (SDM vol. 2 §2.3.11).
const MAX_INSTRUCTION_BYTES: usize = 15;
const PAGE_BYTES: u64 = 0x1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddressSpace {
    Physical,
    Linear,
}

/// An address in one of the two spaces the page reads (R4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GuestAddress {
    pub(crate) space: AddressSpace,
    pub(crate) value: u64,
}

/// Where the memory pane looks: an address, or wherever the selected
/// processor's RIP or RSP points each time it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryTarget {
    At(GuestAddress),
    Rip,
    Rsp,
}

/// What the page asks to be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ViewRequest {
    pub(crate) cpu: usize,
    pub(crate) memory: MemoryTarget,
}

impl Default for ViewRequest {
    fn default() -> Self {
        Self {
            cpu: 0,
            memory: MemoryTarget::Rip,
        }
    }
}

/// Everything the page shows of one pause.
#[derive(Debug)]
pub(crate) struct DebugSnapshot {
    pub(crate) cpus: Vec<CpuDebugView>,
    /// The processor the panes below were read through.
    pub(crate) cpu: usize,
    pub(crate) code: Vec<CodeLine>,
    pub(crate) memory: MemoryWindow,
    pub(crate) stack: Vec<StackSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodeLine {
    pub(crate) address: u64,
    pub(crate) bytes: Vec<u8>,
    pub(crate) text: CodeText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CodeText {
    /// The decoder's opcode name: raw, until a formatter exists.
    Opcode(String),
    Undecodable,
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemoryWindow {
    pub(crate) start: GuestAddress,
    pub(crate) rows: Vec<MemoryRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemoryRow {
    pub(crate) address: u64,
    pub(crate) bytes: Option<[u8; ROW_BYTES]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StackSlot {
    pub(crate) address: u64,
    pub(crate) value: Option<u64>,
}

/// Whether a read came back whole.
enum Readable {
    Read,
    Unreadable,
}

/// The width of a stack slot, and of a pointer, at `size`.
pub(crate) fn slot_bytes(size: CodeSize) -> usize {
    match size {
        CodeSize::Bits16 => 2,
        CodeSize::Bits32 => 4,
        CodeSize::Bits64 => 8,
    }
}

/// The linear address of `offset` in the segment based at `base`, as a
/// processor decoding at `size` forms it: 64-bit code ignores CS's and SS's
/// base, and 16- and 32-bit code wraps at 4 GiB.
pub(crate) fn linear_of(base: u64, offset: u64, size: CodeSize) -> u64 {
    match size {
        CodeSize::Bits64 => offset,
        CodeSize::Bits16 | CodeSize::Bits32 => {
            base.wrapping_add(offset & 0xFFFF_FFFF) & 0xFFFF_FFFF
        }
    }
}

/// Reads one pause through processor `view.cpu`, or through the boot
/// processor when that index no longer exists.
pub(crate) fn read_snapshot<T: Instrumentation, E: SliceEngine<T>>(
    emu: &mut Emulator<T, E>,
    view: &ViewRequest,
) -> DebugSnapshot {
    let mut cpus = Vec::new();
    for index in 0..emu.cpu_count() {
        if let Some(cpu) = emu.debug_cpu(index) {
            cpus.push(cpu.view());
        }
    }
    let index = if view.cpu < cpus.len() { view.cpu } else { 0 };
    let Some(mut cpu) = emu.debug_cpu(index) else {
        return DebugSnapshot {
            cpus,
            cpu: index,
            code: Vec::new(),
            memory: MemoryWindow {
                start: GuestAddress {
                    space: AddressSpace::Linear,
                    value: 0,
                },
                rows: Vec::new(),
            },
            stack: Vec::new(),
        };
    };
    let shown = cpu.view();
    let size = shown.code_size;
    let rip = linear_of(shown.segment_bases.cs, shown.registers.rip, size);
    let rsp = linear_of(shown.segment_bases.ss, shown.registers.rsp, size);
    let code = read_code(&mut cpu, rip, size);
    let start = match view.memory {
        MemoryTarget::At(address) => address,
        MemoryTarget::Rip => GuestAddress {
            space: AddressSpace::Linear,
            value: rip,
        },
        MemoryTarget::Rsp => GuestAddress {
            space: AddressSpace::Linear,
            value: rsp,
        },
    };
    let memory = read_memory(&mut cpu, start);
    let stack = read_stack(&mut cpu, rsp, slot_bytes(size));
    DebugSnapshot {
        cpus,
        cpu: index,
        code,
        memory,
        stack,
    }
}

/// Reads `buf.len()` bytes at `at`, which lie within one page.
fn read_within_page<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    at: GuestAddress,
    buf: &mut [u8],
) -> Readable {
    let physical = match at.space {
        AddressSpace::Physical => at.value,
        AddressSpace::Linear => match cpu.dbg_xlate_linear2phy(at.value) {
            Some(physical) => physical,
            None => return Readable::Unreadable,
        },
    };
    match cpu.dbg_fetch_mem(physical, buf) {
        Ok(Fetched::Valid) => Readable::Read,
        Ok(Fetched::OutsideMemory) => Readable::Unreadable,
        Err(error) => {
            tracing::warn!("debugger read at {physical:#x} failed: {error}");
            Readable::Unreadable
        }
    }
}

/// The readable bytes from `at` up to `len`, stopping at the first page that
/// does not read.
fn read_prefix<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    at: GuestAddress,
    len: usize,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(len);
    let mut address = at.value;
    while bytes.len() < len {
        let in_page = usize::try_from(PAGE_BYTES - (address % PAGE_BYTES)).unwrap_or(len);
        let mut piece = vec![0u8; in_page.min(len - bytes.len())];
        let at_piece = GuestAddress {
            space: at.space,
            value: address,
        };
        match read_within_page(cpu, at_piece, &mut piece) {
            Readable::Read => bytes.extend_from_slice(&piece),
            Readable::Unreadable => break,
        }
        address = address.wrapping_add(piece.len() as u64);
    }
    bytes
}

fn read_code<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    rip: u64,
    size: CodeSize,
) -> Vec<CodeLine> {
    let at = GuestAddress {
        space: AddressSpace::Linear,
        value: rip,
    };
    let bytes = read_prefix(cpu, at, CODE_LINES * MAX_INSTRUCTION_BYTES);
    if bytes.is_empty() {
        return vec![CodeLine {
            address: rip,
            bytes: Vec::new(),
            text: CodeText::Unreadable,
        }];
    }
    decode_lines(&bytes, rip, size)
}

/// Splits `bytes`, read at `address`, into at most `CODE_LINES` lines: one
/// per instruction, and one per byte that starts none.
pub(crate) fn decode_lines(bytes: &[u8], address: u64, size: CodeSize) -> Vec<CodeLine> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    while lines.len() < CODE_LINES {
        let Some(rest) = bytes.get(offset..).filter(|rest| !rest.is_empty()) else {
            break;
        };
        let decoded = match size.decode(rest) {
            Ok(instruction) => {
                let length = usize::from(instruction.ilen());
                (length > 0 && length <= rest.len())
                    .then(|| (length, format!("{:?}", instruction.get_ia_opcode())))
            }
            Err(_) => None,
        };
        let (length, text) = match decoded {
            Some((length, name)) => (length, CodeText::Opcode(name)),
            None => (1, CodeText::Undecodable),
        };
        lines.push(CodeLine {
            address: address.wrapping_add(offset as u64),
            bytes: rest[..length].to_vec(),
            text,
        });
        offset += length;
    }
    lines
}

fn read_memory<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    start: GuestAddress,
) -> MemoryWindow {
    let aligned = GuestAddress {
        space: start.space,
        value: start.value & !(ROW_BYTES as u64 - 1),
    };
    let mut rows = Vec::with_capacity(MEMORY_ROWS);
    for row in 0..MEMORY_ROWS as u64 {
        let address = aligned.value.wrapping_add(row * ROW_BYTES as u64);
        let mut bytes = [0u8; ROW_BYTES];
        let at = GuestAddress {
            space: aligned.space,
            value: address,
        };
        let read = read_within_page(cpu, at, &mut bytes);
        rows.push(MemoryRow {
            address,
            bytes: match read {
                Readable::Read => Some(bytes),
                Readable::Unreadable => None,
            },
        });
    }
    MemoryWindow {
        start: aligned,
        rows,
    }
}

fn read_stack<T: Instrumentation, E: SliceEngine<T>>(
    cpu: &mut DebugCpu<'_, T, E>,
    rsp: u64,
    width: usize,
) -> Vec<StackSlot> {
    let mut slots = Vec::with_capacity(STACK_SLOTS);
    for slot in 0..STACK_SLOTS {
        let address = rsp.wrapping_add((slot * width) as u64);
        let at = GuestAddress {
            space: AddressSpace::Linear,
            value: address,
        };
        let bytes = read_prefix(cpu, at, width);
        let value = (bytes.len() == width).then(|| {
            let mut raw = [0u8; 8];
            raw[..width].copy_from_slice(&bytes);
            u64::from_le_bytes(raw)
        });
        slots.push(StackSlot { address, value });
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box::cpu::{CpuSetupMode, X86Reg};
    use rusty_box::emulator::{Emulator, EmulatorConfig};

    #[test]
    fn a_linear_address_is_formed_as_the_processor_forms_it() {
        assert_eq!(linear_of(0xFFFF_0000, 0xFFF0, CodeSize::Bits16), 0xFFFF_FFF0);
        assert_eq!(
            linear_of(0x1000, 0xFFFF_FFFF, CodeSize::Bits32),
            0xFFF,
            "wraps at 4 GiB"
        );
        assert_eq!(
            linear_of(0x1000, 0xFFFF_8000_0000_0000, CodeSize::Bits64),
            0xFFFF_8000_0000_0000,
            "64-bit code ignores the segment base"
        );
    }

    #[test]
    fn code_is_split_on_instruction_boundaries() {
        // NOP; JMP $
        let lines = decode_lines(&[0x90, 0xEB, 0xFE], 0x7C00, CodeSize::Bits16);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (lines[0].address, lines[0].bytes.as_slice()),
            (0x7C00, &[0x90][..])
        );
        assert_eq!(
            (lines[1].address, lines[1].bytes.as_slice()),
            (0x7C01, &[0xEB, 0xFE][..])
        );
        assert!(lines
            .iter()
            .all(|line| matches!(line.text, CodeText::Opcode(_))));
    }

    #[test]
    fn bytes_that_decode_to_nothing_take_one_line_each() {
        // An operand-size prefix with nothing after it.
        let lines = decode_lines(&[0x66], 0x100, CodeSize::Bits16);
        assert_eq!(
            lines,
            [CodeLine {
                address: 0x100,
                bytes: vec![0x66],
                text: CodeText::Undecodable,
            }]
        );
    }

    fn long_mode_machine() -> Box<Emulator> {
        Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
            .expect("machine")
    }

    #[test]
    fn a_paused_machine_reads_back_as_the_page_shows_it() {
        let mut machine = long_mode_machine();
        let code = CpuSetupMode::FlatLong64.first_free_physical_address();
        // mov rax, 42 ; hlt
        machine
            .mem_write(code, &[0x48, 0xC7, 0xC0, 0x2A, 0x00, 0x00, 0x00, 0xF4])
            .expect("write");
        machine.reg_write(X86Reg::Rip, code);
        machine.reg_write(X86Reg::Rax, 0x2A);

        let snapshot = read_snapshot(&mut machine, &ViewRequest::default());

        assert_eq!(snapshot.cpus.len(), 1);
        assert_eq!(snapshot.cpus[0].registers.rax, 0x2A);
        assert_eq!(snapshot.code[0].address, code);
        assert_eq!(snapshot.code[0].bytes.len(), 7);
        assert_eq!(snapshot.code[1].bytes, [0xF4]);
        assert_eq!(
            snapshot.memory.start,
            GuestAddress {
                space: AddressSpace::Linear,
                value: code & !0xF
            }
        );
        let first = snapshot.memory.rows[0].bytes.expect("RAM is readable");
        assert_eq!(first[(code & 0xF) as usize], 0x48);
        assert_eq!(snapshot.stack.len(), STACK_SLOTS);
    }

    #[test]
    fn an_unmapped_or_absent_address_reads_as_unreadable_rows() {
        let mut machine = long_mode_machine();
        for target in [
            GuestAddress {
                space: AddressSpace::Linear,
                value: 0xFFFF_FF80_0000_0000,
            },
            GuestAddress {
                space: AddressSpace::Physical,
                value: 0xD000_0000,
            },
            GuestAddress {
                space: AddressSpace::Physical,
                value: 0x40_0000_0000,
            },
        ] {
            let snapshot = read_snapshot(
                &mut machine,
                &ViewRequest {
                    cpu: 0,
                    memory: MemoryTarget::At(target),
                },
            );
            assert_eq!(snapshot.memory.rows.len(), MEMORY_ROWS, "{target:?}");
            assert!(
                snapshot.memory.rows.iter().all(|row| row.bytes.is_none()),
                "{target:?} reads as ?? rows"
            );
        }
    }

    #[test]
    fn a_processor_index_past_the_last_reads_the_boot_processor() {
        let mut machine = long_mode_machine();
        let snapshot = read_snapshot(
            &mut machine,
            &ViewRequest {
                cpu: 5,
                memory: MemoryTarget::Rip,
            },
        );
        assert_eq!(snapshot.cpu, 0);
    }
}
