//! A downstream physical memory provider with two separately stored byte lanes.

use std::error::Error;
use std::time::Instant;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::builder::BxCpuBuilder;
use rusty_box::cpu::instrumentation::ProcessorActivity;
use rusty_box::cpu::memory_session::{EntryMode, MemorySession, SessionStop};
use rusty_box::cpu::registers::{EAX, EBX, EIP};
use rusty_box::memory::direct::{DirectMemory, DirectRegion};
use rusty_box::memory::flat::FlatMemory;
use rusty_box::memory::indirect::{IndirectRam, IndirectRamRange};
use rusty_box::memory::mapped::{MappedMemory, PhysicalRegion, RegionKind};

const PAGE: usize = 4096;
const CODE: u32 = 0x1000;
const DATA: u32 = 0x5000;
const STACK: u32 = 0x8000;
const DATA_RANGE: IndirectRamRange = IndirectRamRange {
    start: physical(DATA),
    len: 2 * PAGE as u64,
};
const PROGRAM: &[u8] = &[
    0xB8, 0, 0x20, 0, 0, // mov eax,0x2000 (directory)
    0x0F, 0x22, 0xD8, // mov cr3,eax
    0x0F, 0x20, 0xC0, // mov eax,cr0
    0x0D, 0, 0, 0, 0x80, // or eax,CR0.PG
    0x0F, 0x22, 0xC0, // mov cr0,eax
    0x40, // inc eax
    0xA3, 0, 0x50, 0, 0, // mov [0x5000],eax
    0x8B, 0x1D, 0, 0x50, 0, 0, // mov ebx,[0x5000]
    0xEB, 0xF2, // jmp to inc
];
const LOOP: u32 = CODE + 19;
const DIRECT: [PhysicalRegion; 2] = [
    PhysicalRegion {
        guest_start: physical(0),
        arena_offset: 0,
        len: 4 * PAGE,
        kind: RegionKind::Ram,
    },
    PhysicalRegion {
        guest_start: physical(DATA),
        arena_offset: 4 * PAGE,
        len: 2 * PAGE,
        kind: RegionKind::Ram,
    },
];

type ExampleResult<T> = Result<T, Box<dyn Error>>;

struct ByteLanes {
    even: [u8; PAGE],
    odd: [u8; PAGE],
}

impl ByteLanes {
    fn new() -> Self {
        Self {
            even: [0; PAGE],
            odd: [0; PAGE],
        }
    }
}

