//! The `virt` board with the virtualization extensions
//! (`MachineBuilder::with_el2`, QEMU `-M virt,virtualization=on`), as a
//! guest and a host see it (R9): the processor starts at EL2, the device
//! tree names `SMC` as the PSCI conduit, the EL2 physical timer interrupts
//! on PPI 10, a `CNTVOFF_EL2` write that moves the virtual count past
//! `CVAL` has the virtual timer interrupt at once, a List register's virtual
//! interrupt reaches EL1 and its EOI's maintenance interrupt reaches EL2
//! (the GIC's virtual interface is there only with the extensions), a
//! snapshot keeps the EL2
//! state and is refused by a machine
//! built without the extensions, and a processor model that cannot have EL2
//! yet is refused it.
//!
//! Guest code is GNU as's for aarch64 (`-march=armv8.2-a`), linked at the
//! base of RAM; each snippet's source is beside its words.

use rusty_box_aarch64::board::{FLASH0_BASE, RAM_BASE};
use rusty_box_aarch64::boot::{fdt, LinuxImage};
use rusty_box_aarch64::machine::BuildError;
use rusty_box_aarch64::{
    Cpu, CpuConfig, CpuModel, El, El2Unavailable, LentStorage, Levels, MachineBuilder, MemoryLoan,
    NoEl2, PowerState, Pstate, PstateRefusal, RamSize, Reg, RunBudget, StopReason, SysReg,
    VirtConfig, VirtMachine, WithEl2,
};
use rusty_box_core::{ClockHz, SnapError};

#[path = "support/qemu_virt.rs"]
mod qemu_virt;

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
        machine
            .mem_write(RAM_BASE + offset, &word.to_le_bytes())
            .expect("code is in RAM");
    }
}

/// What a machine running `SPIN` as its firmware starts with.
struct Started {
    pstate: Pstate,
    pc: u64,
    tree: Vec<u8>,
}

/// [`Started`] of a machine of `kind`, given its device tree from `source`
/// or, without one, the board's own.
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
    let loan = MemoryLoan {
        ram: &mut ram,
        vars: &mut vars,
    };
    match kind {
        Kind::WithoutEl2 => observe(&builder.build_in(loan).expect("builds")),
        Kind::WithEl2 => observe(&builder.with_el2().build_in(loan).expect("builds")),
    }
}

/// The GIC node's `reg` with the Virtualization Extensions: the
/// distributor's, CPU interface's, virtual interface control registers' and
/// virtual CPU interface's frames, each the size it decodes (the
/// GIC binding's regions, IHI 0048B's 4 KiB distributor and 8 KiB
/// interfaces).
const GIC_REG_WITH_EL2: [u64; 8] = [
    0x0800_0000,
    0x1000,
    0x0801_0000,
    0x2000,
    0x0803_0000,
    0x2000,
    0x0804_0000,
    0x2000,
];

fn be_cells(cells: &[u64]) -> Vec<u8> {
    cells.iter().flat_map(|c| c.to_be_bytes()).collect()
}

/// The GIC node's `interrupts` with the Virtualization Extensions: the
/// maintenance interrupt, PPI 9 (INTID 25), level-high, to the one
/// processor.
fn maintenance_interrupt() -> Vec<u8> {
    [1u32, 9, 0x104].iter().flat_map(|c| c.to_be_bytes()).collect()
}

/// With virtualization, firmware starts at EL2 with `SP_EL2` selected (Arm
/// `AArch64_TakeReset`: the highest implemented level, there being no
/// EL3), and the device tree's `/psci` node names `smc` as its method, the
/// conduit to a firmware at EL2 when there is no EL3; without, the firmware
/// starts at EL1 and the method is `hvc`. With virtualization the GIC node
/// also names the virtual interface's two frames and its maintenance
/// interrupt, PPI 9 (INTID 25) level-high (the GIC binding).
/// Nothing else in the tree differs.
#[test]
fn virtualization_starts_at_el2_and_calls_psci_with_smc() {
    let gic_reg_without = be_cells(&GIC_REG_WITH_EL2[..4]);
    let gic_reg_with = be_cells(&GIC_REG_WITH_EL2);
    let maintenance = maintenance_interrupt();
    let mut trees = Vec::new();
    for (kind, el, method, gic_reg, gic_interrupts) in [
        (
            Kind::WithoutEl2,
            El::El1,
            &b"hvc\0"[..],
            &gic_reg_without[..],
            None,
        ),
        (
            Kind::WithEl2,
            El::El2,
            &b"smc\0"[..],
            &gic_reg_with[..],
            Some(&maintenance[..]),
        ),
    ] {
        let Started { pstate, pc, tree } = started(kind, None);
        assert_eq!(
            (pstate.el, pstate.sp_sel, pc),
            (el, true, FLASH0_BASE),
            "{kind:?}"
        );
        assert_eq!(
            fdt::property(&tree, "psci", "method"),
            Ok(Some(method)),
            "{kind:?}"
        );
        assert_eq!(
            fdt::property(&tree, "psci", "compatible"),
            Ok(Some(&b"arm,psci-1.0\0arm,psci-0.2\0"[..]))
        );
        assert_eq!(
            fdt::property(&tree, "interrupt-controller@8000000", "reg"),
            Ok(Some(gic_reg)),
            "{kind:?}"
        );
        assert_eq!(
            fdt::property(&tree, "interrupt-controller@8000000", "interrupts"),
            Ok(gic_interrupts),
            "{kind:?}"
        );
        trees.push(tree);
    }
    // Without `/psci`'s method and the GIC node's `reg` and `interrupts`
    // the trees name the same nodes in the same order, each with the same
    // properties holding the same bytes.
    let differing = [
        ("/psci", "method"),
        ("/interrupt-controller@8000000", "reg"),
        ("/interrupt-controller@8000000", "interrupts"),
    ];
    // Each differing property is there at most once in each tree: one that
    // an edit added twice is not hidden by the comparison.
    let without_differing = |tree: &[u8]| {
        let mut contents = TreeContents::of(tree);
        for (node, name) in differing {
            let count = contents
                .properties
                .iter()
                .filter(|(n, p, _)| n == node && p == name)
                .count();
            assert!(count <= 1, "{node} {name} appears {count} times");
        }
        contents
            .properties
            .retain(|(node, name, _)| !differing.contains(&(node.as_str(), name.as_str())));
        contents
    };
    assert_eq!(without_differing(&trees[0]), without_differing(&trees[1]));
}

