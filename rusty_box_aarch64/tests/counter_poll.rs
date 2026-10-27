//! A guest that waits by reading the system counter in a loop sees exactly
//! what it would see were every instruction of the wait run (R9: each test
//! compares two machines' whole snapshots — registers, flags, time,
//! instructions retired, the PC history, RAM and devices — after every step,
//! and asserts what the guest computed or where it stopped).
//!
//! The rule: a machine nobody watches instruction by instruction may jump
//! over whole iterations of a loop that reads the counter, compares it once
//! with a value that does not change, and branches on an ordering of the
//! two, because along a counter that only grows each branch of such a loop
//! changes its outcome at most once; it stops short of every iteration that
//! could go another way, of the budget, of the next timer or device event
//! and of every exit address.
//!
//! One machine here lets the processor fast-forward ([`Skipping`]), the
//! other hears of every instruction and so runs each one ([`Watching`]);
//! both hear of every taken branch, which counts how much was run.
//!
//! Guest code is assembled with GNU as for aarch64 at the base of RAM, where
//! a bare machine starts; each list's source is beside its words.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    Execution, Instrumentation, LentStorage, MachineBuilder, MemoryLoan, RamSize, Reg, RunBudget, StopReason,
    SysReg, VirtConfig, VirtMachine,
};

/// Lets the machine fast-forward; counts the branches it hears of.
#[derive(Default)]
struct Skipping {
    branches: u64,
}

impl Instrumentation for Skipping {
    const EXECUTION: Execution = Execution::Skippable;
    fn branch(&mut self, _from: u64, _to: u64) {
        self.branches += 1;
    }
}

/// Hears of every instruction, so the machine runs each one; counts the
/// branches it hears of.
#[derive(Default)]
struct Watching {
    branches: u64,
    executed: u64,
}

impl Instrumentation for Watching {
    fn before_execution(&mut self, _pc: u64, _insn: u32) {
        self.executed += 1;
    }
    fn branch(&mut self, _from: u64, _to: u64) {
        self.branches += 1;
    }
}

const RAM_LEN: usize = 1 << 20;

/// Where [`run_both`] gives up on a guest that has not finished.
const GIVE_UP_AFTER: u64 = 2_000_000;

/// A direct poll: wait until the counter passes a target 0x10000 counts
/// ahead, then set `x2`.
///
/// ```text
///         mrs  x1, cntvct_el0
///         add  x19, x1, #0x10, lsl #12   // the target
/// 1:      mrs  x0, cntvct_el0
///         cmp  x19, x0
///         b.hi 1b                        // until the counter reaches it
///         mov  x2, #1
///         b    .                         // 0x18
/// ```
const DIRECT: &[(u64, u32)] = &[
    (0x00, 0xD53B_E041),
    (0x04, 0x9140_4033),
    (0x08, 0xD53B_E040),
    (0x0C, 0xEB00_027F),
    (0x10, 0x54FF_FFC8),
    (0x14, 0xD280_0022),
    (0x18, 0x1400_0000),
];

/// The shape of EDK2's delays: the counter is read by a function the loop
/// calls, so the loop writes the link register in every iteration, and it
/// holds the same value at each iteration's end. The first call returns
/// elsewhere, so its iteration is not one of the loop's.
///
/// ```text
///         bl   get
///         add  x19, x0, #0x10, lsl #12   // the target
/// loop:   cmp  x19, x0
///         b.hi wait
///         mov  x2, #1
///         b    .                         // 0x14
/// wait:   bl   get
///         b    loop
/// get:    mrs  x0, cntvct_el0
///         ret
/// ```
const CALL: &[(u64, u32)] = &[
    (0x00, 0x9400_0008),
    (0x04, 0x9140_4013),
    (0x08, 0xEB00_027F),
    (0x0C, 0x5400_0068),
    (0x10, 0xD280_0022),
    (0x14, 0x1400_0000),
    (0x18, 0x9400_0002),
    (0x1C, 0x17FF_FFFB),
    (0x20, 0xD53B_E040),
    (0x24, 0xD65F_03C0),
];

