//! The host-facing API exercised the way a caller writes it, each test
//! asserting what the guest can observe (R9). Guest code is hand-assembled
//! (GNU as for aarch64); each snippet's source is beside its words.

use core::sync::atomic::{AtomicBool, Ordering};

use rusty_box_aarch64::board::{GPIO_BASE, RAM_BASE, RTC_BASE, UART0_BASE};
use rusty_box_aarch64::boot::{fdt, LinuxImage, PlacementError};
use rusty_box_aarch64::cpu::{ExceptionClass, TranslationFault};
use rusty_box_aarch64::instrumentation::{Direction, ExceptionKind, ExceptionTaken, MemoryAccess};
use rusty_box_aarch64::machine::{BuildError, MemoryError, OnResetRequest};
use rusty_box_aarch64::{
    El, Instrumentation, Key, KeyAction, KeyboardInput, LentStorage, Machine, MachineBuilder, MemoryLoan, PowerState,
    Progress, Pstate, RamSize, Reg, RunBudget, RunOutcome, SerialPort, StopReason, SysReg, SysRegRefusal, Typed, VReg,
    VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 4 << 20;
/// `b .`
const SPIN: u32 = 0x1400_0000;

fn ram_size() -> RamSize {
    RamSize::bytes(RAM_LEN as u64).expect("a valid size")
}

fn bare<'m, I: Instrumentation>(ram: &'m mut [u8], observer: I) -> VirtMachine<LentStorage<'m>, I> {
    MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(ram_size())
        .bare()
        .instrumentation(observer)
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

fn load<I: Instrumentation>(machine: &mut VirtMachine<LentStorage<'_>, I>, at: u64, words: &[u32]) {
    for (i, word) in words.iter().enumerate() {
        machine.mem_write(at + 4 * i as u64, &word.to_le_bytes()).expect("code is in RAM");
    }
}

/// `mov x0, #42; add x0, x0, #1; b .`
const ADD: [u32; 3] = [0xD280_0540, 0x9100_0400, SPIN];

#[test]
fn emu_start_runs_to_until_or_for_count_instructions() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &ADD);

    let out = m.emu_start(RAM_BASE, Some(RAM_BASE + 8), None);
    assert_eq!(out.stop, StopReason::ExitReached);
    assert_eq!(out.progress, Progress::Instructions(2));
    assert_eq!(m.reg_read(Reg::X0), 43);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 8, "nothing at the exit executed");

    let out = m.emu_start(RAM_BASE, None, Some(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X0), 42);
    assert!(!out.is_terminal());

    // A configured exit ends ordinary steps too, and emu_start's `until`
    // does not outlive its call.
    m.set_exits(&[RAM_BASE + 4]).expect("one exit");
    m.reg_write(Reg::Pc, RAM_BASE);
    let out = m.step(RunBudget::Instructions(100));
    assert_eq!((out.stop, m.reg_read(Reg::Pc)), (StopReason::ExitReached, RAM_BASE + 4));
    assert!(out.is_terminal());
    m.set_exits(&[]).expect("no exits");
    let out = m.step(RunBudget::Instructions(100));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(out.progress, Progress::Instructions(100));
    assert_eq!(m.set_exits(&[0; 17]).err().map(|e| e.requested), Some(17));
}

#[test]
fn registers_and_system_registers_by_name() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    m.reg_write(Reg::X5, 0x1234);
    m.reg_write(Reg::Fpcr, u64::MAX);
    let v7 = VReg::new(7).expect("V7 exists");
    m.vreg_write(v7, u128::MAX - 1);
    assert_eq!(m.reg_read(Reg::X5), 0x1234);
    assert_eq!(m.reg_read(Reg::Fpcr), 0x07C0_0000, "only AHP, DN, FZ and RMode exist");
    assert_eq!(m.vreg_read(v7), u128::MAX - 1);
    assert_eq!(VReg::new(32), None);

    assert_eq!(m.sysreg_read(SysReg::MIDR_EL1), Ok(0x411F_D070), "a Cortex-A57 r1p0");
    assert_eq!(m.sysreg_write(SysReg::MIDR_EL1, 0), Err(SysRegRefusal::ReadOnly));
    assert_eq!(m.sysreg_read(SysReg::OSLAR_EL1), Err(SysRegRefusal::WriteOnly));
    let unallocated = SysReg::new(3, 0, 15, 15, 7).expect("in the system register space");
    assert_eq!(m.sysreg_read(unallocated), Err(SysRegRefusal::Absent));
    assert_eq!(SysReg::new(1, 0, 0, 0, 0), None, "op0 1 is the SYS instruction space");
    m.sysreg_write(SysReg::TPIDR_EL0, 0xABCD).expect("TPIDR_EL0 is writable");
    assert_eq!(m.sysreg_read(SysReg::TPIDR_EL0), Ok(0xABCD));
    // A host reads FPCR whatever CPACR_EL1 says, where a guest would trap.
    assert_eq!(m.sysreg_read(SysReg::CPACR_EL1), Ok(0));
    assert_eq!(m.sysreg_read(SysReg::FPCR), Ok(0x07C0_0000));

    let mut p = m.pstate();
    p.el = rusty_box_aarch64::El::El0;
    p.sp_sel = true;
    m.set_pstate(p);
    assert!(!m.pstate().sp_sel, "EL0 has only SP_EL0");
    assert_eq!(m.cpu_snapshot().x[5], 0x1234);
}