/// A host's own device tree with no `/psci` node gains the board's, as
/// QEMU's `fdt_add_psci_node` adds its own to every tree it hands a guest,
/// with the machine's method; and an empty `/chosen`, which `arm_load_dtb`
/// adds where a tree has none, with the memory node every tree is given.
/// The node added last comes first, as libfdt adds a node.
#[test]
fn a_tree_without_psci_gains_the_boards_psci_node() {
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
    let qemu = started(Kind::WithEl2, None).tree;
    let built = started(Kind::WithEl2, Some(&tree)).tree;
    assert_eq!(
        fdt::property(&built, "psci", "method"),
        Ok(Some(&b"smc\0"[..]))
    );
    let node_of = |tree: &[u8], node: &str| {
        let mut properties = TreeContents::of(tree).properties;
        properties.retain(|(path, _, _)| path == node);
        properties
    };
    assert_eq!(node_of(&built, "/psci"), node_of(&qemu, "/psci"));
    assert_eq!(
        fdt::property(&built, "memory@40000000", "device_type"),
        Ok(Some(&b"memory\0"[..]))
    );
    assert_eq!(node_of(&built, "/chosen"), []);
    assert_eq!(
        TreeContents::of(&built).nodes,
        ["", "/psci", "/chosen", "/memory@40000000"]
    );
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
        assert_eq!(
            fdt::property(&built, "psci", "method"),
            Ok(Some(method)),
            "{kind:?}"
        );
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
            let len = tree[at..]
                .iter()
                .position(|&b| b == 0)
                .expect("a terminated string");
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
///         mov x4, #16000        ; msr cnthp_tval_el2, x4
///         mov x4, #1            ; msr cnthp_ctl_el2, x4
///         mov x20, #0           ; msr daifclr, #2
/// 1:      wfi                   ; cmp x20, #3 ; b.lt 1b
///         ldr x0, =0x84000008   ; smc #0                    // SYSTEM_OFF
/// 2:      b 2b
///         .org 0x1280                                       // IRQ, EL2 with SP_EL2
///         ldr w5, [x3, #0xc]    ; add x20, x20, #1          // GICC_IAR
///         mov x4, #16000        ; msr cnthp_tval_el2, x4
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
        (0x38, 0xD287_D004),
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
        (0x1288, 0xD287_D004),
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

/// A hypervisor at EL2 sets the virtual timer 2^40 counts ahead of the
/// virtual count, then writes `CNTVOFF_EL2` so the virtual count stands
/// 2^41 counts further on, past `CVAL`: the timer's interrupt, PPI 11
/// (INTID 27), arrives within the next few instructions, not where the
/// virtual count would have reached `CVAL` before the write.
///
/// ```text
/// _start: ldr x0, =0x40001000   ; msr vbar_el2, x0
///         mov x0, #(1 << 4)     ; msr hcr_el2, x0          // IMO
///         ldr x1, =0x08000000   ; mov w2, #1 ; str w2, [x1] // GICD_CTLR
///         mov w2, #(1 << 27)    ; str w2, [x1, #0x100]      // ISENABLER0
///         ldr x3, =0x08010000   ; mov w2, #1 ; str w2, [x3] // GICC_CTLR
///         mov w2, #0xff         ; str w2, [x3, #4]          // GICC_PMR
///         msr cntvoff_el2, xzr
///         mrs x4, cntvct_el0    ; mov x5, #(1 << 40) ; add x4, x4, x5
///         msr cntv_cval_el0, x4
///         mov x4, #1            ; msr cntv_ctl_el0, x4
///         msr daifclr, #2       ; mov x21, #0
///         ldr x5, =0xFFFFFE0000000000 ; msr cntvoff_el2, x5  // -2^41
/// 1:      add x21, x21, #1      ; b 1b
///         .org 0x1280                                       // IRQ, EL2 with SP_EL2
///         ldr w5, [x3, #0xc]    ; mov x23, x21              // GICC_IAR
///         ldr x0, =0x84000008   ; smc #0                    // SYSTEM_OFF
/// 2:      b 2b
/// ```
#[test]
fn moving_cntvoff_el2_past_cval_interrupts_at_once() {
    const PROGRAM: [(u64, u32); 42] = [
        (0x00, 0x5800_0380),
        (0x04, 0xD51C_C000),
        (0x08, 0xD280_0200),
        (0x0C, 0xD51C_1100),
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
        (0x38, 0xD51C_E07F),
        (0x3C, 0xD53B_E044),
        (0x40, 0xD2C0_2005),
        (0x44, 0x8B05_0084),
        (0x48, 0xD51B_E344),
        (0x4C, 0xD280_0024),
        (0x50, 0xD51B_E324),
        (0x54, 0xD503_42FF),
        (0x58, 0xD280_0015),
        (0x5C, 0x5800_0165),
        (0x60, 0xD51C_E065),
        (0x64, 0x9100_06B5),
        (0x68, 0x17FF_FFFF),
        (0x6C, 0x0000_0000),
        (0x70, 0x4000_1000),
        (0x74, 0x0000_0000),
        (0x78, 0x0800_0000),
        (0x7C, 0x0000_0000),
        (0x80, 0x0801_0000),
        (0x84, 0x0000_0000),
        (0x88, 0x0000_0000),
        (0x8C, 0xFFFF_FE00),
        (0x1280, 0xB940_0C65),
        (0x1284, 0xAA15_03F7),
        (0x1288, 0x5800_0080),
        (0x128C, 0xD400_0003),
        (0x1290, 0x1400_0000),
        (0x1298, 0x8400_0008),
    ];
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &PROGRAM);
    let out = m.step(RunBudget::Instructions(1_000_000));
    assert_eq!(out.stop, StopReason::PoweredOff, "the interrupt arrived");
    assert_eq!(m.reg_read(Reg::X5), 27, "INTID 27, PPI 11");
    let spins = m.reg_read(Reg::X23);
    assert!(spins <= 2, "{spins} iterations after the write");
}