/// [`DIRECT`] waiting for the counter to equal the target (`b.ne 1b` at
/// 0x10): an equality may hold for no counter value the loop reads, and
/// may flip back, so the rule does not cover it.
const EQUAL: &[(u64, u32)] = &[
    (0x00, 0xD53B_E041),
    (0x04, 0x9140_4033),
    (0x08, 0xD53B_E040),
    (0x0C, 0xEB00_027F),
    (0x10, 0x54FF_FFC1),
    (0x14, 0xD280_0022),
    (0x18, 0x1400_0000),
];

/// [`DIRECT`] comparing 32 bits (`cmp w19, w0` at 0x0C): the low word of a
/// growing counter wraps, so the rule does not cover it.
const NARROW: &[(u64, u32)] = &[
    (0x00, 0xD53B_E041),
    (0x04, 0x9140_4033),
    (0x08, 0xD53B_E040),
    (0x0C, 0x6B00_027F),
    (0x10, 0x54FF_FFC8),
    (0x14, 0xD280_0022),
    (0x18, 0x1400_0000),
];

/// [`DIRECT`] storing every value it reads through `x20`: an iteration
/// that writes memory is not one whose only effect is time.
///
/// ```text
///         mrs  x1, cntvct_el0
///         add  x19, x1, #0x10, lsl #12
/// 1:      mrs  x0, cntvct_el0
///         str  x0, [x20]
///         cmp  x19, x0
///         b.hi 1b
///         mov  x2, #1
///         b    .                         // 0x1C
/// ```
const STORE: &[(u64, u32)] = &[
    (0x00, 0xD53B_E041),
    (0x04, 0x9140_4033),
    (0x08, 0xD53B_E040),
    (0x0C, 0xF900_0280),
    (0x10, 0xEB00_027F),
    (0x14, 0x54FF_FFA8),
    (0x18, 0xD280_0022),
    (0x1C, 0x1400_0000),
];

/// `tests/timer_interrupts.rs`'s guest with its `WFI` wait replaced by a
/// poll: the virtual timer interrupts every 1000 counts, and the IRQ
/// handler counts the interrupts in `x20`, while the main line polls the
/// counter for 0x3000 counts and then powers off.
///
/// ```text
/// _start: ldr x0, =0x40001000      ; msr vbar_el1, x0
///         ldr x0, =0x40010000      ; mov sp, x0
///         ldr x1, =0x08000000      ; mov w2, #1        ; str w2, [x1]          // GICD_CTLR
///         mov w2, #(1 << 27)       ; str w2, [x1, #0x100]                      // ISENABLER0
///         ldr x3, =0x08010000      ; mov w2, #1        ; str w2, [x3]          // GICC_CTLR
///         mov w2, #0xff            ; str w2, [x3, #4]                          // GICC_PMR
///         mov x4, #1000            ; msr cntv_tval_el0, x4
///         mov x4, #1               ; msr cntv_ctl_el0, x4
///         mov x20, #0              ; msr daifclr, #2
///         mrs x1, cntvct_el0       ; add x19, x1, #0x3, lsl #12
/// 1:      mrs x0, cntvct_el0       ; cmp x19, x0       ; b.hi 1b
///         ldr x0, =0x84000008      ; hvc #0            // PSCI SYSTEM_OFF
/// 2:      b 2b
///         .org 0x1280              // IRQ, current EL with SP_ELx
///         ldr w5, [x3, #0xc]       ; add x20, x20, #1  // GICC_IAR
///         mov x4, #1000            ; msr cntv_tval_el0, x4
///         str w5, [x3, #0x10]      ; eret              // GICC_EOIR
/// ```
const IRQ_POLL: &[(u64, u32)] = &[
    (0x00, 0x5800_0380),
    (0x04, 0xD518_C000),
    (0x08, 0x5800_0380),
    (0x0C, 0x9100_001F),
    (0x10, 0x5800_0381),
    (0x14, 0x5280_0022),
    (0x18, 0xB900_0022),
    (0x1C, 0x52A1_0002),
    (0x20, 0xB901_0022),
    (0x24, 0x5800_0323),
    (0x28, 0x5280_0022),
    (0x2C, 0xB900_0062),
    (0x30, 0x5280_1FE2),
    (0x34, 0xB900_0462),
    (0x38, 0xD280_7D04),
    (0x3C, 0xD51B_E304),
    (0x40, 0xD280_0024),
    (0x44, 0xD51B_E324),
    (0x48, 0xD280_0014),
    (0x4C, 0xD503_42FF),
    (0x50, 0xD53B_E041),
    (0x54, 0x9140_0C33),
    (0x58, 0xD53B_E040),
    (0x5C, 0xEB00_027F),
    (0x60, 0x54FF_FFC8),
    (0x64, 0x5800_0160),
    (0x68, 0xD400_0002),
    (0x6C, 0x1400_0000),
    (0x70, 0x4000_1000),
    (0x74, 0x0000_0000),
    (0x78, 0x4001_0000),
    (0x7C, 0x0000_0000),
    (0x80, 0x0800_0000),
    (0x84, 0x0000_0000),
    (0x88, 0x0801_0000),
    (0x8C, 0x0000_0000),
    (0x90, 0x8400_0008),
    (0x1280, 0xB940_0C65),
    (0x1284, 0x9100_0694),
    (0x1288, 0xD280_7D04),
    (0x128C, 0xD51B_E304),
    (0x1290, 0xB900_1065),
    (0x1294, 0xD69F_03E0),
];

