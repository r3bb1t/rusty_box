# Boot-sector probe: does a VEX-encoded shift-by-immediate accept a memory
# operand?
#
# The SDM lists the VEX forms of the shift-by-immediate groups (66 0F 71 /2 /4
# /6, 72 /2 /4 /6, 73 /2 /3 /6 /7) with a register operand only, and Bochs's
# own legacy SSE table marks the same groups ATTR_MODC0. Bochs's VEX table
# (fetchdecode_opmap_avx.cc, BxOpcodeGroup_VEX_0F71/72/73) carries no
# ATTR_MODC0, so Bochs decodes and executes the memory form.
#
# The probe enters 32-bit protected mode, enables SSE and AVX state
# (CR4.OSFXSR|OSXMMEXCPT|OSXSAVE, XCR0 = x87|SSE|AVX), and runs each test. A
# test that retires prints "+", a fault prints "#" and a letter: U = #UD,
# N = #NM, G = #GP, P = #PF, ? = any other vector. Every memory operand is
# [0x6000], ordinary RAM.
#
#   R     VEX.128 VPSRLW xmm1, xmm2, 4      register form (control: AVX works)
#   S     legacy PSRLW xmm, [mem], 4         memory form (control: SSE's #UD)
#   71    VEX.128 VPSRLW xmm1, [mem], 4      66 0F 71 /2
#   72    VEX.128 VPSRLD xmm1, [mem], 4      66 0F 72 /2
#   73/2  VEX.128 VPSRLQ xmm1, [mem], 4      66 0F 73 /2
#   73/3  VEX.128 VPSRLDQ xmm1, [mem], 4     66 0F 73 /3
#   YR    VEX.256 VPSRLW ymm1, ymm2, 4      register form (control: AVX2)
#   Y71   VEX.256 VPSRLW ymm1, [mem], 4      66 0F 71 /2
#
# Output on port 0xE9 (Bochs port_e9_hack, QEMU -debugcon), then "Shutdown"
# to port 0x8900, which ends a Bochs run, and HLT. Measured 2026-10-09:
#   Intel Core i5-12450H, real hardware
#     R:+ S:#U 71:#U 72:#U 73/2:#U 73/3:#U YR:+ Y71:#U
#   Bochs master f87c5e226, cpu: model=corei7_skylake_x
#     R:+ S:#U 71:+ 72:+ 73/2:+ 73/3:+ YR:+ Y71:+
#   Bochs master f87c5e226 + bochs-vex-shift-imm-memory-fix.patch
#     R:+ S:#U 71:#U 72:#U 73/2:#U 73/3:#U YR:+ Y71:#U
#
# Build (GNU binutils):
#   as -o probe.o bochs-vex-shift-imm-memory-probe.S
#   objcopy -O binary -j .text probe.o probe.img
# probe.img is one 512-byte boot sector; boot it as a floppy image.

        .code16
        .intel_syntax noprefix
        .text

        .set    BASE, 0x7C00
        .set    CODE32, 0x08
        .set    DATA32, 0x10
        .set    STACK, 0x7000
        .set    IDT, 0x5000
        .set    MEM, 0x6000
        .set    RESUME, 0x5800

_start:
        cli
        xor     ax, ax
        mov     ds, ax
        lgdt    [BASE + (gdtr - _start)]
        mov     eax, cr0
        or      al, 1
        mov     cr0, eax
        .byte   0x66, 0xEA                              # jmp CODE32:pm32
        .long   BASE + (pm32 - _start)
        .word   CODE32

        .code32
pm32:   mov     ax, DATA32
        mov     ds, ax
        mov     es, ax
        mov     ss, ax
        mov     esp, STACK

        # Every vector to `other`, then #UD, #NM, #GP and #PF to their own.
        mov     edi, IDT
        mov     ecx, 32