/// A hypervisor at EL2 puts virtual interrupt 40 in `GICH_LR0`, pending with
/// `EOI` set, and enters EL1. The guest enables its virtual CPU interface and
/// unmasks IRQs: the virtual IRQ arrives at EL1 (Arm
/// `AArch64_InterruptPending`, `HCR_EL2.IMO` set), `GICV_IAR` reads 40, and
/// the guest's `GICV_EOIR` empties the List register, whose `EOI` raises the
/// maintenance interrupt, INTID 25, which `IMO` routes to EL2 although the
/// guest has IRQs masked (IHI 0048B §5.2.5, Tables 5-5 and 5-6).
///
/// ```text
/// _start: ldr x0, =0x40001000   ; msr vbar_el2, x0
///         mov x0, #(1 << 31)    ; orr x0, x0, #(1 << 4)     // RW, IMO
///         msr hcr_el2, x0
///         ldr x1, =0x08000000   ; mov w2, #1 ; str w2, [x1] // GICD_CTLR
///         mov w2, #(1 << 25)    ; str w2, [x1, #0x100]      // ISENABLER0
///         ldr x3, =0x08010000   ; mov w2, #1 ; str w2, [x3] // GICC_CTLR
///         mov w2, #0xff         ; str w2, [x3, #4]          // GICC_PMR
///         ldr x10, =0x08030000
///         ldr w2, =0x10080028   ; str w2, [x10, #0x100]     // GICH_LR0
///         mov w2, #1            ; str w2, [x10]             // GICH_HCR.En
///         ldr x0, =0x40002000   ; msr vbar_el1, x0
///         mov x0, #5            ; msr spsr_el2, x0          // EL1h
///         adr x0, el1           ; msr elr_el2, x0
///         mov x21, #0           ; eret
/// el1:    ldr x11, =0x08040000  ; mov w2, #1 ; str w2, [x11] // GICV_CTLR
///         mov w2, #0xf8         ; str w2, [x11, #4]         // GICV_PMR
///         msr daifclr, #2
/// 1:      add x21, x21, #1      ; b 1b
///         .org 0x1480                                       // IRQ from EL1
///         ldr w7, [x3, #0xc]                                // GICC_IAR
///         ldr w8, [x10, #0x20]  ; ldr w9, [x10, #0x10]      // EISR0, MISR
///         ldr w12, [x10, #0x100]                            // GICH_LR0
///         ldr x0, =0x84000008   ; smc #0                    // SYSTEM_OFF
/// 2:      b 2b
///         .org 0x2280                                       // IRQ, EL1h
///         ldr w6, [x11, #0xc]   ; mov x22, x21              // GICV_IAR
///         str w6, [x11, #0x10]                              // GICV_EOIR
/// 3:      b 3b
/// ```
#[test]
fn a_virtual_interrupt_reaches_el1_and_its_eoi_reaches_el2() {
    const PROGRAM: [(u64, u32); 61] = [
        (0x00, 0x5800_A500),
        (0x04, 0xD51C_C000),
        (0x08, 0xD2B0_0000),
        (0x0C, 0xB27C_0000),
        (0x10, 0xD51C_1100),
        (0x14, 0x5800_A4A1),
        (0x18, 0x5280_0022),
        (0x1C, 0xB900_0022),
        (0x20, 0x52A0_4002),
        (0x24, 0xB901_0022),
        (0x28, 0x5800_A443),
        (0x2C, 0x5280_0022),
        (0x30, 0xB900_0062),
        (0x34, 0x5280_1FE2),
        (0x38, 0xB900_0462),
        (0x3C, 0x5800_A3EA),
        (0x40, 0x1800_A2E2),
        (0x44, 0xB901_0142),
        (0x48, 0x5280_0022),
        (0x4C, 0xB900_0142),
        (0x50, 0x5800_A380),
        (0x54, 0xD518_C000),
        (0x58, 0xD280_00A0),
        (0x5C, 0xD51C_4000),
        (0x60, 0x1000_0080),
        (0x64, 0xD51C_4020),
        (0x68, 0xD280_0015),
        (0x6C, 0xD69F_03E0),
        (0x70, 0x5800_A2CB),
        (0x74, 0x5280_0022),
        (0x78, 0xB900_0162),
        (0x7C, 0x5280_1F02),
        (0x80, 0xB900_0562),
        (0x84, 0xD503_42FF),
        (0x88, 0x9100_06B5),
        (0x8C, 0x17FF_FFFF),
        (0x1480, 0xB940_0C67),
        (0x1484, 0xB940_2148),
        (0x1488, 0xB940_1149),
        (0x148C, 0xB941_014C),
        (0x1490, 0x5800_0200),
        (0x1494, 0xD400_0003),
        (0x1498, 0x1400_0000),
        (0x149C, 0x1008_0028),
        (0x14A0, 0x4000_1000),
        (0x14A4, 0x0000_0000),
        (0x14A8, 0x0800_0000),
        (0x14AC, 0x0000_0000),
        (0x14B0, 0x0801_0000),
        (0x14B4, 0x0000_0000),
        (0x14B8, 0x0803_0000),
        (0x14BC, 0x0000_0000),
        (0x14C0, 0x4000_2000),
        (0x14C4, 0x0000_0000),
        (0x14C8, 0x0804_0000),
        (0x14CC, 0x0000_0000),
        (0x14D0, 0x8400_0008),
        (0x2280, 0xB940_0D66),
        (0x2284, 0xAA15_03F6),
        (0x2288, 0xB900_1166),
        (0x228C, 0x1400_0000),
    ];
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &PROGRAM);
    let out = m.step(RunBudget::Instructions(1_000_000));
    assert_eq!(out.stop, StopReason::PoweredOff, "the maintenance interrupt reached EL2");
    assert_eq!(m.reg_read(Reg::X6), 40, "GICV_IAR: virtual interrupt 40");
    assert!(m.reg_read(Reg::X22) <= 2, "taken once IRQs were unmasked");
    assert_eq!(m.reg_read(Reg::X7), 25, "GICC_IAR: the maintenance interrupt");
    assert_eq!(m.reg_read(Reg::X8), 1, "GICH_EISR0: List register 0");
    assert_eq!(m.reg_read(Reg::X9), 1, "GICH_MISR.EOI");
    assert_eq!(m.reg_read(Reg::X12), 0x0008_0028, "GICH_LR0: invalid, EOI kept");
    assert_eq!(m.pstate().el, El::El2);
}

