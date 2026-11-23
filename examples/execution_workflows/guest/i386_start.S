.section .text.startup, "ax", @progbits
.global _start
.type _start, @function
_start:
    xor %ebp, %ebp
    mov %esp, %eax
    and $-16, %esp
    sub $12, %esp
    push %eax
    call guest_main
    mov %eax, %ebx
    mov $1, %eax                  # Linux i386 exit
    int $0x80
.size _start, .-_start
.section .note.GNU-stack, "", @progbits