/// Page tables a host writes: VA 0x4000_0000 maps 1 GiB of RAM identically
/// (code, tables and vectors live there); VA 0x8000_0000 maps one page to
/// RAM + 0x20000 and the page after it is unmapped.
fn map_test_pages<I: Instrumentation>(m: &mut VirtMachine<LentStorage<'_>, I>) {
    const AF: u64 = 1 << 10;
    const INNER_SHAREABLE: u64 = 3 << 8;
    let l1 = RAM_BASE + 0x1_0000;
    let l2 = RAM_BASE + 0x1_1000;
    let l3 = RAM_BASE + 0x1_2000;
    let entry = |m: &mut VirtMachine<LentStorage<'_>, I>, table: u64, index: u64, desc: u64| {
        m.mem_write(table + 8 * index, &desc.to_le_bytes()).expect("tables are in RAM");
    };
    entry(m, l1, 1, RAM_BASE | AF | INNER_SHAREABLE | 0b01);
    entry(m, l1, 2, l2 | 0b11);
    entry(m, l2, 0, l3 | 0b11);
    entry(m, l3, 0, (RAM_BASE + 0x2_0000) | AF | INNER_SHAREABLE | 0b11);
    m.sysreg_write(SysReg::MAIR_EL1, 0xFF).expect("MAIR");
    // T0SZ 25 (a 39-bit space, walks start at level 1), TTBR1 walks off.
    m.sysreg_write(SysReg::TCR_EL1, 25 | (1 << 23)).expect("TCR");
    m.sysreg_write(SysReg::TTBR0_EL1, l1).expect("TTBR0");
    m.sysreg_write(SysReg::VBAR_EL1, RAM_BASE + 0x3000).expect("VBAR");
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR");
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr | 1).expect("SCTLR");
}

/// `str x1, [x0]`
const STORE: [u32; 2] = [0xF900_0001, SPIN];

#[test]
fn a_store_across_a_page_reports_the_page_that_faulted() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    map_test_pages(&mut m);
    let vector = RAM_BASE + 0x3000 + 0x200;
    load(&mut m, vector, &[SPIN]);
    load(&mut m, RAM_BASE + 0x100, &STORE);
    m.set_exits(&[vector]).expect("one exit");

    assert_eq!(m.virt_to_phys(0x8000_0FF9), Ok(RAM_BASE + 0x2_0FF9));
    assert_eq!(m.virt_to_phys(0x8000_1000), Err(TranslationFault::Translation { level: 3 }));

    m.reg_write(Reg::X0, 0x8000_0FF9);
    m.reg_write(Reg::X1, 0x1122_3344_5566_7788);
    let out = m.emu_start(RAM_BASE + 0x100, None, Some(10));
    assert_eq!(out.stop, StopReason::ExitReached, "the store took a data abort");
    // EC 0x25 (data abort, same EL), IL, WnR, translation fault at level 3.
    assert_eq!(m.sysreg_read(SysReg::ESR_EL1), Ok(0x9600_0047));
    assert_eq!(
        m.sysreg_read(SysReg::FAR_EL1),
        Ok(0x8000_1000),
        "the first byte that faulted, in the page an OS must map"
    );
    let mut first = [0u8; 7];
    m.mem_read(RAM_BASE + 0x2_0FF9, &mut first).expect("RAM");
    assert_eq!(first, [0; 7], "nothing was written");

    // Map the second page: the retried store completes across both.
    let l3 = RAM_BASE + 0x1_2000;
    let desc = (RAM_BASE + 0x2_1000) | (1 << 10) | (3 << 8) | 0b11;
    m.mem_write(l3 + 8, &desc.to_le_bytes()).expect("RAM");
    let out = m.emu_start(RAM_BASE + 0x100, Some(RAM_BASE + 0x104), None);
    assert_eq!(out.stop, StopReason::ExitReached);
    let mut stored = [0u8; 8];
    m.virt_read(0x8000_0FF9, &mut stored).expect("mapped now");
    assert_eq!(u64::from_le_bytes(stored), 0x1122_3344_5566_7788);
    m.virt_write(0x8000_0FFC, &[0xAA; 8]).expect("mapped now");
    m.mem_read(RAM_BASE + 0x2_1000, &mut stored[..4]).expect("RAM");
    assert_eq!(stored[..4], [0xAA; 4], "a host write crosses the page too");
    assert_eq!(
        m.virt_read(0x8000_2000, &mut stored),
        Err(MemoryError::Translation {
            va: 0x8000_2000,
            fault: TranslationFault::Translation { level: 3 }
        })
    );
    assert_eq!(m.mem_read(0x10, &mut stored), Err(MemoryError::NotRam { addr: 0x10 }));
}

/// Switch the UART on (`UARTCR` = UARTEN | TXE | RXE), then poll it and
/// copy every byte received to the buffer at x3:
/// `mov w1, #0x301; str w1, [x0, #0x30];`
/// `f: ldr w1, [x0, #0x18]; tbnz w1, #4, f; ldr w2, [x0]; strb w2, [x3], #1; b f`
const ECHO: [u32; 7] = [
    0x5280_6021,
    0xB900_3001,
    0xB940_1801,
    0x3727_FFE1,
    0xB940_0002,
    0x3800_1462,
    0x17FF_FFFC,
];
/// Where [`ECHO`]'s polling loop starts, past the write that switches the
/// UART on.
const ECHO_POLL: u64 = 8;

#[test]
fn a_guest_receives_nothing_until_it_switches_the_receiver_on() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &ECHO);
    let buffer = RAM_BASE + 0x1000;
    m.reg_write(Reg::X0, UART0_BASE);
    m.reg_write(Reg::X3, buffer);
    let capacity = rusty_box_aarch64::board::SERIAL_INPUT_CAPACITY;
    // At reset UARTCR.UARTEN is clear (DDI 0183F Table 3-1): the bytes wait.
    assert_eq!(m.serial(0).expect("UART0").input_room(), capacity);
    assert_eq!(m.serial(0).expect("UART0").send(b"ok"), 2);
    assert_eq!(m.serial(0).expect("UART0").input_room(), capacity - 2);
    m.reg_write(Reg::Pc, RAM_BASE + ECHO_POLL);
    let out = m.step(RunBudget::Instructions(200));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    let mut got = [0u8; 2];
    m.mem_read(buffer, &mut got).expect("RAM");
    assert_eq!(got, [0, 0], "the polling loop saw RXFE throughout");
    // Switched on, the receiver takes them.
    m.reg_write(Reg::Pc, RAM_BASE);
    let out = m.step(RunBudget::Instructions(200));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    m.mem_read(buffer, &mut got).expect("RAM");
    assert_eq!(&got, b"ok");
    // The buffer, and the holding register (UARTLCR_H.FEN is clear).
    assert_eq!(m.serial(0).expect("UART0").input_room(), capacity + 1);
}

