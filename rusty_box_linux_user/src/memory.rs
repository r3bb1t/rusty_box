use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuEmulator;

use crate::elf::Image;

pub(crate) const PAGE: u32 = 4096;
pub(crate) const USER_MIN: u32 = 0x10000;
pub(crate) const STACK_TOP: u32 = 0xC000_0000;
pub(crate) const STACK_SIZE: u32 = 16 * PAGE;
pub(crate) const STACK_BASE: u32 = STACK_TOP - STACK_SIZE;
pub(crate) const STACK_GUARD: u32 = STACK_BASE - PAGE;
pub(crate) const PDPT: u32 = 3 * PAGE;

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("guest buffer extends beyond the 32-bit address space")]
    AddressOverflow,
    #[error("guest buffer is unmapped at {address:#x}")]
    Unmapped { address: u32 },
    #[error("guest buffer is read-only at {address:#x}")]
    ReadOnly { address: u32 },
    #[error(transparent)]
    Backing(#[from] rusty_box::Error),
}

#[derive(Clone, Copy)]
struct Region {
    start: u32,
    end: u32,
    frame: u32,
    writable: bool,
    executable: bool,
}

pub(crate) struct AddressSpace {
    regions: Vec<Region>,
    frames_end: u32,
}

impl AddressSpace {
    pub(crate) fn new(image: &Image<'_>) -> Self {
        let mut frames_end = PDPT + PAGE;
        let mut regions = Vec::with_capacity(image.segments.len() + 1);
        for seg in &image.segments {
            regions.push(Region {
                start: seg.page_start,
                end: seg.page_end,
                frame: frames_end,
                writable: seg.writable,
                executable: seg.executable,
            });
            frames_end += seg.page_end - seg.page_start;
        }
        regions.push(Region {
            start: STACK_BASE,
            end: STACK_TOP,
            frame: frames_end,
            writable: true,
            executable: false,
        });
        Self {
            regions,
            frames_end: frames_end + STACK_SIZE,
        }
    }

    pub(crate) fn tables(&self) -> PageTables {
        let mut tables = PageTables {
            base: self.frames_end,
            pages: Vec::new(),
            directories: [None; 4],
            leaves: BTreeMap::new(),
        };
        tables.map(0, 0, 3, true);
        tables.map(PAGE, PAGE, 1, false);
        tables.map(2 * PAGE, 2 * PAGE, 3, true);
        for region in &self.regions {
            let mut flags = 1 | 4;
            if region.writable {
                flags |= 2;
            }
            for offset in (0..region.end - region.start).step_by(PAGE as usize) {
                tables.map(
                    region.start + offset,
                    region.frame + offset,
                    flags,
                    !region.executable,
                );
            }
        }
        tables
    }

    pub(crate) fn load(&self, cpu: &mut CpuEmulator, image: &Image<'_>) -> rusty_box::Result<()> {
        for (region, segment) in self.regions.iter().zip(&image.segments) {
            let frame = region.frame + (segment.start - region.start);
            cpu.memory().write_phys(physical(frame), segment.data)?;
        }
        Ok(())
    }

    fn span(&self, address: u32, write: bool) -> Result<(u32, u32), MemoryError> {
        let index = self
            .regions
            .partition_point(|region| region.start <= address);
        let region = index
            .checked_sub(1)
            .and_then(|index| self.regions.get(index))
            .filter(|region| address < region.end)
            .ok_or(MemoryError::Unmapped { address })?;
        if write && !region.writable {
            return Err(MemoryError::ReadOnly { address });
        }
        Ok((
            region.frame + (address - region.start),
            region.end - address,
        ))
    }

