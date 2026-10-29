//! The run loop's sleep: a processor waiting for an interrupt sleeps to the
//! first tick strictly after now at which a timer or device output changes
//! by itself, and halts when there is none. A device that reports a
//! deadline it has reached and never clears (the board's test double)
//! changes neither: a run ends as it does on a machine without that device,
//! under an instruction budget, under `emu_start` with no count, and under
//! a tick budget alike.
//!
//! Each run happens on a thread of its own and has [`ANSWER_WITHIN`] to
//! answer, so a run that never ends fails its test instead of hanging it.

use std::sync::mpsc;
use std::time::Duration;
use std::vec;

use rusty_box_core::VmInstant;

use super::VirtMachine;
use crate::board::{GIC_CPU_BASE, GIC_DIST_BASE, RAM_BASE};
use crate::{
    LentStorage, MachineBuilder, MemoryLoan, NoEl2, Progress, RamSize, Reg, RunBudget, StopReason,
    SysReg, VirtConfig,
};

const RAM_LEN: usize = 1 << 20;

/// Far longer than any run here takes, debug build included.
const ANSWER_WITHIN: Duration = Duration::from_secs(60);

const SPIN: u32 = 0x1400_0000;

/// `wfi; b .`
const WFI: [u32; 2] = [0xD503_207F, SPIN];

/// Let the virtual timer's interrupt through the distributor (`x1`) and the
/// CPU interface (`x3`), then wait for it:
/// `mov w2, #1; str w2, [x1]; mov w2, #(1 << 27); str w2, [x1, #0x100];
/// mov w2, #1; str w2, [x3]; mov w2, #0xff; str w2, [x3, #4]; wfi; b .`
const TIMER_WFI: [u32; 10] = [
    0x5280_0022,
    0xB900_0022,
    0x52A1_0002,
    0xB901_0022,
    0x5280_0022,
    0xB900_0062,
    0x5280_1FE2,
    0xB900_0462,
    0xD503_207F,
    SPIN,
];
/// The `b .` after `TIMER_WFI`'s `wfi`.
const AFTER_TIMER_WFI: u64 = RAM_BASE + 9 * 4;

/// Ticks to the virtual timer's deadline in [`timer_set`]: a tick at a time
/// would take hours, the slices of the run loop a fraction of a second.
const TIMER_TICKS: u64 = 1 << 34;

/// The device double, plugged in or not.
#[derive(Clone, Copy)]
enum Double {
    Absent,
    /// It reports this tick as its deadline and never clears it.
    StuckAt(u64),
}

/// How the host runs the machine.
#[derive(Clone, Copy)]
enum Run {
    Step(RunBudget),
    /// `emu_start` from RAM's base with no count.
    EmuStart {
        until: Option<u64>,
    },
}

/// What a run ended with, and the guest time after it.
#[derive(Debug, PartialEq, Eq)]
struct Ran {
    stop: StopReason,
    progress: Progress,
    ticks: u64,
}

type Machine<'m> = VirtMachine<LentStorage<'m>, NoEl2>;

fn machine<'m>(ram: &'m mut [u8], code: &[u32]) -> Machine<'m> {
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds");
    for (i, word) in code.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("the code is in RAM");
    }
    m
}

/// No setup: no timer runs.
fn nothing(_: &mut Machine<'_>) {}

/// The virtual timer, enabled and unmasked, [`TIMER_TICKS`] away, and the
/// GIC's registers where `TIMER_WFI` expects them.
fn timer_set(m: &mut Machine<'_>) {
    let config = VirtConfig::DEFAULT;
    let cval = u128::from(TIMER_TICKS) * u128::from(config.cpu.counter_hz().get())
        / u128::from(config.ips.get());
    let cval = u64::try_from(cval).expect("the deadline fits the counter");
    m.sysreg_write(SysReg::CNTV_CVAL_EL0, cval)
        .expect("CNTV_CVAL_EL0");
    m.sysreg_write(SysReg::CNTV_CTL_EL0, 1)
        .expect("CNTV_CTL_EL0");
    m.reg_write(Reg::X1, GIC_DIST_BASE);
    m.reg_write(Reg::X3, GIC_CPU_BASE);
}

/// The power button pressed: its release is a device deadline, 100 ms
/// away.
fn button_pressed(m: &mut Machine<'_>) {
    m.power().press_power_button();
}

/// Build a machine running `code`, set it up, plug in `double`, and run it
/// on a thread of its own; a run still going after [`ANSWER_WITHIN`] fails
/// the test.
fn ran(code: &'static [u32], setup: fn(&mut Machine<'_>), double: Double, run: Run) -> Ran {
    let (answer, answered) = mpsc::channel();
    std::thread::spawn(move || {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = machine(&mut ram, code);
        setup(&mut m);
        match double {
            Double::Absent => {}
            Double::StuckAt(tick) => {
                m.board.stuck_deadline = Some(VmInstant::from_ticks(tick));
                // As a guest access that sets a deadline leaves the board.
                m.board.deadline_moved = true;
            }
        }
        let outcome = match run {
            Run::Step(budget) => m.step(budget),
            Run::EmuStart { until } => m.emu_start(RAM_BASE, until, None),
        };
        let result = Ran {
            stop: outcome.stop,
            progress: outcome.progress,
            ticks: m.ticks(),
        };
        answer
            .send(result)
            .expect("the test waits for the answer until it has failed");
    });
    answered.recv_timeout(ANSWER_WITHIN).expect("the run ends")
}