/// A poll comparing the counter with an immediate on a signed ordering:
/// wait until it reaches 0x10000, then set `x2`.
///
/// ```text
/// 1:      mrs  x0, cntvct_el0
///         cmp  x0, #0x10, lsl #12
///         b.lt 1b
///         mov  x2, #1
///         b    .                         // 0x10
/// ```
const IMMEDIATE_SIGNED: &[(u64, u32)] = &[
    (0x00, 0xD53B_E040),
    (0x04, 0xF140_401F),
    (0x08, 0x54FF_FFCB),
    (0x0C, 0xD280_0022),
    (0x10, 0x1400_0000),
];

/// [`DIRECT`] with the compare's operands the other way round, the counter
/// first, on the unsigned ordering that then waits the same way.
///
/// ```text
///         mrs  x1, cntvct_el0
///         add  x19, x1, #0x10, lsl #12
/// 1:      mrs  x0, cntvct_el0
///         cmp  x0, x19
///         b.lo 1b
///         mov  x2, #1
///         b    .                         // 0x18
/// ```
const COUNTER_FIRST: &[(u64, u32)] = &[
    (0x00, 0xD53B_E041),
    (0x04, 0x9140_4033),
    (0x08, 0xD53B_E040),
    (0x0C, 0xEB13_001F),
    (0x10, 0x54FF_FFC3),
    (0x14, 0xD280_0022),
    (0x18, 0x1400_0000),
];

/// A poll that leaves forward on a signed ordering and otherwise goes back
/// with `B`, so each iteration runs a `B.cond` it does not take.
///
/// ```text
///         mrs  x1, cntvct_el0
///         add  x19, x1, #0x10, lsl #12
/// 1:      mrs  x0, cntvct_el0
///         cmp  x0, x19
///         b.ge 2f
///         b    1b
/// 2:      mov  x2, #1
///         b    .                         // 0x1C
/// ```
const FORWARD_SIGNED: &[(u64, u32)] = &[
    (0x00, 0xD53B_E041),
    (0x04, 0x9140_4033),
    (0x08, 0xD53B_E040),
    (0x0C, 0xEB13_001F),
    (0x10, 0x5400_004A),
    (0x14, 0x17FF_FFFD),
    (0x18, 0xD280_0022),
    (0x1C, 0x1400_0000),
];