    /// Verify the entire transfer before observing or changing any bytes.
    pub(crate) fn validate(
        &self,
        address: u32,
        len: usize,
        write: bool,
    ) -> Result<(), MemoryError> {
        let len = u32::try_from(len).map_err(|_| MemoryError::AddressOverflow)?;
        let end = address
            .checked_add(len)
            .ok_or(MemoryError::AddressOverflow)?;
        let mut at = address;
        while at < end {
            let (_, span) = self.span(at, write)?;
            at += span.min(end - at);
        }
        Ok(())
    }

    pub(crate) fn read(
        &self,
        cpu: &mut CpuEmulator,
        mut address: u32,
        mut out: &mut [u8],
    ) -> Result<(), MemoryError> {
        self.validate(address, out.len(), false)?;
        while !out.is_empty() {
            let (frame, span) = self.span(address, false)?;
            let len = (span as usize).min(out.len());
            cpu.memory().read_phys(physical(frame), &mut out[..len])?;
            address += len as u32;
            out = &mut out[len..];
        }
        Ok(())
    }

    pub(crate) fn write(
        &self,
        cpu: &mut CpuEmulator,
        mut address: u32,
        mut input: &[u8],
    ) -> Result<(), MemoryError> {
        self.validate(address, input.len(), true)?;
        while !input.is_empty() {
            let (frame, span) = self.span(address, true)?;
            let len = (span as usize).min(input.len());
            cpu.memory().write_phys(physical(frame), &input[..len])?;
            address += len as u32;
            input = &input[len..];
        }
        Ok(())
    }
}

pub(crate) fn physical(address: u32) -> BxPhyAddress {
    BxPhyAddress::new(u64::from(address))
}

/// Non-long-mode PAE paging with 4 KiB leaves (Intel SDM Vol. 3A, 4.4).
/// Non-leaf PDEs permit user reads and writes. Each PTE supplies its own
/// supervisor/user, write and NX permissions. The PDPT has four entries.
pub(crate) struct PageTables {
    base: u32,
    pages: Vec<[u64; 512]>,
    directories: [Option<usize>; 4],
    leaves: BTreeMap<u32, usize>,
}

impl PageTables {
    fn allocate(&mut self) -> usize {
        let index = self.pages.len();
        self.pages.push([0; 512]);
        index
    }

    fn frame(&self, index: usize) -> u32 {
        self.base + index as u32 * PAGE
    }

    fn map(&mut self, address: u32, frame: u32, flags: u64, nx: bool) {
        let top = (address >> 30) as usize;
        let key = address >> 21;
        let pte = ((address >> 12) & 511) as usize;
        let leaf = match self.leaves.get(&key) {
            Some(&leaf) => leaf,
            None => {
                let dir = match self.directories[top] {
                    Some(dir) => dir,
                    None => {
                        let dir = self.allocate();
                        self.directories[top] = Some(dir);
                        dir
                    }
                };
                let leaf = self.allocate();
                self.pages[dir][(key & 511) as usize] = u64::from(self.frame(leaf)) | 7;
                self.leaves.insert(key, leaf);
                leaf
            }
        };
        self.pages[leaf][pte] = u64::from(frame) | flags | (u64::from(nx) << 63);
    }

    pub(crate) fn ram_size(&self) -> usize {
        self.frame(self.pages.len()) as usize
    }

    pub(crate) fn install(&self, cpu: &mut CpuEmulator) -> rusty_box::Result<()> {
        let mut buffer = [0; PAGE as usize];
        let entries = buffer.as_chunks_mut::<8>().0;
        for (bytes, dir) in entries.iter_mut().zip(self.directories) {
            let value = dir.map_or(0, |index| u64::from(self.frame(index)) | 1);
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        cpu.memory().write_phys(physical(PDPT), &buffer[..32])?;
        for (index, table) in self.pages.iter().enumerate() {
            for (bytes, value) in buffer.as_chunks_mut::<8>().0.iter_mut().zip(table) {
                bytes.copy_from_slice(&value.to_le_bytes());
            }
            cpu.memory()
                .write_phys(physical(self.frame(index)), &buffer)?;
        }
        Ok(())
    }
}
