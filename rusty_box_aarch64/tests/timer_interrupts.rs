//! The interrupt path end to end, without firmware: a guest programs the GIC
//! and the virtual timer, sleeps in `WFI`, counts timer interrupts in its IRQ
//! handler, and powers off through PSCI after ten.
//!
//! Guest source (assembled with GNU as for aarch64; flash offsets and words
//! below are its `objdump`):
//!
//! ```text
//! _start: ldr x0, =0x1000          ; msr vbar_el1, x0
//!         ldr x0, =0x40010000      ; mov sp, x0
//!         ldr x1, =0x08000000      ; mov w2, #1        ; str w2, [x1]          // GICD_CTLR
//!         mov w2, #(1 << 27)       ; str w2, [x1, #0x100]                      // ISENABLER0
//!         ldr x3, =0x08010000      ; mov w2, #1        ; str w2, [x3]          // GICC_CTLR
//!         mov w2, #0xff            ; str w2, [x3, #4]                          // GICC_PMR
//!         mov x4, #1000            ; msr cntv_tval_el0, x4
//!         mov x4, #1               ; msr cntv_ctl_el0, x4
//!         mov x20, #0              ; msr daifclr, #2
//! 1:      wfi                      ; cmp x20, #10      ; b.lt 1b
//!         ldr x0, =0x84000008      ; hvc #0            // PSCI SYSTEM_OFF
//! 2:      b 2b
//!         .org 0x1280              // IRQ, current EL with SP_ELx
//!         ldr w5, [x3, #0xc]       ; add x20, x20, #1  // GICC_IAR
//!         mov x4, #1000            ; msr cntv_tval_el0, x4
//!         str w5, [x3, #0x10]      ; eret              // GICC_EOIR
//! ```

use rusty_box_aarch64::{RunBudget, StopReason, VirtConfig, VirtMachine, VirtMemory};

const PROGRAM: &[(usize, u32)] = &[
    (0x00, 0x5800_0340),
    (0x04, 0xD518_C000),
    (0x08, 0x5800_0340),
    (0x0C, 0x9100_001F),
    (0x10, 0x5800_0341),
    (0x14, 0x5280_0022),
    (0x18, 0xB900_0022),
    (0x1C, 0x52A1_0002),
    (0x20, 0xB901_0022),
    (0x24, 0x5800_02E3),
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
    (0x50, 0xD503_207F),
    (0x54, 0xF100_2A9F),
    (0x58, 0x54FF_FFCB),
    (0x5C, 0x5800_0160),
    (0x60, 0xD400_0002),
    (0x64, 0x1400_0000),
    (0x68, 0x0000_1000),
    (0x6C, 0x0000_0000),
    (0x70, 0x4001_0000),
    (0x74, 0x0000_0000),
    (0x78, 0x0800_0000),
    (0x7C, 0x0000_0000),
    (0x80, 0x0801_0000),
    (0x84, 0x0000_0000),
    (0x88, 0x8400_0008),
    (0x1280, 0xB940_0C65),
    (0x1284, 0x9100_0694),
    (0x1288, 0xD280_7D04),
    (0x128C, 0xD51B_E304),
    (0x1290, 0xB900_1065),
    (0x1294, 0xD69F_03E0),
];

/// Ticks per re-arm: 1000 counter ticks at 62.5 MHz, at 50 M ticks/s.
const TICKS_PER_PERIOD: u64 = 800;
/// Far more than ten periods and the code around them.
const GIVE_UP_AFTER: u64 = 1_000_000;

struct Outcome {
    reason: StopReason,
    ticks: u64,
    interrupts_counted: u64,
    pc: u64,
}

fn run_sliced(slice: u64) -> Outcome {
    let mut flash0 = vec![0u8; 256 * 1024];
    for (offset, word) in PROGRAM {
        flash0[*offset..*offset + 4].copy_from_slice(&word.to_le_bytes());
    }
    let mut flash1 = vec![0xFFu8; 256 * 1024];
    let mut ram = vec![0u8; 1 << 20];
    let memory = VirtMemory {
        ram: &mut ram,
        flash0: &flash0,
        flash1: &mut flash1,
        dtb: &[],
    };
    let mut machine = match VirtMachine::new(VirtConfig::DEFAULT, memory) {
        Ok(m) => m,
        Err(e) => panic!("the test board must build: {e:?}"),
    };
    loop {
        match machine.run(RunBudget::ticks(slice)) {
            StopReason::BudgetExhausted if machine.now().ticks() < GIVE_UP_AFTER => continue,
            reason => {
                return Outcome {
                    reason,
                    ticks: machine.now().ticks(),
                    interrupts_counted: machine.cpu().xreg(20),
                    pc: machine.cpu().pc(),
                }
            }
        }
    }
}

/// GIC distributor and CPU interface, the virtual timer's PPI, IRQ entry
/// through `VBAR_EL1`, `GICC_IAR`/`EOIR`, `ERET` and `WFI` all work together:
/// the guest sees ten interrupts and asks to be powered off.
#[test]
fn the_guest_counts_ten_timer_interrupts_then_powers_off() {
    let out = run_sliced(1_000_000);
    assert_eq!(out.reason, StopReason::PoweredOff, "pc {:#x}", out.pc);
    assert_eq!(out.interrupts_counted, 10);
    assert!(
        out.ticks >= 10 * TICKS_PER_PERIOD,
        "ten periods of the timer must have elapsed, not {} ticks",
        out.ticks
    );
}

/// Determinism across budgets: however the run is cut, it powers off at the
/// same tick with the same count — including one tick per call, which puts a
/// budget boundary inside every wait.
#[test]
fn slicing_the_run_changes_nothing_the_guest_can_see() {
    let whole = run_sliced(1_000_000);
    for slice in [1, 7, 799, 801, 7_919] {
        let cut = run_sliced(slice);
        assert_eq!(cut.reason, whole.reason, "slice {slice}");
        assert_eq!(cut.ticks, whole.ticks, "slice {slice}");
        assert_eq!(cut.interrupts_counted, whole.interrupts_counted, "slice {slice}");
    }
}
