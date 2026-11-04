//! What one instruction refers to (spec §2 goal 1): the address a relative
//! branch goes to, the address a memory operand names when it is
//! RIP-relative or absolute, and an immediate that is an address.

use rusty_box::cpu::decoder::{GprIndex, Instruction, Opcode};
use rusty_box::cpu::CodeSize;

use crate::debugger::pe::ImageWidth;

/// How a site refers to its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum XrefKind {
    /// A relative `call`.
    Call,
    /// A relative `jmp`, `jcc`, `loop` or `jrcxz`.
    Jump,
    /// `call [address]`: the target is the slot the call reads.
    CallThrough,
    /// `jmp [address]`.
    JumpThrough,
    /// Any other memory operand: a read, a write, or both.
    Memory,
    /// The address is taken: a `lea`, or an immediate that is an address.
    AddressOf,
    /// A pointer-sized value in data.
    Pointer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Reference {
    pub(crate) target: u64,
    pub(crate) kind: XrefKind,
}

/// What one instruction refers to: at most a branch target, a memory
/// operand's address and an immediate address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InstructionRefs {
    refs: [Option<Reference>; 3],
}

impl InstructionRefs {
    fn push(&mut self, reference: Reference) {
        if let Some(slot) = self.refs.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(reference);
        }
    }

    pub(crate) fn iter(&self) -> core::iter::Flatten<core::slice::Iter<'_, Option<Reference>>> {
        self.refs.iter().flatten()
    }
}

/// Immediates below this are taken for numbers, not addresses: Windows maps
/// nothing in the first 64 KiB.
pub(crate) const MIN_IMMEDIATE_ADDRESS: u64 = 0x1_0000;

/// The decoder's mode for an image of `width`.
pub(crate) fn code_size(width: ImageWidth) -> CodeSize {
    match width {
        ImageWidth::Bits32 => CodeSize::Bits32,
        ImageWidth::Bits64 => CodeSize::Bits64,
    }
}

/// What `instruction`, at `at` in an image of `width`, refers to.
pub(crate) fn references_of(instruction: &Instruction, at: u64, width: ImageWidth) -> InstructionRefs {
    let size = code_size(width);
    let next = at.wrapping_add(u64::from(instruction.ilen()));
    let opcode = instruction.get_ia_opcode();
    let mut refs = InstructionRefs { refs: [None; 3] };
    if let Some(target) = branch_target(instruction, next, size) {
        let kind = match opcode {
            Opcode::CallJd | Opcode::CallJq => XrefKind::Call,
            _ => XrefKind::Jump,
        };
        refs.push(Reference { target, kind });
    }
    if let Some(target) = moffs_address(instruction, width) {
        refs.push(Reference { target, kind: XrefKind::Memory });
    } else if !instruction.mod_c0() {
        if let Some(target) = memory_address(instruction, next, size) {
            let kind = match opcode {
                Opcode::CallEq | Opcode::CallEd | Opcode::CallEw => XrefKind::CallThrough,
                Opcode::JmpEq | Opcode::JmpEd | Opcode::JmpEw => XrefKind::JumpThrough,
                Opcode::LeaGqM | Opcode::LeaGdM | Opcode::LeaGwM => XrefKind::AddressOf,
                _ => XrefKind::Memory,
            };
            refs.push(Reference { target, kind });
        }
    }
    if let Some(target) = immediate_address(instruction, width) {
        refs.push(Reference { target, kind: XrefKind::AddressOf });
    }
    refs
}

