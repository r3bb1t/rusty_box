//! The `virt` board with the virtualization extensions
//! (`MachineBuilder::with_el2`, QEMU `-M virt,virtualization=on`), as a
//! guest and a host see it (R9): the processor starts at EL2, the device
//! tree names `SMC` as the PSCI conduit, the EL2 physical timer interrupts
//! on PPI 10, and a snapshot keeps the EL2 state and is refused by a machine
//! built without the extensions.
//!
//! Guest code is GNU as's for aarch64 (`-march=armv8.2-a`), linked at the
//! base of RAM; each snippet's source is beside its words.

use rusty_box_aarch64::board::{FLASH0_BASE, RAM_BASE};
use rusty_box_aarch64::boot::fdt;
use rusty_box_aarch64::{
    El, LentStorage, Levels, MachineBuilder, MemoryLoan, NoEl2, PowerState, Pstate, PstateRefusal, RamSize, Reg,
    RunBudget, StopReason, SysReg, VirtConfig, VirtMachine, WithEl2,
};
use rusty_box_core::SnapError;

const RAM_LEN: usize = 4 << 20;
/// `b .`
const SPIN: u32 = 0x1400_0000;

/// The level set a test machine is built with, as a value a table can hold:
/// [`MachineBuilder::with_el2`] or not.
#[derive(Debug, Clone, Copy)]
enum Kind {
    WithoutEl2,
    WithEl2,
}

fn ram_size() -> RamSize {
    RamSize::bytes(RAM_LEN as u64).expect("a valid size")
}

/// A bare machine with EL2.
fn bare(ram: &mut [u8]) -> VirtMachine<LentStorage<'_>, WithEl2> {
    MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(ram_size())
        .bare()
        .with_el2()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

/// A bare machine without EL2.
fn bare_without_el2(ram: &mut [u8]) -> VirtMachine<LentStorage<'_>, NoEl2> {
    MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(ram_size())
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

fn load<L: Levels>(machine: &mut VirtMachine<LentStorage<'_>, L>, program: &[(u64, u32)]) {
    for &(offset, word) in program {
        machine.mem_write(RAM_BASE + offset, &word.to_le_bytes()).expect("code is in RAM");
    }
}

/// What a machine running `SPIN` as its firmware starts with.
struct Started {
    pstate: Pstate,
    pc: u64,
    tree: Vec<u8>,
}

/// [`Started`] of a machine of `kind`, given its device tree from `source`
/// or, without one, from QEMU's.
fn started(kind: Kind, source: Option<&[u8]>) -> Started {
    fn observe<L: Levels>(m: &VirtMachine<LentStorage<'_>, L>) -> Started {
        Started {
            pstate: m.pstate(),
            pc: m.reg_read(Reg::Pc),
            tree: m.device_tree().to_vec(),
        }
    }
    let code = SPIN.to_le_bytes();
    let mut ram = vec![0u8; RAM_LEN];
    let mut vars = vec![0xFFu8; 4096];
    let builder = MachineBuilder::new(VirtConfig::DEFAULT).ram(ram_size());
    let builder = match source {
        Some(tree) => builder.device_tree(tree),
        None => builder,
    };
    let builder = builder.firmware(&code);
    let loan = MemoryLoan { ram: &mut ram, vars: &mut vars };
    match kind {
        Kind::WithoutEl2 => observe(&builder.build_in(loan).expect("builds")),
        Kind::WithEl2 => observe(&builder.with_el2().build_in(loan).expect("builds")),
    }
}

