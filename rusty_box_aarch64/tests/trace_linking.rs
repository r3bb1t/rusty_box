//! A trace that ends in a direct branch goes on into the trace it branches
//! to without returning to the run loop (Bochs cpu.cc `linkTrace`), and a
//! machine runs the same instructions with the same results, and tells its
//! observer the same things, as one that never links (R9: each test
//! compares two machines' whole snapshots — registers, flags, time,
//! instructions retired, the PC history, RAM and devices — and what their
//! observers heard, after every step, and asserts what the guest computed).
//!
//! The machine that never links runs one instruction per step: a trace
//! then ends after its first instruction with nothing left of its run, so
//! the run loop finds every trace. The other runs 7, 1000 or 2^24
//! instructions per step, and its loops link at every branch.
//!
//! Guest code is assembled with GNU as for aarch64 at the base of RAM,
//! where a bare machine starts; each list's source is beside its words.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::instrumentation::ExceptionTaken;
use rusty_box_aarch64::{
    Execution, Instrumentation, LentStorage, MachineBuilder, MemoryLoan, RamSize, Reg, RunBudget, StopReason,
    VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 1 << 17;

/// Where a run gives up on a guest that has not finished.
const GIVE_UP_AFTER: u64 = 4_000_000;

const SLICES: [u64; 4] = [1, 7, 1000, 1 << 24];

/// Hears of every instruction, branch, exception and wait, and keeps an
/// FNV-1a hash of all of it, in order.
#[derive(Debug, PartialEq, Eq)]
struct Heard {
    hash: u64,
    events: u64,
}

impl Default for Heard {
    fn default() -> Self {
        Self {
            hash: 0xCBF2_9CE4_8422_2325,
            events: 0,
        }
    }
}

impl Heard {
    fn mix(&mut self, kind: u8, values: &[u64]) {
        self.events += 1;
        for byte in core::iter::once(kind).chain(values.iter().flat_map(|v| v.to_le_bytes())) {
            self.hash = (self.hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
        }
    }
}

impl Instrumentation for Heard {
    fn before_execution(&mut self, pc: u64, insn: u32) {
        self.mix(0, &[pc, u64::from(insn)]);
    }
    fn after_execution(&mut self, pc: u64, insn: u32) {
        self.mix(1, &[pc, u64::from(insn)]);
    }
    fn branch(&mut self, from: u64, to: u64) {
        self.mix(2, &[from, to]);
    }
    fn exception(&mut self, taken: ExceptionTaken) {
        let mut text = Vec::new();
        text.extend(format!("{taken:?}").bytes().map(u64::from));
        self.mix(3, &text);
    }
    fn wait_for_interrupt(&mut self, pc: u64) {
        self.mix(4, &[pc]);
    }
}

/// Hears nothing, so the machine may fast-forward a counter poll; the
/// snapshots alone are compared.
#[derive(Debug, Default, PartialEq, Eq)]
struct Quiet;

impl Instrumentation for Quiet {
    const EXECUTION: Execution = Execution::Skippable;
}

fn machine<'r, I: Instrumentation>(ram: &'r mut [u8], observer: I, program: &[(u64, u32)]) -> VirtMachine<LentStorage<'r>, I> {
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .instrumentation(observer)
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds");
    for &(offset, word) in program {
        m.mem_write(RAM_BASE + offset, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    m
}

fn snapshot<I: Instrumentation>(m: &VirtMachine<LentStorage<'_>, I>) -> Vec<u8> {
    let mut image = Vec::new();
    m.save_snapshot(&mut image).expect("a snapshot saves");
    image
}

/// How a guest ended, the same on both machines.
struct Ended {
    stop: StopReason,
    x0: u64,
    x20: u64,
    x21: u64,
}

/// Run `program` on a machine stepping `slice` instructions at a time and
/// on one stepping a single instruction at a time, which never links; after
/// every step of the first, the second is brought to the same instruction
/// count and the two must be the same machine whose observers heard the
/// same. Until a step stops for a reason other than its budget, or
/// [`GIVE_UP_AFTER`] ticks.
fn run_both<I: Instrumentation + Default + PartialEq + core::fmt::Debug>(program: &[(u64, u32)], slice: u64) -> Ended {
    let mut ram_a = vec![0u8; RAM_LEN];
    let mut ram_b = vec![0u8; RAM_LEN];
    let mut a = machine(&mut ram_a, I::default(), program);
    let mut b = machine(&mut ram_b, I::default(), program);
    loop {
        let stepped = a.step(RunBudget::Instructions(slice));
        let mut single = StopReason::BudgetExhausted;
        while b.instructions() < a.instructions() && single == StopReason::BudgetExhausted {
            single = b.step(RunBudget::Instructions(1)).stop;
        }
        if stepped.stop != StopReason::BudgetExhausted && single == StopReason::BudgetExhausted {
            single = b.step(RunBudget::Instructions(1)).stop;
        }
        assert_eq!(stepped.stop, single, "slice {slice}, tick {}", a.ticks());
        assert_eq!(a.instructions(), b.instructions(), "slice {slice}");
        assert!(snapshot(&a) == snapshot(&b), "slice {slice}: the machines differ at tick {}", a.ticks());
        assert_eq!(a.instrumentation(), b.instrumentation(), "slice {slice}: the observers heard different runs");
        if stepped.stop != StopReason::BudgetExhausted {
            return Ended {
                stop: stepped.stop,
                x0: a.reg_read(Reg::X0),
                x20: a.reg_read(Reg::X20),
                x21: a.reg_read(Reg::X21),
            };
        }
        assert!(a.ticks() < GIVE_UP_AFTER, "slice {slice}: the guest has not finished");
    }
}

/// A loop of two traces, each ending in a branch to the other, run 1000
/// times; then the first trace's `add` is rewritten by a store from
/// another trace, and the loop runs 1000 times more, entered by a direct
/// branch to the rewritten trace.
///
/// ```text
/// _start: mov x0, #0          ; mov x1, #1000
/// 1:      add x0, x0, #1      ; b 2f              ; nop
/// 2:      subs x1, x1, #1     ; b.ne 1b
///         cbnz x2, 3f
///         ldr w3, patch       ; adr x4, 1b        ; str w3, [x4]
///         mov x1, #1000       ; mov x2, #1        ; b 1b
/// 3:      wfi                 ; b 3b
/// patch:  add x0, x0, #2
/// ```
const REWRITTEN_LOOP: &[(u64, u32)] = &[
    (0x00, 0xD280_0000),
    (0x04, 0xD280_7D01),
    (0x08, 0x9100_0400),
    (0x0C, 0x1400_0002),
    (0x10, 0xD503_201F),
    (0x14, 0xF100_0421),
    (0x18, 0x54FF_FF81),
    (0x1C, 0xB500_00E2),
    (0x20, 0x1800_0103),
    (0x24, 0x10FF_FF24),
    (0x28, 0xB900_0083),
    (0x2C, 0xD280_7D01),
    (0x30, 0xD280_0022),
    (0x34, 0x17FF_FFF5),
    (0x38, 0xD503_207F),
    (0x3C, 0x17FF_FFFF),
    (0x40, 0x9100_0800),
];

/// The virtual timer interrupts every 1000 counts while a loop of two
/// linked traces runs 0x3000 times; the IRQ handler counts the interrupts
/// in `x20` and sums the addresses they interrupted in `x21`, and the guest
/// powers off at the end.
///
/// ```text
/// _start: ldr x0, =0x40001000 ; msr vbar_el1, x0
///         ldr x0, =0x40010000 ; mov sp, x0
///         ldr x1, =0x08000000 ; mov w2, #1        ; str w2, [x1]          // GICD_CTLR
///         mov w2, #(1 << 27)  ; str w2, [x1, #0x100]                      // ISENABLER0
///         ldr x3, =0x08010000 ; mov w2, #1        ; str w2, [x3]          // GICC_CTLR
///         mov w2, #0xff       ; str w2, [x3, #4]                          // GICC_PMR
///         mov x4, #1000       ; msr cntv_tval_el0, x4
///         mov x4, #1          ; msr cntv_ctl_el0, x4
///         mov x20, #0         ; mov x21, #0       ; msr daifclr, #2
///         mov x1, #0x3000
/// 1:      add x0, x0, #1      ; b 2f              ; nop
/// 2:      subs x1, x1, #1     ; b.ne 1b
///         ldr x0, =0x84000008 ; hvc #0            // PSCI SYSTEM_OFF
/// 3:      b 3b
///         .org 0x1280         // IRQ, current EL with SP_ELx
///         ldr w5, [x3, #0xc]  ; add x20, x20, #1  // GICC_IAR
///         mrs x6, elr_el1     ; add x21, x21, x6
///         mov x4, #1000       ; msr cntv_tval_el0, x4
///         str w5, [x3, #0x10] ; eret              // GICC_EOIR
/// ```
const INTERRUPTED_LOOP: &[(u64, u32)] = &[
    (0x00, 0x5800_03C0),
    (0x04, 0xD518_C000),
    (0x08, 0x5800_03C0),
    (0x0C, 0x9100_001F),
    (0x10, 0x5800_03C1),
    (0x14, 0x5280_0022),
    (0x18, 0xB900_0022),
    (0x1C, 0x52A1_0002),
    (0x20, 0xB901_0022),
    (0x24, 0x5800_0363),
    (0x28, 0x5280_0022),
    (0x2C, 0xB900_0062),
    (0x30, 0x5280_1FE2),
    (0x34, 0xB900_0462),
    (0x38, 0xD280_7D04),
    (0x3C, 0xD51B_E304),
    (0x40, 0xD280_0024),
    (0x44, 0xD51B_E324),
    (0x48, 0xD280_0014),
    (0x4C, 0xD280_0015),
    (0x50, 0xD503_42FF),
    (0x54, 0xD286_0001),
    (0x58, 0x9100_0400),
    (0x5C, 0x1400_0002),
    (0x60, 0xD503_201F),
    (0x64, 0xF100_0421),
    (0x68, 0x54FF_FF81),
    (0x6C, 0x5800_0160),
    (0x70, 0xD400_0002),
    (0x74, 0x1400_0000),
    (0x78, 0x4000_1000),
    (0x7C, 0x0000_0000),
    (0x80, 0x4001_0000),
    (0x84, 0x0000_0000),
    (0x88, 0x0800_0000),
    (0x8C, 0x0000_0000),
    (0x90, 0x0801_0000),
    (0x94, 0x0000_0000),
    (0x98, 0x8400_0008),
    (0x1280, 0xB940_0C65),
    (0x1284, 0x9100_0694),
    (0x1288, 0xD538_4026),
    (0x128C, 0x8B06_02B5),
    (0x1290, 0xD280_7D04),
    (0x1294, 0xD51B_E304),
    (0x1298, 0xB900_1065),
    (0x129C, 0xD69F_03E0),
];

#[test]
fn a_linked_trace_rewritten_by_a_store_runs_as_rewritten() {
    for slice in SLICES {
        for ended in [run_both::<Heard>(REWRITTEN_LOOP, slice), run_both::<Quiet>(REWRITTEN_LOOP, slice)] {
            assert_eq!(ended.stop, StopReason::Halted, "slice {slice}");
            assert_eq!(ended.x0, 1000 + 2 * 1000, "slice {slice}: the second pass ran the old add");
        }
    }
}

#[test]
fn an_interrupt_due_inside_a_run_of_linked_traces_is_taken_where_it_is_due() {
    for slice in SLICES {
        for ended in [run_both::<Heard>(INTERRUPTED_LOOP, slice), run_both::<Quiet>(INTERRUPTED_LOOP, slice)] {
            assert_eq!(ended.stop, StopReason::PoweredOff, "slice {slice}");
            assert!(ended.x20 > 10, "slice {slice}: {} interrupts", ended.x20);
            assert_ne!(ended.x21, 0, "slice {slice}");
        }
    }
}