/// A poll that goes round through registers: a `BLR` to a `BR` back to the
/// read. The `BLR` writes `x30` the same in every iteration; before the
/// first one runs, `x30` holds something else, so that iteration is not
/// one of the loop's.
///
/// ```text
///         mrs  x1, cntvct_el0
///         add  x19, x1, #0x10, lsl #12
///         adr  x21, loop
///         adr  x22, back
/// loop:   mrs  x0, cntvct_el0
///         cmp  x19, x0
///         b.ls done
///         blr  x22
///         b    .
/// back:   br   x21
/// done:   mov  x2, #1
///         b    .                         // 0x2C
/// ```
const THROUGH_REGISTERS: &[(u64, u32)] = &[
    (0x00, 0xD53B_E041),
    (0x04, 0x9140_4033),
    (0x08, 0x1000_0055),
    (0x0C, 0x1000_00D6),
    (0x10, 0xD53B_E040),
    (0x14, 0xEB00_027F),
    (0x18, 0x5400_0089),
    (0x1C, 0xD63F_02C0),
    (0x20, 0x1400_0000),
    (0x24, 0xD61F_02A0),
    (0x28, 0xD280_0022),
    (0x2C, 0x1400_0000),
];

/// [`DIRECT`] with a `NOP` between the read and the compare.
///
/// ```text
///         mrs  x1, cntvct_el0
///         add  x19, x1, #0x10, lsl #12
/// 1:      mrs  x0, cntvct_el0
///         nop
///         cmp  x19, x0
///         b.hi 1b
///         mov  x2, #1
///         b    .                         // 0x1C
/// ```
const WITH_NOP: &[(u64, u32)] = &[
    (0x00, 0xD53B_E041),
    (0x04, 0x9140_4033),
    (0x08, 0xD53B_E040),
    (0x0C, 0xD503_201F),
    (0x10, 0xEB00_027F),
    (0x14, 0x54FF_FFA8),
    (0x18, 0xD280_0022),
    (0x1C, 0x1400_0000),
];

/// [`DIRECT`] reading the physical counter.
///
/// ```text
///         mrs  x1, cntpct_el0
///         add  x19, x1, #0x10, lsl #12
/// 1:      mrs  x0, cntpct_el0
///         cmp  x19, x0
///         b.hi 1b
///         mov  x2, #1
///         b    .                         // 0x18
/// ```
const PHYSICAL: &[(u64, u32)] = &[
    (0x00, 0xD53B_E021),
    (0x04, 0x9140_4033),
    (0x08, 0xD53B_E020),
    (0x0C, 0xEB00_027F),
    (0x10, 0x54FF_FFC8),
    (0x14, 0xD280_0022),
    (0x18, 0x1400_0000),
];

/// The virtual address [`TRANSLATED`]'s tables also map RAM at.
const ALIAS: u64 = 0x8000_0000;

/// [`DIRECT`] run with stage 1 translation on, from an alias of RAM that
/// only the translation tables provide, so every fetch of it is
/// translated. The level 1 table at offset 0x10000 maps two 1 GiB blocks
/// of Normal memory (attribute 0, inner shareable, EL1 read/write) to the
/// base of RAM: VA 0x4000_0000, where the code runs as it turns
/// translation on, and VA [`ALIAS`].
///
/// ```text
///         mov  x0, #0xff                 ; msr mair_el1, x0
///         mov  x0, #0x19                 ; movk x0, #0x80, lsl #16
///         msr  tcr_el1, x0               // T0SZ 25, 4 KiB granule, EPD1
///         adr  x0, l1                    ; msr ttbr0_el1, x0
///         mrs  x0, sctlr_el1             ; orr x0, x0, #1
///         msr  sctlr_el1, x0             ; isb
///         mov  x0, #0x80000000           ; add x0, x0, #(alias - _start)
///         br   x0
/// alias:  mrs  x1, cntvct_el0
///         add  x19, x1, #0x10, lsl #12
/// 1:      mrs  x0, cntvct_el0
///         cmp  x19, x0
///         b.hi 1b
///         mov  x2, #1
///         b    .                         // ALIAS + 0x50
///         .org 0x10000
/// l1:     .quad 0, 0x40000701, 0x40000701
/// ```
const TRANSLATED: &[(u64, u32)] = &[
    (0x00, 0xD280_1FE0),
    (0x04, 0xD518_A200),
    (0x08, 0xD280_0320),
    (0x0C, 0xF2A0_1000),
    (0x10, 0xD518_2040),
    (0x14, 0x1007_FF60),
    (0x18, 0xD518_2000),
    (0x1C, 0xD538_1000),
    (0x20, 0xB240_0000),
    (0x24, 0xD518_1000),
    (0x28, 0xD503_3FDF),
    (0x2C, 0xD2B0_0000),
    (0x30, 0x9100_E000),
    (0x34, 0xD61F_0000),
    (0x38, 0xD53B_E041),
    (0x3C, 0x9140_4033),
    (0x40, 0xD53B_E040),
    (0x44, 0xEB00_027F),
    (0x48, 0x54FF_FFC8),
    (0x4C, 0xD280_0022),
    (0x50, 0x1400_0000),
    (0x1_0008, 0x4000_0701),
    (0x1_000C, 0x0000_0000),
    (0x1_0010, 0x4000_0701),
    (0x1_0014, 0x0000_0000),
];