/// With virtualization, firmware starts at EL2 with `SP_EL2` selected (Arm
/// `AArch64_TakeReset`: the highest implemented level, there being no
/// EL3), and the device tree's `/psci` node names `smc` as its method, as
/// QEMU's `fdt_add_psci_node` writes it for a processor with EL2 and no
/// EL3; without, the firmware starts at EL1 and the method is `hvc`. Nothing
/// else in the tree differs.
#[test]
fn virtualization_starts_at_el2_and_calls_psci_with_smc() {
    let mut trees = Vec::new();
    for (kind, el, method) in [
        (Kind::WithoutEl2, El::El1, &b"hvc\0"[..]),
        (Kind::WithEl2, El::El2, &b"smc\0"[..]),
    ] {
        let Started { pstate, pc, tree } = started(kind, None);
        assert_eq!((pstate.el, pstate.sp_sel, pc), (el, true, FLASH0_BASE), "{kind:?}");
        assert_eq!(fdt::property(&tree, "psci", "method"), Ok(Some(method)), "{kind:?}");
        assert_eq!(
            fdt::property(&tree, "psci", "compatible"),
            Ok(Some(&b"arm,psci-1.0\0arm,psci-0.2\0arm,psci\0"[..]))
        );
        trees.push(tree);
    }
    // Without `/psci`'s method the trees name the same nodes in the same
    // order, each with the same properties holding the same bytes (the
    // Devicetree Specification gives the order of a node's properties no
    // meaning, and the edit writes `method` at the head of its node).
    let without_method = |tree: &[u8]| {
        let mut contents = TreeContents::of(tree);
        let before = contents.properties.len();
        contents.properties.retain(|(node, name, _)| (node.as_str(), name.as_str()) != ("/psci", "method"));
        assert_eq!(contents.properties.len(), before - 1, "one method");
        contents
    };
    assert_eq!(without_method(&trees[0]), without_method(&trees[1]));
}

/// A host's own device tree with no `/psci` node describes no PSCI
/// firmware: with virtualization it gains none, only the memory node every
/// tree is given.
#[test]
fn a_tree_without_psci_gains_no_psci_node() {
    // A flattened device tree holding an empty root node: the header, the
    // memory reservation block's terminator, the structure block, and no
    // strings.
    let mut tree = Vec::new();
    for word in [0xD00D_FEEDu32, 72, 56, 72, 40, 17, 16, 0, 0, 16] {
        tree.extend_from_slice(&word.to_be_bytes());
    }
    tree.extend_from_slice(&[0; 16]);
    for word in [1u32, 0, 2, 9] {
        tree.extend_from_slice(&word.to_be_bytes());
    }
    let built = started(Kind::WithEl2, Some(&tree)).tree;
    assert_eq!(fdt::property(&built, "psci", "method"), Ok(None));
    assert_eq!(fdt::property(&built, "memory@40000000", "device_type"), Ok(Some(&b"memory\0"[..])));
    assert_eq!(TreeContents::of(&built).nodes, ["", "/memory@40000000"]);
}

/// A host's own device tree whose `/psci` method names the other conduit
/// is given the machine's, the one its firmware answers on: a tree written
/// for a processor with EL2 names `hvc` on one without, and a tree written
/// without names `smc` on one with.
#[test]
fn a_host_tree_names_the_machine_s_psci_conduit() {
    let with_el2 = started(Kind::WithEl2, None).tree;
    let without_el2 = started(Kind::WithoutEl2, None).tree;
    for (kind, source, method) in [
        (Kind::WithoutEl2, &with_el2, &b"hvc\0"[..]),
        (Kind::WithEl2, &without_el2, &b"smc\0"[..]),
        (Kind::WithoutEl2, &without_el2, &b"hvc\0"[..]),
        (Kind::WithEl2, &with_el2, &b"smc\0"[..]),
    ] {
        let built = started(kind, Some(source)).tree;
        assert_eq!(fdt::property(&built, "psci", "method"), Ok(Some(method)), "{kind:?}");
    }
}

/// The nodes of a flattened device tree in order, and its properties as
/// (node path, name, value) sorted by node path and name.
#[derive(Debug, PartialEq)]
struct TreeContents {
    nodes: Vec<String>,
    properties: Vec<(String, String, Vec<u8>)>,
}

