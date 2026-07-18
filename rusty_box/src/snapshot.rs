//! Version-3 streaming machine snapshot container.

#[cfg(feature = "std")]
use std::io::{self, Error, ErrorKind, Read, Write};

#[cfg(feature = "std")]
use crate::{
    cpu::cpuid::BxCpuIdTrait,
    emulator::Emulator,
    memory::{BxMemC, MemorySnapshotGeometry, MemorySnapshotResidency},
    pc_system::TimerOwner,
};

#[cfg(feature = "std")]
const SNAPSHOT_MAGIC: &[u8; 8] = b"RBXSNAP1";
#[cfg(feature = "std")]
pub(crate) const SNAPSHOT_V3_VERSION: u32 = 3;
#[cfg(feature = "std")]
pub(crate) const SNAPSHOT_SECTION_VERSION: u32 = 1;
#[cfg(feature = "std")]
const PLATFORM_SNAPSHOT_SECTION_VERSION: u32 = 2;

#[cfg(feature = "std")]
pub(crate) const MAX_SNAPSHOT_SECTION_LEN: u64 = 4 * 1024 * 1024 * 1024;
#[cfg(feature = "std")]
pub(crate) const MAX_SNAPSHOT_COUNT: usize = 1 << 20;
#[cfg(feature = "std")]
pub(crate) const MAX_SNAPSHOT_QUEUE_LEN: usize = 1 << 16;

#[cfg(feature = "std")]
pub(crate) mod bounds {
    pub(crate) use super::{
        MAX_SNAPSHOT_COUNT, MAX_SNAPSHOT_QUEUE_LEN, MAX_SNAPSHOT_SECTION_LEN,
    };
}

#[cfg(feature = "std")]
pub(crate) const SEC_CPU: u32 = 1;
#[cfg(feature = "std")]
pub(crate) const SEC_MEMORY: u32 = 10;
#[cfg(feature = "std")]
pub(crate) const SEC_PIC: u32 = 20;
#[cfg(feature = "std")]
pub(crate) const SEC_PIT: u32 = 21;
#[cfg(feature = "std")]
pub(crate) const SEC_CMOS: u32 = 22;
#[cfg(feature = "std")]
pub(crate) const SEC_DMA: u32 = 23;
#[cfg(feature = "std")]
pub(crate) const SEC_VGA: u32 = 24;
#[cfg(feature = "std")]
pub(crate) const SEC_KEYBOARD: u32 = 25;
#[cfg(feature = "std")]
pub(crate) const SEC_SERIAL: u32 = 26;
#[cfg(feature = "std")]
pub(crate) const SEC_HARDDRV: u32 = 27;
#[cfg(feature = "std")]
pub(crate) const SEC_IOAPIC: u32 = 28;
#[cfg(feature = "std")]
pub(crate) const SEC_LAPIC: u32 = 29;
#[cfg(feature = "std")]
pub(crate) const SEC_PC_SYSTEM: u32 = 30;
#[cfg(feature = "std")]
pub(crate) const SEC_PCI: u32 = 31;
#[cfg(feature = "std")]
pub(crate) const SEC_ACPI: u32 = 32;
#[cfg(feature = "std")]
pub(crate) const SEC_PLATFORM: u32 = 33;

#[cfg(feature = "std")]
pub(crate) const SNAPSHOT_V3_SECTION_ORDER: [u32; 16] = [
    SEC_MEMORY, SEC_PC_SYSTEM, SEC_PLATFORM, SEC_CPU, SEC_PIC, SEC_PIT, SEC_CMOS, SEC_DMA,
    SEC_KEYBOARD, SEC_SERIAL, SEC_HARDDRV, SEC_PCI, SEC_ACPI, SEC_VGA, SEC_IOAPIC, SEC_LAPIC,
];

#[cfg(feature = "std")]
fn invalid_snapshot(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
fn checked_snapshot_section_len(len: u64) -> io::Result<u64> {
    if len > MAX_SNAPSHOT_SECTION_LEN {
        return Err(invalid_snapshot("snapshot length exceeds implementation bound"));
    }
    Ok(len)
}

#[cfg(feature = "std")]
pub(crate) fn checked_snapshot_len_add(lhs: u64, rhs: u64) -> io::Result<u64> {
    lhs.checked_add(rhs)
        .ok_or_else(|| invalid_snapshot("snapshot length addition overflows"))
        .and_then(checked_snapshot_section_len)
}

#[cfg(feature = "std")]
pub(crate) fn checked_snapshot_len_mul(lhs: u64, rhs: u64) -> io::Result<u64> {
    lhs.checked_mul(rhs)
        .ok_or_else(|| invalid_snapshot("snapshot length multiplication overflows"))
        .and_then(checked_snapshot_section_len)
}

/// Little-endian primitive writes shared by the bounded codecs.
#[cfg(feature = "std")]
pub(crate) trait SnapshotWriteExt: Write {
    fn write_u8(&mut self, value: u8) -> io::Result<()> { self.write_all(&[value]) }
    fn write_bool(&mut self, value: bool) -> io::Result<()> { self.write_u8(u8::from(value)) }
    fn write_u16(&mut self, value: u16) -> io::Result<()> { self.write_all(&value.to_le_bytes()) }
    fn write_u32(&mut self, value: u32) -> io::Result<()> { self.write_all(&value.to_le_bytes()) }
    fn write_u64(&mut self, value: u64) -> io::Result<()> { self.write_all(&value.to_le_bytes()) }
    fn write_i64(&mut self, value: i64) -> io::Result<()> { self.write_all(&value.to_le_bytes()) }
    fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> { self.write_all(bytes) }
}
#[cfg(feature = "std")]
impl<W: Write + ?Sized> SnapshotWriteExt for W {}

/// Reader limited to one section (or one nested CPU/LAPIC record).
#[cfg(feature = "std")]
pub(crate) struct SnapshotReader<R> { inner: R, remaining: u64 }
#[cfg(feature = "std")]
impl<R: Read> SnapshotReader<R> {
    pub(crate) fn new(inner: R, remaining: u64) -> io::Result<Self> {
        Self::new_with_limit(inner, remaining, MAX_SNAPSHOT_SECTION_LEN)
    }

    fn new_with_limit(inner: R, remaining: u64, maximum: u64) -> io::Result<Self> {
        if remaining > maximum {
            return Err(invalid_snapshot("snapshot length exceeds implementation bound"));
        }
        Ok(Self { inner, remaining })
    }
    pub(crate) fn read_u8(&mut self) -> io::Result<u8> { let mut b = [0; 1]; self.read_bytes(&mut b)?; Ok(b[0]) }
    pub(crate) fn read_bool(&mut self) -> io::Result<bool> {
        match self.read_u8()? { 0 => Ok(false), 1 => Ok(true), _ => Err(invalid_snapshot("snapshot boolean is not canonical")) }
    }
    pub(crate) fn read_u16(&mut self) -> io::Result<u16> { let mut b = [0; 2]; self.read_bytes(&mut b)?; Ok(u16::from_le_bytes(b)) }
    pub(crate) fn read_u32(&mut self) -> io::Result<u32> { let mut b = [0; 4]; self.read_bytes(&mut b)?; Ok(u32::from_le_bytes(b)) }
    pub(crate) fn read_u64(&mut self) -> io::Result<u64> { let mut b = [0; 8]; self.read_bytes(&mut b)?; Ok(u64::from_le_bytes(b)) }
    pub(crate) fn read_i64(&mut self) -> io::Result<i64> { let mut b = [0; 8]; self.read_bytes(&mut b)?; Ok(i64::from_le_bytes(b)) }
    pub(crate) fn read_count(&mut self, maximum: usize) -> io::Result<usize> {
        let value = usize::try_from(self.read_u32()?).map_err(|_| invalid_snapshot("snapshot count does not fit usize"))?;
        if value > maximum.min(MAX_SNAPSHOT_COUNT) { return Err(invalid_snapshot("snapshot count exceeds bound")); }
        Ok(value)
    }
    pub(crate) fn read_len(&mut self, maximum: usize) -> io::Result<usize> {
        let value = self.read_u64()?;
        if value > u64::try_from(maximum).unwrap_or(u64::MAX).min(MAX_SNAPSHOT_SECTION_LEN) { return Err(invalid_snapshot("snapshot length exceeds bound")); }
        usize::try_from(value).map_err(|_| invalid_snapshot("snapshot length does not fit usize"))
    }
    pub(crate) fn read_bytes(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        let len = u64::try_from(bytes.len()).map_err(|_| invalid_snapshot("snapshot byte length does not fit u64"))?;
        if len > self.remaining { return Err(Error::new(ErrorKind::UnexpectedEof, "snapshot section is truncated")); }
        self.inner.read_exact(bytes)?;
        self.remaining -= len;
        Ok(())
    }
    pub(crate) fn discard(&mut self) -> io::Result<()> {
        let mut scratch = [0u8; 64 * 1024];
        while self.remaining != 0 { let n = self.remaining.min(scratch.len() as u64) as usize; self.read_bytes(&mut scratch[..n])?; }
        Ok(())
    }
    pub(crate) fn finish_exact(&self) -> io::Result<()> {
        if self.remaining == 0 { Ok(()) } else { Err(invalid_snapshot("snapshot section has trailing bytes")) }
    }
}
#[cfg(feature = "std")]
impl<R: Read> Read for SnapshotReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() || self.remaining == 0 { return Ok(0); }
        let n = (self.remaining.min(bytes.len() as u64)) as usize;
        let got = self.inner.read(&mut bytes[..n])?;
        self.remaining -= got as u64;
        Ok(got)
    }
}