/// A poll at EL0. EL1 sets `CNTKCTL_EL1` from `x11`, its vectors and the
/// target 0x10000, and returns to EL0 at the poll, whose counter read is
/// the first instruction EL0 runs; a synchronous exception from EL0 lands
/// at 0xC00 and stays there.
///
/// ```text
/// _start: msr  cntkctl_el1, x11
///         adr  x0, vectors               ; msr vbar_el1, x0
///         adr  x0, 1f                    ; msr elr_el1, x0
///         msr  spsr_el1, xzr             // EL0t
///         mov  x19, #0x10000
///         isb
///         eret
/// 1:      mrs  x0, cntvct_el0            // 0x24
///         cmp  x19, x0
///         b.hi 1b
///         mov  x2, #1
///         b    .                         // 0x34
///         .org 0x800
/// vectors:
///         .org vectors + 0x400           // lower EL, AArch64, synchronous
///         b    .                         // 0xC00
/// ```
const AT_EL0: &[(u64, u32)] = &[
    (0x00, 0xD518_E10B),
    (0x04, 0x1000_3FE0),
    (0x08, 0xD518_C000),
    (0x0C, 0x1000_00C0),
    (0x10, 0xD518_4020),
    (0x14, 0xD518_401F),
    (0x18, 0xD2A0_0033),
    (0x1C, 0xD503_3FDF),
    (0x20, 0xD69F_03E0),
    (0x24, 0xD53B_E040),
    (0x28, 0xEB00_027F),
    (0x2C, 0x54FF_FFC8),
    (0x30, 0xD280_0022),
    (0x34, 0x1400_0000),
    (0xC00, 0x1400_0000),
];

/// `CNTKCTL_EL1.EL0PCTEN` and `EL0VCTEN`: EL0 may read `CNTPCT_EL0`, and
/// `CNTVCT_EL0`, without a trap.
const EL0PCTEN: u64 = 1 << 0;
const EL0VCTEN: u64 = 1 << 1;

/// `ESR_EL1` of [`AT_EL0`]'s `mrs x0, cntvct_el0` trapped from EL0 (Arm ARM
/// `CNTVCT_EL0` access pseudocode: `AArch64.SystemAccessTrap(EL1, 0x18)`):
/// EC 0x18, IL, and the ISS of a read (direction 1) of op0 3, op2 2,
/// op1 3, CRn 14, CRm 0 into `x0`.
const ESR_CNTVCT_TRAP: u64 = (0x18 << 26) | (1 << 25) | (3 << 20) | (2 << 17) | (3 << 14) | (14 << 10) | 1;

/// A bare machine watched by `observer`, with `program`'s words at their
/// offsets from the base of RAM and `regs` set.
fn machine<'r, I: Instrumentation>(
    ram: &'r mut [u8],
    observer: I,
    program: &[(u64, u32)],
    regs: &[(Reg, u64)],
) -> VirtMachine<LentStorage<'r>, I> {
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .instrumentation(observer)
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds");
    for &(offset, word) in program {
        m.mem_write(RAM_BASE + offset, &word.to_le_bytes()).expect("the program is in RAM");
    }
    for &(reg, value) in regs {
        m.reg_write(reg, value);
    }
    m
}