impl TreeContents {
    fn of(tree: &[u8]) -> Self {
        const FDT_BEGIN_NODE: u32 = 1;
        const FDT_END_NODE: u32 = 2;
        const FDT_PROP: u32 = 3;
        const FDT_NOP: u32 = 4;
        const FDT_END: u32 = 9;
        let be32 = |at: usize| u32::from_be_bytes(tree[at..at + 4].try_into().expect("four bytes"));
        let c_string = |at: usize| {
            let len = tree[at..].iter().position(|&b| b == 0).expect("a terminated string");
            String::from_utf8(tree[at..at + len].to_vec()).expect("an ASCII name")
        };
        let strings = be32(12) as usize;
        let mut at = be32(8) as usize;
        let mut path: Vec<String> = Vec::new();
        let mut contents = Self {
            nodes: Vec::new(),
            properties: Vec::new(),
        };
        loop {
            match be32(at) {
                FDT_BEGIN_NODE => {
                    let name = c_string(at + 4);
                    at += 4 + (name.len() + 1).next_multiple_of(4);
                    path.push(name);
                    contents.nodes.push(path.join("/"));
                }
                FDT_END_NODE => {
                    path.pop().expect("a node to end");
                    at += 4;
                }
                FDT_PROP => {
                    let len = be32(at + 4) as usize;
                    let name = c_string(strings + be32(at + 8) as usize);
                    let value = tree[at + 12..at + 12 + len].to_vec();
                    contents.properties.push((path.join("/"), name, value));
                    at += 12 + len.next_multiple_of(4);
                }
                FDT_NOP => at += 4,
                FDT_END => break,
                token => panic!("token {token:#x} at {at:#x}"),
            }
        }
        contents.properties.sort();
        contents
    }
}

/// A hypervisor at EL2 routes physical IRQs to itself (`HCR_EL2.IMO`),
/// enables PPI 10 (INTID 26) at the GIC and runs the EL2 physical timer:
/// three interrupts arrive at its IRQ vector, each acknowledged as INTID 26,
/// and `SMC` reaches the machine's PSCI `SYSTEM_OFF`.
///
/// ```text
/// _start: ldr x0, =0x40001000   ; msr vbar_el2, x0
///         mov x0, #(1 << 4)     ; msr hcr_el2, x0          // IMO
///         ldr x1, =0x08000000   ; mov w2, #1 ; str w2, [x1] // GICD_CTLR
///         mov w2, #(1 << 26)    ; str w2, [x1, #0x100]      // ISENABLER0
///         ldr x3, =0x08010000   ; mov w2, #1 ; str w2, [x3] // GICC_CTLR
///         mov w2, #0xff         ; str w2, [x3, #4]          // GICC_PMR
///         mov x4, #1000         ; msr cnthp_tval_el2, x4
///         mov x4, #1            ; msr cnthp_ctl_el2, x4
///         mov x20, #0           ; msr daifclr, #2
/// 1:      wfi                   ; cmp x20, #3 ; b.lt 1b
///         ldr x0, =0x84000008   ; smc #0                    // SYSTEM_OFF
/// 2:      b 2b
///         .org 0x1280                                       // IRQ, EL2 with SP_EL2
///         ldr w5, [x3, #0xc]    ; add x20, x20, #1          // GICC_IAR
///         mov x4, #1000         ; msr cnthp_tval_el2, x4
///         str w5, [x3, #0x10]   ; eret                      // GICC_EOIR
/// ```
#[test]
fn the_el2_timer_interrupts_on_ppi_10() {
    const PROGRAM: [(u64, u32); 39] = [
        (0x00, 0x5800_0340),
        (0x04, 0xD51C_C000),
        (0x08, 0xD280_0200),
        (0x0C, 0xD51C_1100),
        (0x10, 0x5800_0301),
        (0x14, 0x5280_0022),
        (0x18, 0xB900_0022),
        (0x1C, 0x52A0_8002),
        (0x20, 0xB901_0022),
        (0x24, 0x5800_02A3),
        (0x28, 0x5280_0022),
        (0x2C, 0xB900_0062),
        (0x30, 0x5280_1FE2),
        (0x34, 0xB900_0462),
        (0x38, 0xD280_7D04),
        (0x3C, 0xD51C_E204),
        (0x40, 0xD280_0024),
        (0x44, 0xD51C_E224),
        (0x48, 0xD280_0014),
        (0x4C, 0xD503_42FF),
        (0x50, 0xD503_207F),
        (0x54, 0xF100_0E9F),
        (0x58, 0x54FF_FFCB),
        (0x5C, 0x5800_0120),
        (0x60, 0xD400_0003),
        (0x64, 0x1400_0000),
        (0x68, 0x4000_1000),
        (0x6C, 0x0000_0000),
        (0x70, 0x0800_0000),
        (0x74, 0x0000_0000),
        (0x78, 0x0801_0000),
        (0x7C, 0x0000_0000),
        (0x80, 0x8400_0008),
        (0x1280, 0xB940_0C65),
        (0x1284, 0x9100_0694),
        (0x1288, 0xD280_7D04),
        (0x128C, 0xD51C_E204),
        (0x1290, 0xB900_1065),
        (0x1294, 0xD69F_03E0),
    ];
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &PROGRAM);
    let out = m.step(RunBudget::Instructions(1_000_000));
    assert_eq!(out.stop, StopReason::PoweredOff);
    assert_eq!(m.power().state(), PowerState::PoweredOff);
    assert_eq!(m.reg_read(Reg::X20), 3, "three timer interrupts");
    assert_eq!(m.reg_read(Reg::X5), 26, "each one INTID 26, PPI 10");
    assert_eq!(m.pstate().el, El::El2);
}