#[test]
fn the_terminal_keyboard_types_what_an_xterm_sends() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &ECHO);
    let buffer = RAM_BASE + 0x1000;
    m.reg_write(Reg::X0, UART0_BASE);
    m.reg_write(Reg::X3, buffer);

    let mut keyboard = m.keyboard();
    assert_eq!(keyboard.type_text("Hi!\n"), Typed::All { delivered: 4 });
    assert!(keyboard.tap(Key::Up));
    assert!(keyboard.key(Key::CtrlL, KeyAction::Press));
    assert!(keyboard.tap(Key::C));
    assert!(keyboard.key(Key::CtrlL, KeyAction::Release));
    assert!(keyboard.tap(Key::F1));
    assert_eq!(keyboard.type_text("\u{20AC}"), Typed::Unmappable { delivered: 0, ch: '\u{20AC}' });
    let out = m.step(RunBudget::Instructions(500));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    let expected = b"Hi!\r\x1b[A\x03\x1bOP";
    let mut got = [0u8; 12];
    m.mem_read(buffer, &mut got).expect("RAM");
    assert_eq!(&got[..expected.len()], expected);
    assert_eq!(got[expected.len()], 0, "nothing more arrived");

    // A full input buffer refuses a whole character rather than half of one.
    let filler = vec![b'x'; 2 * rusty_box_aarch64::board::SERIAL_INPUT_CAPACITY];
    let sent = m.serial(0).expect("UART0").send(&filler);
    assert!(sent < filler.len(), "the buffer is bounded");
    assert_eq!(m.serial(0).expect("UART0").input_room(), 0);
    assert_eq!(m.keyboard().type_text("c"), Typed::Refused { delivered: 0 });
    // Let the guest read until exactly two bytes are free.
    for _ in 0..100 {
        if m.serial(0).expect("UART0").input_room() == 2 {
            break;
        }
        let out = m.step(RunBudget::Instructions(1));
        assert_eq!(out.progress, Progress::Instructions(1), "the echo loop keeps running");
    }
    assert_eq!(m.serial(0).expect("UART0").input_room(), 2);
    let mut keyboard = m.keyboard();
    assert!(!keyboard.tap(Key::Up), "ESC [ A needs three bytes, two are free");
    assert_eq!(keyboard.type_text("ab"), Typed::All { delivered: 2 });
    assert_eq!(keyboard.type_text("c"), Typed::Refused { delivered: 0 });
    assert!(m.serial(1).is_none(), "virt has one UART");
}

/// Code written against the shared `Machine` trait — the same function an
/// x86 machine runs — types a line and reads back what the guest echoed.
fn echo_through_any_machine<M: Machine>(machine: &mut M, text: &str) -> Result<usize, M::Error> {
    let typed = machine.keyboard().type_text(text);
    assert_eq!(typed, Typed::All { delivered: text.chars().count() });
    let sent = machine.serial(0).map_or(0, |mut port| port.send(b"!"));
    let outcome = machine.step(RunBudget::Instructions(1000))?;
    assert!(!outcome.is_terminal());
    assert_eq!(outcome.progress(), Progress::Instructions(1000));
    Ok(sent)
}

#[test]
fn the_shared_machine_verbs_drive_this_machine_too() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &ECHO);
    let buffer = RAM_BASE + 0x1000;
    m.reg_write(Reg::X0, UART0_BASE);
    m.reg_write(Reg::X3, buffer);
    let Ok(sent) = echo_through_any_machine(&mut m, "ls\n");
    assert_eq!(sent, 1);
    let mut got = [0u8; 4];
    m.mem_read(buffer, &mut got).expect("RAM");
    assert_eq!(&got, b"ls\r!");
}

#[derive(Default)]
struct Watcher {
    executed: u64,
    retired: u64,
    branches: u64,
    exceptions: Vec<ExceptionTaken>,
    stores: Vec<MemoryAccess>,
}

impl Instrumentation for Watcher {
    fn before_execution(&mut self, _pc: u64, _insn: u32) {
        self.executed += 1;
    }
    fn after_execution(&mut self, _pc: u64, _insn: u32) {
        self.retired += 1;
    }
    fn branch(&mut self, _from: u64, _to: u64) {
        self.branches += 1;
    }
    fn exception(&mut self, taken: ExceptionTaken) {
        self.exceptions.push(taken);
    }
    fn memory_access(&mut self, access: MemoryAccess) {
        if access.direction == Direction::Write {
            self.stores.push(access);
        }
    }
}

/// `str x1, [x0]; svc #7; b .`
const STORE_THEN_SVC: [u32; 3] = [0xF900_0001, 0xD400_00E1, SPIN];

#[test]
fn an_observer_sees_instructions_stores_and_exceptions() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, Watcher::default());
    load(&mut m, RAM_BASE, &STORE_THEN_SVC);
    let vector = RAM_BASE + 0x3000;
    m.sysreg_write(SysReg::VBAR_EL1, vector).expect("VBAR");
    load(&mut m, vector + 0x200, &[SPIN]);
    m.reg_write(Reg::X0, RAM_BASE + 0x800);
    m.reg_write(Reg::X1, 0xFEED);
    let out = m.emu_start(RAM_BASE, Some(vector + 0x200), None);
    assert_eq!(out.stop, StopReason::ExitReached);
    let w = m.instrumentation();
    assert_eq!(w.executed, 2);
    assert_eq!(w.retired, 1, "the SVC raised an exception instead of retiring");
    assert_eq!(w.branches, 0);
    assert_eq!(w.stores.len(), 1);
    assert_eq!(w.stores[0].address.get(), RAM_BASE + 0x800);
    assert_eq!(w.stores[0].value, 0xFEED);
    assert_eq!(w.exceptions.len(), 1);
    let svc = w.exceptions[0];
    assert_eq!(svc.kind, ExceptionKind::Synchronous(ExceptionClass::Svc));
    assert_eq!(svc.return_address, RAM_BASE + 8, "SVC returns past itself");
    assert_eq!(svc.vector, vector + 0x200);
    assert_eq!(svc.syndrome, (0x15 << 26) | (1 << 25) | 7);
    assert_eq!(svc.fault_address, None);
}

