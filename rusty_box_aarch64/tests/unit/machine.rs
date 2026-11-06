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
    let cval = u128::from(TIMER_TICKS) * u128::from(config.counter.hz().get())
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

/// A device tree QEMU 11.1.2 generated for `-M virt -cpu cortex-a57 -m 256`
/// (`-machine dumpdtb=`): a host's tree to hand the machine, as a host may
/// hand it any tree.
const HOST_TREE: &[u8] = include_bytes!("../../assets/qemu-virt-a57-256m.dtb");

/// A host's device tree gets what QEMU's `arm_load_dtb` gives one: every
/// memory node replaced by one for the RAM, and the `/psci` node replaced
/// whole by the board's, whatever compatible, method or function
/// identifiers its own named. The rest of the tree, its GIC node among it,
/// is the host's, with EL2 or without.
#[test]
fn a_host_tree_gets_the_boards_memory_and_psci_nodes() {
    use super::{
        describe, BootChoice, Description, DeviceTree, PsciConduit, TreeLevels, TreeSource,
    };
    use crate::boot::fdt::{self, Edit, NodePath, Removal, Value};
    use crate::cpu::{NoEl2, WithEl2};
    use crate::identity::BoardIdentity;
    use crate::CpuModel;

    let psci = NodePath::of("psci");
    let identity = BoardIdentity::RUSTY_BOX;
    // A host's tree with a PSCI 0.1 node of other identifiers in place of
    // its own and a second memory node.
    let mut foreign = vec![0u8; 64 * 1024];
    let len = fdt::rebuild(
        HOST_TREE,
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
    for (levels, conduit) in [
        (TreeLevels::of::<NoEl2>(), PsciConduit::Hvc),
        (TreeLevels::of::<WithEl2>(), PsciConduit::Smc),
    ] {
        assert_eq!(levels.psci, conduit);
        let machine = Description {
            ram_len: 512 << 20,
            cpu: CpuModel::CortexA57,
            choice: BootChoice::Bare,
            layout: None,
            levels,
            wiring: crate::interrupt_map::InterruptMap::DEFAULT
                .wire(crate::SpiCount::DEFAULT)
                .expect("the default fits"),
            windows: crate::DeviceLayout::DEFAULT.windows(),
        };
        let mut from_host = std::boxed::Box::new(DeviceTree::EMPTY);
        describe(
            TreeSource::Host(foreign),
            machine,
            &identity,
            &mut from_host,
        )
        .expect("describes");
        let tree = from_host.as_slice();
        let get = |path, name| fdt::property(tree, path, name).expect("parses");
        assert_eq!(
            get("psci", "compatible"),
            Some(&b"arm,psci-1.0\0arm,psci-0.2\0"[..]),
            "{conduit:?}"
        );
        let method: &[u8] = match conduit {
            PsciConduit::Hvc => b"hvc\0",
            PsciConduit::Smc => b"smc\0",
        };
        assert_eq!(get("psci", "method"), Some(method));
        for gone in ["cpu_on", "cpu_off"] {
            assert_eq!(get("psci", gone), None, "{conduit:?}: the host's {gone}");
        }
        assert_eq!(get("memory@80000000", "device_type"), None, "{conduit:?}");
        assert_eq!(
            get("memory@40000000", "reg"),
            Some(&[0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0x20, 0, 0, 0][..])
        );
        // The host's GIC node, as the host wrote it: two frames, whatever
        // levels the machine has.
        assert_eq!(
            get("intc@8000000", "reg").map(<[u8]>::len),
            Some(32),
            "{conduit:?}"
        );
        assert_eq!(get("intc@8000000", "interrupts"), None);
    }
}

/// Every tree, the board's or a host's, names the board as its identity
/// says, and none holds a node for QEMU's `fw_cfg` or its platform bus,
/// which the board does not have.
#[test]
fn the_device_tree_names_the_board_by_its_identity() {
    use super::{describe, BootChoice, Description, DeviceTree, TreeLevels, TreeSource};
    use crate::boot::fdt;
    use crate::cpu::NoEl2;
    use crate::identity::{BoardIdentity, IdentityNames};
    use crate::CpuModel;

    let machine = Description {
        ram_len: 256 << 20,
        cpu: CpuModel::CortexA57,
        choice: BootChoice::Bare,
        layout: None,
        levels: TreeLevels::of::<NoEl2>(),
        wiring: crate::interrupt_map::InterruptMap::DEFAULT
            .wire(crate::SpiCount::DEFAULT)
            .expect("the default fits"),
        windows: crate::DeviceLayout::DEFAULT.windows(),
    };
    let host = BoardIdentity::new(IdentityNames {
        model: "Acme Board",
        compatible: &["acme,board-1", "acme,board"],
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|e| panic!("{e}"));
    for (identity, model, compatible) in [
        (
            BoardIdentity::RUSTY_BOX,
            &b"Rusty Box virt\0"[..],
            &b"rusty-box,virt\0"[..],
        ),
        (
            host,
            &b"Acme Board\0"[..],
            &b"acme,board-1\0acme,board\0"[..],
        ),
    ] {
        let mut tree = std::boxed::Box::new(DeviceTree::EMPTY);
        describe(TreeSource::Board, machine, &identity, &mut tree).expect("describes");
        let tree = tree.as_slice();
        assert_eq!(fdt::property(tree, "", "model"), Ok(Some(model)));
        assert_eq!(fdt::property(tree, "", "compatible"), Ok(Some(compatible)));
        for gone in ["fw-cfg@9020000", "platform-bus@c000000"] {
            assert_eq!(fdt::property(tree, gone, "compatible"), Ok(None), "{gone}");
        }
    }
    // A host's tree, here QEMU's own `linux,dummy-virt` one, is named by
    // the identity too.
    let mut tree = std::boxed::Box::new(DeviceTree::EMPTY);
    describe(
        TreeSource::Host(HOST_TREE),
        machine,
        &host,
        &mut tree,
    )
    .expect("describes");
    let tree = tree.as_slice();
    assert_eq!(
        fdt::property(tree, "", "model"),
        Ok(Some(&b"Acme Board\0"[..]))
    );
    assert_eq!(
        fdt::property(tree, "", "compatible"),
        Ok(Some(&b"acme,board-1\0acme,board\0"[..]))
    );
    for gone in ["fw-cfg@9020000", "platform-bus@c000000"] {
        assert_eq!(fdt::property(tree, gone, "compatible"), Ok(None), "{gone}");
    }
}


/// The interrupt map QEMU's `virt` wires (`a15irqmap`), which its tree
/// names.
fn qemu_wiring() -> crate::interrupt_map::Wiring {
    use crate::interrupt_map::{InterruptMap, MsiCount, MsiExtent, MsiSpis, Spi};
    let spi = |n| Spi::new(n).expect("an SPI");
    InterruptMap {
        uart: spi(1),
        rtc: spi(2),
        gpio: spi(7),
        virtio: spi(16),
        pcie_intx: spi(3),
        msi: MsiSpis {
            first: spi(48),
            count: MsiExtent::Count(MsiCount::new(64).expect("a count")),
        },
    }
    .wire(crate::SpiCount::DEFAULT)
    .expect("fits")
}

/// The host-tree check of `tree` on QEMU's layout, where its devices are,
/// and `wiring`.
fn check_qemu_tree(
    tree: &[u8],
    wiring: crate::interrupt_map::Wiring,
) -> Result<(), super::BuildError> {
    super::check_host_tree(tree, wiring, crate::DeviceLayout::QemuVirt.windows())
}

/// QEMU's tree edited by `edits`.
fn edited_host_tree(edits: &[crate::boot::fdt::Edit<'_>]) -> std::vec::Vec<u8> {
    let mut tree = vec![0u8; 64 * 1024];
    let len = crate::boot::fdt::rebuild(HOST_TREE, &[], edits, &mut tree).expect("rebuild");
    tree.truncate(len);
    tree
}

/// Each of the GIC node's `reg` regions is checked against the layout, in
/// the GICv2 binding's order, as many as the node names: QEMU's tree with
/// its CPU interface moved, or with the virtual interface's frames added
/// and its virtual CPU interface moved, is refused for that frame, and
/// with both frames where QEMU's layout has them it is not.
#[test]
fn every_region_of_the_gic_is_where_the_layout_has_it() {
    use super::{BuildError, HostTreeAddress, TreeDevice};
    use crate::boot::fdt::{Edit, NodePath, Value};
    let intc = NodePath::of("intc@8000000");
    // Each address and size two cells, as QEMU's root has them.
    let with_reg = |frames: &[u64]| {
        let bytes: std::vec::Vec<u8> = frames.iter().flat_map(|c| c.to_be_bytes()).collect();
        edited_host_tree(&[Edit::set(intc, "reg", Value::Bytes(&bytes))])
    };
    let refused = |device, named: u64, board: u64| {
        Err(BuildError::HostTreeAddress(HostTreeAddress {
            device,
            named: crate::bus::PhysAddr::new(named),
            board: crate::bus::PhysAddr::new(board),
        }))
    };
    let qemu = qemu_wiring();
    let moved_cpu = with_reg(&[0x0800_0000, 0x1_0000, 0x0801_8000, 0x1_0000]);
    assert_eq!(
        check_qemu_tree(&moved_cpu, qemu),
        refused(TreeDevice::GicCpuInterface, 0x0801_8000, 0x0801_0000)
    );
    let virtual_frames = |vcpu| {
        with_reg(&[
            0x0800_0000,
            0x1_0000,
            0x0801_0000,
            0x1_0000,
            0x0803_0000,
            0x1_0000,
            vcpu,
            0x1_0000,
        ])
    };
    assert_eq!(check_qemu_tree(&virtual_frames(0x0804_0000), qemu), Ok(()));
    assert_eq!(
        check_qemu_tree(&virtual_frames(0x0805_0000), qemu),
        refused(TreeDevice::GicVirtualCpu, 0x0805_0000, 0x0804_0000)
    );
}

/// A root whose cell counts no `reg` could hold names no region: the check
/// finds no device's address in such a tree, rather than overflowing.
#[test]
fn cell_counts_no_reg_holds_name_no_region() {
    use crate::boot::fdt::{Edit, NodePath, Value};
    let root = NodePath::of("");
    let huge = u32::MAX.to_be_bytes();
    let tree = edited_host_tree(&[
        Edit::set(root, "#address-cells", Value::Bytes(&huge)),
        Edit::set(root, "#size-cells", Value::Bytes(&huge)),
    ]);
    assert_eq!(check_qemu_tree(&tree, qemu_wiring()), Ok(()));
    let cells = super::RegCells {
        address: u32::MAX,
        size: 1,
    };
    assert_eq!(super::reg_address(&[0; 16], cells, 1), None);
}

/// A device's interrupt is the first its `interrupts-extended` names, which
/// Linux reads before `interrupts`: the UART's naming the GIC's SPI 1 by
/// it, whatever its `interrupts` says, fits QEMU's map and not the board's
/// own; one naming another controller names no SPI of the GIC's. Its
/// parent is found wherever it is, by `phandle` or `linux,phandle`, and a
/// phandle no node has is a malformed tree.
#[test]
fn a_devices_interrupts_extended_is_the_interrupt_it_names() {
    use super::{BuildError, HostTreeInterrupt};
    use crate::boot::fdt::{DeviceTreeError, Edit, NodePath, Value};
    use crate::interrupt_map::{InterruptMap, InterruptSource, Spi};
    let qemu = qemu_wiring();
    let own = InterruptMap::DEFAULT
        .wire(crate::SpiCount::DEFAULT)
        .expect("fits");
    let uart = NodePath::of("pl011@9000000");
    let intc = NodePath::of("intc@8000000");
    let spi = |n| Spi::new(n).expect("an SPI");
    let be = |cells: &[u32]| -> std::vec::Vec<u8> { cells.iter().flat_map(|c| c.to_be_bytes()).collect() };
    let (gic_spi1, gic_spi9, other_spi1) = (
        be(&[0x8002, 0, 1, 4]),
        be(&[0, 9, 4]),
        be(&[0x8004, 0, 1, 4]),
    );
    let extended = edited_host_tree(&[
        Edit::set(uart, "interrupts", Value::Bytes(&gic_spi9)),
        Edit::set(uart, "interrupts-extended", Value::Bytes(&gic_spi1)),
    ]);
    assert_eq!(check_qemu_tree(&extended, qemu), Ok(()));
    match check_qemu_tree(&extended, own) {
        Err(BuildError::HostTreeInterrupt(HostTreeInterrupt { source, named, wired })) => {
            assert_eq!(
                (source, named, wired),
                (InterruptSource::Virtio, Some(spi(16)), spi(3)),
                "the first device the tree names otherwise is refused"
            );
        }
        other => panic!("{other:?}"),
    }
    // The GPIO controller (phandle 0x8004 in QEMU's tree) is no GIC.
    let via_gpio = edited_host_tree(&[Edit::set(uart, "interrupts-extended", Value::Bytes(&other_spi1))]);
    assert_eq!(
        check_qemu_tree(&via_gpio, qemu),
        Err(BuildError::HostTreeInterrupt(HostTreeInterrupt {
            source: InterruptSource::Uart,
            named: None,
            wired: spi(1),
        }))
    );
    let (unused, gic) = (be(&[0x7777]), be(&[0x8002]));
    let linux_phandle = edited_host_tree(&[
        Edit::set(uart, "interrupts-extended", Value::Bytes(&gic_spi1)),
        Edit::set(intc, "phandle", Value::Bytes(&unused)),
        Edit::set(intc, "linux,phandle", Value::Bytes(&gic)),
    ]);
    assert_eq!(check_qemu_tree(&linux_phandle, qemu), Ok(()));
    let nested = NodePath::of("soc/gic");
    let three = be(&[3]);
    let nested_parent = edited_host_tree(&[
        Edit::set(uart, "interrupts-extended", Value::Bytes(&gic_spi1)),
        Edit::set(intc, "phandle", Value::Bytes(&unused)),
        Edit::node(nested),
        Edit::set(nested, "phandle", Value::Bytes(&gic)),
        Edit::set(nested, "interrupt-controller", Value::Bytes(&[])),
        Edit::set(nested, "#interrupt-cells", Value::Bytes(&three)),
    ]);
    assert_eq!(check_qemu_tree(&nested_parent, qemu), Ok(()));
    let orphaned = edited_host_tree(&[
        Edit::set(uart, "interrupts-extended", Value::Bytes(&gic_spi1)),
        Edit::set(intc, "phandle", Value::Bytes(&unused)),
    ]);
    assert_eq!(
        check_qemu_tree(&orphaned, qemu),
        Err(BuildError::DeviceTree(DeviceTreeError::Malformed))
    );
}

/// A device's `interrupts` go to its `interrupt-parent`, or the root's: the
/// UART's routed to the GPIO controller names no SPI of the GIC's,
/// whatever its specifier says, and is refused as its
/// `interrupts-extended` would be. A child of the root whose `reg` holds no
/// address is none of the board's devices, and the tree keeps it.
#[test]
fn a_devices_interrupts_follow_its_interrupt_parent() {
    use super::{BuildError, HostTreeInterrupt};
    use crate::boot::fdt::{Edit, NodePath, Value};
    use crate::interrupt_map::{InterruptSource, Spi};
    let qemu = qemu_wiring();
    let uart = NodePath::of("pl011@9000000");
    let gpio = 0x8004u32.to_be_bytes();
    let via_gpio = edited_host_tree(&[Edit::set(uart, "interrupt-parent", Value::Bytes(&gpio))]);
    assert_eq!(
        check_qemu_tree(&via_gpio, qemu),
        Err(BuildError::HostTreeInterrupt(HostTreeInterrupt {
            source: InterruptSource::Uart,
            named: None,
            wired: Spi::new(1).expect("an SPI"),
        }))
    );
    let odd = NodePath::of("odd");
    let one_cell = 1u32.to_be_bytes();
    let short_reg = edited_host_tree(&[
        Edit::node(odd),
        Edit::set(odd, "reg", Value::Bytes(&one_cell)),
    ]);
    assert_eq!(check_qemu_tree(&short_reg, qemu), Ok(()));
}

/// A disabled device node is no device a guest drives, so what it names
/// is the tree's: QEMU's tree with its virtio transports and its PrimeCells
/// disabled fits the board's own map, the PCIe host's lines and the GICv2m
/// frame being no device's.
#[test]
fn a_disabled_device_node_is_left_to_the_tree() {
    use crate::boot::fdt::{Edit, NodePath, Value};
    use crate::interrupt_map::InterruptMap;
    let own = InterruptMap::DEFAULT
        .wire(crate::SpiCount::DEFAULT)
        .expect("fits");
    let names: std::vec::Vec<std::string::String> = (0..32u64)
        .map(|n| std::format!("virtio_mmio@{:x}", 0xa00_0000 + n * 0x200))
        .chain(["pl011@9000000", "pl031@9010000", "pl061@9030000"].map(std::string::String::from))
        .collect();
    let edits: std::vec::Vec<Edit<'_>> = names
        .iter()
        .map(|name| Edit::set(NodePath::of(name), "status", Value::String("disabled")))
        .collect();
    assert_eq!(check_qemu_tree(&edited_host_tree(&edits), own), Ok(()));
}
