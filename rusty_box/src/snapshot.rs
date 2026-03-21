//! Emulator snapshot save/restore — binary serialization of full machine state.
//!
//! Format: sectioned binary with magic header.
//! ```text
//! [MAGIC: 8 bytes "RBXSNAP1"]
//! [VERSION: u32 LE]
//! [SECTION_COUNT: u32 LE]
//! For each section:
//!   [SECTION_ID: u32 LE]
//!   [SECTION_LEN: u64 LE]
//!   [DATA: SECTION_LEN bytes]
//! ```

#[cfg(feature = "std")]
use std::io::{Read, Write};

#[cfg(feature = "std")]
use crate::cpu::cpuid::BxCpuIdTrait;
#[cfg(feature = "std")]
use crate::emulator::Emulator;

const SNAPSHOT_MAGIC: &[u8; 8] = b"RBXSNAP1";
const SNAPSHOT_VERSION: u32 = 1;

// Section IDs — matching Bochs siminterface.cc section layout
const SEC_CPU: u32 = 1;
const SEC_MEMORY: u32 = 10;
const SEC_PIC: u32 = 20;
const SEC_PIT: u32 = 21;
const SEC_CMOS: u32 = 22;
#[allow(dead_code)]
const SEC_DMA: u32 = 23;
#[allow(dead_code)]
const SEC_VGA: u32 = 24;
#[allow(dead_code)]
const SEC_KEYBOARD: u32 = 25;
#[allow(dead_code)]
const SEC_SERIAL: u32 = 26;
#[allow(dead_code)]
const SEC_HARDDRV: u32 = 27;
#[allow(dead_code)]
const SEC_IOAPIC: u32 = 28;
#[allow(dead_code)]
const SEC_LAPIC: u32 = 29;
const SEC_PC_SYSTEM: u32 = 30;
#[allow(dead_code)]
const SEC_PCI: u32 = 31;
#[allow(dead_code)]
const SEC_ACPI: u32 = 32;

#[cfg(feature = "std")]
impl<I: BxCpuIdTrait> Emulator<'_, I> {
    /// Save a complete emulator snapshot to a writer.
    pub fn save_snapshot<W: Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        w.write_all(SNAPSHOT_MAGIC)?;
        w.write_all(&SNAPSHOT_VERSION.to_le_bytes())?;

        let mut sections: Vec<(u32, Vec<u8>)> = Vec::new();

        // CPU state (all registers, FPU, VMM, MSRs, mode)
        sections.push((SEC_CPU, self.cpu.save_snapshot_state()));

        // Memory: raw dump of actual_vector
        {
            let mem_stub = self.memory.get_stub_mut();
            let actual = mem_stub.actual_vector();
            sections.push((SEC_MEMORY, actual.to_vec()));
        }

        // PIC state
        sections.push((SEC_PIC, save_pic_state(&self.device_manager.pic)));

        // PIT state
        sections.push((SEC_PIT, save_pit_state(&self.device_manager.pit)));

        // CMOS state
        {
            let mut buf = Vec::new();
            buf.extend_from_slice(&self.device_manager.cmos.ram);
            buf.push(self.device_manager.cmos.address);
            sections.push((SEC_CMOS, buf));
        }

        // PC System
        {
            let mut buf = Vec::new();
            let pc = &self.pc_system;
            buf.extend_from_slice(&(pc.num_timers() as u32).to_le_bytes());
            buf.push(pc.enable_a20 as u8);
            buf.extend_from_slice(&pc.a20_mask.to_le_bytes());
            for i in 0..pc.num_timers() {
                buf.extend_from_slice(&pc.timers[i].period.to_le_bytes());
                buf.extend_from_slice(&pc.timers[i].time_to_fire.to_le_bytes());
                buf.push(pc.timers[i].flags.bits());
            }
            sections.push((SEC_PC_SYSTEM, buf));
        }

        // Write count + sections
        w.write_all(&(sections.len() as u32).to_le_bytes())?;
        for (id, data) in &sections {
            w.write_all(&id.to_le_bytes())?;
            w.write_all(&(data.len() as u64).to_le_bytes())?;
            w.write_all(data)?;
        }

        Ok(())
    }

    /// Restore emulator state from a snapshot.
    pub fn restore_snapshot<R: Read>(&mut self, r: &mut R) -> std::io::Result<()> {
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;
        if &magic != SNAPSHOT_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not a valid snapshot file",
            ));
        }
        let version = read_u32(r)?;
        if version != SNAPSHOT_VERSION {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("snapshot version {version} not supported (expected {SNAPSHOT_VERSION})"),
            ));
        }

        let section_count = read_u32(r)?;

        for _ in 0..section_count {
            let sec_id = read_u32(r)?;
            let sec_len = read_u64(r)?;
            let mut data = vec![0u8; sec_len as usize];
            r.read_exact(&mut data)?;

            match sec_id {
                SEC_CPU => self.cpu.restore_snapshot_state(&data),
                SEC_MEMORY => {
                    let mem_stub = self.memory.get_stub_mut();
                    let actual = mem_stub.actual_vector();
                    let copy_len = data.len().min(actual.len());
                    actual[..copy_len].copy_from_slice(&data[..copy_len]);
                }
                SEC_PIC => restore_pic_state(&mut self.device_manager.pic, &data),
                SEC_PIT => restore_pit_state(&mut self.device_manager.pit, &data),
                SEC_CMOS => {
                    let reg_len = self.device_manager.cmos.ram.len();
                    if data.len() >= reg_len + 1 {
                        self.device_manager.cmos.ram.copy_from_slice(&data[..reg_len]);
                        self.device_manager.cmos.address = data[reg_len];
                    }
                }
                SEC_PC_SYSTEM => {
                    let mut off = 0;
                    let num_timers = u32_at(&data, &mut off) as usize;
                    self.pc_system.enable_a20 = data[off] != 0; off += 1;
                    self.pc_system.a20_mask = u64_at(&data, &mut off);
                    for i in 0..num_timers.min(self.pc_system.timers.len()) {
                        self.pc_system.timers[i].period = u64_at(&data, &mut off);
                        self.pc_system.timers[i].time_to_fire = u64_at(&data, &mut off);
                        self.pc_system.timers[i].flags = crate::pc_system::TimerFlags::from_bits_truncate(data[off]); off += 1;
                    }
                }
                _ => { /* skip unknown sections */ }
            }
        }

        // Sync A20 mask to memory subsystem
        self.memory.set_a20_mask(self.pc_system.a20_mask);

        Ok(())
    }
}