/// A List register's pending interrupt is the GIC's virtual IRQ input, which
/// the processor at EL1 under `HCR_EL2.IMO` sees as `HCR_EL2.VI` would show
/// it (Arm `AArch64_InterruptPending`): `ISR_EL1.I` reads 1 with IRQs
/// masked, and `WFI` does not wait (`IsIRQPending()` counts a masked
/// interrupt). A snapshot taken then, IRQs still masked, restores into a
/// machine that takes the virtual IRQ once the guest unmasks it.
///
/// ```text
/// _start: mov x0, #(1 << 31)    ; orr x0, x0, #(1 << 4)     // RW, IMO
///         msr hcr_el2, x0
///         ldr x10, =0x08030000
///         ldr w2, =0x10000028   ; str w2, [x10, #0x100]     // GICH_LR0
///         mov w2, #1            ; str w2, [x10]             // GICH_HCR.En
///         ldr x0, =0x40002000   ; msr vbar_el1, x0
///         mov x0, #0x3c5        ; msr spsr_el2, x0          // EL1h, DAIF masked
///         adr x0, el1           ; msr elr_el2, x0
///         eret
/// el1:    ldr x11, =0x08040000  ; mov w2, #1 ; str w2, [x11] // GICV_CTLR
///         mov w2, #0xf8         ; str w2, [x11, #4]         // GICV_PMR
///         mrs x7, isr_el1
///         wfi
///         mov x8, #1
/// 1:      cbz x9, 1b                                        // the host's go
///         msr daifclr, #2
/// 2:      b 2b
///         .org 0x2280                                       // IRQ, EL1h
///         ldr w6, [x11, #0xc]                               // GICV_IAR
/// 3:      b 3b
/// ```
#[test]
fn a_virtual_irq_input_wakes_wfi_shows_in_isr_and_survives_a_snapshot() {
    const PROGRAM: [(u64, u32); 35] = [
        (0x00, 0xD2B0_0000),
        (0x04, 0xB27C_0000),
        (0x08, 0xD51C_1100),
        (0x0C, 0x5800_032A),
        (0x10, 0x1800_02C2),
        (0x14, 0xB901_0142),
        (0x18, 0x5280_0022),
        (0x1C, 0xB900_0142),
        (0x20, 0x5800_02C0),
        (0x24, 0xD518_C000),
        (0x28, 0xD280_78A0),
        (0x2C, 0xD51C_4000),
        (0x30, 0x1000_0060),
        (0x34, 0xD51C_4020),
        (0x38, 0xD69F_03E0),
        (0x3C, 0x5800_022B),
        (0x40, 0x5280_0022),
        (0x44, 0xB900_0162),
        (0x48, 0x5280_1F02),
        (0x4C, 0xB900_0562),
        (0x50, 0xD538_C107),
        (0x54, 0xD503_207F),
        (0x58, 0xD280_0028),
        (0x5C, 0xB400_0009),
        (0x60, 0xD503_42FF),
        (0x64, 0x1400_0000),
        (0x68, 0x1000_0028),
        (0x6C, 0x0000_0000),
        (0x70, 0x0803_0000),
        (0x74, 0x0000_0000),
        (0x78, 0x4000_2000),
        (0x7C, 0x0000_0000),
        (0x80, 0x0804_0000),
        (0x2280, 0xB940_0D66),
        (0x2284, 0x1400_0000),
    ];
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &PROGRAM);
    m.reg_write(Reg::X9, 0);
    let out = m.step(RunBudget::Instructions(200));
    assert_eq!(out.stop, StopReason::BudgetExhausted, "spinning, not waiting");
    assert_eq!(m.reg_read(Reg::X7), 1 << 7, "ISR_EL1.I: the virtual IRQ");
    assert_eq!(m.reg_read(Reg::X8), 1, "WFI did not wait");
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 0x5C, "masked, waiting for the go");
    let mut image = Vec::new();
    m.save_snapshot(&mut image).expect("save");

    let mut ram = vec![0u8; RAM_LEN];
    let mut restored = bare(&mut ram);
    restored
        .restore_snapshot(&mut image.as_slice())
        .expect("restores");
    restored.reg_write(Reg::X9, 1);
    let out = restored.step(RunBudget::Instructions(20));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(restored.reg_read(Reg::X6), 40, "GICV_IAR: the restored List register's");
    assert_eq!(restored.pstate().el, El::El1);
}