static STOP: AtomicBool = AtomicBool::new(false);

#[test]
fn a_raised_stop_flag_ends_every_run_until_lowered() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &[SPIN]);
    m.set_stop_flag(&STOP);
    STOP.store(true, Ordering::Relaxed);
    let out = m.step(RunBudget::Instructions(1000));
    assert_eq!((out.stop, out.progress), (StopReason::StopRequested, Progress::Instructions(0)));
    assert!(out.is_terminal());
    assert_eq!(m.step(RunBudget::Ticks(10)).stop, StopReason::StopRequested);
    STOP.store(false, Ordering::Relaxed);
    assert_eq!(m.step(RunBudget::Instructions(1000)).progress, Progress::Instructions(1000));
}

/// `hvc #0; b .` with x0 the PSCI function.
const HVC: [u32; 2] = [0xD400_0002, SPIN];
const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_SYSTEM_RESET: u64 = 0x8400_0009;

#[test]
fn psci_power_off_sticks_until_the_host_resets() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &HVC);
    m.reg_write(Reg::X0, PSCI_SYSTEM_OFF);
    assert_eq!(m.step(RunBudget::Instructions(10)).stop, StopReason::PoweredOff);
    assert_eq!(m.power().state(), PowerState::PoweredOff);
    let again = m.step(RunBudget::Instructions(10));
    assert_eq!((again.stop, again.progress), (StopReason::PoweredOff, Progress::Instructions(0)));
    let ticks = m.ticks();
    m.power().reset();
    assert_eq!(m.power().state(), PowerState::Running);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE, "a bare machine starts at the base of RAM");
    assert_eq!(m.reg_read(Reg::X0), 0, "registers are back at their reset values");
    assert_eq!(m.ticks(), ticks, "guest time carries on across a reset");
    let mut code = [0u8; 4];
    m.mem_read(RAM_BASE, &mut code).expect("RAM");
    assert_eq!(u32::from_le_bytes(code), HVC[0], "RAM a boot does not load is kept");
}

#[test]
fn a_reset_request_is_the_hosts_when_asked() {
    let mut ram = vec![0u8; RAM_LEN];
    let config = VirtConfig {
        on_reset_request: OnResetRequest::Stop,
        ..VirtConfig::DEFAULT
    };
    let mut m = MachineBuilder::new(config)
        .ram(ram_size())
        .bare()
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })
        .expect("builds");
    load(&mut m, RAM_BASE, &HVC);
    m.reg_write(Reg::X0, PSCI_SYSTEM_RESET);
    let out = m.step(RunBudget::Instructions(10));
    assert_eq!(out.stop, StopReason::ResetRequested);
    assert!(out.is_terminal());
}

/// `ldr w1, [x0]; b .`
const LOAD: [u32; 2] = [0xB940_0001, SPIN];

#[test]
fn the_power_button_holds_gpio_line_three_for_100_ms() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &LOAD);
    let gpio_data = GPIO_BASE + 0x3FC;
    m.power().press_power_button();
    let hold = m.ticks_to_next_timer_deadline().expect("the release is scheduled");
    assert_eq!(hold, VirtConfig::DEFAULT.ips.get() / 10, "100 ms of guest time");
    m.reg_write(Reg::X0, gpio_data);
    let read = m.emu_start(RAM_BASE, Some(RAM_BASE + 4), None);
    assert_eq!(read.stop, StopReason::ExitReached);
    assert_eq!(m.reg_read(Reg::X1) & 0x8, 0x8, "line 3 is high while held");
    assert_eq!(m.step(RunBudget::Ticks(hold)).stop, StopReason::BudgetExhausted);
    let read = m.emu_start(RAM_BASE, Some(RAM_BASE + 4), None);
    assert_eq!(read.stop, StopReason::ExitReached);
    assert_eq!(m.reg_read(Reg::X1) & 0x8, 0, "and low once released");
    assert_eq!(m.ticks_to_next_timer_deadline(), None);
}

/// `str w1, [x0, #4]` (RTCMR), then poll RTCRIS until it is set:
/// `ldr w2, [x0, #0x14]; cbz w2, .-4; b .`
const RTC_MATCH_POLL: [u32; 4] = [0xB900_0401, 0xB940_1402, 0x34FF_FFE2, SPIN];

/// A guest that sets the RTC's match register sees the raw status rise one
/// second later, masked or not: the write arms the alarm (QEMU
/// `pl031_set_alarm`), and the run loop takes the new deadline at once.
#[test]
fn the_rtc_alarm_a_guest_arms_fires_on_time() {
    let ips = 1_000_000;
    let config = VirtConfig {
        ips: rusty_box_core::ClockHz::new(ips).expect("a non-zero rate"),
        ..VirtConfig::DEFAULT
    };
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = MachineBuilder::new(config)
        .ram(ram_size())
        .bare()
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })
        .expect("builds");
    load(&mut m, RAM_BASE, &RTC_MATCH_POLL);
    m.reg_write(Reg::X0, RTC_BASE);
    m.reg_write(Reg::X1, (VirtConfig::DEFAULT_RTC_EPOCH + 1) & 0xFFFF_FFFF);
    let out = m.emu_start(RAM_BASE, Some(RAM_BASE + 12), Some(4 * ips));
    assert_eq!(out.stop, StopReason::ExitReached);
    assert!(m.ticks() > ips, "not before a second has passed: {}", m.ticks());
    assert!(m.ticks() < ips + 16, "and as soon as it has: {}", m.ticks());
}

/// `at s1e1r, x0; b .`
const AT_S1E1R: [u32; 2] = [0xD508_7800, SPIN];

/// A successful `AT` reports in QEMU's 64-bit `PAR_EL1` format: the page,
/// RES1 bit 11, NS, and Outer Shareable for Device memory, which is what
/// every data access is with the MMU off.
#[test]
fn address_translation_reports_par_el1_as_qemu_does() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &AT_S1E1R);
    m.reg_write(Reg::X0, RAM_BASE + 0x1234);
    let out = m.emu_start(RAM_BASE, Some(RAM_BASE + 4), None);
    assert_eq!(out.stop, StopReason::ExitReached);
    let (res1, ns, outer_shareable) = (1 << 11, 1 << 9, 2 << 7);
    assert_eq!(m.sysreg_read(SysReg::PAR_EL1), Ok((RAM_BASE + 0x1000) | res1 | ns | outer_shareable));
}

