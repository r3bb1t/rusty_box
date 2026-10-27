//! The interrupt path end to end, without firmware: a guest programs the GIC
//! and the virtual timer, sleeps in `WFI`, counts timer interrupts in its IRQ
//! handler, and powers off through PSCI after ten.
//!
//! Guest source (assembled with GNU as for aarch64 and linked at the base of
//! RAM, where a bare machine starts; offsets and words below are its
//! `objdump`):
//!
//! ```text
//! _start: ldr x0, =0x40001000      ; msr vbar_el1, x0
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
//!
//! A second guest, [`ROUTING`], shows which of the processor's two interrupt
//! inputs the GIC signals an interrupt on; a third, [`UART_INPUT`], takes
//! typed input the way a Linux console does, through the PL011's receive
//! timeout and the GIC.

use rusty_box_aarch64::board::{RAM_BASE, UART_CLOCK};
use rusty_box_aarch64::{
    MachineBuilder, MemoryLoan, RamSize, Reg, RunBudget, StopReason, VirtConfig,
};

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
    (0x68, 0x4000_1000),
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

const RAM_LEN: usize = 1 << 20;

fn run_sliced(slice: u64) -> Outcome {
    let mut ram = vec![0u8; RAM_LEN];
    let mut machine = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("the test board builds");
    for (offset, word) in PROGRAM {
        machine
            .mem_write(RAM_BASE + *offset as u64, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    loop {
        let outcome = machine.step(RunBudget::Ticks(slice));
        match outcome.stop {
            StopReason::BudgetExhausted if machine.ticks() < GIVE_UP_AFTER => continue,
            reason => {
                return Outcome {
                    reason,
                    ticks: machine.ticks(),
                    interrupts_counted: machine.reg_read(Reg::X20),
                    pc: machine.reg_read(Reg::Pc),
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

/// A snapshot taken mid-run, in the middle of a wait, restores the same
/// future: the restored machine powers off at the same tick with the same
/// count as the one that kept going, and an image is refused by a machine
/// built differently without changing it.
#[test]
fn a_snapshot_restores_the_same_future() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut machine = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("the test board builds");
    for (offset, word) in PROGRAM {
        machine
            .mem_write(RAM_BASE + *offset as u64, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    // Five periods in: the guest has counted a few interrupts and waits.
    assert_eq!(
        machine
            .step(RunBudget::Ticks(5 * TICKS_PER_PERIOD + 123))
            .stop,
        StopReason::BudgetExhausted
    );
    let mut image = Vec::new();
    machine.save_snapshot(&mut image).expect("a snapshot saves");
    let midway = (machine.ticks(), machine.reg_read(Reg::X20));
    assert!(midway.1 > 0 && midway.1 < 10, "{midway:?}");

    let finish = |m: &mut rusty_box_aarch64::VirtMachine<rusty_box_aarch64::LentStorage<'_>>| {
        let out = m.step(RunBudget::Ticks(GIVE_UP_AFTER));
        (out.stop, m.ticks(), m.reg_read(Reg::X20))
    };
    let kept_going = finish(&mut machine);
    assert_eq!(kept_going.0, StopReason::PoweredOff);

    machine
        .restore_snapshot(&mut image.as_slice())
        .expect("the image restores");
    assert_eq!((machine.ticks(), machine.reg_read(Reg::X20)), midway);
    assert_eq!(finish(&mut machine), kept_going);

    // Twice the RAM is a different machine.
    let mut other_ram = vec![0u8; 2 * RAM_LEN];
    let mut other = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(2 * RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut other_ram,
            vars: &mut [],
        })
        .expect("builds");
    let before = other.cpu_snapshot();
    assert!(other.restore_snapshot(&mut image.as_slice()).is_err());
    assert_eq!(
        other.cpu_snapshot(),
        before,
        "a refused image changes nothing"
    );
    assert_eq!(
        other.power().state(),
        rusty_box_aarch64::PowerState::Running
    );

    // A truncated one leaves the machine off until it is reset.
    let truncated = &image[..image.len() / 2];
    assert!(machine.restore_snapshot(&mut &truncated[..]).is_err());
    assert_eq!(
        machine.power().state(),
        rusty_box_aarch64::PowerState::PoweredOff
    );
    machine.power().reset();
    assert_eq!(
        machine.power().state(),
        rusty_box_aarch64::PowerState::Running
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
        assert_eq!(
            cut.interrupts_counted, whole.interrupts_counted,
            "slice {slice}"
        );
    }
}

/// The guest enables both groups at the distributor (`x6`), sets
/// `GICD_IGROUPR0` (`x7`) and `GICC_CTLR` (`x8`), sends itself an SGI through
/// `GICD_SGIR` (`x9`), reads `ISR_EL1` with `DAIF` still masked from reset,
/// then unmasks IRQ and FIQ; the vector it enters records its offset and
/// acknowledges through `GICC_IAR`:
///
/// ```text
/// _start: ldr x0, =0x40001000      ; msr vbar_el1, x0
///         ldr x1, =0x08000000      ; ldr x3, =0x08010000
///         str w6, [x1]             ; str w7, [x1, #0x80]   // GICD_CTLR, GICD_IGROUPR0
///         str w8, [x3]                                     // GICC_CTLR
///         mov w2, #0xff            ; str w2, [x3, #4]      // GICC_PMR
///         str w9, [x1, #0xf00]                             // GICD_SGIR
///         mrs x10, isr_el1         ; msr daifclr, #3
///         nop
/// 1:      b 1b
///         .org 0x1280              // IRQ, current EL with SP_ELx
///         mov x11, #0x280          ; ldr w12, [x3, #0xc]   // GICC_IAR
/// 2:      b 2b
///         .org 0x1300              // FIQ, current EL with SP_ELx
///         mov x11, #0x300          ; ldr w12, [x3, #0xc]   // GICC_IAR
/// 3:      b 3b
/// ```
const ROUTING: &[(usize, u32)] = &[
    (0x00, 0x5800_01C0),
    (0x04, 0xD518_C000),
    (0x08, 0x5800_01C1),
    (0x0C, 0x5800_01E3),
    (0x10, 0xB900_0026),
    (0x14, 0xB900_8027),
    (0x18, 0xB900_0068),
    (0x1C, 0x5280_1FE2),
    (0x20, 0xB900_0462),
    (0x24, 0xB90F_0029),
    (0x28, 0xD538_C10A),
    (0x2C, 0xD503_43FF),
    (0x30, 0xD503_201F),
    (0x34, 0x1400_0000),
    (0x38, 0x4000_1000),
    (0x3C, 0x0000_0000),
    (0x40, 0x0800_0000),
    (0x44, 0x0000_0000),
    (0x48, 0x0801_0000),
    (0x4C, 0x0000_0000),
    (0x1280, 0xD280_500B),
    (0x1284, 0xB940_0C6C),
    (0x1288, 0x1400_0000),
    (0x1300, 0xD280_600B),
    (0x1304, 0xB940_0C6C),
    (0x1308, 0x1400_0000),
];

/// `ISR_EL1.F` and `ISR_EL1.I`.
const ISR_F: u64 = 1 << 6;
const ISR_I: u64 = 1 << 7;
/// `GICC_CTLR`: EnableGrp0, EnableGrp1, AckCtl (so `GICC_IAR` takes Group 1
/// too) and FIQEn.
const GICC_CTLR_BOTH_GROUPS_ACKCTL: u64 = 0b0111;
const GICC_CTLR_FIQ_EN: u64 = 1 << 3;
/// `GICD_SGIR`: SGI 1, target filter "this processor only".
const SGI_1_TO_SELF: u64 = (2 << 24) | 1;

/// What the [`ROUTING`] guest saw.
#[derive(Debug, PartialEq, Eq)]
struct Routed {
    /// `ISR_EL1` before the interrupt was unmasked.
    isr: u64,
    /// The vector offset the processor entered.
    vector: u64,
    /// What `GICC_IAR` returned there.
    acknowledged: u64,
}

fn route(group1_sgis: u64, cpu_ctlr: u64) -> Routed {
    let mut ram = vec![0u8; RAM_LEN];
    let mut machine = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("the test board builds");
    for (offset, word) in ROUTING {
        machine
            .mem_write(RAM_BASE + *offset as u64, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    machine.reg_write(Reg::X6, 0b11);
    machine.reg_write(Reg::X7, group1_sgis);
    machine.reg_write(Reg::X8, cpu_ctlr);
    machine.reg_write(Reg::X9, SGI_1_TO_SELF);
    assert_eq!(
        machine.step(RunBudget::Instructions(40)).stop,
        StopReason::BudgetExhausted
    );
    Routed {
        isr: machine.reg_read(Reg::X10),
        vector: machine.reg_read(Reg::X11),
        acknowledged: machine.reg_read(Reg::X12),
    }
}

/// IHI 0048B §4.4.1 GICC_CTLR.FIQEn: while it is set the CPU interface
/// signals a Group 0 interrupt as FIQ and a Group 1 interrupt as IRQ; while
/// it is clear both are IRQ. The guest sees which input is asserted in
/// `ISR_EL1` and enters the FIQ or the IRQ vector.
#[test]
fn fiqen_signals_group_0_as_fiq_and_group_1_as_irq() {
    let fiq = Routed {
        isr: ISR_F,
        vector: 0x300,
        acknowledged: 1,
    };
    let irq = Routed {
        isr: ISR_I,
        vector: 0x280,
        acknowledged: 1,
    };
    assert_eq!(
        route(0, GICC_CTLR_BOTH_GROUPS_ACKCTL | GICC_CTLR_FIQ_EN),
        fiq,
        "Group 0 with FIQEn"
    );
    assert_eq!(
        route(1 << 1, GICC_CTLR_BOTH_GROUPS_ACKCTL | GICC_CTLR_FIQ_EN),
        irq,
        "Group 1 with FIQEn"
    );
    assert_eq!(
        route(0, GICC_CTLR_BOTH_GROUPS_ACKCTL),
        irq,
        "Group 0 without FIQEn"
    );
}

/// The guest sets up the GIC for the UART's SPI (INTID 33) and the PL011
/// as Linux's driver does — 115200 baud, FIFOs on, the receive interrupt at
/// half full and the receive timeout interrupt unmasked — then waits in
/// `WFI`. Its IRQ handler acknowledges through `GICC_IAR`, records
/// `ELR_EL1` and `UARTMIS`, reads the receive FIFO dry into the buffer at
/// `x24`, records `UARTRIS` and completes the interrupt:
///
/// ```text
/// _start: ldr x0, =0x40001000      ; msr vbar_el1, x0
///         ldr x1, =0x08000000      ; ldr x3, =0x08010000
///         ldr x0, =0x09000000
///         mov w2, #1               ; str w2, [x1]           // GICD_CTLR: EnableGrp0
///         strb w2, [x1, #0x821]                             // GICD_ITARGETSR8, byte 1: INTID 33 to CPU 0
///         mov w2, #2               ; str w2, [x1, #0x104]   // GICD_ISENABLER1: INTID 33
///         mov w2, #1               ; str w2, [x3]           // GICC_CTLR: EnableGrp0
///         mov w2, #0xff            ; str w2, [x3, #4]       // GICC_PMR
///         mov w2, #13              ; str w2, [x0, #0x24]    // UARTIBRD
///         mov w2, #1               ; str w2, [x0, #0x28]    // UARTFBRD
///         mov w2, #0x70            ; str w2, [x0, #0x2c]    // UARTLCR_H: 8N1, FEN
///         mov w2, #0x12            ; str w2, [x0, #0x34]    // UARTIFLS: RX and TX at 1/2
///         mov w2, #0x50            ; str w2, [x0, #0x38]    // UARTIMSC: RXIM, RTIM
///         mov w2, #0x301           ; str w2, [x0, #0x30]    // UARTCR: UARTEN, TXE, RXE
///         ldr x24, =0x40002000     ; mov x21, #0
///         msr daifclr, #2
/// 1:      wfi
///         cbz x21, 1b
/// 2:      b 2b
///         .ltorg
///         .org 0x1280              // IRQ, current EL with SP_ELx
///         ldr w21, [x3, #0xc]      ; mrs x22, elr_el1       // GICC_IAR
///         ldr w23, [x0, #0x40]                              // UARTMIS
/// 3:      ldr w2, [x0, #0x18]      ; tbnz w2, #4, 4f        // UARTFR.RXFE
///         ldr w2, [x0]             ; strb w2, [x24], #1     // UARTDR
///         b 3b
/// 4:      ldr w25, [x0, #0x3c]                              // UARTRIS
///         str w21, [x3, #0x10]     ; eret                   // GICC_EOIR
/// ```
const UART_INPUT: &[(usize, u32)] = &[
    (0x00, 0x5800_0400),
    (0x04, 0xD518_C000),
    (0x08, 0x5800_0401),
    (0x0C, 0x5800_0423),
    (0x10, 0x5800_0440),
    (0x14, 0x5280_0022),
    (0x18, 0xB900_0022),
    (0x1C, 0x3920_8422),
    (0x20, 0x5280_0042),
    (0x24, 0xB901_0422),
    (0x28, 0x5280_0022),
    (0x2C, 0xB900_0062),
    (0x30, 0x5280_1FE2),
    (0x34, 0xB900_0462),
    (0x38, 0x5280_01A2),
    (0x3C, 0xB900_2402),
    (0x40, 0x5280_0022),
    (0x44, 0xB900_2802),
    (0x48, 0x5280_0E02),
    (0x4C, 0xB900_2C02),
    (0x50, 0x5280_0242),
    (0x54, 0xB900_3402),
    (0x58, 0x5280_0A02),
    (0x5C, 0xB900_3802),
    (0x60, 0x5280_6022),
    (0x64, 0xB900_3002),
    (0x68, 0x5800_01D8),
    (0x6C, 0xD280_0015),
    (0x70, 0xD503_42FF),
    (0x74, 0xD503_207F),
    (0x78, 0xB4FF_FFF5),
    (0x7C, 0x1400_0000),
    (0x80, 0x4000_1000),
    (0x84, 0x0000_0000),
    (0x88, 0x0800_0000),
    (0x8C, 0x0000_0000),
    (0x90, 0x0801_0000),
    (0x94, 0x0000_0000),
    (0x98, 0x0900_0000),
    (0x9C, 0x0000_0000),
    (0xA0, 0x4000_2000),
    (0x1280, 0xB940_0C75),
    (0x1284, 0xD538_4036),
    (0x1288, 0xB940_4017),
    (0x128C, 0xB940_1802),
    (0x1290, 0x3720_0082),
    (0x1294, 0xB940_0002),
    (0x1298, 0x3800_1702),
    (0x129C, 0x17FF_FFFC),
    (0x12A0, 0xB940_3C19),
    (0x12A4, 0xB900_1075),
    (0x12A8, 0xD69F_03E0),
];

/// The instruction after the guest's `WFI`, where its IRQ returns.
const UART_AFTER_WFI: u64 = RAM_BASE + 0x78;
/// The guest's IRQ vector.
const UART_IRQ_VECTOR: u64 = RAM_BASE + 0x1280;
/// The spin the guest ends in once its handler has run.
const UART_DONE: u64 = RAM_BASE + 0x7C;
/// Where the handler stores what it reads from `UARTDR`.
const UART_BUFFER: u64 = RAM_BASE + 0x2000;
/// A line typed at a login prompt: five characters, under the eight that
/// raise the receive interrupt at half full, so only the receive timeout
/// tells the guest they are there.
const TYPED: &[u8] = b"root\r";
/// The UART's SPI: QEMU `virt`'s `VIRT_UART` line 1, INTID 32 + 1.
const INTID_UART0: u64 = 33;
/// `UARTMIS.RTMIS` (DDI 0183F Table 3-17).
const RTMIS: u64 = 1 << 6;
/// 32 bit periods (DDI 0183F §3.4.4) at the divisor the guest programs,
/// `UARTIBRD` 13 + `UARTFBRD` 1/64: 512 × (13 + 1/64) = 6664 cycles of
/// `UARTCLK`, which at 24 MHz are 13883.3 ticks of a machine running 50
/// million a second; the timeout is due at the first whole tick.
const RECEIVE_TIMEOUT_TICKS: u64 = {
    let cycles: u64 = 512 * (64 * 13 + 1) / 64;
    (cycles * VirtConfig::DEFAULT.ips.get()).div_ceil(UART_CLOCK.get())
};
const _: () = assert!(RECEIVE_TIMEOUT_TICKS == 13_884);

/// What the [`UART_INPUT`] guest saw.
#[derive(Debug, PartialEq, Eq)]
struct TypedInput {
    /// Ticks from the characters' arrival to the IRQ vector's first
    /// instruction.
    taken_after: u64,
    /// `ELR_EL1` in the handler: where the interrupted `WFI` resumes.
    resumes_at: u64,
    /// `GICC_IAR`.
    acknowledged: u64,
    /// `UARTMIS` on entry.
    masked_status: u64,
    /// `UARTRIS` once the FIFO is read dry.
    raw_status_after: u64,
    /// The characters the handler read.
    received: Vec<u8>,
}

/// Run the [`UART_INPUT`] guest to its `WFI`, type [`TYPED`] at it, and run
/// on in budgets of `slice` ticks until its handler has run.
fn type_at_a_waiting_guest(slice: u64) -> TypedInput {
    let mut ram = vec![0u8; RAM_LEN];
    let mut machine = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("the test board builds");
    for (offset, word) in UART_INPUT {
        machine
            .mem_write(RAM_BASE + *offset as u64, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    assert_eq!(
        machine.step(RunBudget::Instructions(100)).stop,
        StopReason::Halted,
        "the guest waits in WFI"
    );
    let arrived = machine.ticks();
    assert_eq!(
        machine.serial(0).expect("UART0").send(TYPED),
        TYPED.len(),
        "the FIFO takes the line"
    );
    // Bounded in calls as well as in ticks: a machine whose time stands
    // still must fail the test, not hang it.
    let mut run_to = |exit: u64| {
        machine.set_exits(&[exit]).expect("one exit");
        for _ in 0..GIVE_UP_AFTER {
            match machine.step(RunBudget::Ticks(slice)).stop {
                StopReason::ExitReached => return machine.ticks() - arrived,
                StopReason::BudgetExhausted | StopReason::Halted => {
                    assert!(
                        machine.ticks() - arrived < GIVE_UP_AFTER,
                        "slice {slice}: {exit:#x} never reached"
                    )
                }
                other => panic!("slice {slice}: {other:?}"),
            }
        }
        panic!(
            "slice {slice}: {exit:#x} never reached at tick {}",
            machine.ticks()
        );
    };
    let taken_after = run_to(UART_IRQ_VECTOR);
    run_to(UART_DONE);
    let mut received = vec![0u8; TYPED.len() + 1];
    machine
        .mem_read(UART_BUFFER, &mut received)
        .expect("the buffer is in RAM");
    let stored = (machine.reg_read(Reg::X24) - UART_BUFFER) as usize;
    received.truncate(stored);
    TypedInput {
        taken_after,
        resumes_at: machine.reg_read(Reg::X22),
        acknowledged: machine.reg_read(Reg::X21),
        masked_status: machine.reg_read(Reg::X23),
        raw_status_after: machine.reg_read(Reg::X25),
        received,
    }
}

/// The path a Linux shell's input takes when a line is shorter than the
/// receive interrupt's level: the PL011's receive timeout (DDI 0183F
/// §3.4.4) expires 32 bit periods after the last character, its
/// `UARTINTR` drives the GIC's SPI 33, the GIC signals IRQ, and the
/// processor leaves `WFI` and takes it. The guest acknowledges INTID 33,
/// sees `RTMIS` alone in `UARTMIS`, and reads the five characters, after
/// which `RTRIS` has gone with them — and the interrupt is taken at the
/// timeout's deadline, at the same tick however the run is sliced.
#[test]
fn typed_input_shorter_than_the_trigger_level_wakes_wfi_through_the_receive_timeout() {
    let expected = TypedInput {
        taken_after: RECEIVE_TIMEOUT_TICKS,
        resumes_at: UART_AFTER_WFI,
        acknowledged: INTID_UART0,
        masked_status: RTMIS,
        raw_status_after: 0,
        received: TYPED.to_vec(),
    };
    for slice in [1, 7, 1000, 1 << 24] {
        assert_eq!(
            type_at_a_waiting_guest(slice),
            expected,
            "slices of {slice} ticks"
        );
    }
}