1:      mov     eax, (CODE32 << 16) | (BASE + (other - _start))
        stosd
        mov     eax, 0x00008E00
        stosd
        loop    1b
        mov     word ptr [IDT + 6 * 8], BASE + (ud - _start)
        mov     word ptr [IDT + 7 * 8], BASE + (nm - _start)
        mov     word ptr [IDT + 13 * 8], BASE + (gp - _start)
        mov     word ptr [IDT + 14 * 8], BASE + (pf - _start)
        lidt    [BASE + (idtr - _start)]

        # CR0: EM = 0, MP = 1, TS = 0. CR4: OSFXSR, OSXMMEXCPT, OSXSAVE.
        mov     eax, cr0
        and     eax, 0xFFFFFFF3
        or      eax, 2
        mov     cr0, eax
        mov     eax, cr4
        or      eax, 0x00040600
        mov     cr4, eax
        xor     ecx, ecx
        xor     edx, edx
        mov     eax, 7
        xsetbv

        # Each test: `call begin`, its name, a 9-byte instruction slot, then
        # `call pass`. `begin` prints the name, arms the fault handlers to
        # resume after `call pass`, and jumps into the slot.
        call    begin
        .asciz  "R"
        .byte   0xC5, 0xF1, 0x71, 0xD2, 0x04, 0x90, 0x90, 0x90, 0x90
        call    pass
        call    begin
        .asciz  "S"
        .byte   0x66, 0x0F, 0x71, 0x15
        .long   MEM
        .byte   0x04
        call    pass
        call    begin
        .asciz  "71"
        .byte   0xC5, 0xF1, 0x71, 0x15
        .long   MEM
        .byte   0x04
        call    pass
        call    begin
        .asciz  "72"
        .byte   0xC5, 0xF1, 0x72, 0x15
        .long   MEM
        .byte   0x04
        call    pass
        call    begin
        .asciz  "73/2"
        .byte   0xC5, 0xF1, 0x73, 0x15
        .long   MEM
        .byte   0x04
        call    pass
        call    begin
        .asciz  "73/3"
        .byte   0xC5, 0xF1, 0x73, 0x1D
        .long   MEM
        .byte   0x04
        call    pass
        call    begin
        .asciz  "YR"
        .byte   0xC5, 0xF5, 0x71, 0xD2, 0x04, 0x90, 0x90, 0x90, 0x90
        call    pass
        call    begin
        .asciz  "Y71"
        .byte   0xC5, 0xF5, 0x71, 0x15
        .long   MEM
        .byte   0x04
        call    pass

        mov     al, 0x0A
        out     0xE9, al
        mov     dx, 0x8900
        mov     esi, BASE + (shutdown - _start)
        mov     ecx, 8
        rep outsb
2:      hlt
        jmp     2b

# Print the name that follows the caller, then run the slot after it.
begin:  pop     esi
3:      lodsb
        test    al, al
        jz      4f
        out     0xE9, al
        jmp     3b
4:      mov     al, ':'
        out     0xE9, al
        lea     eax, [esi + 14]
        mov     dword ptr [RESUME], eax
        jmp     esi

pass:   mov     al, '+'
        out     0xE9, al
        mov     al, ' '
        out     0xE9, al
        ret

ud:     mov     bl, 'U'
        jmp     fault
nm:     mov     bl, 'N'
        jmp     fault
gp:     mov     bl, 'G'
        jmp     fault
pf:     mov     bl, 'P'
        jmp     fault
other:  mov     bl, '?'
fault:  mov     al, '#'
        out     0xE9, al
        mov     al, bl
        out     0xE9, al
        mov     al, ' '
        out     0xE9, al
        mov     esp, STACK
        jmp     dword ptr [RESUME]

shutdown:
        .ascii  "Shutdown"

        .p2align 3
gdt:    .quad   0
        .quad   0x00CF9A000000FFFF                      # 0x08: code, 4 GiB, 32-bit
        .quad   0x00CF92000000FFFF                      # 0x10: data, 4 GiB
gdtr:   .word   gdtr - gdt - 1
        .long   BASE + (gdt - _start)
idtr:   .word   32 * 8 - 1
        .long   IDT

        .org    510
        .word   0xAA55