/// `br x1; nop; b .`
const BRANCH_THROUGH_X1: [u32; 3] = [0xD61F_0020, 0xD503_201F, SPIN];

/// Under TCR_EL1.TBI0 a branch to a tagged address drops the tag (Arm ARM
/// `AArch64.BranchAddr`; ARMv8.0 has no TBID to exempt instructions), so the
/// guest runs at the untagged address.
#[test]
fn a_branch_drops_the_tag_under_top_byte_ignore() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &BRANCH_THROUGH_X1);
    m.sysreg_write(SysReg::TCR_EL1, 1 << 37).expect("TCR_EL1");
    let target = RAM_BASE + 8;
    m.reg_write(Reg::X1, (0x5A << 56) | target);
    let out = m.emu_start(RAM_BASE, Some(target), Some(10));
    assert_eq!(out.stop, StopReason::ExitReached);
    assert_eq!(m.reg_read(Reg::Pc), target);
}

/// A host write of SPSel while the processor is at EL0 leaves SP_EL0
/// selected — EL0 has no other stack pointer — so the state is still one a
/// snapshot can record and restore.
#[test]
fn spsel_written_at_el0_keeps_sp_el0() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    m.set_pstate(Pstate {
        el: El::El0,
        sp_sel: false,
        ..m.pstate()
    });
    assert_eq!(m.sysreg_write(SysReg::SPSEL, 1), Ok(()));
    assert!(!m.pstate().sp_sel);
    let mut image = Vec::new();
    m.save_snapshot(&mut image).expect("a snapshot saves");
    assert_eq!(m.restore_snapshot(&mut image.as_slice()), Ok(()));
    assert_eq!((m.pstate().el, m.power().state()), (El::El0, PowerState::Running));
}

/// An image another version of the machine wrote — here, the version before
/// this one, whose layout differs — is refused before anything changes: the
/// machine keeps its state and keeps running.
#[test]
fn a_snapshot_of_an_earlier_version_is_refused_and_changes_nothing() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &ADD);
    let mut image = Vec::new();
    m.save_snapshot(&mut image).expect("a snapshot saves");
    // The image starts with its 8-byte magic and then its version.
    let version = u32::from_le_bytes(image[8..12].try_into().expect("four bytes"));
    image[8..12].copy_from_slice(&(version - 1).to_le_bytes());
    assert_eq!(m.step(RunBudget::Instructions(2)).stop, StopReason::BudgetExhausted);
    let before = (m.cpu_snapshot(), m.ram().to_vec());
    assert!(m.restore_snapshot(&mut image.as_slice()).is_err());
    assert_eq!((m.cpu_snapshot(), m.ram().to_vec()), before, "a refused image changes nothing");
    assert_eq!(m.power().state(), PowerState::Running);
    image[8..12].copy_from_slice(&version.to_le_bytes());
    assert_eq!(m.restore_snapshot(&mut image.as_slice()), Ok(()), "the same image at this version restores");
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE);
}

/// `wfi; b .`
const WFI: [u32; 2] = [0xD503_207F, SPIN];

#[test]
fn a_halted_machine_lets_a_tick_budget_pass() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &WFI);
    let out = m.step(RunBudget::Ticks(5000));
    assert_eq!(out.stop, StopReason::Halted);
    assert_eq!(out.progress, Progress::Instructions(1));
    assert_eq!(m.ticks(), 5000, "time passed while it waited");
    assert!(!out.is_terminal());
    let out = m.step(RunBudget::Instructions(5));
    assert_eq!((out.stop, out.progress), (StopReason::Halted, Progress::Instructions(0)));
    assert_eq!(m.ticks(), 5000, "an instruction budget cannot bound an idle wait");
}

/// `mov x9, #300; loop: add x0, x0, x9; str x0, [x10], #8;
/// eor x1, x1, x0, lsl #1; ldur x2, [x10, #-8]; add x3, x3, x2;
/// subs x9, x9, #1; b.ne loop; b .` — 2,102 instructions of loads, stores
/// and arithmetic, then a spin.
const SLICED_LOOP: [u32; 9] =
    [0xD280_2589, 0x8B09_0000, 0xF800_8540, 0xCA00_0421, 0xF85F_8142, 0x8B02_0063, 0xF100_0529, 0x54FF_FF41, SPIN];

/// The guest-visible state: the registers and RAM.
fn registers_and_ram<I: Instrumentation>(m: &VirtMachine<LentStorage<'_>, I>) -> (rusty_box_aarch64::CpuState, Vec<u8>) {
    (m.cpu_snapshot(), m.ram().to_vec())
}

/// However an instruction budget is cut into runs — one instruction at a
/// time, at every count that ends a run inside a block of straight-line
/// code, or all at once — the machine ends in the same state after the
/// same instructions.
#[test]
fn every_slicing_of_a_run_ends_in_the_same_state() {
    const TOTAL: u64 = 2_200;
    let run = |slice: u64| {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
        load(&mut m, RAM_BASE, &SLICED_LOOP);
        m.reg_write(Reg::X10, RAM_BASE + 0x8000);
        let mut done = 0;
        while done < TOTAL {
            let out = m.step(RunBudget::Instructions(slice.min(TOTAL - done)));
            assert_eq!(out.stop, StopReason::BudgetExhausted);
            done += out.progress.instructions().unwrap_or(0);
        }
        assert_eq!(m.instructions(), TOTAL);
        registers_and_ram(&m)
    };
    let whole = run(TOTAL);
    // x3 sums x0 after each iteration k, which is 300 + 299 + ... (k terms).
    assert_eq!(whole.0.x[3], (1..=300u64).map(|k| 300 * k - k * (k - 1) / 2).sum::<u64>(), "the loop's sum");
    for slice in [1, 2, 3, 5, 7, 11, 13, 31, 32, 33, 64, 100, 1000] {
        assert!(run(slice) == whole, "runs of {slice} instructions");
    }
}