/// Where a relative branch goes: `next` plus its sign-extended displacement
/// (Bochs ctrl_xfer32.cc `JMP_Jd` and the `Jcc`/`LOOP`/`JECXZ` forms,
/// ctrl_xfer64.cc `JMP_Jq`, `CALL_Jq`, `JRCXZ_Jb`), wrapping at 4 GiB in
/// 32-bit code. A 16-bit branch is an offset in CS, whose base is not known
/// here, so it gives none.
pub(crate) fn branch_target(instruction: &Instruction, next: u64, size: CodeSize) -> Option<u64> {
    let displacement = i64::from(instruction.id() as i32);
    match size {
        CodeSize::Bits16 => None,
        CodeSize::Bits32 => match instruction.get_ia_opcode() {
            Opcode::CallJd
            | Opcode::JmpJd
            | Opcode::JmpJbd
            | Opcode::JoJd
            | Opcode::JnoJd
            | Opcode::JbJd
            | Opcode::JnbJd
            | Opcode::JzJd
            | Opcode::JnzJd
            | Opcode::JbeJd
            | Opcode::JnbeJd
            | Opcode::JsJd
            | Opcode::JnsJd
            | Opcode::JpJd
            | Opcode::JnpJd
            | Opcode::JlJd
            | Opcode::JnlJd
            | Opcode::JleJd
            | Opcode::JnleJd
            | Opcode::JoJbd
            | Opcode::JnoJbd
            | Opcode::JbJbd
            | Opcode::JnbJbd
            | Opcode::JzJbd
            | Opcode::JnzJbd
            | Opcode::JbeJbd
            | Opcode::JnbeJbd
            | Opcode::JsJbd
            | Opcode::JnsJbd
            | Opcode::JpJbd
            | Opcode::JnpJbd
            | Opcode::JlJbd
            | Opcode::JnlJbd
            | Opcode::JleJbd
            | Opcode::JnleJbd
            | Opcode::JecxzJbd
            | Opcode::LoopJbd
            | Opcode::LoopeJbd
            | Opcode::LoopneJbd => Some(u64::from((next as u32).wrapping_add(displacement as u32))),
            _ => None,
        },
        CodeSize::Bits64 => match instruction.get_ia_opcode() {
            Opcode::CallJq
            | Opcode::JmpJq
            | Opcode::JmpJbq
            | Opcode::JoJq
            | Opcode::JnoJq
            | Opcode::JbJq
            | Opcode::JnbJq
            | Opcode::JzJq
            | Opcode::JnzJq
            | Opcode::JbeJq
            | Opcode::JnbeJq
            | Opcode::JsJq
            | Opcode::JnsJq
            | Opcode::JpJq
            | Opcode::JnpJq
            | Opcode::JlJq
            | Opcode::JnlJq
            | Opcode::JleJq
            | Opcode::JnleJq
            | Opcode::JoJbq
            | Opcode::JnoJbq
            | Opcode::JbJbq
            | Opcode::JnbJbq
            | Opcode::JzJbq
            | Opcode::JnzJbq
            | Opcode::JbeJbq
            | Opcode::JnbeJbq
            | Opcode::JsJbq
            | Opcode::JnsJbq
            | Opcode::JpJbq
            | Opcode::JnpJbq
            | Opcode::JlJbq
            | Opcode::JnlJbq
            | Opcode::JleJbq
            | Opcode::JnleJbq
            | Opcode::JrcxzJbq
            | Opcode::LoopJbq
            | Opcode::LoopeJbq
            | Opcode::LoopneJbq => Some(next.wrapping_add(displacement as u64)),
            _ => None,
        },
    }
}

/// The address a memory operand names when it does not depend on a
/// register: RIP plus its displacement (64-bit), or its bare displacement,
/// sign-extended in 64-bit code; a 32-bit address size keeps the low half
/// (Bochs `BxResolve32*`). 16-bit addressing gives none.
pub(crate) fn memory_address(instruction: &Instruction, next: u64, size: CodeSize) -> Option<u64> {
    match size {
        CodeSize::Bits16 => None,
        CodeSize::Bits32 => {
            if instruction.as32_l() == 0 {
                return None;
            }
            bare_displacement(instruction).map(|value| u64::from(value as u32))
        }
        CodeSize::Bits64 => {
            let address = if instruction.base_reg() == GprIndex::Rip && has_no_index(instruction) {
                next.wrapping_add(i64::from(instruction.displ32s()) as u64)
            } else {
                bare_displacement(instruction)? as u64
            };
            Some(if instruction.as64_l() == 0 { address & 0xFFFF_FFFF } else { address })
        }
    }
}

