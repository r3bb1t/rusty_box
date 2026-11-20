OUTPUT_FORMAT(elf32-i386)
ENTRY(_start)
PHDRS { image PT_LOAD FILEHDR PHDRS FLAGS(5); }
SECTIONS {
    . = 0x10900000 + SIZEOF_HEADERS;
    .text : { *(.text) } :image
    /DISCARD/ : { *(.note*) *(.comment*) }
}