/// Set up the GIC for SPI 40 and unmask IRQs, then `str w2, [x1, #0x204]`
/// (GICD_ISPENDR1: SPI 40 pending) followed by three `add x20, x20, #1`
/// and `b .`; the IRQ vector records x20 and ELR_EL1 (`mov x21, x20;
/// mrs x22, elr_el1; b .`). Source:
///
/// ```text
/// _start: ldr x0, =0x40001000 ; msr vbar_el1, x0
///         ldr x1, =0x08000000 ; mov w2, #1        ; str w2, [x1]          // GICD_CTLR
///         mov w2, #(1 << 8)   ; str w2, [x1, #0x104]                      // ISENABLER1
///         ldr x3, =0x08010000 ; mov w4, #1        ; str w4, [x3]          // GICC_CTLR
///         mov w4, #0xff       ; str w4, [x3, #4]                          // GICC_PMR
///         mov x20, #0         ; msr daifclr, #2
///         str w2, [x1, #0x204]                                            // ISPENDR1
///         add x20, x20, #1    ; add x20, x20, #1  ; add x20, x20, #1 ; b .
///         .org 0x1280         // IRQ, current EL with SP_ELx
///         mov x21, x20        ; mrs x22, elr_el1  ; b .
/// ```
const PEND_THEN_COUNT: &[(u64, u32)] = &[
    (0x00, 0x5800_0280),
    (0x04, 0xD518_C000),
    (0x08, 0x5800_0281),
    (0x0C, 0x5280_0022),
    (0x10, 0xB900_0022),
    (0x14, 0x5280_2002),
    (0x18, 0xB901_0422),
    (0x1C, 0x5800_0223),
    (0x20, 0x5280_0024),
    (0x24, 0xB900_0064),
    (0x28, 0x5280_1FE4),
    (0x2C, 0xB900_0464),
    (0x30, 0xD280_0014),
    (0x34, 0xD503_42FF),
    (0x38, 0xB902_0422),
    (0x3C, 0x9100_0694),
    (0x40, 0x9100_0694),
    (0x44, 0x9100_0694),
    (0x48, SPIN),
    (0x50, 0x4000_1000),
    (0x58, 0x0800_0000),
    (0x60, 0x0801_0000),
    (0x1280, 0xAA14_03F5),
    (0x1284, 0xD538_4036),
    (0x1288, SPIN),
];

/// An interrupt a store raises is taken at the very next instruction
/// boundary, before any instruction after the store runs.
#[test]
fn an_interrupt_a_store_raises_is_taken_before_the_next_instruction() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    for &(offset, word) in PEND_THEN_COUNT {
        m.mem_write(RAM_BASE + offset, &word.to_le_bytes()).expect("code is in RAM");
    }
    let out = m.step(RunBudget::Instructions(100));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X22), RAM_BASE + 0x3C, "the IRQ returns to the first add");
    assert_eq!(m.reg_read(Reg::X21), 0, "no add ran before the IRQ");
}

/// `add x0, x0, #1`, forty times, then `b .`.
fn straight_line() -> Vec<u32> {
    let mut code = vec![0x9100_0400; 40];
    code.push(SPIN);
    code
}

/// An exit at any instruction of straight-line code stops the run before
/// that instruction, with exactly the instructions before it retired.
#[test]
fn an_exit_inside_straight_line_code_stops_right_before_it() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &straight_line());
    for at in 0..=40u64 {
        m.reg_write(Reg::X0, 0);
        let out = m.emu_start(RAM_BASE, Some(RAM_BASE + 4 * at), None);
        assert_eq!(out.stop, StopReason::ExitReached, "exit at instruction {at}");
        assert_eq!(out.progress, Progress::Instructions(at), "exit at instruction {at}");
        assert_eq!((m.reg_read(Reg::X0), m.reg_read(Reg::Pc)), (at, RAM_BASE + 4 * at));
    }
    // An exit nowhere on the path, and one not on an instruction boundary,
    // leave the count bound to decide.
    let out = m.emu_start(RAM_BASE, Some(RAM_BASE + 2), Some(25));
    assert_eq!((out.stop, out.progress), (StopReason::BudgetExhausted, Progress::Instructions(25)));
}

/// A 64-byte arm64 `Image` header (text_offset 0, so it loads 2 MiB up),
/// followed by `b .` as the kernel.
fn fake_kernel(image_size: u64) -> Vec<u8> {
    let mut k = vec![0u8; 68];
    k[16..24].copy_from_slice(&image_size.to_le_bytes());
    k[56..60].copy_from_slice(b"ARM\x64");
    k[0..4].copy_from_slice(&SPIN.to_le_bytes());
    k
}