#[cfg(feature = "std")]
struct SectionWriter<'a, W: Write + ?Sized> { inner: &'a mut W, id: u32, remaining: u64 }
#[cfg(feature = "std")]
impl<W: Write + ?Sized> Write for SectionWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let len = u64::try_from(bytes.len()).map_err(|_| invalid_snapshot("snapshot write length does not fit u64"))?;
        if len > self.remaining {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("snapshot section {} writer overran declared length", self.id),
            ));
        }
        self.inner.write_all(bytes)?;
        self.remaining -= len;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { self.inner.flush() }
}

#[cfg(feature = "std")]
fn write_section_with_limit<W: Write + ?Sized>(
    writer: &mut W,
    id: u32,
    len: u64,
    maximum: u64,
    body: impl FnOnce(&mut SectionWriter<'_, W>) -> io::Result<()>,
) -> io::Result<()> {
    if len > maximum {
        return Err(invalid_snapshot("snapshot length exceeds implementation bound"));
    }
    writer.write_u32(id)?;
    writer.write_u64(len)?;
    let mut section = SectionWriter { inner: writer, id, remaining: len };
    body(&mut section)?;
    if section.remaining == 0 {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::InvalidData,
            format!("snapshot section {id} writer under-ran declared length"),
        ))
    }
}