/// The address of a `mov` to or from a memory offset (`A0`–`A3`), which has
/// no ModRM byte: `Id` in 32-bit code (Bochs data_xfer32.cc `MOV_EAXOd`),
/// `Iq` in 64-bit code (data_xfer64.cc `MOV_RAXOq`).
fn moffs_address(instruction: &Instruction, width: ImageWidth) -> Option<u64> {
    let opcode = instruction.get_ia_opcode();
    match width {
        ImageWidth::Bits32 => matches!(
            opcode,
            Opcode::MovAlod | Opcode::MovAxod | Opcode::MovEaxod | Opcode::MovOdAl | Opcode::MovOdAx | Opcode::MovOdEax
        )
        .then(|| u64::from(instruction.id())),
        ImageWidth::Bits64 => matches!(
            opcode,
            Opcode::MovRaxoq
                | Opcode::MovOqRax
                | Opcode::MovEaxoq
                | Opcode::MovOqEax
                | Opcode::MovAxoq
                | Opcode::MovOqAx
                | Opcode::MovAloq
                | Opcode::MovOqAl
        )
        .then(|| instruction.iq()),
    }
}

/// An immediate that is an address: `mov r64, imm64` in 64-bit code; `push
/// imm32` and `mov r/m32, imm32` in 32-bit code, where addresses fit an
/// immediate. Values below `MIN_IMMEDIATE_ADDRESS` are numbers.
fn immediate_address(instruction: &Instruction, width: ImageWidth) -> Option<u64> {
    let opcode = instruction.get_ia_opcode();
    let value = match width {
        ImageWidth::Bits64 => (opcode == Opcode::MovRrxiq).then(|| instruction.iq())?,
        ImageWidth::Bits32 => matches!(opcode, Opcode::PushId | Opcode::MovEdId).then(|| u64::from(instruction.id()))?,
    };
    (value >= MIN_IMMEDIATE_ADDRESS).then_some(value)
}

/// The address an operand of no base and no index names: its displacement,
/// sign-extended.
pub(crate) fn bare_displacement(instruction: &Instruction) -> Option<i64> {
    (instruction.base_reg() == GprIndex::Nil && has_no_index(instruction)).then(|| i64::from(instruction.displ32s()))
}