const HOST_RUNS: [Run; 3] = [
    Run::Step(RunBudget::Instructions(1000)),
    Run::EmuStart { until: None },
    Run::Step(RunBudget::Ticks(5000)),
];

#[test]
fn a_deadline_reached_and_never_cleared_lets_a_waiting_processor_halt() {
    for run in HOST_RUNS {
        let healthy = ran(&WFI, nothing, Double::Absent, run);
        let expected_ticks = match run {
            Run::Step(RunBudget::Ticks(n)) => n,
            Run::Step(RunBudget::Instructions(_)) | Run::EmuStart { .. } => 1,
        };
        assert_eq!(
            healthy,
            Ran {
                stop: StopReason::Halted,
                progress: Progress::Instructions(1),
                ticks: expected_ticks,
            },
            "nothing will wake it"
        );
        assert_eq!(
            ran(&WFI, nothing, Double::StuckAt(0), run),
            healthy,
            "a deadline reached before the run starts"
        );
        assert_eq!(
            ran(&WFI, nothing, Double::StuckAt(1), run),
            healthy,
            "a deadline reached as the processor starts waiting"
        );
    }
}

#[test]
fn a_waiting_processor_sleeps_to_a_deadline_and_halts_once_it_is_stuck() {
    // It sleeps to the deadline, which is in the future when it starts
    // waiting; reached and not cleared, it wakes nothing.
    for run in HOST_RUNS {
        let expected_ticks = match run {
            Run::Step(RunBudget::Ticks(n)) => n,
            Run::Step(RunBudget::Instructions(_)) | Run::EmuStart { .. } => 50,
        };
        assert_eq!(
            ran(&WFI, nothing, Double::StuckAt(50), run),
            Ran {
                stop: StopReason::Halted,
                progress: Progress::Instructions(1),
                ticks: expected_ticks,
            }
        );
    }
}

#[test]
fn a_waiting_processor_sleeps_past_a_stuck_deadline_to_another_devices() {
    let release = VirtConfig::DEFAULT.ips.get() / 10;
    let runs = [
        Run::Step(RunBudget::Instructions(1000)),
        Run::EmuStart { until: None },
        Run::Step(RunBudget::Ticks(release + 5000)),
    ];
    for run in runs {
        let healthy = ran(&WFI, button_pressed, Double::Absent, run);
        let expected_ticks = match run {
            Run::Step(RunBudget::Ticks(n)) => n,
            Run::Step(RunBudget::Instructions(_)) | Run::EmuStart { .. } => release,
        };
        assert_eq!(
            healthy,
            Ran {
                stop: StopReason::Halted,
                progress: Progress::Instructions(1),
                ticks: expected_ticks,
            },
            "it sleeps to the release, after which nothing will wake it"
        );
        assert_eq!(ran(&WFI, button_pressed, Double::StuckAt(0), run), healthy);
    }
}

#[test]
fn a_waiting_processor_sleeps_past_a_stuck_deadline_to_the_timer() {
    let runs = [
        Run::EmuStart {
            until: Some(AFTER_TIMER_WFI),
        },
        Run::Step(RunBudget::Instructions(100)),
        Run::Step(RunBudget::Ticks(TIMER_TICKS + 100)),
    ];
    for run in runs {
        let healthy = ran(&TIMER_WFI, timer_set, Double::Absent, run);
        assert!(
            healthy.ticks >= TIMER_TICKS,
            "the timer wakes it: {healthy:?}"
        );
        let expected_stop = match run {
            Run::EmuStart { .. } => StopReason::ExitReached,
            Run::Step(_) => StopReason::BudgetExhausted,
        };
        assert_eq!(healthy.stop, expected_stop);
        assert_eq!(
            ran(&TIMER_WFI, timer_set, Double::StuckAt(0), run),
            healthy,
            "the same wake-up, the same instant"
        );
    }
}