/// A reset of a machine with EL2 resets the GIC's virtual interface and
/// keeps it: `GICH_HCR` reads 0 again, `GICH_VTR` still the GIC-400's,
/// and `GICV_IIDR` still the identity's.
///
/// ```text
///         ldr w2, [x1]          // x1: the register
/// 1:      b 1b
/// ```
#[test]
fn a_reset_keeps_the_virtual_interface() {
    let read = |m: &mut VirtMachine<LentStorage<'_>, WithEl2>, addr: u64| {
        load(m, &[(0x00, 0xB940_0022), (0x04, 0x1400_0000)]);
        m.reg_write(Reg::Pc, RAM_BASE);
        m.reg_write(Reg::X1, addr);
        let out = m.step(RunBudget::Instructions(1));
        assert_eq!(out.stop, StopReason::BudgetExhausted);
        m.reg_read(Reg::X2)
    };
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    let (gich_hcr, gich_vtr, gicv_iidr) = (0x0803_0000, 0x0803_0004, 0x0804_00FC);
    // `str w2, [x1]` with GICH_HCR.En.
    load(&mut m, &[(0x00, 0xB900_0022), (0x04, 0x1400_0000)]);
    m.reg_write(Reg::X1, gich_hcr);
    m.reg_write(Reg::X2, 1);
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(read(&mut m, gich_hcr), 1);
    let iidr = read(&mut m, gicv_iidr);
    m.power().reset();
    assert_eq!(m.pstate().el, El::El2);
    assert_eq!(read(&mut m, gich_hcr), 0, "reset");
    assert_eq!(read(&mut m, gich_vtr), 0x9000_0003, "still there");
    assert_eq!(read(&mut m, gicv_iidr), iidr, "still the identity's");
}