impl IndirectRam for ByteLanes {
    fn read(&mut self, offset: u64, out: &mut [u8]) {
        for (index, byte) in (offset as usize..).zip(out) {
            let lane = if index & 1 == 0 {
                &self.even
            } else {
                &self.odd
            };
            *byte = lane[index / 2];
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        for (index, &byte) in (offset as usize..).zip(data) {
            let lane = if index & 1 == 0 {
                &mut self.even
            } else {
                &mut self.odd
            };
            lane[index / 2] = byte;
        }
    }
}

struct CopyRam([u8; 2 * PAGE]);

impl IndirectRam for CopyRam {
    fn read(&mut self, offset: u64, out: &mut [u8]) {
        let start = offset as usize;
        out.copy_from_slice(&self.0[start..start + out.len()]);
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        let start = offset as usize;
        self.0[start..start + data.len()].copy_from_slice(data);
    }
}

const fn physical(address: u32) -> BxPhyAddress {
    BxPhyAddress::new(address as u64)
}

fn prepare(session: &mut MemorySession<'_>) -> ExampleResult<()> {
    session.reset_at(EntryMode::Protected32, CODE, 0x2000)?;
    session.write_ram(physical(CODE), PROGRAM)?;
    session.write_ram(physical(0x2000), &0x3003u32.to_le_bytes())?;
    for address in [CODE, DATA, DATA + PAGE as u32] {
        session.write_ram(
            physical(0x3000 + (address >> 12) * 4),
            &(address | 3).to_le_bytes(),
        )?;
    }
    if session.run(5)? != 5 || session.regs().read(EIP) != LOOP {
        return Err("expected paging bootstrap to reach the loop".into());
    }
    session.regs().write(EAX, 0);
    Ok(())
}

fn measure(session: &mut MemorySession<'_>, count: u64) -> ExampleResult<f64> {
    prepare(session)?;
    if session.run(4096)? != 4096 {
        return Err("memory loop stopped during warmup".into());
    }
    let start = Instant::now();
    let run = session.run_controlled(count);
    let elapsed = start.elapsed();
    if !matches!(run.stop, SessionStop::Budget)
        || run.instructions != count
        || session.regs().read(EBX) != (count / 4) as u32 + 1024
    {
        return Err("memory loop returned the wrong count or guest result".into());
    }
    Ok(elapsed.as_nanos() as f64 / count as f64)
}

fn demonstrate() -> ExampleResult<()> {
    let mut arena = [0; 6 * PAGE];
    let mut stamps = [0; 6];
    let mut lanes = ByteLanes::new();
    let mut cpu = BxCpuBuilder::new().build()?;
    {
        let mut direct = MappedMemory::new(&mut arena, &mut stamps, &DIRECT)?;
        let mut session = MemorySession::new(&mut cpu, direct.loan());
        prepare(&mut session)?;
        // Warm the direct translation before replacing the backing behind it.
        assert_eq!(session.run(4)?, 4);
        assert_eq!(session.regs().read(EBX), 1);
    }
    {
        let mut direct = MappedMemory::new(&mut arena, &mut stamps, &DIRECT[..1])?;
        let loan = direct.loan().with_indirect_ram(DATA_RANGE, &mut lanes)?;
        let mut session = MemorySession::new(&mut cpu, loan);
        session.write_ram(physical(DATA), &0x4433_2211u32.to_le_bytes())?;
        // Use the same paged load with a fresh provider at the original GPA.
        session.regs().write(EIP, LOOP + 6);
        assert_eq!(session.run(2)?, 2);
        assert_eq!(session.regs().read(EBX), 0x4433_2211);
        session.write_ram(physical(LOOP), &[0x48])?; // dec eax instead of inc
        assert_eq!(session.run(4)?, 4);
        assert_eq!(session.regs().read(EBX), 0);
        // A split word crosses two backed provider pages.
        session.write_ram(physical(DATA + PAGE as u32 - 1), &[0x21, 0x34])?;
        session.write_ram(physical(LOOP), &[0x66, 0xA1, 0xFF, 0x5F, 0, 0])?;
        assert_eq!(session.run(1)?, 1);
        assert_eq!(session.regs().read(EAX) & 0xFFFF, 0x3421);
        // No fetch path is issued for the external data range.
        session.regs().write(EIP, DATA);
        assert!(matches!(
            session.step().stop,
            SessionStop::Exit(rusty_box::cpu::CpuError::VetoedDirectRead { .. })
        ));
    }
    assert_eq!(lanes.odd[PAGE / 2 - 1], 0x21);
    assert_eq!(lanes.even[PAGE / 2], 0x34);
    println!("Byte-lane RAM passed remap, code invalidation, split read and fetch refusal");
    Ok(())
}

/// This provider owns the three allocations separately. No bytes are moved
/// into an emulator arena, and the caller can recover them after the session.
struct SeparateRam {
    code_and_tables: Box<[u8]>,
    data: Box<[u8]>,
    stack: Box<[u8]>,
    stamps: [[u32; 4]; 3],
}

impl SeparateRam {
    fn new() -> Self {
        Self {
            code_and_tables: vec![0; 4 * PAGE].into_boxed_slice(),
            data: vec![0; 2 * PAGE].into_boxed_slice(),
            stack: vec![0; PAGE].into_boxed_slice(),
            stamps: [[0; 4]; 3],
        }
    }

