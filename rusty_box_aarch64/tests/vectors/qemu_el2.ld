/* The layout gen_qemu_el2.py and the replay (tests/unit/el2_vectors.rs)
 * agree on: the runner, EL2's and EL1's vector tables, the slot the word
 * under test and its sentinel run from, a data region an address operand
 * points into, and the cases, each at a fixed address in the RAM of QEMU's
 * `virt`. */
ENTRY(_start)
SECTIONS
{
    . = 0x40100000;
    .text : { *(.text) }
    .rodata : { *(.rodata*) }
    .data : { *(.data*) }
    .bss : { *(.bss*) }
    . = 0x40180000;
    .el2_vectors : { *(.el2_vectors) }
    . = 0x40181000;
    .el1_vectors : { *(.el1_vectors) }
    . = 0x40200000;
    .slot : { *(.slot) }
    . = 0x40400000;
    .xdata : { *(.xdata) }
    . = 0x40500000;
    .cases : { *(.cases) }
    /DISCARD/ : { *(.note*) *(.comment) }
}