// ============================================================================
// PIC save/restore
// ============================================================================

#[cfg(feature = "std")]
fn save_pic_state(pic: &crate::iodev::pic::BxPicC) -> Vec<u8> {
    let mut buf = Vec::new();
    for p in [&pic.master, &pic.slave] {
        buf.push(p.interrupt_offset);
        buf.push(p.sfnm as u8);
        buf.push(p.buffered_mode as u8);
        buf.push(p.master_slave as u8);
        buf.push(p.auto_eoi as u8);
        buf.push(p.imr);
        buf.push(p.isr);
        buf.push(p.irr);
        buf.push(p.read_reg_select as u8);
        buf.push(p.irq);
        buf.push(p.lowest_priority);
        buf.push(p.int_pin as u8);
        buf.push(p.init.in_init as u8);
        buf.push(p.init.requires_4 as u8);
        buf.push(p.init.byte_expected);
        buf.push(p.special_mask as u8);
        buf.push(p.polled as u8);
        buf.push(p.rotate_on_autoeoi as u8);
        buf.push(p.edge_level);
    }
    buf
}

#[cfg(feature = "std")]
fn restore_pic_state(pic: &mut crate::iodev::pic::BxPicC, d: &[u8]) {
    let mut off = 0;
    for p in [&mut pic.master, &mut pic.slave] {
        p.interrupt_offset = d[off]; off += 1;
        p.sfnm = d[off] != 0; off += 1;
        p.buffered_mode = d[off] != 0; off += 1;
        p.master_slave = d[off] != 0; off += 1;
        p.auto_eoi = d[off] != 0; off += 1;
        p.imr = d[off]; off += 1;
        p.isr = d[off]; off += 1;
        p.irr = d[off]; off += 1;
        p.read_reg_select = d[off] != 0; off += 1;
        p.irq = d[off]; off += 1;
        p.lowest_priority = d[off]; off += 1;
        p.int_pin = d[off] != 0; off += 1;
        p.init.in_init = d[off] != 0; off += 1;
        p.init.requires_4 = d[off] != 0; off += 1;
        p.init.byte_expected = d[off]; off += 1;
        p.special_mask = d[off] != 0; off += 1;
        p.polled = d[off] != 0; off += 1;
        p.rotate_on_autoeoi = d[off] != 0; off += 1;
        p.edge_level = d[off]; off += 1;
    }
}

// ============================================================================
// PIT save/restore
// ============================================================================

#[cfg(feature = "std")]
fn save_pit_state(pit: &crate::iodev::pit::BxPitC) -> Vec<u8> {
    let mut buf = Vec::new();
    for c in &pit.counters {
        buf.push(c.mode);
        buf.extend_from_slice(&c.inlatch.to_le_bytes());
        buf.extend_from_slice(&c.count.to_le_bytes());
        buf.extend_from_slice(&c.count_binary.to_le_bytes());
        buf.extend_from_slice(&c.outlatch.to_le_bytes());
        buf.push(c.rw_mode);
        buf.push(c.null_count as u8);
        buf.push(c.gate as u8);
        buf.push(c.output as u8);
        buf.push(c.bcd_mode as u8);
        buf.push(c.count_written as u8);
    }
    buf.extend_from_slice(&pit.total_ticks.to_le_bytes());
    buf
}

#[cfg(feature = "std")]
fn restore_pit_state(pit: &mut crate::iodev::pit::BxPitC, d: &[u8]) {
    let mut off = 0;
    for c in pit.counters.iter_mut() {
        c.mode = d[off]; off += 1;
        c.inlatch = u16_at(d, &mut off);
        c.count = u16_at(d, &mut off);
        c.count_binary = u16_at(d, &mut off);
        c.outlatch = u16_at(d, &mut off);
        c.rw_mode = d[off]; off += 1;
        c.null_count = d[off] != 0; off += 1;
        c.gate = d[off] != 0; off += 1;
        c.output = d[off] != 0; off += 1;
        c.bcd_mode = d[off] != 0; off += 1;
        c.count_written = d[off] != 0; off += 1;
    }
    pit.total_ticks = u64_at(d, &mut off);
}

// ============================================================================
// Binary helpers
// ============================================================================

#[cfg(feature = "std")]
fn read_u32<R: Read>(r: &mut R) -> std::io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

#[cfg(feature = "std")]
fn read_u64<R: Read>(r: &mut R) -> std::io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn u16_at(d: &[u8], off: &mut usize) -> u16 {
    let v = u16::from_le_bytes([d[*off], d[*off + 1]]);
    *off += 2;
    v
}

fn u32_at(d: &[u8], off: &mut usize) -> u32 {
    let v = u32::from_le_bytes(d[*off..*off + 4].try_into().unwrap());
    *off += 4;
    v
}

fn u64_at(d: &[u8], off: &mut usize) -> u64 {
    let v = u64::from_le_bytes(d[*off..*off + 8].try_into().unwrap());
    *off += 8;
    v
}
