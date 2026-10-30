//! The page write stamps give every page of a machine's RAM a stamp of its
//! own, from the board's default 256 MiB up to Bochs's 4 GiB of pages; a
//! trace is marked as one that may begin a counter poll only when its
//! `B.cond` tests an ordering, the conditions `poll.rs`'s walk accepts; and
//! a trace whose fetch walked through a device does not link past it.

use rusty_box_core::{ClockHz, VmClock};

use super::{classify, predecode, Slot, TraceCache, MAX_STAMPS, MIN_STAMPS};
use crate::bus::{AccessWidth, BusFault, CodeEpoch, FetchSource, PhysAddr, PhysBus};
use crate::cpu::features::{Features, Implementation};
use crate::cpu::{id, Cpu, CpuConfig, Step};
use crate::instrumentation::NoInstrumentation;

const RAM_BASE: u64 = 0x4000_0000;

/// RAM whose translation tables, once `tables_on_a_device` is set, count as
/// a device's: reading a descriptor marks a device touched.
struct TablesOnADevice {
    ram: std::vec::Vec<u8>,
    tables_on_a_device: bool,
    touched: bool,
}

impl PhysBus for TablesOnADevice {
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let bytes = self
            .ram
            .get(a..a + width.bytes() as usize)
            .ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(
        &mut self,
        _addr: PhysAddr,
        _width: AccessWidth,
        _value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
        Err(BusFault::Unmapped)
    }

    fn read_descriptor(&mut self, addr: PhysAddr, clock: VmClock) -> Result<u64, BusFault> {
        self.touched |= self.tables_on_a_device;
        self.read(addr, AccessWidth::Dword, clock)
    }

    fn fetch_source(&self, _addr: PhysAddr) -> FetchSource {
        FetchSource::Stable(CodeEpoch::new(0))
    }

    fn devices_touched(&self) -> bool {
        self.touched
    }
}

/// A fetch's table walk that reads a descriptor from a device touches the
/// device as any access does: the trace it starts, here a taken branch,
/// goes back to the machine, which looks at the device, instead of linking
/// on to the trace at the branch's target.
#[test]
fn a_trace_whose_fetch_walked_through_a_device_does_not_link() {
    const CODE: u64 = 0x1000;
    const TABLE: u64 = 0x4000;
    const BRANCH_FORWARD: u32 = 0x1400_0002; // b .+8
    const BRANCH_BACK: u32 = 0x17FF_FFFE; // b .-8
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut cpu = Cpu::implementing(
        CpuConfig {
            ips,
            reset_pc: CODE,
        },
        Implementation {
            counter_hz: ips,
            ..Implementation::CORTEX_A57
        },
    );
    let mut bus = TablesOnADevice {
        ram: std::vec![0; 0x8000],
        tables_on_a_device: false,
        touched: false,
    };
    let code = CODE as usize;
    bus.ram[code..code + 4].copy_from_slice(&BRANCH_FORWARD.to_le_bytes());
    bus.ram[code + 8..code + 12].copy_from_slice(&BRANCH_BACK.to_le_bytes());
    // One 1 GiB block at level 1, identity mapped, Normal Write-Back.
    let block: u64 = (1 << 10) | 0b01;
    let table = TABLE as usize;
    bus.ram[table..table + 8].copy_from_slice(&block.to_le_bytes());
    cpu.sys.mair_el1 = 0xFF;
    cpu.sys.tcr_el1 = 25;
    cpu.sys.ttbr0_el1 = TABLE;
    cpu.sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1;

    // Both traces in the cache, the first's branch retired.
    let run = |cpu: &mut Cpu, bus: &mut TablesOnADevice, limit: u64| {
        cpu.bound_run(limit);
        assert_eq!(
            cpu.run_trace(bus, &mut NoInstrumentation, limit, &[]),
            Step::Retired
        );
    };
    run(&mut cpu, &mut bus, 2);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.pc, CODE, "the loop comes back");

    cpu.tlb.flush();
    bus.tables_on_a_device = true;
    let retired = cpu.retired;
    run(&mut cpu, &mut bus, 100);
    assert_eq!(cpu.retired - retired, 1, "the branch alone ran");
}

#[test]
fn every_page_of_ram_has_a_stamp_of_its_own() {
    for (bytes, stamps) in [
        (64u64 << 20, MIN_STAMPS),
        (256 << 20, MIN_STAMPS),
        ((256 << 20) + 4096, 2 * MIN_STAMPS),
        (1 << 30, 4 * MIN_STAMPS),
        (16 << 30, MAX_STAMPS),
    ] {
        let mut cache = TraceCache::new();
        cache.cover(bytes);
        assert_eq!(cache.stamp_count(), stamps, "{bytes:#x} bytes");
        let pages = (bytes / 4096).min(stamps as u64);
        let first = cache.stamp_index(RAM_BASE);
        let last = cache.stamp_index(RAM_BASE + 4096 * (pages - 1));
        assert_eq!(
            last,
            (first + pages as usize - 1) % stamps,
            "{bytes:#x} bytes: pages numbered apart"
        );
        // A page as far past the first as the table is long shares its stamp.
        assert_eq!(cache.stamp_index(RAM_BASE + 4096 * stamps as u64), first);
    }
}

#[test]
fn only_a_poll_branching_on_an_ordering_is_marked() {
    for cond in 0..16u32 {
        // mrs x0, cntvct_el0; cmp x19, x0; b.<cond> 1b
        let words = [0xD53B_E040, 0xEB00_027F, 0x54FF_FFC0 | cond];
        let mut cache = TraceCache::new();
        for word in words {
            cache.pool.push(Slot {
                op: predecode(classify(word, Features::CORTEX_A57), word),
                word,
            });
        }
        // Arm ARM `ConditionHolds`: HS, LO, HI, LS, GE, LT, GT and LE order
        // the compare's operands; EQ, NE, MI, PL, VS, VC, AL and NV do not.
        let ordering = matches!(cond, 2 | 3 | 8..=13);
        assert_eq!(cache.may_poll(0, words.len()), ordering, "condition {cond}");
    }
}