/// A snapshot of a machine with virtualization holds its EL2 state: a
/// machine built the same way restores `SP_EL2`, the registers an exception
/// to EL2 wrote and those EL2 set. A machine built without virtualization
/// refuses it before changing anything, as it refuses a host's `PSTATE`
/// naming EL2.
///
/// ```text
///         hvc #1        // at EL2: to EL2's own vector, VBAR_EL2 + 0x200
/// ```
#[test]
fn a_snapshot_keeps_the_el2_state_and_needs_virtualization() {
    const HVC_1: u32 = 0xD400_0022;
    const VBAR: u64 = RAM_BASE + 0x1000;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &[(0, HVC_1), (0x1200, SPIN)]);
    m.sysreg_write(SysReg::VBAR_EL2, VBAR).expect("EL2 has VBAR_EL2");
    m.sysreg_write(SysReg::HCR_EL2, 1 << 4).expect("and HCR_EL2");
    m.sysreg_write(SysReg::TPIDR_EL2, 0x1234).expect("and TPIDR_EL2");
    m.set_sp_el2(RAM_BASE + 0x8000);
    let out = m.step(RunBudget::Instructions(2));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::Pc), VBAR + 0x200);
    let mut image = Vec::new();
    m.save_snapshot(&mut image).expect("save");

    let registers = [
        SysReg::VBAR_EL2,
        SysReg::HCR_EL2,
        SysReg::TPIDR_EL2,
        SysReg::ESR_EL2,
        SysReg::ELR_EL2,
        SysReg::SPSR_EL2,
    ];
    let values = |m: &VirtMachine<LentStorage<'_>, WithEl2>| {
        let mut out: Vec<u64> = registers.iter().map(|&r| m.sysreg_read(r).expect("EL2 register")).collect();
        out.extend([m.sp_el2(), m.reg_read(Reg::Pc)]);
        out
    };
    let saved = values(&m);
    assert_eq!(saved[3], (0x16 << 26) | (1 << 25) | 1, "ESR_EL2: HVC #1");
    assert_eq!(saved[4], RAM_BASE + 4, "ELR_EL2: the next instruction");

    let mut again = vec![0u8; RAM_LEN];
    let mut same = bare(&mut again);
    same.restore_snapshot(&mut image.as_slice()).expect("restores");
    assert_eq!(values(&same), saved);
    assert_eq!(same.pstate(), m.pstate());

    let mut other = vec![0u8; RAM_LEN];
    let mut without = bare_without_el2(&mut other);
    // A host cannot put a machine without EL2 there either.
    let el2h = Pstate {
        el: El::El2,
        ..without.pstate()
    };
    assert_eq!(without.set_pstate(el2h), Err(PstateRefusal::AbsentLevel));
    let before = (without.cpu_snapshot(), without.ticks());
    assert_eq!(
        without.restore_snapshot(&mut image.as_slice()),
        Err(SnapError::Invalid("snapshot is of a machine built differently"))
    );
    assert_eq!((without.cpu_snapshot(), without.ticks()), before, "nothing changed");
    assert_eq!(without.pstate().el, El::El1);
}