/// A kernel booted directly with EL2, a command line and an initrd is
/// entered at EL2, and its device tree says all of it: the identity's
/// names, `/chosen` with the command line and the initrd, the `/psci` node
/// for `SMC`, and the GIC node's virtual interface.
#[test]
fn a_kernel_booted_with_el2_finds_all_of_it_in_its_device_tree() {
    const MIB: u64 = 1 << 20;
    let (kernel, initrd) = kernel_and_initrd();
    let image = LinuxImage::parse(&kernel).expect("an Image");
    // Room for the kernel at 2 MiB, the initrd above it and the tree.
    let ram_len = 6 * MIB;
    let mut ram = vec![0u8; ram_len as usize];
    let m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(ram_len).expect("a valid size"))
        .linux(image)
        .initrd(&initrd)
        .command_line("console=ttyAMA0 quiet")
        .with_el2()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("builds");
    assert_eq!(m.pstate().el, El::El2);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 2 * MIB);
    let tree = m.device_tree();
    let prop = |node: &str, name: &str| {
        fdt::property(tree, node, name)
            .expect("well formed")
            .map(<[u8]>::to_vec)
    };
    assert_eq!(
        prop("chosen", "bootargs"),
        Some(b"console=ttyAMA0 quiet\0".to_vec())
    );
    let initrd_start = prop("chosen", "linux,initrd-start").expect("an initrd");
    let initrd_end = prop("chosen", "linux,initrd-end").expect("an initrd");
    let start = u64::from_be_bytes(initrd_start.try_into().expect("two cells"));
    let end = u64::from_be_bytes(initrd_end.try_into().expect("two cells"));
    assert_eq!(end - start, 3000);
    assert_eq!(prop("psci", "method"), Some(b"smc\0".to_vec()));
    assert_eq!(
        prop("interrupt-controller@8000000", "reg"),
        Some(be_cells(&GIC_REG_WITH_EL2))
    );
    assert_eq!(
        prop("interrupt-controller@8000000", "interrupts"),
        Some(maintenance_interrupt())
    );
    assert_eq!(
        prop("", "model"),
        Some(b"Rusty Box virt\0".to_vec()),
        "the identity's"
    );
    // No entropy a guest would credit, the same bytes every boot.
    assert_eq!(prop("chosen", "rng-seed"), None);
    assert_eq!(prop("chosen", "kaslr-seed"), None);
}

/// A kernel and its initrd: a 64-byte arm64 `Image` header (text_offset 0)
/// with `b .` as the kernel, and 3000 bytes of initrd.
fn kernel_and_initrd() -> (Vec<u8>, Vec<u8>) {
    let mut kernel = vec![0u8; 68];
    kernel[16..24].copy_from_slice(&0x18_0000u64.to_le_bytes());
    kernel[56..60].copy_from_slice(b"ARM\x64");
    kernel[0..4].copy_from_slice(&SPIN.to_le_bytes());
    (kernel, vec![0x5Au8; 3000])
}

/// A host's tree booting a kernel with a command line and an initrd gets
/// every edit a host's tree is given: the identity's names, the memory
/// node, `/chosen` with the command line and both initrd properties, and
/// the board's `/psci` node. What else its `/chosen` holds is the host's.
#[test]
fn a_host_tree_booting_a_kernel_gets_every_edit() {
    const MIB: u64 = 1 << 20;
    const HOST_TREE: &[u8] = include_bytes!("../assets/qemu-virt-a57-256m.dtb");
    let (kernel, initrd) = kernel_and_initrd();
    let image = LinuxImage::parse(&kernel).expect("an Image");
    let ram_len = 6 * MIB;
    let mut ram = vec![0u8; ram_len as usize];
    // QEMU's tree names the devices' interrupts as QEMU's `virt` wires them,
    // which the board takes as its map.
    let qemu_wired = VirtConfig {
        interrupts: qemu_virt::qemu_virt_interrupts(),
        ..VirtConfig::DEFAULT
    };
    let m = MachineBuilder::new(qemu_wired)
        .ram(RamSize::bytes(ram_len).expect("a valid size"))
        .device_tree(HOST_TREE)
        .linux(image)
        .initrd(&initrd)
        .command_line("console=ttyAMA0 quiet")
        .with_el2()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("builds");
    let tree = m.device_tree();
    let prop = |node: &str, name: &str| {
        fdt::property(tree, node, name)
            .expect("well formed")
            .map(<[u8]>::to_vec)
    };
    assert_eq!(prop("", "model"), Some(b"Rusty Box virt\0".to_vec()));
    assert_eq!(prop("", "compatible"), Some(b"rusty-box,virt\0".to_vec()));
    assert_eq!(
        prop("memory@40000000", "reg"),
        Some(be_cells(&[RAM_BASE, ram_len]))
    );
    assert_eq!(
        prop("chosen", "bootargs"),
        Some(b"console=ttyAMA0 quiet\0".to_vec())
    );
    let start = prop("chosen", "linux,initrd-start").expect("an initrd");
    let end = prop("chosen", "linux,initrd-end").expect("an initrd");
    let span = |b: Vec<u8>| u64::from_be_bytes(b.try_into().expect("two cells"));
    assert_eq!(span(end) - span(start), 3000);
    assert_eq!(
        prop("psci", "compatible"),
        Some(b"arm,psci-1.0\0arm,psci-0.2\0".to_vec())
    );
    assert_eq!(prop("psci", "method"), Some(b"smc\0".to_vec()));
    // QEMU's dump carries its own run's seeds, which the host's tree keeps
    // as the host gave them.
    let dumped = |name| fdt::property(HOST_TREE, "chosen", name).expect("well formed");
    assert_eq!(prop("chosen", "rng-seed").as_deref(), dumped("rng-seed"));
}