/// Whether a memory operand has no index register: the decoder leaves the
/// index slot at 4, RSP's number, which no SIB byte can name as an index
/// (Bochs fetchdecode `BX_NO_INDEX`).
pub(crate) fn has_no_index(instruction: &Instruction) -> bool {
    instruction.index_reg() == GprIndex::Rsp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::inspect::decode_lines;
    use crate::debugger::pe::ImageWidth;
    use rusty_box::cpu::CodeSize;

    const AT: u64 = 0x1_4000_1000;
    const AT32: u64 = 0x40_1000;

    fn refs(bytes: &[u8], width: ImageWidth) -> Vec<Reference> {
        let at = match width {
            ImageWidth::Bits64 => AT,
            ImageWidth::Bits32 => AT32,
        };
        let instruction = code_size(width).decode(bytes).expect("the bytes decode");
        assert_eq!(usize::from(instruction.ilen()), bytes.len(), "one whole instruction");
        references_of(&instruction, at, width).iter().copied().collect()
    }

    fn one(target: u64, kind: XrefKind) -> Vec<Reference> {
        vec![Reference { target, kind }]
    }

    #[test]
    fn a_relative_call_and_jump_name_their_targets() {
        assert_eq!(refs(&[0xE8, 0x10, 0, 0, 0], ImageWidth::Bits64), one(AT + 0x15, XrefKind::Call));
        assert_eq!(refs(&[0x74, 0xFE], ImageWidth::Bits64), one(AT, XrefKind::Jump));
        assert_eq!(refs(&[0x0F, 0x85, 0, 1, 0, 0], ImageWidth::Bits64), one(AT + 0x106, XrefKind::Jump));
        assert_eq!(refs(&[0xE9, 0xFB, 0xFF, 0xFF, 0xFF], ImageWidth::Bits32), one(AT32, XrefKind::Jump));
    }

    #[test]
    fn a_call_through_rip_names_its_slot() {
        assert_eq!(refs(&[0xFF, 0x15, 0xF2, 0x0F, 0, 0], ImageWidth::Bits64), one(AT + 6 + 0xFF2, XrefKind::CallThrough));
        assert_eq!(refs(&[0xFF, 0x25, 0xF2, 0x0F, 0, 0], ImageWidth::Bits64), one(AT + 6 + 0xFF2, XrefKind::JumpThrough));
        // 32-bit `call [0x403000]`.
        assert_eq!(refs(&[0xFF, 0x15, 0, 0x30, 0x40, 0], ImageWidth::Bits32), one(0x40_3000, XrefKind::CallThrough));
    }

    #[test]
    fn a_rip_relative_load_and_store_name_their_address() {
        assert_eq!(refs(&[0x48, 0x8B, 0x05, 0x10, 0, 0, 0], ImageWidth::Bits64), one(AT + 7 + 0x10, XrefKind::Memory));
        assert_eq!(refs(&[0x89, 0x0D, 0x10, 0, 0, 0], ImageWidth::Bits64), one(AT + 6 + 0x10, XrefKind::Memory));
    }

    #[test]
    fn lea_takes_an_address() {
        assert_eq!(refs(&[0x48, 0x8D, 0x0D, 0, 0x10, 0, 0], ImageWidth::Bits64), one(AT + 7 + 0x1000, XrefKind::AddressOf));
    }

    #[test]
    fn an_imm64_and_a_32_bit_push_are_addresses() {
        let mut mov = vec![0x48, 0xB8];
        mov.extend_from_slice(&0x1_4000_3000u64.to_le_bytes());
        assert_eq!(refs(&mov, ImageWidth::Bits64), one(0x1_4000_3000, XrefKind::AddressOf));
        assert_eq!(refs(&[0x68, 0, 0x30, 0x40, 0], ImageWidth::Bits32), one(0x40_3000, XrefKind::AddressOf));
        assert_eq!(refs(&[0xB9, 0, 0x30, 0x40, 0], ImageWidth::Bits32), one(0x40_3000, XrefKind::AddressOf));
        assert_eq!(refs(&[0xA1, 0, 0x30, 0x40, 0], ImageWidth::Bits32), one(0x40_3000, XrefKind::Memory));
        let mut moffs = vec![0x48, 0xA1];
        moffs.extend_from_slice(&0x7FFE_0014u64.to_le_bytes());
        assert_eq!(refs(&moffs, ImageWidth::Bits64), one(0x7FFE_0014, XrefKind::Memory));
        // `mov dword [0x403000], 0x401000`: the operand and the immediate.
        assert_eq!(
            refs(&[0xC7, 0x05, 0, 0x30, 0x40, 0, 0, 0x10, 0x40, 0], ImageWidth::Bits32),
            vec![
                Reference { target: 0x40_3000, kind: XrefKind::Memory },
                Reference { target: 0x40_1000, kind: XrefKind::AddressOf },
            ]
        );
    }

    #[test]
    fn small_immediates_and_64_bit_imm32_are_not_addresses() {
        assert_eq!(refs(&[0x68, 0x10, 0, 0, 0], ImageWidth::Bits32), []);
        assert_eq!(refs(&[0xB8, 1, 0, 0, 0], ImageWidth::Bits32), []);
        assert_eq!(refs(&[0xB8, 0, 0x30, 0x40, 0], ImageWidth::Bits64), []);
    }

    #[test]
    fn register_forms_and_based_operands_name_nothing() {
        assert_eq!(refs(&[0xFF, 0xD0], ImageWidth::Bits64), []);
        assert_eq!(refs(&[0x48, 0x8B, 0x03], ImageWidth::Bits64), []);
        assert_eq!(refs(&[0x48, 0x8B, 0x44, 0x24, 0x08], ImageWidth::Bits64), []);
        assert_eq!(refs(&[0x90], ImageWidth::Bits64), []);
        assert_eq!(refs(&[0xC3], ImageWidth::Bits64), []);
    }

    #[test]
    fn the_code_pane_and_the_index_agree_on_branch_targets() {
        let branches: [&[u8]; 4] = [&[0xE8, 0x10, 0, 0, 0], &[0x74, 0xFE], &[0x0F, 0x85, 0, 1, 0, 0], &[0xE2, 0x00]];
        for bytes in branches {
            let line = decode_lines(bytes, AT, CodeSize::Bits64);
            let instruction = CodeSize::Bits64.decode(bytes).expect("decodes");
            let next = AT + u64::from(instruction.ilen());
            assert_eq!(line[0].target, branch_target(&instruction, next, CodeSize::Bits64), "{bytes:02X?}");
            assert!(line[0].target.is_some());
        }
    }
}
