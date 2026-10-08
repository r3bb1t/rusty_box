# Boot-sector probe: how do RET imm16 and RETF imm16 release parameters from a
# 32-bit stack in real mode?
#
# The processor is put in "big real mode": SS is loaded in protected mode from
# a descriptor with B = 1, and real mode is re-entered without reloading SS, so
# the stack address size stays 32 bits. Each probe returns with imm16 = 0x8000
# (16-bit operand size) and prints ESP afterwards, in hex. There are three
# candidate behaviours, and two starting points are needed to tell them apart,
# because an add of 0x8000 either carries out of SP or it does not:
#
#   RETF 0x8000 (CA 00 80), pops IP and CS (4 bytes):
#     start ESP     ESP + zext(imm16)   ESP + sext(imm16)   SP += imm16 only
#     0x00020000    0x00028004          0x00018004          0x00028004
#     0x00029000    0x00031004          0x00021004          0x00021004
#   RET 0x8000 (C2 00 80), pops IP (2 bytes):
#     0x00020000    0x00028002          0x00018002          0x00028002
#     0x00029000    0x00031002          0x00021002          0x00021002
#
# Output, on port 0xE9 (Bochs port_e9_hack, QEMU -debugcon) and in VGA text
# memory, for example:
#   RETF 00028004 00031004 RET 00028002 00031002
# The probe then writes "Shutdown" to port 0x8900, which ends a Bochs run,
# and halts.
#
# Build (GNU binutils; no linker needed — every address is BASE + an offset
# the assembler resolves itself):
#   as -o probe.o bochs-retf16-imm16-probe.S
#   objcopy -O binary -j .text probe.o probe.img
# probe.img is one 512-byte boot sector; boot it as a floppy image.

        .code16
        .intel_syntax noprefix
        .text

        .set    BASE, 0x7C00
        .set    GDT_CODE16, 0x08
        .set    GDT_DATA32, 0x10

_start:
        cli
        xor     ax, ax
        mov     ds, ax
        lgdt    [BASE + (gdtr - _start)]

        # SS from the B = 1 descriptor, then back to real mode with it cached.
        mov     eax, cr0
        or      al, 1
        mov     cr0, eax
        .byte   0xEA                                    # jmp GDT_CODE16:pm
        .word   BASE + (pm - _start), GDT_CODE16
pm:     mov     ax, GDT_DATA32
        mov     ss, ax
        mov     eax, cr0
        and     al, 0xFE
        mov     cr0, eax
        .byte   0xEA                                    # jmp 0000:rm
        .word   BASE + (rm - _start), 0x0000
rm:     mov     esp, BASE                               # the stack is 32-bit now
        xor     di, di                                  # VGA text cursor

        mov     si, BASE + (label_retf - _start)
        call    puts

        # RETF 0x8000 from ESP = 0x20000; frame = IP, CS.
        mov     esp, 0x00020004
        .byte   0x6A, 0x00                              # push word 0 (CS)
        .byte   0x68                                    # push word retf_a (IP)
        .word   BASE + (retf_a - _start)
        .byte   0xCA                                    # retf 0x8000
        .word   0x8000
retf_a: mov     edx, esp
        mov     esp, BASE
        call    puthex32

        # RETF 0x8000 from ESP = 0x29000.
        mov     esp, 0x00029004
        .byte   0x6A, 0x00
        .byte   0x68
        .word   BASE + (retf_b - _start)
        .byte   0xCA
        .word   0x8000
retf_b: mov     edx, esp
        mov     esp, BASE
        call    puthex32

        mov     si, BASE + (label_ret - _start)
        call    puts

        # RET 0x8000 from ESP = 0x20000; frame = IP.
        mov     esp, 0x00020002
        .byte   0x68                                    # push word ret_a (IP)
        .word   BASE + (ret_a - _start)
        .byte   0xC2                                    # ret 0x8000
        .word   0x8000
ret_a:  mov     edx, esp
        mov     esp, BASE
        call    puthex32

        # RET 0x8000 from ESP = 0x29000.
        mov     esp, 0x00029002
        .byte   0x68
        .word   BASE + (ret_b - _start)
        .byte   0xC2
        .word   0x8000
ret_b:  mov     edx, esp
        mov     esp, BASE
        call    puthex32

        mov     al, 0x0A
        out     0xE9, al
        mov     si, BASE + (shutdown - _start)
        mov     dx, 0x8900
1:      lodsb
        test    al, al
        jz      2f
        out     dx, al
        jmp     1b
2:      hlt
        jmp     2b

# EDX as eight hex digits, then a space.
puthex32:
        mov     cx, 8
1:      rol     edx, 4
        mov     al, dl
        and     al, 0x0F
        add     al, '0'
        cmp     al, '9'
        jbe     2f
        add     al, 'A' - '0' - 10
2:      call    putc
        loop    1b
        mov     al, ' '
        jmp     putc

# DS:SI = NUL-terminated string.
puts:   lodsb
        test    al, al
        jz      1f
        call    putc
        jmp     puts
1:      ret

# AL -> port 0xE9 and the next VGA text cell at B800:DI.
putc:   out     0xE9, al
        push    es
        push    ax
        mov     ax, 0xB800
        mov     es, ax
        pop     ax
        mov     ah, 0x0F
        mov     word ptr es:[di], ax
        add     di, 2
        pop     es
        ret

label_retf:     .asciz  "RETF "
label_ret:      .asciz  "RET "
shutdown:       .asciz  "Shutdown"

        .p2align 3
gdt:    .quad   0
        .quad   0x00009A000000FFFF      # 0x08: 16-bit code, base 0, limit 64 KiB
        .quad   0x00CF92000000FFFF      # 0x10: data, B = 1, base 0, limit 4 GiB
gdt_end:
gdtr:   .word   gdt_end - gdt - 1
        .long   BASE + (gdt - _start)

        .org    510
        .word   0xAA55