#[test]
fn a_directly_booted_kernel_is_entered_as_qemu_enters_it() {
    const MIB: u64 = 1 << 20;
    let kernel = fake_kernel(0x18_0000);
    let initrd = vec![0x5Au8; 3000];
    let image = LinuxImage::parse(&kernel).expect("an Image");
    let ram_len = 6 * MIB;
    let mut ram = vec![0u8; ram_len as usize];
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(ram_len).expect("a valid size"))
        .linux(image)
        .initrd(&initrd)
        .command_line("console=ttyAMA0 quiet")
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })
        .expect("builds");
    let entry = RAM_BASE + 2 * MIB;
    // QEMU's arm_setup_direct_kernel_boot: RAM / 2 (3 MiB) is below the
    // kernel's 1.5 MiB footprint at 2 MiB, so the initrd goes above it, at
    // 3.5 MiB, and the device tree at the next 2 MiB boundary, 4 MiB.
    let initrd_start = RAM_BASE + 3 * MIB + MIB / 2;
    let tree_at = RAM_BASE + 4 * MIB;
    assert_eq!(m.reg_read(Reg::Pc), entry);
    assert_eq!(m.reg_read(Reg::X0), tree_at, "x0 holds the device tree");
    assert_eq!((m.reg_read(Reg::X1), m.reg_read(Reg::X4)), (0, entry));
    assert_eq!(m.pstate().el, rusty_box_aarch64::El::El1);

    let tree = m.device_tree().to_vec();
    let mut placed = vec![0u8; tree.len()];
    m.mem_read(tree_at, &mut placed).expect("RAM");
    assert_eq!(placed, tree, "the tree is where x0 points");
    let prop = |name: &str| fdt::property(&tree, "chosen", name).expect("well formed").map(<[u8]>::to_vec);
    assert_eq!(prop("bootargs"), Some(b"console=ttyAMA0 quiet\0".to_vec()));
    assert_eq!(prop("linux,initrd-start"), Some(initrd_start.to_be_bytes().to_vec()));
    assert_eq!(prop("linux,initrd-end"), Some((initrd_start + 3000).to_be_bytes().to_vec()));
    let mut reg = [0u8; 16];
    reg[..8].copy_from_slice(&RAM_BASE.to_be_bytes());
    reg[8..].copy_from_slice(&ram_len.to_be_bytes());
    assert_eq!(fdt::property(&tree, "memory@40000000", "reg"), Ok(Some(&reg[..])));
    let mut first = [0u8; 4];
    m.mem_read(initrd_start, &mut first).expect("RAM");
    assert_eq!(first, [0x5A; 4]);

    // The kernel scribbles over itself; a reset loads it afresh.
    m.mem_write(entry, &[0; 4]).expect("RAM");
    m.mem_write(tree_at, &[0; 16]).expect("RAM");
    let ran = m.step(RunBudget::Instructions(3));
    assert_eq!(ran.progress, Progress::Instructions(3));
    m.power().reset();
    m.mem_read(entry, &mut first).expect("RAM");
    assert_eq!(u32::from_le_bytes(first), SPIN);
    m.mem_read(tree_at, &mut placed).expect("RAM");
    assert_eq!(placed, tree, "and the device tree with it");
    assert_eq!(m.reg_read(Reg::Pc), entry);

    // In 4 MiB the 2 MiB boundary after the initrd is the end of RAM, and
    // QEMU refuses the same machine ("Not enough space for DTB").
    let mut small = vec![0u8; RAM_LEN];
    let refused = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(ram_size())
        .linux(image)
        .initrd(&initrd)
        .build_in(MemoryLoan { ram: &mut small, vars: &mut [] });
    assert_eq!(
        refused.err(),
        Some(BuildError::Placement(PlacementError::DeviceTreeDoesNotFit))
    );
}

/// The PL011 transmits only while `UARTCR.UARTEN` and `TXE` are set (DDI
/// 0183F §3.3.8, Note); reset leaves `UARTEN` clear (Table 3-1). A character
/// written then waits in the transmit holding register with `UARTFR.BUSY`
/// and `TXFF` set, and goes out when the guest enables the UART:
///
/// ```text
/// movz x0, #0x0900, lsl #16 ; mov w5, #0x4b ; str w5, [x0]
/// ldr w6, [x0, #0x18] ; mov w1, #0x301 ; str w1, [x0, #0x30]
/// ldr w7, [x0, #0x18] ; b .
/// ```
#[test]
fn a_bare_guest_transmits_nothing_until_it_enables_the_uart() {
    const PROBE: [u32; 8] = [
        0xD2A1_2000, 0x5280_0965, 0xB900_0005, 0xB940_1806, 0x5280_6021, 0xB900_3001, 0xB940_1807, SPIN,
    ];
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram, rusty_box_aarch64::NoInstrumentation);
    load(&mut m, RAM_BASE, &PROBE);
    assert_eq!(m.step(RunBudget::Instructions(4)).stop, StopReason::BudgetExhausted);
    let output = |m: &mut VirtMachine<LentStorage<'_>>| m.serial(0).expect("UART0").take_output().collect::<Vec<u8>>();
    assert_eq!(output(&mut m), b"", "UARTEN is clear: nothing leaves");
    // RXFE, TXFF and BUSY: the holding register (FEN is clear) is full.
    assert_eq!(m.reg_read(Reg::X6), 0x10 | 0x20 | 0x08);
    assert_eq!(m.step(RunBudget::Instructions(4)).stop, StopReason::BudgetExhausted);
    assert_eq!(output(&mut m), b"K", "enabling the UART sent it");
    assert_eq!(m.reg_read(Reg::X7), 0x10 | 0x80, "RXFE and TXFE");
}

/// A kernel booted directly finds the UART as firmware leaves a console it
/// has set up: 115200 baud from the 24 MHz `UARTCLK` the device tree
/// declares (`UARTIBRD` 13, `UARTFBRD` 1), eight data bits with the FIFOs on
/// (`UARTLCR_H` 0x70), and the UART, transmitter and receiver enabled
/// (`UARTCR` 0x301); a character it writes goes out at once. The kernel
/// branches over its `Image` header to:
///
/// ```text
/// movz x0, #0x0900, lsl #16 ; ldr w1, [x0, #0x30] ; ldr w2, [x0, #0x24]
/// ldr w3, [x0, #0x28] ; ldr w4, [x0, #0x2c] ; mov w5, #0x4b
/// str w5, [x0] ; ldr w6, [x0, #0x18] ; b .
/// ```
#[test]
fn a_directly_booted_kernel_finds_the_console_set_up() {
    const MIB: u64 = 1 << 20;
    const PROBE: [u32; 9] = [
        0xD2A1_2000, 0xB940_3001, 0xB940_2402, 0xB940_2803, 0xB940_2C04, 0x5280_0965, 0xB900_0005, 0xB940_1806, SPIN,
    ];
    let mut kernel = fake_kernel(0x10_0000);
    // `b 0x40`, over the header.
    kernel[0..4].copy_from_slice(&0x1400_0010u32.to_le_bytes());
    kernel.resize(0x40, 0);
    for word in PROBE {
        kernel.extend_from_slice(&word.to_le_bytes());
    }
    let image = LinuxImage::parse(&kernel).expect("an Image");
    let mut ram = vec![0u8; (8 * MIB) as usize];
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(8 * MIB).expect("a valid size"))
        .linux(image)
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })
        .expect("builds");
    assert_eq!(m.step(RunBudget::Instructions(10)).stop, StopReason::BudgetExhausted);
    let regs = [Reg::X1, Reg::X2, Reg::X3, Reg::X4].map(|r| m.reg_read(r));
    assert_eq!(regs, [0x301, 13, 1, 0x70], "UARTCR, UARTIBRD, UARTFBRD, UARTLCR_H");
    assert_eq!(m.serial(0).expect("UART0").take_output().collect::<Vec<u8>>(), b"K");
    assert_eq!(m.reg_read(Reg::X6), 0x10 | 0x80, "RXFE and TXFE: nothing waits");
    // The device tree's UART clock is the one the divisor was worked out for.
    let tree = m.device_tree().to_vec();
    let clock = fdt::property(&tree, "apb-pclk", "clock-frequency").expect("well formed");
    let declared = rusty_box_aarch64::board::UART_CLOCK.get() as u32;
    assert_eq!(clock, Some(&declared.to_be_bytes()[..]));
    // A reset sets it up again.
    m.power().reset();
    assert_eq!(m.step(RunBudget::Instructions(10)).stop, StopReason::BudgetExhausted);
    assert_eq!(m.serial(0).expect("UART0").take_output().collect::<Vec<u8>>(), b"K");
}