/// The whole machine as a snapshot image.
fn snapshot<I: Instrumentation>(m: &VirtMachine<LentStorage<'_>, I>) -> Vec<u8> {
    let mut image = Vec::new();
    m.save_snapshot(&mut image).expect("a snapshot saves");
    image
}

/// Two machines run side by side, and what each observer heard.
struct Both {
    skipping: Skipping,
    watching: Watching,
    /// Whether the guest finished before [`GIVE_UP_AFTER`] ticks.
    finished: bool,
    /// Why the last step stopped, the same for both.
    stop: StopReason,
    skipping_x20: u64,
    watching_x20: u64,
    /// `ESR_EL1` at the end, the same for both.
    esr: u64,
    /// `x2` at the end, the same for both: each program sets it once its
    /// poll is over.
    x2: u64,
}

/// Run `program` on a [`Skipping`] and a [`Watching`] machine, `slice`
/// instructions per step, until a step stops for a reason other than its
/// budget, or [`GIVE_UP_AFTER`] ticks have passed. Both machines stop at
/// `RAM_BASE + done`, an exit address, before the instruction there runs.
/// After every step the two machines are the same machine.
fn run_both(program: &[(u64, u32)], regs: &[(Reg, u64)], slice: u64, done: Option<u64>) -> Both {
    let mut ram_a = vec![0u8; RAM_LEN];
    let mut ram_b = vec![0u8; RAM_LEN];
    let mut a = machine(&mut ram_a, Skipping::default(), program, regs);
    let mut b = machine(&mut ram_b, Watching::default(), program, regs);
    if let Some(offset) = done {
        a.set_exits(&[RAM_BASE + offset]).expect("one exit");
        b.set_exits(&[RAM_BASE + offset]).expect("one exit");
    }
    let (finished, stop) = loop {
        let skipped = a.step(RunBudget::Instructions(slice));
        let watched = b.step(RunBudget::Instructions(slice));
        assert_eq!(skipped, watched, "slice {slice}, tick {}", b.ticks());
        assert_eq!(a.ticks(), b.ticks(), "slice {slice}");
        assert!(snapshot(&a) == snapshot(&b), "slice {slice}: the machines differ at tick {}", b.ticks());
        if watched.stop == StopReason::ExitReached {
            let exit = done.map(|offset| RAM_BASE + offset);
            assert_eq!(Some(b.reg_read(Reg::Pc)), exit, "slice {slice}");
        }
        if watched.stop != StopReason::BudgetExhausted {
            break (true, watched.stop);
        }
        if b.ticks() >= GIVE_UP_AFTER {
            break (false, watched.stop);
        }
    };
    Both {
        skipping: core::mem::take(a.instrumentation_mut()),
        watching: core::mem::take(b.instrumentation_mut()),
        finished,
        stop,
        skipping_x20: a.reg_read(Reg::X20),
        watching_x20: b.reg_read(Reg::X20),
        esr: b.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1"),
        x2: b.reg_read(Reg::X2),
    }
}

#[test]
fn a_direct_poll_ends_as_running_every_instruction_ends_it() {
    for slice in [1, 7, 1000, 1 << 24] {
        let both = run_both(DIRECT, &[], slice, Some(0x18));
        assert!(both.finished, "slice {slice}");
        assert_eq!(both.stop, StopReason::ExitReached, "slice {slice}");
        if slice == 1 << 24 {
            assert!(
                both.skipping.branches * 10 < both.watching.branches,
                "slice {slice}: {} of {} branches run",
                both.skipping.branches,
                both.watching.branches
            );
        }
    }
}

#[test]
fn a_poll_through_a_call_and_return_is_fast_forwarded_too() {
    for slice in [1, 7, 1000, 1 << 24] {
        let both = run_both(CALL, &[], slice, Some(0x14));
        assert!(both.finished, "slice {slice}");
        assert_eq!(both.stop, StopReason::ExitReached, "slice {slice}");
        if slice == 1 << 24 {
            assert!(
                both.skipping.branches * 10 < both.watching.branches,
                "slice {slice}: {} of {} branches run",
                both.skipping.branches,
                both.watching.branches
            );
        }
    }
}

