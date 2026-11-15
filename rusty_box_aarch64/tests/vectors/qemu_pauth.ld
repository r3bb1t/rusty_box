/* The layout gen_qemu_pauth.py and the replay (tests/unit/pauth.rs) agree
 * on: the slot whose fifth word is the one under test, the landing pad an
 * authenticated branch goes to, and the data region an authenticated load
 * reads, each at a fixed address in the RAM of QEMU's `virt`; the cases
 * after them. */
ENTRY(_start)
SECTIONS
{
    . = 0x40100000;
    .text : { *(.text) }
    .rodata : { *(.rodata*) }
    .data : { *(.data*) }
    .bss : { *(.bss*) }
    . = 0x40180000;
    .vectors : { *(.vectors) }
    . = 0x40200000;
    .slot : { *(.slot) }
    . = 0x40300000;
    .land : { *(.land) }
    . = 0x40400000;
    .pdata : { *(.pdata) }
    . = 0x40500000;
    .cases : { *(.cases) }
    /DISCARD/ : { *(.note*) *(.comment) }
}