#[test]
fn a_machine_that_cannot_be_built_says_why() {
    let mut ram = vec![0u8; RAM_LEN];
    let wrong = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::mib(8).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] });
    assert_eq!(
        wrong.err(),
        Some(BuildError::LentRamSize {
            lent: RAM_LEN as u64,
            configured: 8 << 20
        })
    );

    let too_big = fake_kernel(64 << 20);
    let image = LinuxImage::parse(&too_big).expect("an Image");
    let mut ram = vec![0u8; RAM_LEN];
    let placement = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(ram_size())
        .linux(image)
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] });
    assert_eq!(placement.err(), Some(BuildError::Placement(PlacementError::KernelDoesNotFit)));

    let code = [0u8; 16];
    let variables = [0x11u8; 8];
    let mut ram = vec![0xABu8; RAM_LEN];
    let mut vars = [0x22u8; 32];
    let tree = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(ram_size())
        .device_tree(b"definitely not a device tree")
        .firmware(&code)
        .variables(&variables)
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut vars });
    assert_eq!(
        tree.err(),
        Some(BuildError::DeviceTree(rusty_box_aarch64::boot::DeviceTreeError::NotADeviceTree))
    );
    // Everything that can fail is checked before the loan is written.
    assert!(ram.iter().all(|&b| b == 0xAB), "a refused build leaves the lent RAM alone");
    assert_eq!(vars, [0x22; 32], "and the lent variable store");

    // Flash bank 1 decodes the lent array on every boot, so every boot holds
    // it to the bank's 64 MiB.
    let mut ram = vec![0u8; RAM_LEN];
    let mut oversized = vec![0u8; (64 << 20) + 1];
    let vars = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(ram_size())
        .bare()
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut oversized });
    assert_eq!(vars.err(), Some(BuildError::VariablesTooLarge));

    assert_eq!(RamSize::bytes(4097), None, "not a whole number of pages");
    assert_eq!(RamSize::mib(0), None);
}

/// A machine moves to another thread, owned or lent, as the guide says.
#[test]
fn a_machine_can_move_to_another_thread() {
    fn moves<T: Send>() {}
    moves::<VirtMachine<LentStorage<'static>>>();
    #[cfg(feature = "alloc")]
    moves::<VirtMachine<rusty_box_aarch64::OwnedStorage>>();
}

/// The guest finds the disk where QEMU's `virt` puts the device given last:
/// below the ones before it, so on its own at the top transport and under
/// the keyboard with one. The machine keeps the disk, and a snapshot names
/// its size.
#[cfg(feature = "alloc")]
#[test]
fn a_disk_is_a_virtio_blk_the_guest_finds_and_the_machine_keeps() {
    use rusty_box_aarch64::board::VIRTIO_MMIO_BASE;
    use rusty_box_aarch64::{Disk, OwnedStorage};

    /// The virtio device ID the guest reads at transport `slot`: `ldr w0,
    /// [x1]; b .` with x1 at its `DeviceID` register.
    fn device_id(m: &mut VirtMachine<OwnedStorage>, slot: u64) -> u64 {
        m.mem_write(RAM_BASE, &0xB940_0020u32.to_le_bytes()).expect("code is in RAM");
        m.mem_write(RAM_BASE + 4, &SPIN.to_le_bytes()).expect("code is in RAM");
        m.reg_write(Reg::Pc, RAM_BASE);
        m.reg_write(Reg::X1, VIRTIO_MMIO_BASE + 0x200 * slot + 8);
        let out = m.step(RunBudget::Instructions(2));
        assert_eq!(out.progress, Progress::Instructions(2), "the load and the branch ran");
        m.reg_read(Reg::X0)
    }
    let build = |sectors: usize, keyboard: bool| {
        let builder = MachineBuilder::new(VirtConfig::DEFAULT).ram(ram_size()).bare();
        let builder = if keyboard { builder.virtio_keyboard() } else { builder };
        builder.virtio_disk(Disk::memory(vec![0x5A; sectors * 512])).build().expect("a machine with a disk builds")
    };

    let mut m = build(8, false);
    assert_eq!(device_id(&mut m, 31), 2, "virtio-blk at the top transport");
    let mut with_keyboard = build(8, true);
    assert_eq!(device_id(&mut with_keyboard, 31), 18, "the keyboard first");
    assert_eq!(device_id(&mut with_keyboard, 30), 2, "then the disk");

    assert_eq!(m.disk().map(Disk::sectors), Some(8));
    assert_eq!(m.disk().and_then(Disk::memory_bytes), Some(&[0x5A; 8 * 512][..]));
    let mut image = Vec::new();
    m.save_snapshot(&mut image).expect("a snapshot saves");
    assert!(build(16, false).restore_snapshot(&mut image.as_slice()).is_err(), "a disk of another size");
    assert_eq!(build(8, false).restore_snapshot(&mut image.as_slice()), Ok(()));
}