#[test]
fn polls_outside_the_rule_run_every_instruction() {
    struct Case {
        name: &'static str,
        program: &'static [(u64, u32)],
        regs: &'static [(Reg, u64)],
        done: u64,
    }
    let cases = [
        Case { name: "eq", program: EQUAL, regs: &[], done: 0x18 },
        Case { name: "narrow", program: NARROW, regs: &[], done: 0x18 },
        Case { name: "store", program: STORE, regs: &[(Reg::X20, RAM_BASE + 0x8000)], done: 0x1C },
    ];
    for Case { name, program, regs, done } in cases {
        let both = run_both(program, regs, 1 << 24, Some(done));
        assert_eq!(both.skipping.branches, both.watching.branches, "{name}");
        if name != "eq" {
            assert!(both.finished, "{name}");
            assert_eq!(both.stop, StopReason::ExitReached, "{name}");
        }
    }
}

#[test]
fn timer_interrupts_during_a_poll_arrive_at_the_same_instruction() {
    for slice in [997, 1 << 24] {
        let both = run_both(IRQ_POLL, &[], slice, None);
        assert!(both.finished, "slice {slice}");
        assert_eq!(both.stop, StopReason::PoweredOff, "slice {slice}");
        assert_eq!(both.skipping_x20, both.watching_x20, "slice {slice}");
        assert!(both.watching_x20 >= 10, "slice {slice}: {} interrupts", both.watching_x20);
        if slice == 1 << 24 {
            assert!(
                both.skipping.branches < both.watching.branches,
                "slice {slice}: {} of {} branches run",
                both.skipping.branches,
                both.watching.branches
            );
        }
    }
}

#[test]
fn watched_runs_hear_of_every_instruction() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(&mut ram, Watching::default(), DIRECT, &[]);
    while m.reg_read(Reg::Pc) != RAM_BASE + 0x18 {
        assert!(m.ticks() < GIVE_UP_AFTER, "the poll never ended");
        assert_eq!(m.step(RunBudget::Instructions(1000)).stop, StopReason::BudgetExhausted);
    }
    assert_eq!(m.instrumentation().executed, m.instructions());
}

#[test]
fn an_exit_address_after_a_poll_is_reached_by_both() {
    let mut ram_a = vec![0u8; RAM_LEN];
    let mut ram_b = vec![0u8; RAM_LEN];
    let mut a = machine(&mut ram_a, Skipping::default(), DIRECT, &[]);
    let mut b = machine(&mut ram_b, Watching::default(), DIRECT, &[]);
    a.set_exits(&[RAM_BASE + 0x14]).expect("one exit");
    b.set_exits(&[RAM_BASE + 0x14]).expect("one exit");
    let skipped = a.step(RunBudget::Instructions(1 << 24));
    let watched = b.step(RunBudget::Instructions(1 << 24));
    assert_eq!(skipped.stop, StopReason::ExitReached);
    assert_eq!(skipped, watched);
    assert_eq!(a.reg_read(Reg::Pc), RAM_BASE + 0x14);
    assert_eq!(b.reg_read(Reg::Pc), RAM_BASE + 0x14);
    assert_eq!(a.ticks(), b.ticks());
    assert!(snapshot(&a) == snapshot(&b), "the machines differ");
}