#[cfg(feature = "std")]
fn write_section<W: Write + ?Sized>(
    writer: &mut W,
    id: u32,
    len: u64,
    body: impl FnOnce(&mut SectionWriter<'_, W>) -> io::Result<()>,
) -> io::Result<()> {
    write_section_with_limit(writer, id, len, MAX_SNAPSHOT_SECTION_LEN, body)
}

#[cfg(feature = "std")]
fn memory_block_len(geometry: MemorySnapshotGeometry, block: u32) -> io::Result<u64> {
    let start = u64::from(block).checked_mul(geometry.block_size).ok_or_else(|| invalid_snapshot("snapshot memory block offset overflows"))?;
    if start >= geometry.guest_len { return Err(invalid_snapshot("snapshot memory block exceeds guest RAM")); }
    Ok((geometry.guest_len - start).min(geometry.block_size))
}
#[cfg(feature = "std")]
fn memory_payload_len_for_geometry(geometry: MemorySnapshotGeometry) -> io::Result<u64> {
    let descriptors = u64::from(geometry.num_blocks)
        .checked_mul(5)
        .ok_or_else(|| invalid_snapshot("snapshot memory descriptor length overflows"))?;
    44u64
        .checked_add(descriptors)
        .and_then(|len| len.checked_add(geometry.guest_len))
        .ok_or_else(|| invalid_snapshot("snapshot memory payload length overflows"))
}

#[cfg(feature = "std")]
fn memory_payload_len(memory: &BxMemC<'_>) -> io::Result<u64> {
    memory_payload_len_for_geometry(memory.snapshot_geometry())
}
#[cfg(feature = "std")]
fn save_memory<W: Write>(memory: &BxMemC<'_>, writer: &mut W) -> io::Result<()> {
    let g = memory.snapshot_geometry();
    writer.write_u32(SNAPSHOT_SECTION_VERSION)?;
    writer.write_u64(g.guest_len)?; writer.write_u64(g.host_ram_len)?; writer.write_u64(g.block_size)?;
    writer.write_u32(g.num_blocks)?; writer.write_u32(g.resident_capacity)?; writer.write_u32(g.used_blocks)?; writer.write_u32(g.next_swapout_guest_block)?;
    for block in 0..g.num_blocks {
        match memory.snapshot_residency(block)? {
            MemorySnapshotResidency::Swapped => { writer.write_u8(0)?; writer.write_u32(u32::MAX)?; }
            MemorySnapshotResidency::Resident { slot } => { writer.write_u8(1)?; writer.write_u32(slot)?; }
        }
        memory.write_snapshot_block(block, writer)?;
    }
    Ok(())
}
#[cfg(feature = "std")]
fn restore_memory<R: Read>(memory: &mut BxMemC<'_>, reader: &mut SnapshotReader<R>) -> io::Result<()> {
    if reader.read_u32()? != SNAPSHOT_SECTION_VERSION { return Err(invalid_snapshot("unsupported memory snapshot section version")); }
    let saved = MemorySnapshotGeometry {
        guest_len: reader.read_u64()?, host_ram_len: reader.read_u64()?, block_size: reader.read_u64()?,
        num_blocks: reader.read_u32()?, resident_capacity: reader.read_u32()?, used_blocks: reader.read_u32()?, next_swapout_guest_block: reader.read_u32()?,
    };
    let live = memory.snapshot_geometry();
    let rounded = saved.guest_len.checked_add(saved.block_size.checked_sub(1).ok_or_else(|| invalid_snapshot("snapshot memory block size is zero"))?).ok_or_else(|| invalid_snapshot("snapshot memory geometry overflows"))? / saved.block_size;
    if saved.guest_len != live.guest_len || saved.host_ram_len != live.host_ram_len || saved.block_size != live.block_size || saved.num_blocks != live.num_blocks || saved.resident_capacity != live.resident_capacity || rounded != u64::from(saved.num_blocks) || saved.used_blocks > saved.resident_capacity || saved.used_blocks > saved.num_blocks || (saved.num_blocks == 0 && saved.next_swapout_guest_block != 0) || (saved.num_blocks != 0 && saved.next_swapout_guest_block >= saved.num_blocks) { return Err(invalid_snapshot("snapshot memory geometry does not match machine")); }
    let count = usize::try_from(saved.num_blocks).map_err(|_| invalid_snapshot("snapshot memory block count does not fit usize"))?;
    let slots = usize::try_from(saved.resident_capacity).map_err(|_| invalid_snapshot("snapshot resident capacity does not fit usize"))?;
    let mut map = Vec::new(); map.try_reserve_exact(count).map_err(|_| invalid_snapshot("unable to allocate snapshot memory descriptors"))?;
    let mut seen = Vec::new(); seen.try_reserve_exact(slots).map_err(|_| invalid_snapshot("unable to allocate snapshot resident slots"))?; seen.resize(slots, false);
    let mut used = 0usize;
    for block in 0..saved.num_blocks {
        let tag = reader.read_u8()?; let slot = reader.read_u32()?;
        let residency = match (tag, slot) {
            (0, u32::MAX) => MemorySnapshotResidency::Swapped,
            (1, slot) => {
                let index = usize::try_from(slot).map_err(|_| invalid_snapshot("snapshot resident slot does not fit usize"))?;
                if index >= slots || seen[index] { return Err(invalid_snapshot("snapshot resident slot is invalid or duplicate")); }
                let offset = u64::from(slot).checked_mul(saved.block_size).ok_or_else(|| invalid_snapshot("snapshot resident slot offset overflows"))?;
                if offset.checked_add(memory_block_len(saved, block)?).ok_or_else(|| invalid_snapshot("snapshot resident slot extent overflows"))? > saved.host_ram_len { return Err(invalid_snapshot("snapshot resident slot exceeds host RAM")); }
                seen[index] = true; used += 1; MemorySnapshotResidency::Resident { slot }
            }
            _ => return Err(invalid_snapshot("snapshot memory residency tag is invalid")),
        };
        map.push(residency);
        memory.read_snapshot_block(block, residency, reader)?;
    }
    if used != usize::try_from(saved.used_blocks).map_err(|_| invalid_snapshot("snapshot used count does not fit usize"))? || seen[..used].iter().any(|present| !present) { return Err(invalid_snapshot("snapshot memory residency count is inconsistent")); }
    memory.finish_snapshot_restore(saved, &map)
}

#[cfg(feature = "std")]
fn cpu_len<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(emu: &Emulator<'_, I, T>) -> io::Result<u64> {
    let count = u32::try_from(emu.cpu_count()).map_err(|_| invalid_snapshot("CPU count does not fit snapshot"))?;
    let mut len = 8u64;
    for index in 0..count as usize { len = checked_snapshot_len_add(len, checked_snapshot_len_add(12, emu.cpu_ref(index).snapshot_v3_body_len()?)?)?; }
    Ok(len)
}
#[cfg(feature = "std")]
fn save_cpus<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation, W: Write>(emu: &Emulator<'_, I, T>, writer: &mut W) -> io::Result<()> {
    writer.write_u32(SNAPSHOT_SECTION_VERSION)?; writer.write_u32(u32::try_from(emu.cpu_count()).map_err(|_| invalid_snapshot("CPU count does not fit snapshot"))?)?;
    for index in 0..emu.cpu_count() { let cpu = emu.cpu_ref(index); writer.write_u32(cpu.snapshot_cpu_id())?; writer.write_u64(cpu.snapshot_v3_body_len()?)?; cpu.save_snapshot_v3_body(writer)?; }
    Ok(())
}
#[cfg(feature = "std")]
fn lapic_len<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(emu: &Emulator<'_, I, T>) -> io::Result<u64> {
    let mut len = 8u64;
    for index in 0..emu.cpu_count() { len = checked_snapshot_len_add(len, checked_snapshot_len_add(12, emu.cpu_ref(index).lapic.snapshot_v3_body_len()?)?)?; }
    Ok(len)
}
#[cfg(feature = "std")]
fn save_lapics<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation, W: Write>(emu: &Emulator<'_, I, T>, writer: &mut W) -> io::Result<()> {
    writer.write_u32(SNAPSHOT_SECTION_VERSION)?; writer.write_u32(u32::try_from(emu.cpu_count()).map_err(|_| invalid_snapshot("CPU count does not fit snapshot"))?)?;
    for index in 0..emu.cpu_count() { let cpu = emu.cpu_ref(index); writer.write_u32(cpu.snapshot_cpu_id())?; writer.write_u64(cpu.lapic.snapshot_v3_body_len()?)?; cpu.lapic.save_snapshot_v3_body(writer)?; }
    Ok(())
}

#[cfg(feature = "std")]
impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> Emulator<'_, I, T> {
    pub fn save_snapshot<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(SNAPSHOT_MAGIC)?; writer.write_u32(SNAPSHOT_V3_VERSION)?; writer.write_u32(SNAPSHOT_V3_SECTION_ORDER.len() as u32)?;
        let memory_len = memory_payload_len(&self.memory)?;
        write_section_with_limit(writer, SEC_MEMORY, memory_len, memory_len, |s| save_memory(&self.memory, s))?;
        write_section(writer, SEC_PC_SYSTEM, self.pc_system.snapshot_v3_len()?, |s| self.pc_system.save_snapshot_v3(s))?;
        let platform_len = checked_snapshot_len_add(4, checked_snapshot_len_add(self.device_manager.fw_cfg.snapshot_v3_body_len()?, checked_snapshot_len_add(self.devices.snapshot_v3_body_len()?, self.device_manager.snapshot_v3_body_len()?)?)?)?;
        write_section(writer, SEC_PLATFORM, platform_len, |s| { s.write_u32(PLATFORM_SNAPSHOT_SECTION_VERSION)?; self.device_manager.fw_cfg.save_snapshot_v3_body(s)?; self.devices.save_snapshot_v3_body(s)?; self.device_manager.save_snapshot_v3_body(s) })?;
        write_section(writer, SEC_CPU, cpu_len(self)?, |s| save_cpus(self, s))?;
        write_section(writer, SEC_PIC, self.device_manager.pic.snapshot_v3_len()?, |s| self.device_manager.pic.save_snapshot_v3(s))?;
        write_section(writer, SEC_PIT, self.device_manager.pit.snapshot_v3_len()?, |s| self.device_manager.pit.save_snapshot_v3(s))?;
        write_section(writer, SEC_CMOS, self.device_manager.cmos.snapshot_v3_len()?, |s| self.device_manager.cmos.save_snapshot_v3(s))?;
        write_section(writer, SEC_DMA, self.device_manager.dma.snapshot_v3_len()?, |s| self.device_manager.dma.save_snapshot_v3(s))?;
        write_section(writer, SEC_KEYBOARD, self.device_manager.keyboard.snapshot_v3_len()?, |s| self.device_manager.keyboard.save_snapshot_v3(s))?;
        write_section(writer, SEC_SERIAL, self.device_manager.serial.snapshot_v3_len()?, |s| self.device_manager.serial.save_snapshot_v3(s))?;
        write_section(writer, SEC_HARDDRV, self.device_manager.harddrv.snapshot_v3_len()?, |s| self.device_manager.harddrv.save_snapshot_v3(s))?;
        let pci_len = checked_snapshot_len_add(4, checked_snapshot_len_add(self.device_manager.pci_bridge.snapshot_v3_body_len()?, checked_snapshot_len_add(self.device_manager.pci2isa.snapshot_v3_body_len()?, self.device_manager.pci_ide.snapshot_v3_body_len()?)?)?)?;
        write_section(writer, SEC_PCI, pci_len, |s| { s.write_u32(SNAPSHOT_SECTION_VERSION)?; self.device_manager.pci_bridge.save_snapshot_v3_body(s)?; self.device_manager.pci2isa.save_snapshot_v3_body(s)?; self.device_manager.pci_ide.save_snapshot_v3_body(s) })?;
        write_section(writer, SEC_ACPI, self.device_manager.acpi.snapshot_v3_len()?, |s| self.device_manager.acpi.save_snapshot_v3(s))?;
        write_section(writer, SEC_VGA, self.device_manager.vga.snapshot_v3_len()?, |s| self.device_manager.vga.save_snapshot_v3(s))?;
        write_section(writer, SEC_IOAPIC, self.device_manager.ioapic.snapshot_v3_len()?, |s| self.device_manager.ioapic.save_snapshot_v3(s))?;
        write_section(writer, SEC_LAPIC, lapic_len(self)?, |s| save_lapics(self, s))
    }

    pub fn restore_snapshot<R: Read>(&mut self, reader: &mut R) -> io::Result<()> {
        let result = self.restore_snapshot_inner(reader);
        if result.is_err() {
            self.mark_snapshot_restore_failed();
        }
        result
    }

    fn restore_snapshot_inner<R: Read>(&mut self, reader: &mut R) -> io::Result<()> {
        let mut magic = [0u8; 8]; reader.read_exact(&mut magic)?;
        if magic != *SNAPSHOT_MAGIC { return Err(invalid_snapshot("not a valid snapshot file")); }
        let version = read_outer_u32(reader)?;
        if version != SNAPSHOT_V3_VERSION { return Err(invalid_snapshot("snapshot version is not supported")); }
        let count = usize::try_from(read_outer_u32(reader)?).map_err(|_| invalid_snapshot("snapshot section count does not fit usize"))?;
        if !(SNAPSHOT_V3_SECTION_ORDER.len()..=MAX_SNAPSHOT_COUNT).contains(&count) { return Err(invalid_snapshot("snapshot section count is invalid")); }
        let live_bmdma = self.device_manager.bmdma_ports_base;
        let live_pm = self.device_manager.pm_ports_base;
        let live_sm = self.device_manager.sm_ports_base;
        let live_vga = self.device_manager.vga.snapshot_v3_committed_mapping_target();
        let mut expected = 0usize;
        let mut platform = None;
        let mut io_platform = None;
        let mut pit_decoded = false;
        let mut cmos_decoded = false;
        let mut keyboard = None;
        let mut acpi = None;
        let mut pci = None;
        let mut vga = None;
        for _ in 0..count {
            let id = read_outer_u32(reader)?;
            let len = read_outer_u64(reader)?;
            if expected == SNAPSHOT_V3_SECTION_ORDER.len() || id != SNAPSHOT_V3_SECTION_ORDER[expected] {
                if SNAPSHOT_V3_SECTION_ORDER.contains(&id) { return Err(invalid_snapshot("snapshot known section is duplicate or out of order")); }
                let mut extension = SnapshotReader::new(&mut *reader, len)?;
                extension.discard()?;
                continue;
            }
            let mut section = if id == SEC_MEMORY {
                SnapshotReader::new_with_limit(
                    &mut *reader,
                    len,
                    memory_payload_len(&self.memory)?,
                )?
            } else {
                SnapshotReader::new(&mut *reader, len)?
            };
            let decoded = (|| -> io::Result<()> {
                match id {
                    SEC_MEMORY => restore_memory(&mut self.memory, &mut section)?,
                    SEC_PC_SYSTEM => self.pc_system.restore_snapshot_v3(&mut section)?,
                    SEC_PLATFORM => {
                        if section.read_u32()? != PLATFORM_SNAPSHOT_SECTION_VERSION {
                            return Err(invalid_snapshot(
                                "unsupported platform snapshot section version",
                            ));
                        }
                        self.device_manager
                            .fw_cfg
                            .restore_snapshot_v3_body(&mut section)
                            .map_err(|error| {
                                Error::new(
                                    error.kind(),
                                    format!("fw_cfg platform state is invalid: {error}"),
                                )
                            })?;
                        io_platform = Some(
                            self.devices
                                .restore_snapshot_v3_body(&mut section)
                                .map_err(|error| {
                                    Error::new(
                                        error.kind(),
                                        format!("I/O platform state is invalid: {error}"),
                                    )
                                })?,
                        );
                        platform = Some(
                            self.device_manager
                                .restore_snapshot_v3_body(&mut section)
                                .map_err(|error| {
                                    Error::new(
                                        error.kind(),
                                        format!("device-manager platform state is invalid: {error}"),
                                    )
                                })?,
                        );
                    }
                    SEC_CPU => self.restore_cpus(&mut section)?,
                    SEC_PIC => self.device_manager.pic.restore_snapshot_v3(&mut section)?,
                    SEC_PIT => {
                        self.device_manager.pit.restore_snapshot_v3(&mut section)?;
                        pit_decoded = true;
                    }
                    SEC_CMOS => {
                        self.device_manager.cmos.restore_snapshot_v3(&mut section)?;
                        cmos_decoded = true;
                    }
                    SEC_DMA => self.device_manager.dma.restore_snapshot_v3(&mut section)?,
                    SEC_KEYBOARD => {
                        keyboard =
                            Some(self.device_manager.keyboard.restore_snapshot_v3(&mut section)?);
                    }
                    SEC_SERIAL => self.device_manager.serial.restore_snapshot_v3(&mut section)?,
                    SEC_HARDDRV => self.device_manager.harddrv.restore_snapshot_v3(&mut section)?,
                    SEC_PCI => {
                        if section.read_u32()? != SNAPSHOT_SECTION_VERSION {
                            return Err(invalid_snapshot(
                                "unsupported PCI snapshot section version",
                            ));
                        }
                        self.device_manager
                            .pci_bridge
                            .restore_snapshot_v3_body(&mut section)?;
                        self.device_manager
                            .pci2isa
                            .restore_snapshot_v3_body(&mut section)?;
                        pci = Some(
                            self.device_manager
                                .pci_ide
                                .restore_snapshot_v3_body(&mut section)?,
                        );
                    }
                    SEC_ACPI => {
                        acpi = Some(self.device_manager.acpi.restore_snapshot_v3(&mut section)?);
                    }
                    SEC_VGA => {
                        vga = Some(self.device_manager.vga.restore_snapshot_v3(&mut section)?);
                    }
                    SEC_IOAPIC => self.device_manager.ioapic.restore_snapshot_v3(&mut section)?,
                    SEC_LAPIC => self.restore_lapics(&mut section)?,
                    _ => unreachable!(),
                }
                Ok(())
            })();
            decoded.map_err(|error| {
                Error::new(
                    error.kind(),
                    format!("snapshot section {id} could not be restored: {error}"),
                )
            })?;
            section.finish_exact().map_err(|error| {
                Error::new(
                    error.kind(),
                    format!("snapshot section {id} was not consumed exactly: {error}"),
                )
            })?;
            expected += 1;
        }
        if expected != SNAPSHOT_V3_SECTION_ORDER.len() { return Err(invalid_snapshot("snapshot is missing a required section")); }
        let mut trailing = [0u8; 1]; if reader.read(&mut trailing)? != 0 { return Err(invalid_snapshot("snapshot has trailing bytes")); }
        let platform = platform.ok_or_else(|| invalid_snapshot("snapshot platform section was not decoded"))?;
        if !pit_decoded { return Err(invalid_snapshot("snapshot PIT section was not decoded")); }
        if !cmos_decoded { return Err(invalid_snapshot("snapshot CMOS section was not decoded")); }
        let keyboard = keyboard.ok_or_else(|| invalid_snapshot("snapshot keyboard section was not decoded"))?;
        let acpi = acpi.ok_or_else(|| invalid_snapshot("snapshot ACPI section was not decoded"))?;
        let pci = pci.ok_or_else(|| invalid_snapshot("snapshot PCI section was not decoded"))?;
        let vga = vga.ok_or_else(|| invalid_snapshot("snapshot VGA section was not decoded"))?;
        let io_platform = io_platform.ok_or_else(|| invalid_snapshot("snapshot I/O platform state was not decoded"))?;
        if io_platform.pci_conf_addr != platform.pci_conf_addr {
            return Err(invalid_snapshot("snapshot PCI config latches disagree"));
        }
        for index in 0..self.cpu_count() {
            let cpu = self.cpu_ref(index);
            if cpu.snapshot_a20_mask() != self.pc_system.a20_mask() {
                return Err(invalid_snapshot(
                    "snapshot CPU and PC-system A20 masks disagree",
                ));
            }
            cpu.validate_snapshot_lapic_binding(
                cpu.lapic.get_base(),
                cpu.lapic.get_mode() as u64,
            )?;
        }
        let pit = self.device_manager.pit.post_restore_snapshot_v3();
        let cmos = self.device_manager.cmos.post_restore_snapshot_v3();
        self.memory.set_a20_mask(self.pc_system.a20_mask());
        self.invalidate_all_cpu_host_mappings();
        self.validate_post_restore_handles(pit, cmos, keyboard, acpi)?;
        self.device_manager.pci_ide.validate_snapshot_v3_timer_owners(&self.pc_system)?;
        self.device_manager.serial.validate_snapshot_v3_timer_handles(|port, handle| self.pc_system.validate_timer_handle_owner(handle, TimerOwner::SerialFifo(port)))?;
        if platform.desired_bmdma_base != pci.bmdma_base || platform.desired_pm_base != acpi.pm_base || platform.desired_sm_base != acpi.sm_base || platform.desired_vga_lfb_base != vga.lfb_base || platform.desired_vga_mmio_base != vga.mmio_base { return Err(invalid_snapshot("snapshot mapping targets disagree across sections")); }
        self.finish_snapshot_restore_v3(
            live_bmdma, live_pm, live_sm, live_vga, platform, keyboard, acpi, vga, pci,
        )
    }

    fn restore_cpus<R: Read>(&mut self, section: &mut SnapshotReader<R>) -> io::Result<()> {
        if section.read_u32()? != SNAPSHOT_SECTION_VERSION { return Err(invalid_snapshot("unsupported CPU snapshot section version")); }
        let count = section.read_count(self.cpu_count())?; if count != self.cpu_count() { return Err(invalid_snapshot("snapshot CPU count does not match machine")); }
        for index in 0..count { let id = section.read_u32()?; if id != self.cpu_ref(index).snapshot_cpu_id() { return Err(invalid_snapshot("snapshot CPU ID is not in configured order")); } let len = section.read_u64()?; let mut body = SnapshotReader::new(&mut *section, len)?; self.cpu_mut_at(index).restore_snapshot_v3_body(&mut body, id)?; body.finish_exact()?; }
        Ok(())
    }

    fn restore_lapics<R: Read>(&mut self, section: &mut SnapshotReader<R>) -> io::Result<()> {
        if section.read_u32()? != SNAPSHOT_SECTION_VERSION { return Err(invalid_snapshot("unsupported LAPIC snapshot section version")); }
        let count = section.read_count(self.cpu_count())?; if count != self.cpu_count() { return Err(invalid_snapshot("snapshot LAPIC count does not match machine")); }
        for index in 0..count { let id = section.read_u32()?; if id != self.cpu_ref(index).snapshot_cpu_id() { return Err(invalid_snapshot("snapshot LAPIC CPU ID is not in configured order")); } let len = section.read_u64()?; let mut body = SnapshotReader::new(&mut *section, len)?; let restored = self.cpu_mut_at(index).lapic.restore_snapshot_v3_body(&mut body)?; body.finish_exact()?; for handle in [restored.timer_handle, restored.vmx_timer_handle, restored.mwaitx_timer_handle].into_iter().flatten() { self.pc_system.validate_timer_handle_owner(handle, TimerOwner::Lapic(index))?; } }
        Ok(())
    }

    fn validate_post_restore_handles(&self, pit: crate::iodev::pit::PitSnapshotRestoreState, cmos: crate::iodev::cmos::CmosSnapshotRestoreState, keyboard: crate::iodev::keyboard::KeyboardSnapshotRestore, acpi: crate::iodev::acpi::AcpiSnapshotRestore) -> io::Result<()> {
        if let Some(handle) = pit.timer_handle { self.pc_system.validate_timer_handle_owner(handle, TimerOwner::Pit)?; }
        for (handle, owner) in [(cmos.periodic_timer_handle, TimerOwner::CmosPeriodic), (cmos.one_second_timer_handle, TimerOwner::CmosOneSecond), (cmos.uip_timer_handle, TimerOwner::CmosUip), (keyboard.timer_handle, TimerOwner::Keyboard), (acpi.overflow_timer_handle, TimerOwner::AcpiPmOverflow)] { if let Some(handle) = handle { self.pc_system.validate_timer_handle_owner(handle, owner)?; } }
        Ok(())
    }
}

