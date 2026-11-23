.section .text.start,"ax",@progbits
.global _start
.type _start,@function
_start:
    xorl %ebp, %ebp
    movl %esp, %eax
    subl $12, %esp
    pushl %eax
    call fixture_main
    movl %eax, %ebx
    movl $1, %eax
    int $0x80
    ud2
.size _start, .-_start
.section .note.GNU-stack,"",@progbits