    fn regions(&mut self) -> [DirectRegion<'_>; 3] {
        let [code_stamps, data_stamps, stack_stamps] = &mut self.stamps;
        [
            DirectRegion {
                guest_start: physical(0),
                bytes: &mut self.code_and_tables,
                stamps: code_stamps,
                kind: RegionKind::Ram,
            },
            DirectRegion {
                guest_start: physical(DATA),
                bytes: &mut self.data,
                stamps: data_stamps,
                kind: RegionKind::Ram,
            },
            DirectRegion {
                guest_start: physical(STACK),
                bytes: &mut self.stack,
                stamps: stack_stamps,
                kind: RegionKind::Ram,
            },
        ]
    }
}

fn demonstrate_direct() -> ExampleResult<()> {
    let mut provider = SeparateRam::new();
    let mut cpu = BxCpuBuilder::new().build()?;
    {
        let mut regions = provider.regions();
        let mut memory = DirectMemory::new(&mut regions)?;
        let mut session = MemorySession::new(&mut cpu, memory.loan());
        session.reset_at(EntryMode::Protected32, CODE, STACK + PAGE as u32)?;
        session.memory().write_phys(
            physical(CODE),
            &[
                0xA1, 0, 0x50, 0, 0,    // mov eax,[0x5000]
                0x40, // inc eax
                0x50, // push eax (independent stack)
                0x5B, // pop ebx
                0x89, 0x1D, 4, 0x50, 0, 0,    // mov [0x5004],ebx
                0xF4, // hlt
            ],
        )?;
        session
            .memory()
            .write_phys(physical(DATA), &41u32.to_le_bytes())?;
        let run = session.run_controlled(32);
        assert!(matches!(
            run.stop,
            SessionStop::Inactive(ProcessorActivity::Halted)
        ));
        assert_eq!(session.regs().read(EBX), 42);
    }
    assert_eq!(&provider.data[4..8], &42u32.to_le_bytes());
    assert_eq!(&provider.stack[PAGE - 4..], &42u32.to_le_bytes());
    println!("Direct memory executed with independent code, data, and stack allocations");
    Ok(())
}

fn benchmark() -> ExampleResult<()> {
    let count = std::env::var("RB_MEMORY_COUNT")
        .ok()
        .map_or(Ok(8_000_000), |value| value.parse::<u64>())?;
    if count == 0 || !count.is_multiple_of(4) || count / 4 > u64::from(u32::MAX - 1024) {
        return Err(
            "dispatch count must be a nonzero multiple of four within the 32-bit loop range".into(),
        );
    }
    let mut arena = [0; 7 * PAGE];
    let mut stamps = [0; 7];
    let mut cpu = BxCpuBuilder::new().build()?;
    let mut lanes = ByteLanes::new();
    let mut copy = CopyRam([0; 2 * PAGE]);
    let mut separate = SeparateRam::new();
    let names = [
        "dense",
        "mapped",
        "indirect copy",
        "indirect byte lanes",
        "multi-backing direct",
    ];
    let mut samples = [[0.0; 5]; 7];
    for (round, times) in samples.iter_mut().enumerate() {
        for position in 0..names.len() {
            let mode = (round + position) % names.len();
            if mode == 0 {
                let mut direct = FlatMemory::new(&mut arena, &mut stamps)?;
                times[mode] = measure(&mut MemorySession::new(&mut cpu, direct.loan()), count)?;
            } else if mode == 4 {
                let mut regions = separate.regions();
                let mut direct = DirectMemory::new(&mut regions)?;
                times[mode] = measure(&mut MemorySession::new(&mut cpu, direct.loan()), count)?;
            } else {
                let regions = if mode == 1 { &DIRECT[..] } else { &DIRECT[..1] };
                let mut direct = MappedMemory::new(&mut arena, &mut stamps, regions)?;
                let loan = match mode {
                    1 => direct.loan(),
                    2 => direct.loan().with_indirect_ram(DATA_RANGE, &mut copy)?,
                    _ => direct.loan().with_indirect_ram(DATA_RANGE, &mut lanes)?,
                };
                times[mode] = measure(&mut MemorySession::new(&mut cpu, loan), count)?;
            }
        }
    }
    for (mode, name) in names.into_iter().enumerate() {
        let mut values = samples.map(|times| times[mode]);
        values.sort_unstable_by(f64::total_cmp);
        println!(
            "{name}: {:.2} ns/count [{:.2}..{:.2}]",
            values[3], values[0], values[6]
        );
    }
    Ok(())
}

fn main() -> ExampleResult<()> {
    match std::env::args().nth(1).as_deref() {
        None => demonstrate(),
        Some("--direct") => demonstrate_direct(),
        Some("--bench") => benchmark(),
        _ => Err("usage: external_ram [--direct|--bench]".into()),
    }
}