/// A host's device tree gets what QEMU's `arm_load_dtb` gives one: every
/// memory node replaced by one for the RAM, and the `/psci` node replaced
/// whole by QEMU's (`fdt_add_psci_node`), whatever compatible, method or
/// function identifiers its own named. QEMU's tree for this board with a
/// PSCI 0.1 node of other identifiers in place of its own and a second
/// memory node comes out as QEMU's tree itself does, byte for byte, for
/// either conduit.
#[test]
fn a_host_tree_gets_qemu_s_memory_and_psci_nodes() {
    use super::{describe, BootChoice, Description, DeviceTree, PsciConduit, TreeSource};
    use crate::boot::fdt::{self, Edit, NodePath, Removal, Value};
    use crate::boot::QEMU_VIRT_DEVICE_TREE;
    use crate::CpuModel;

    let psci = NodePath::of("psci");
    let mut foreign = vec![0u8; 64 * 1024];
    let len = fdt::rebuild(
        QEMU_VIRT_DEVICE_TREE,
        &[Removal::At(psci)],
        &[
            Edit::set(
                NodePath::of("memory@80000000"),
                "device_type",
                Value::String("memory"),
            ),
            Edit::set(psci, "compatible", Value::String("arm,psci")),
            Edit::set(psci, "method", Value::String("smc")),
            Edit::set(psci, "cpu_on", Value::Bytes(&0x95C1_BA60u32.to_be_bytes())),
            Edit::set(psci, "cpu_off", Value::Bytes(&0x95C1_BA5Fu32.to_be_bytes())),
        ],
        &mut foreign,
    )
    .expect("rebuild");
    let foreign = &foreign[..len];
    assert_eq!(
        fdt::property(foreign, "psci", "cpu_on"),
        Ok(Some(&0x95C1_BA60u32.to_be_bytes()[..]))
    );
    for conduit in [PsciConduit::Hvc, PsciConduit::Smc] {
        let machine = Description {
            ram_len: 256 << 20,
            cpu: CpuModel::CortexA57,
            choice: BootChoice::Bare,
            layout: None,
            psci: conduit,
        };
        let mut from_board = std::boxed::Box::new(DeviceTree::EMPTY);
        describe(TreeSource::Board, machine, &mut from_board).expect("describes");
        let mut from_host = std::boxed::Box::new(DeviceTree::EMPTY);
        describe(TreeSource::Host(foreign), machine, &mut from_host).expect("describes");
        assert_eq!(
            from_host.as_slice(),
            from_board.as_slice(),
            "{conduit:?}"
        );
        assert_eq!(
            fdt::property(from_host.as_slice(), "psci", "cpu_on"),
            Ok(Some(&0xC400_0003u32.to_be_bytes()[..])),
            "{conduit:?}: QEMU's identifier"
        );
        assert_eq!(
            fdt::property(from_host.as_slice(), "memory@80000000", "device_type"),
            Ok(None),
            "{conduit:?}"
        );
    }
    // Without EL2, and with the RAM QEMU's tree describes, the board's tree
    // is QEMU's own: the same tokens in the same order, the strings block
    // the same. Only the padding after a value may differ: libfdt leaves
    // whatever bytes were there (`fdt_setprop` copies the value alone),
    // and a property written anew here is padded with zeros.
    let machine = Description {
        ram_len: 256 << 20,
        cpu: CpuModel::CortexA57,
        choice: BootChoice::Bare,
        layout: None,
        psci: PsciConduit::Hvc,
    };
    let mut from_board = std::boxed::Box::new(DeviceTree::EMPTY);
    describe(TreeSource::Board, machine, &mut from_board).expect("describes");
    assert_eq!(
        tokens(from_board.as_slice()),
        tokens(QEMU_VIRT_DEVICE_TREE)
    );
}

/// A token of a flattened device tree's structure block, as a guest reads
/// it: the padding after a name or a value is not part of it.
#[derive(Debug, PartialEq)]
enum Token {
    Begin(vec::Vec<u8>),
    End,
    Property {
        name: vec::Vec<u8>,
        value: vec::Vec<u8>,
    },
}

/// What a flattened device tree holds, as a guest reads it: its structure
/// block token by token (`FDT_NOP` left out), and its strings block.
#[derive(Debug, PartialEq)]
struct Tokens {
    structure: vec::Vec<Token>,
    strings: vec::Vec<u8>,
}

fn tokens(tree: &[u8]) -> Tokens {
    let word = |at: usize| u32::from_be_bytes(tree[at..at + 4].try_into().expect("four bytes"));
    let c_string = |at: usize| {
        let len = tree[at..].iter().position(|&b| b == 0).expect("a NUL");
        tree[at..at + len].to_vec()
    };
    let (structure, strings) = (word(8) as usize, word(12) as usize);
    let mut read = Tokens {
        structure: vec![],
        strings: tree[strings..strings + word(32) as usize].to_vec(),
    };
    let mut at = structure;
    loop {
        match word(at) {
            1 => {
                let name = c_string(at + 4);
                at += 4 + (name.len() + 4) / 4 * 4;
                read.structure.push(Token::Begin(name));
            }
            2 => {
                read.structure.push(Token::End);
                at += 4;
            }
            3 => {
                let len = word(at + 4) as usize;
                read.structure.push(Token::Property {
                    name: c_string(strings + word(at + 8) as usize),
                    value: tree[at + 12..at + 12 + len].to_vec(),
                });
                at += 12 + len.div_ceil(4) * 4;
            }
            4 => at += 4,
            _ => return read,
        }
    }
}
