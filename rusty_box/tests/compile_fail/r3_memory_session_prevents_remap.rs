use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::BxCpuC;
use rusty_box::cpu::memory_session::MemorySession;
use rusty_box::memory::mapped::{MappedMemory, PhysicalRegion, RegionKind};

fn remap(cpu: &mut BxCpuC, arena: &mut [u8], stamps: &mut [u32]) {
    let mut regions = [PhysicalRegion {
        guest_start: BxPhyAddress::new(0),
        arena_offset: 0,
        len: 4096,
        kind: RegionKind::Ram,
    }];
    let mut memory = MappedMemory::new(arena, stamps, &regions).unwrap();
    let mut session = MemorySession::new(cpu, memory.loan());
    regions[0].kind = RegionKind::Rom;
    let _ = session.run(1);
}

fn main() {}