#[test]
fn an_exit_address_inside_a_poll_is_never_skipped() {
    let mut ram_a = vec![0u8; RAM_LEN];
    let mut ram_b = vec![0u8; RAM_LEN];
    let mut a = machine(&mut ram_a, Skipping::default(), DIRECT, &[]);
    let mut b = machine(&mut ram_b, Watching::default(), DIRECT, &[]);
    // One whole iteration: the next read, at 0x08, is reached by the
    // taken `b.hi`, so it starts a trace.
    let first = a.step(RunBudget::Instructions(5));
    assert_eq!(first.stop, StopReason::BudgetExhausted);
    assert_eq!(b.step(RunBudget::Instructions(5)), first);
    assert_eq!(a.reg_read(Reg::Pc), RAM_BASE + 0x08);
    // The poll's compare.
    a.set_exits(&[RAM_BASE + 0x0C]).expect("one exit");
    b.set_exits(&[RAM_BASE + 0x0C]).expect("one exit");
    let skipped = a.step(RunBudget::Instructions(1 << 24));
    let watched = b.step(RunBudget::Instructions(1 << 24));
    assert_eq!(skipped.stop, StopReason::ExitReached);
    assert_eq!(skipped, watched);
    assert_eq!(a.reg_read(Reg::Pc), RAM_BASE + 0x0C);
    assert_eq!(a.instructions(), 6);
    assert_eq!(a.ticks(), b.ticks());
    assert!(snapshot(&a) == snapshot(&b), "the machines differ");
}

#[test]
fn polls_of_every_shape_the_rule_covers_are_fast_forwarded() {
    struct Case {
        name: &'static str,
        program: &'static [(u64, u32)],
        done: u64,
    }
    let cases = [
        Case { name: "immediate, b.lt", program: IMMEDIATE_SIGNED, done: 0x10 },
        Case { name: "counter first, b.lo", program: COUNTER_FIRST, done: 0x18 },
        Case { name: "b.ge forward, b back", program: FORWARD_SIGNED, done: 0x1C },
        Case { name: "blr, br", program: THROUGH_REGISTERS, done: 0x2C },
        Case { name: "nop", program: WITH_NOP, done: 0x1C },
        Case { name: "cntpct_el0", program: PHYSICAL, done: 0x18 },
        Case { name: "translated", program: TRANSLATED, done: ALIAS - RAM_BASE + 0x50 },
    ];
    // Where a step ends inside a skip is the direct poll's subject; these
    // need one step that ends inside a poll and one that runs it whole.
    for Case { name, program, done } in cases {
        for slice in [1000, 1 << 24] {
            let both = run_both(program, &[], slice, Some(done));
            assert!(both.finished, "{name}, slice {slice}");
            assert_eq!(both.stop, StopReason::ExitReached, "{name}, slice {slice}");
            assert_eq!(both.x2, 1, "{name}, slice {slice}");
            if slice == 1 << 24 {
                assert!(
                    both.skipping.branches * 10 < both.watching.branches,
                    "{name}, slice {slice}: {} of {} branches run",
                    both.skipping.branches,
                    both.watching.branches
                );
            }
        }
    }
}

#[test]
fn a_poll_at_el0_traps_where_el0_may_not_read_the_counter_and_is_fast_forwarded_where_it_may() {
    for cntkctl in [0, EL0PCTEN] {
        for slice in [1, 1 << 24] {
            let both = run_both(AT_EL0, &[(Reg::X11, cntkctl)], slice, Some(0xC00));
            assert!(both.finished, "CNTKCTL_EL1 {cntkctl:#x}, slice {slice}");
            assert_eq!(both.stop, StopReason::ExitReached, "CNTKCTL_EL1 {cntkctl:#x}, slice {slice}");
            assert_eq!(both.esr, ESR_CNTVCT_TRAP, "CNTKCTL_EL1 {cntkctl:#x}, slice {slice}");
            assert_eq!(both.x2, 0, "CNTKCTL_EL1 {cntkctl:#x}, slice {slice}");
            assert_eq!(both.skipping.branches, both.watching.branches, "CNTKCTL_EL1 {cntkctl:#x}");
        }
    }
    for slice in [1000, 1 << 24] {
        let both = run_both(AT_EL0, &[(Reg::X11, EL0VCTEN)], slice, Some(0x34));
        assert!(both.finished, "slice {slice}");
        assert_eq!(both.stop, StopReason::ExitReached, "slice {slice}");
        assert_eq!(both.x2, 1, "slice {slice}");
        if slice == 1 << 24 {
            assert!(
                both.skipping.branches * 10 < both.watching.branches,
                "slice {slice}: {} of {} branches run",
                both.skipping.branches,
                both.watching.branches
            );
        }
    }
}