#[cfg(feature = "std")]
fn read_outer_u32<R: Read>(reader: &mut R) -> io::Result<u32> { let mut b = [0; 4]; reader.read_exact(&mut b)?; Ok(u32::from_le_bytes(b)) }
#[cfg(feature = "std")]
fn read_outer_u64<R: Read>(reader: &mut R) -> io::Result<u64> { let mut b = [0; 8]; reader.read_exact(&mut b)?; Ok(u64::from_le_bytes(b)) }

#[cfg(all(test, feature = "std", feature = "alloc"))]
mod tests {
    use super::*;
    use crate::{
        cpu::{
            core_i7_skylake::Corei7SkylakeX,
            cpu::CpuActivityState,
            instrumentation::CpuSetupMode,
            X86Reg,
        },
        emulator::EmulatorConfig,
        params::BxParams,
    };
    use std::io::Cursor;


    fn on_large_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    fn machine() -> Box<Emulator<'static, Corei7SkylakeX>> {
        let config = EmulatorConfig {
            guest_memory_size: 4 * 1024 * 1024,
            host_memory_size: 4 * 1024 * 1024,
            ..EmulatorConfig::default()
        };
        let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
        emu.initialize().unwrap();
        emu.reset(crate::cpu::ResetReason::Hardware).unwrap();
        emu.setup_cpu_mode(CpuSetupMode::FlatProtected32).unwrap();
        emu
    }

    fn smp_machine() -> Box<Emulator<'static, Corei7SkylakeX>> {
        let config = EmulatorConfig {
            guest_memory_size: 4 * 1024 * 1024,
            host_memory_size: 4 * 1024 * 1024,
            cpu_params: BxParams::default().with_topology(2, 1, 1).unwrap(),
            ..EmulatorConfig::default()
        };
        let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
        emu.initialize().unwrap();
        emu.reset(crate::cpu::ResetReason::Hardware).unwrap();
        emu
    }

    fn relocation_machine() -> Box<Emulator<'static, Corei7SkylakeX>> {
        let config = EmulatorConfig {
            guest_memory_size: 4 * 1024 * 1024,
            host_memory_size: 4 * 1024 * 1024,
            pci_enabled: true,
            pci_vga: true,
            ..EmulatorConfig::default()
        };
        let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
        emu.initialize().unwrap();
        emu.reset(crate::cpu::ResetReason::Hardware).unwrap();
        emu
    }

    fn round_trip_smp(
        source: &mut Emulator<'static, Corei7SkylakeX>,
    ) -> (Box<Emulator<'static, Corei7SkylakeX>>, Vec<u8>) {
        source.service_scheduler_boundary(0).unwrap();
        let mut saved = Vec::new();
        source.save_snapshot(&mut saved).unwrap();
        let mut restored = smp_machine();
        restored.restore_snapshot(&mut Cursor::new(&saved)).unwrap();
        (restored, saved)
    }

    #[derive(Clone, Copy, Debug)]
    struct SnapshotSection {
        id: u32,
        header: usize,
        payload: usize,
        len: usize,
    }

    fn snapshot_sections(snapshot: &[u8]) -> Vec<SnapshotSection> {
        assert!(snapshot.len() >= 16);
        let count = u32::from_le_bytes(snapshot[12..16].try_into().unwrap()) as usize;
        let mut cursor = 16usize;
        let mut sections = Vec::with_capacity(count);
        for _ in 0..count {
            assert!(cursor + 12 <= snapshot.len());
            let id = u32::from_le_bytes(snapshot[cursor..cursor + 4].try_into().unwrap());
            let len =
                u64::from_le_bytes(snapshot[cursor + 4..cursor + 12].try_into().unwrap()) as usize;
            let payload = cursor + 12;
            assert!(payload + len <= snapshot.len());
            sections.push(SnapshotSection {
                id,
                header: cursor,
                payload,
                len,
            });
            cursor = payload + len;
        }
        assert_eq!(cursor, snapshot.len());
        sections
    }

    fn write_u32_at(snapshot: &mut [u8], offset: usize, value: u32) {
        snapshot[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64_at(snapshot: &mut [u8], offset: usize, value: u64) {
        snapshot[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn assert_restore_error(
        snapshot: &[u8],
        kind: io::ErrorKind,
        expected_message: &str,
    ) {
        let mut emu = machine();
        assert!(emu.is_initialized());
        let error = emu
            .restore_snapshot(&mut Cursor::new(snapshot))
            .unwrap_err();
        assert_eq!(error.kind(), kind, "{error}");
        assert!(
            error.to_string().contains(expected_message),
            "expected {expected_message:?} in {error:?}"
        );
        assert!(!emu.is_initialized());
        let execution_error = unsafe { emu.run_cpu_batch(1) }.unwrap_err();
        assert!(
            matches!(execution_error, crate::cpu::CpuError::CpuNotInitialized),
            "poisoned machine executed after restore failure: {execution_error}"
        );
    }

    struct SnapshotHeaderWriter {
        bytes: Vec<u8>,
    }

    impl std::io::Write for SnapshotHeaderWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            const HEADER_LEN: usize = 28;
            let remaining = HEADER_LEN.saturating_sub(self.bytes.len());
            if remaining == 0 {
                return Err(Error::new(ErrorKind::Other, "snapshot header captured"));
            }
            let count = remaining.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn same_instance_restore_does_not_lose_the_next_smc_invalidation() {
        on_large_stack(|| {
            const CODE_PAGE: u64 = 0x1000;
            let mut emu = machine();

            for _ in 0..2 {
                emu.memory.smc_mark_icache_mask(CODE_PAGE, u32::MAX);
                emu.memory.smc_dec_write_stamp(CODE_PAGE, 4096);
                emu.service_scheduler_boundary(0).unwrap();
            }
            assert_eq!(emu.cpu_ref(0).smc_seq_seen, 2);

            let mut saved = Vec::new();
            emu.save_snapshot(&mut saved).unwrap();
            emu.restore_snapshot(&mut Cursor::new(saved)).unwrap();
            assert_eq!(emu.memory.smc_seq_next(), 0);
            assert_eq!(
                emu.cpu_ref(0).smc_seq_seen,
                0,
                "restoring reset memory SMC sequence must reset the CPU watermark"
            );

            emu.memory.smc_mark_icache_mask(CODE_PAGE, u32::MAX);
            emu.memory.smc_dec_write_stamp(CODE_PAGE, 4096);
            emu.service_scheduler_boundary(0).unwrap();

            assert_eq!(
                emu.cpu_ref(0).smc_seq_seen,
                emu.memory.smc_seq_next(),
                "the first post-restore SMC event must reach every CPU"
            );
        });
    }

    #[test]
    fn snapshot_memory_section_accepts_four_gib_guest_stream() {
        const MIB: u64 = 1024 * 1024;
        const FOUR_GIB: u64 = 4 * 1024 * MIB;
        let geometry = MemorySnapshotGeometry {
            guest_len: FOUR_GIB,
            host_ram_len: MIB,
            block_size: MIB,
            num_blocks: 4096,
            resident_capacity: 1,
            used_blocks: 0,
            next_swapout_guest_block: 0,
        };
        let declared = memory_payload_len_for_geometry(geometry).unwrap();
        assert!(declared > MAX_SNAPSHOT_SECTION_LEN);

        let mut writer = SnapshotHeaderWriter { bytes: Vec::new() };
        writer.bytes.extend_from_slice(SNAPSHOT_MAGIC);
        writer.bytes.extend_from_slice(&SNAPSHOT_V3_VERSION.to_le_bytes());
        writer
            .bytes
            .extend_from_slice(&(SNAPSHOT_V3_SECTION_ORDER.len() as u32).to_le_bytes());
        let write_error = write_section_with_limit(
            &mut writer,
            SEC_MEMORY,
            declared,
            declared,
            |section| section.write_u32(SNAPSHOT_SECTION_VERSION),
        )
        .unwrap_err();

        assert_eq!(write_error.kind(), ErrorKind::Other);
        assert_eq!(writer.bytes.len(), 28);
        assert_eq!(
            u32::from_le_bytes(writer.bytes[16..20].try_into().unwrap()),
            SEC_MEMORY
        );
        assert_eq!(
            u64::from_le_bytes(writer.bytes[20..28].try_into().unwrap()),
            declared
        );

        let mut empty = Cursor::new([]);
        let mut section =
            SnapshotReader::new_with_limit(&mut empty, declared, declared).unwrap();
        assert_eq!(
            section.read_u32().unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn snapshot_v3_round_trips_ram_and_all_target_device_state() {
        on_large_stack(|| {
            const DATA: u64 = 0x20_000;
            let mut emu = machine();
            emu.reg_write(X86Reg::Rax, 0x0123_4567_89AB_CDEF);
            emu.reg_write(X86Reg::Rcx, 0x0FED_CBA9_7654_3210);
            emu.mem_write(DATA, &[0x10, 0x20, 0x30, 0x40]).unwrap();
            emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = 0x5A;
            emu.device_manager.keyboard.kbd_controller.outb = true;
            emu.service_scheduler_boundary(0).unwrap();


            let mut saved = Vec::new();
            emu.save_snapshot(&mut saved).unwrap();

            emu.reg_write(X86Reg::Rax, 0);
            emu.reg_write(X86Reg::Rcx, 0);
            emu.mem_fill(DATA, 4, 0xCC).unwrap();
            emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = 0;
            emu.device_manager.keyboard.kbd_controller.outb = false;

            emu.restore_snapshot(&mut Cursor::new(&saved)).unwrap();

            assert_eq!(emu.reg_read(X86Reg::Rax), 0x0123_4567_89AB_CDEF);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 0x0FED_CBA9_7654_3210);
            assert_eq!(emu.mem_read_vec(DATA, 4).unwrap(), [0x10, 0x20, 0x30, 0x40]);
            assert_eq!(
                emu.device_manager.keyboard.kbd_controller.kbd_output_buffer,
                0x5A
            );
            assert!(emu.device_manager.keyboard.kbd_controller.outb);

            let mut resaved = Vec::new();
            emu.save_snapshot(&mut resaved).unwrap();
            let first_difference = resaved
                .iter()
                .zip(&saved)
                .position(|(resaved, saved)| resaved != saved);
            assert!(
                resaved == saved,
                "snapshot resave differs: first={first_difference:?}, resaved_len={}, saved_len={}, resaved={:?}, saved={:?}",
                resaved.len(),
                saved.len(),
                first_difference.map(|index| resaved[index]),
                first_difference.map(|index| saved[index]),
            );
        });
    }

    #[test]
    fn snapshot_v3_round_trips_smp_lapics_in_cpu_order() {
        on_large_stack(|| {
            let mut source = smp_machine();
            source.cpu_mut_at(0).lapic.write_aligned(0x80, 0x20, 0);
            source.cpu_mut_at(1).lapic.write_aligned(0x80, 0x40, 0);

            let (mut restored, saved) = round_trip_smp(&mut source);

            assert_eq!(restored.cpu_ref(0).snapshot_cpu_id(), 0);
            assert_eq!(restored.cpu_ref(1).snapshot_cpu_id(), 1);
            assert_eq!(restored.cpu_mut_at(0).lapic.read_aligned(0x80, 0), 0x20);
            assert_eq!(restored.cpu_mut_at(1).lapic.read_aligned(0x80, 0), 0x40);
            let mut resaved = Vec::new();
            restored.save_snapshot(&mut resaved).unwrap();
            let first_difference = resaved
                .iter()
                .zip(&saved)
                .position(|(resaved, saved)| resaved != saved);
            let divergent_section = first_difference.and_then(|index| {
                snapshot_sections(&saved)
                    .into_iter()
                    .find(|section| index >= section.header && index < section.payload + section.len)
                    .map(|section| (section.id, index - section.payload))
            });
            assert!(
                resaved == saved,
                "SMP snapshot resave differs at {first_difference:?} ({divergent_section:?}): resaved={:?}, saved={:?}",
                first_difference.map(|index| resaved[index]),
                first_difference.map(|index| saved[index]),
            );
        });
    }

    #[test]
    fn snapshot_v3_round_trips_distinct_bsp_and_ap_architectural_activity_state() {
        on_large_stack(|| {
            let mut source = smp_machine();
            source.cpu_mut_at(0).set_rax(0x1111_2222_3333_4444);
            source.cpu_mut_at(1).set_rax(0xaaaa_bbbb_cccc_dddd);
            source.cpu_mut_at(0).activity_state = CpuActivityState::Hlt;
            source.cpu_mut_at(1).activity_state = CpuActivityState::WaitForSipi;
            source.rebuild_cpu_masks_from_scan();

            let (restored, _) = round_trip_smp(&mut source);

            assert_eq!(restored.cpu_ref(0).rax(), 0x1111_2222_3333_4444);
            assert_eq!(restored.cpu_ref(1).rax(), 0xaaaa_bbbb_cccc_dddd);
            assert_eq!(restored.cpu_ref(0).activity_state, CpuActivityState::Hlt);
            assert_eq!(
                restored.cpu_ref(1).activity_state,
                CpuActivityState::WaitForSipi
            );
        });
    }

    #[test]
    fn snapshot_relocates_ide_acpi_and_vga_from_different_live_bases() {
        on_large_stack(|| {
            const BMDMA: u32 = 0xc000;
            const PM: u32 = 0xb000;
            const SM: u32 = 0xb100;
            const LFB: u32 = 0xd000_0000;
            const MMIO: u32 = 0xf100_0000;

            let mut source = relocation_machine();
            assert!(source.device_manager.pci_ide.pci_write(0x20, BMDMA | 1, 4));
            source.device_manager.pci_ide_bar4_needs_reregister = true;
            let (pm_changed, _) = source.device_manager.acpi.pci_write(0x40, PM | 1, 4);
            let (_, sm_changed) = source.device_manager.acpi.pci_write(0x90, SM | 1, 4);
            assert!(pm_changed && sm_changed);
            source.device_manager.acpi_pm_needs_reregister = true;
            source.device_manager.acpi_sm_needs_reregister = true;
            let lfb_change = source.device_manager.vga.pci_write(0x10, LFB, 4);
            let mmio_change = source.device_manager.vga.pci_write(0x18, MMIO, 4);
            assert!(lfb_change.lfb && mmio_change.mmio);
            source.device_manager.vga_bar_needs_reregister = true;
            source.service_scheduler_boundary(0).unwrap();

            let desired_vga = source
                .device_manager
                .vga
                .snapshot_v3_committed_mapping_target();
            assert_eq!(source.device_manager.pci_ide.bmdma_base, BMDMA);
            assert_eq!(source.device_manager.acpi.pm_base, PM);
            assert_eq!(source.device_manager.acpi.sm_base, SM);
            assert_eq!(desired_vga.lfb_base, LFB);
            assert_eq!(desired_vga.mmio_base, MMIO);

            let mut saved = Vec::new();
            source.save_snapshot(&mut saved).unwrap();
            let mut restored = relocation_machine();
            let old_vga = restored
                .device_manager
                .vga
                .snapshot_v3_committed_mapping_target();
            assert_ne!(old_vga.lfb_base, LFB);
            assert_ne!(restored.device_manager.pci_ide.bmdma_base, BMDMA);
            assert_ne!(restored.device_manager.acpi.pm_base, PM);
            assert_ne!(restored.device_manager.acpi.sm_base, SM);

            restored.restore_snapshot(&mut Cursor::new(saved)).unwrap();

            let restored_vga = restored
                .device_manager
                .vga
                .snapshot_v3_committed_mapping_target();
            assert_eq!(restored.device_manager.pci_ide.bmdma_base, BMDMA);
            assert_eq!(restored.device_manager.acpi.pm_base, PM);
            assert_eq!(restored.device_manager.acpi.sm_base, SM);
            assert_eq!(restored_vga, desired_vga);
            assert!(!restored.device_manager.pci_ide_bar4_needs_reregister);
            assert!(!restored.device_manager.acpi_pm_needs_reregister);
            assert!(!restored.device_manager.acpi_sm_needs_reregister);
            assert!(!restored.device_manager.vga_bar_needs_reregister);
        });
    }

    #[test]
    fn snapshot_v3_rejects_old_platform_section_layout() {
        on_large_stack(|| {
            let mut source = machine();
            source.service_scheduler_boundary(0).unwrap();
            let mut saved = Vec::new();
            source.save_snapshot(&mut saved).unwrap();
            let platform = snapshot_sections(&saved)
                .into_iter()
                .find(|section| section.id == SEC_PLATFORM)
                .unwrap();
            write_u32_at(&mut saved, platform.payload, 1);

            assert_restore_error(
                &saved,
                io::ErrorKind::InvalidData,
                "unsupported platform snapshot section version",
            );
        });
    }

    #[test]
    fn snapshot_v3_rejects_cross_section_machine_state_mismatch() {
        on_large_stack(|| {
            let mut a20_source = machine();
            a20_source.service_scheduler_boundary(0).unwrap();
            a20_source.pc_system.set_enable_a20(false);
            let mut a20_snapshot = Vec::new();
            a20_source.save_snapshot(&mut a20_snapshot).unwrap();
            assert_restore_error(
                &a20_snapshot,
                io::ErrorKind::InvalidData,
                "CPU and PC-system A20 masks disagree",
            );

            let mut apic_source = machine();
            apic_source.service_scheduler_boundary(0).unwrap();
            apic_source
                .cpu_mut_at(0)
                .write_msr_for_api(0x1b, 0xfee1_0000 | 0x900)
                .unwrap();
            let mut apic_snapshot = Vec::new();
            apic_source.save_snapshot(&mut apic_snapshot).unwrap();
            assert_restore_error(
                &apic_snapshot,
                io::ErrorKind::InvalidData,
                "APIC-base MSR disagrees",
            );

            let mut elcr_source = machine();
            elcr_source.service_scheduler_boundary(0).unwrap();
            elcr_source.device_manager.pic.set_mode(true, 0x20);
            let mut elcr_snapshot = Vec::new();
            elcr_source.save_snapshot(&mut elcr_snapshot).unwrap();
            assert_restore_error(
                &elcr_snapshot,
                io::ErrorKind::InvalidData,
                "PIIX and PIC trigger modes disagree",
            );
        });
    }

    #[test]
    fn snapshot_v3_rejects_malformed_container_and_poisons_machine() {
        on_large_stack(|| {
            let mut source = machine();
            source.service_scheduler_boundary(0).unwrap();
            let mut saved = Vec::new();
            source.save_snapshot(&mut saved).unwrap();
            let sections = snapshot_sections(&saved);
            let first = sections[0];
            let second = sections[1];
            let last = *sections.last().unwrap();

            let mut wrong_version = saved.clone();
            write_u32_at(&mut wrong_version, 8, SNAPSHOT_V3_VERSION - 1);
            assert_restore_error(
                &wrong_version,
                io::ErrorKind::InvalidData,
                "version is not supported",
            );

            let mut bad_count = saved.clone();
            write_u32_at(
                &mut bad_count,
                12,
                u32::try_from(SNAPSHOT_V3_SECTION_ORDER.len() - 1).unwrap(),
            );
            assert_restore_error(
                &bad_count,
                io::ErrorKind::InvalidData,
                "section count is invalid",
            );

            let mut duplicate = saved.clone();
            write_u32_at(&mut duplicate, second.header, first.id);
            assert_restore_error(
                &duplicate,
                io::ErrorKind::InvalidData,
                "duplicate or out of order",
            );

            let mut out_of_order = saved.clone();
            write_u32_at(&mut out_of_order, first.header, second.id);
            assert_restore_error(
                &out_of_order,
                io::ErrorKind::InvalidData,
                "duplicate or out of order",
            );

            let mut missing = saved.clone();
            write_u32_at(&mut missing, last.header, 0xFFFF_0001);
            assert_restore_error(
                &missing,
                io::ErrorKind::InvalidData,
                "missing a required section",
            );

            assert_restore_error(
                &saved[..saved.len() - 1],
                io::ErrorKind::UnexpectedEof,
                "could not be restored",
            );

            let mut oversized = saved.clone();
            write_u64_at(
                &mut oversized,
                first.header + 4,
                MAX_SNAPSHOT_SECTION_LEN + 1,
            );
            assert_restore_error(
                &oversized,
                io::ErrorKind::InvalidData,
                "length exceeds implementation bound",
            );

            let mut short_section = saved.clone();
            write_u64_at(
                &mut short_section,
                last.header + 4,
                u64::try_from(last.len - 1).unwrap(),
            );
            assert_restore_error(
                &short_section,
                io::ErrorKind::UnexpectedEof,
                "could not be restored",
            );

            let mut under_consumed = saved.clone();
            write_u64_at(
                &mut under_consumed,
                last.header + 4,
                u64::try_from(last.len + 1).unwrap(),
            );
            under_consumed.push(0);
            assert_restore_error(
                &under_consumed,
                io::ErrorKind::InvalidData,
                "was not consumed exactly",
            );

            let mut trailing = saved;
            trailing.push(0);
            assert_restore_error(
                &trailing,
                io::ErrorKind::InvalidData,
                "trailing bytes",
            );
        });
    }

    #[test]
    fn snapshot_v3_rejects_noncanonical_bool_and_invalid_enum() {
        on_large_stack(|| {
            let mut source = machine();
            let fw_cfg_len =
                usize::try_from(source.device_manager.fw_cfg.snapshot_v3_body_len().unwrap())
                    .unwrap();
            source.service_scheduler_boundary(0).unwrap();
            let mut saved = Vec::new();
            source.save_snapshot(&mut saved).unwrap();
            let platform = snapshot_sections(&saved)
                .into_iter()
                .find(|section| section.id == SEC_PLATFORM)
                .unwrap();
            let pci_enabled = platform.payload + 4 + fw_cfg_len;
            let devices_len =
                usize::try_from(source.devices.snapshot_v3_body_len().unwrap()).unwrap();
            let port92_len = usize::try_from(
                source
                    .device_manager
                    .port92
                    .snapshot_v3_body_len()
                    .unwrap(),
            )
            .unwrap();
            let manager = platform.payload + 4 + fw_cfg_len + devices_len;
            let mut pci_enable_mismatch = saved.clone();
            pci_enable_mismatch[pci_enabled] ^= 1;
            assert_restore_error(
                &pci_enable_mismatch,
                io::ErrorKind::InvalidData,
                "PCI enablement does not match",
            );

            let mut pci_latch_mismatch = saved.clone();
            let saved_latch = u32::from_le_bytes(
                pci_latch_mismatch[manager + port92_len..manager + port92_len + 4]
                    .try_into()
                    .unwrap(),
            );
            write_u32_at(
                &mut pci_latch_mismatch,
                manager + port92_len,
                saved_latch ^ 0x8000_0000,
            );
            assert_restore_error(
                &pci_latch_mismatch,
                io::ErrorKind::InvalidData,
                "PCI config latches disagree",
            );
            assert!(saved[pci_enabled] <= 1);
            saved[pci_enabled] = 2;
            assert_restore_error(
                &saved,
                io::ErrorKind::InvalidData,
                "boolean is not canonical",
            );

            let mut enum_source = machine();
            enum_source.device_manager.port92.reset_request =
                Some(crate::cpu::ResetReason::Software);
            let fw_cfg_len = usize::try_from(
                enum_source
                    .device_manager
                    .fw_cfg
                    .snapshot_v3_body_len()
                    .unwrap(),
            )
            .unwrap();
            let devices_len =
                usize::try_from(enum_source.devices.snapshot_v3_body_len().unwrap()).unwrap();
            let mut invalid_enum = Vec::new();
            enum_source.save_snapshot(&mut invalid_enum).unwrap();
            let platform = snapshot_sections(&invalid_enum)
                .into_iter()
                .find(|section| section.id == SEC_PLATFORM)
                .unwrap();
            let port92 = platform.payload + 4 + fw_cfg_len + devices_len;
            assert_eq!(invalid_enum[port92 + 3], 1);
            assert_eq!(invalid_enum[port92 + 4], 0);
            invalid_enum[port92 + 4] = 2;
            assert_restore_error(
                &invalid_enum,
                io::ErrorKind::InvalidData,
                "reset reason is invalid",
            );
        });
    }

    #[test]
    fn snapshot_v3_bounded_unknown_extension_does_not_replace_required_sections() {
        on_large_stack(|| {
            let mut source = machine();
            source.reg_write(X86Reg::Rax, 0xA5A5_5A5A_DEAD_BEEF);
            source.service_scheduler_boundary(0).unwrap();
            let mut extended = Vec::new();
            source.save_snapshot(&mut extended).unwrap();
            write_u32_at(
                &mut extended,
                12,
                u32::try_from(SNAPSHOT_V3_SECTION_ORDER.len() + 1).unwrap(),
            );
            extended.extend_from_slice(&0xFFFF_0001u32.to_le_bytes());
            extended.extend_from_slice(&3u64.to_le_bytes());
            extended.extend_from_slice(&[0xA5, 0x5A, 0xC3]);

            let mut restored = machine();
            restored
                .restore_snapshot(&mut Cursor::new(&extended))
                .unwrap();
            assert!(restored.is_initialized());
            assert_eq!(restored.reg_read(X86Reg::Rax), 0xA5A5_5A5A_DEAD_BEEF);
        });
    }
}