/// Without virtualization the virtual interface's windows are unassigned:
/// reading `GICH_VTR` and `GICV_IIDR` is a synchronous external abort, as
/// anywhere the board decodes nothing (QEMU `virt` maps neither frame
/// without `virtualization=on`).
///
/// ```text
/// _start: ldr x0, =0x40001000   ; msr vbar_el1, x0
///         mov x20, #0           ; mov x2, #0x5a
///         ldr x1, =0x08030004   ; ldr w2, [x1]              // GICH_VTR
///         ldr x1, =0x080400fc   ; ldr w2, [x1]              // GICV_IIDR
/// 1:      b 1b
///         .org 0x1200                                       // sync, EL1h
///         mrs x3, esr_el1       ; mrs x5, far_el1
///         add x20, x20, #1
///         mrs x6, elr_el1       ; add x6, x6, #4 ; msr elr_el1, x6
///         eret
/// ```
#[test]
fn without_virtualization_the_virtual_interface_is_not_there() {
    const PROGRAM: [(u64, u32); 22] = [
        (0x00, 0x5800_0140),
        (0x04, 0xD518_C000),
        (0x08, 0xD280_0014),
        (0x0C, 0xD280_0B42),
        (0x10, 0x5800_0101),
        (0x14, 0xB940_0022),
        (0x18, 0x5800_0101),
        (0x1C, 0xB940_0022),
        (0x20, 0x1400_0000),
        (0x24, 0x0000_0000),
        (0x28, 0x4000_1000),
        (0x2C, 0x0000_0000),
        (0x30, 0x0803_0004),
        (0x34, 0x0000_0000),
        (0x38, 0x0804_00FC),
        (0x1200, 0xD538_5203),
        (0x1204, 0xD538_6005),
        (0x1208, 0x9100_0694),
        (0x120C, 0xD538_4026),
        (0x1210, 0x9100_10C6),
        (0x1214, 0xD518_4026),
        (0x1218, 0xD69F_03E0),
    ];
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare_without_el2(&mut ram);
    load(&mut m, &PROGRAM);
    let out = m.step(RunBudget::Instructions(1_000));
    assert_eq!(out.stop, StopReason::BudgetExhausted, "spinning, not stopped");
    assert_eq!(m.reg_read(Reg::X20), 2, "both reads aborted");
    let esr = m.reg_read(Reg::X3);
    assert_eq!(esr >> 26, 0x25, "a data abort from the current level: {esr:#x}");
    assert_eq!(esr & 0x3F, 0x10, "a synchronous external abort: {esr:#x}");
    assert_eq!(m.reg_read(Reg::X5), 0x0804_00FC, "FAR_EL1: GICV_IIDR");
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 0x20);
}

