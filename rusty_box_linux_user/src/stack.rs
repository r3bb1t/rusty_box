use alloc::vec;
use alloc::vec::Vec;

use crate::elf::Image;
use crate::memory::{PAGE, STACK_BASE, STACK_SIZE};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StackError {
    #[error("arguments and environment exceed the initial process stack")]
    Full,
    #[error("argument or environment string contains a NUL byte")]
    InteriorNul,
}

pub(crate) struct Stack {
    bytes: Vec<u8>,
    cursor: usize,
}

impl Stack {
    pub(crate) fn build(
        image: &Image<'_>,
        args: &[&[u8]],
        env: &[&[u8]],
    ) -> Result<Self, StackError> {
        // Bound both the pointer table and allocations before processing strings.
        if args.len().saturating_add(env.len()) > 1024 {
            return Err(StackError::Full);
        }
        let mut stack = Self {
            bytes: vec![0; STACK_SIZE as usize],
            cursor: STACK_SIZE as usize,
        };
        let mut words = Vec::new();
        words.push(args.len() as u32);
        for arg in args {
            words.push(stack.string(arg)?);
        }
        words.push(0);
        for var in env {
            words.push(stack.string(var)?);
        }
        words.push(0);
        // The executable and header table were validated by the ELF loader.
        // No interpreter, VDSO, entropy source or dynamic runtime is installed.
        words.extend_from_slice(&[
            3,
            image.phdr,
            4,
            32,
            5,
            u32::from(image.phnum),
            6,
            PAGE,
            7,
            0,
            8,
            0,
            9,
            image.entry(),
            11,
            1000,
            12,
            1000,
            13,
            1000,
            14,
            1000,
            23,
            0,
        ]);
        if !args.is_empty() {
            words.extend_from_slice(&[31, words[1]]);
        }
        words.extend_from_slice(&[0, 0]);
        stack.cursor = stack
            .cursor
            .checked_sub(words.len() * 4)
            .ok_or(StackError::Full)?
            & !15;
        let table = stack.bytes[stack.cursor..].as_chunks_mut::<4>().0;
        for (bytes, word) in table.iter_mut().zip(words) {
            bytes.copy_from_slice(&word.to_le_bytes());
        }
        Ok(stack)
    }

    fn string(&mut self, data: &[u8]) -> Result<u32, StackError> {
        self.cursor = self
            .cursor
            .checked_sub(1)
            .and_then(|cursor| cursor.checked_sub(data.len()))
            .ok_or(StackError::Full)?;
        if data.contains(&0) {
            return Err(StackError::InteriorNul);
        }
        self.bytes[self.cursor..self.cursor + data.len()].copy_from_slice(data);
        Ok(self.pointer())
    }

    pub(crate) fn pointer(&self) -> u32 {
        STACK_BASE + self.cursor as u32
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes[self.cursor..]
    }
}