/// With virtualization each frame of the virtual interface decodes its
/// first 8 KiB (GIC-400 DDI 0471B Table 3-1: `GICH` at 0x4000 and its
/// processor-specific alias at 0x5000, `GICV` at 0x6000 to 0x7FFF; QEMU
/// `virt` `VIRT_GIC_HYP` and `VIRT_GIC_VCPU`): the reserved last word of
/// each reads 0, `GICV_DIR` at the start of the `GICV` frame's second page
/// deactivates (with `GICV_CTLR.EOImode`, an ID no List register holds
/// counts in `GICH_HCR.EOICount`, §5.5.14), and a load just past either
/// frame is a synchronous external abort, taken to EL2.
///
/// ```text
/// _start: ldr x0, =0x40001000   ; msr vbar_el2, x0
///         mov x20, #0
///         ldr x1, =0x08031ffc   ; mov x2, #0x5a ; ldr w2, [x1] ; mov x3, x2
///         ldr x1, =0x08041ffc   ; mov x2, #0x5a ; ldr w2, [x1] ; mov x9, x2
///         ldr x1, =0x08040000   ; mov x2, #0x200 ; str w2, [x1]  // GICV_CTLR.EOImode
///         ldr x1, =0x08041000   ; mov x2, #5 ; str w2, [x1]      // GICV_DIR
///         ldr x1, =0x08030000   ; ldr w4, [x1]                   // GICH_HCR
///         ldr x1, =0x08032000   ; ldr w2, [x1] ; mov x6, x5
///         ldr x1, =0x08042000   ; ldr w2, [x1]
/// 1:      b 1b
///         .org 0x1200                                            // sync, EL2h
///         mrs x7, esr_el2       ; mrs x5, far_el2
///         add x20, x20, #1
///         mrs x8, elr_el2       ; add x8, x8, #4 ; msr elr_el2, x8
///         eret
/// ```
#[test]
fn each_virtual_interface_frame_decodes_8_kib() {
    const PROGRAM: [(u64, u32); 40] = [
        (0x00, 0x5800_0340),
        (0x04, 0xD51C_C000),
        (0x08, 0xD280_0014),
        (0x0C, 0x5800_0321),
        (0x10, 0xD280_0B42),
        (0x14, 0xB940_0022),
        (0x18, 0xAA02_03E3),
        (0x1C, 0x5800_02E1),
        (0x20, 0xD280_0B42),
        (0x24, 0xB940_0022),
        (0x28, 0xAA02_03E9),
        (0x2C, 0x5800_02A1),
        (0x30, 0xD280_4002),
        (0x34, 0xB900_0022),
        (0x38, 0x5800_0281),
        (0x3C, 0xD280_00A2),
        (0x40, 0xB900_0022),
        (0x44, 0x5800_0261),
        (0x48, 0xB940_0024),
        (0x4C, 0x5800_0261),
        (0x50, 0xB940_0022),
        (0x54, 0xAA05_03E6),
        (0x58, 0x5800_0241),
        (0x5C, 0xB940_0022),
        (0x60, 0x1400_0000),
        (0x68, 0x4000_1000),
        (0x70, 0x0803_1FFC),
        (0x78, 0x0804_1FFC),
        (0x80, 0x0804_0000),
        (0x88, 0x0804_1000),
        (0x90, 0x0803_0000),
        (0x98, 0x0803_2000),
        (0xA0, 0x0804_2000),
        (0x1200, 0xD53C_5207),
        (0x1204, 0xD53C_6005),
        (0x1208, 0x9100_0694),
        (0x120C, 0xD53C_4028),
        (0x1210, 0x9100_1108),
        (0x1214, 0xD51C_4028),
        (0x1218, 0xD69F_03E0),
    ];
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &PROGRAM);
    let out = m.step(RunBudget::Instructions(1_000));
    assert_eq!(out.stop, StopReason::BudgetExhausted, "spinning, not stopped");
    assert_eq!(m.reg_read(Reg::X3), 0, "GICH's last word: reserved");
    assert_eq!(m.reg_read(Reg::X9), 0, "GICV's last word: reserved");
    assert_eq!(m.reg_read(Reg::X4), 1 << 27, "GICH_HCR.EOICount: GICV_DIR counted");
    assert_eq!(m.reg_read(Reg::X20), 2, "both loads past the frames aborted");
    assert_eq!(m.reg_read(Reg::X6), 0x0803_2000, "FAR_EL2: past GICH");
    assert_eq!(m.reg_read(Reg::X5), 0x0804_2000, "FAR_EL2: past GICV");
    let esr = m.reg_read(Reg::X7);
    assert_eq!(esr >> 26, 0x25, "a data abort from the current level: {esr:#x}");
    assert_eq!(esr & 0x3F, 0x10, "a synchronous external abort: {esr:#x}");
    assert_eq!(m.pstate().el, El::El2);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 0x60);
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
    m.sysreg_write(SysReg::VBAR_EL2, VBAR)
        .expect("EL2 has VBAR_EL2");
    m.sysreg_write(SysReg::HCR_EL2, 1 << 4)
        .expect("and HCR_EL2");
    m.sysreg_write(SysReg::TPIDR_EL2, 0x1234)
        .expect("and TPIDR_EL2");
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
        let mut out: Vec<u64> = registers
            .iter()
            .map(|&r| m.sysreg_read(r).expect("EL2 register"))
            .collect();
        out.extend([m.sp_el2(), m.reg_read(Reg::Pc)]);
        out
    };
    let saved = values(&m);
    assert_eq!(saved[3], (0x16 << 26) | (1 << 25) | 1, "ESR_EL2: HVC #1");
    assert_eq!(saved[4], RAM_BASE + 4, "ELR_EL2: the next instruction");

    let mut again = vec![0u8; RAM_LEN];
    let mut same = bare(&mut again);
    same.restore_snapshot(&mut image.as_slice())
        .expect("restores");
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
        Err(SnapError::Invalid(
            "snapshot is of a machine built differently"
        ))
    );
    assert_eq!(
        (without.cpu_snapshot(), without.ticks()),
        before,
        "nothing changed"
    );
    assert_eq!(without.pstate().el, El::El1);
}

/// A processor model that cannot have EL2 yet (`CpuModel::el2_available`:
/// the Armv9.2 model, of which Arm requires features with EL2 this core
/// lacks) is refused it, by a machine's build before the lent RAM is
/// written and by `Cpu::new`; the Cortex-A57 builds with it and starts at
/// EL2, and every model builds without it.
#[test]
fn a_model_that_cannot_have_el2_is_refused_it() {
    assert!(CpuModel::CortexA57.el2_available());
    assert!(!CpuModel::Armv9p2.el2_available());
    let config = CpuConfig {
        ips: ClockHz::BOCHS_DEFAULT,
        reset_pc: RAM_BASE,
    };
    for model in CpuModel::ALL {
        assert!(Cpu::<NoEl2>::new(model, config).is_ok(), "{model:?}");
        match Cpu::<WithEl2>::new(model, config) {
            Ok(cpu) => {
                assert!(model.el2_available(), "{model:?}");
                assert_eq!(cpu.el(), El::El2, "{model:?}");
            }
            Err(e) => assert_eq!(e, El2Unavailable { model }),
        }
    }
    for model in CpuModel::ALL {
        let mut ram = vec![0x5A_u8; RAM_LEN];
        let built = MachineBuilder::new(VirtConfig {
            cpu: model,
            ..VirtConfig::DEFAULT
        })
        .ram(ram_size())
        .bare()
        .with_el2()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        });
        match built {
            Ok(m) => {
                assert!(model.el2_available(), "{model:?}");
                assert_eq!(m.pstate().el, El::El2, "{model:?}");
            }
            Err(e) => {
                assert!(!model.el2_available(), "{model:?}");
                assert_eq!(e, BuildError::El2Unavailable(El2Unavailable { model }));
                assert!(
                    ram.iter().all(|&b| b == 0x5A),
                    "{model:?}: the loan is untouched"
                );
            }
        }
    }
}
