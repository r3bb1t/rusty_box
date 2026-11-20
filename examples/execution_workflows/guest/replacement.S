/* Linux i386. Report ordinary process identity at the observation boundary. */
.globl _start, image_ready
.section .text
_start:
    mov $20, %eax                  /* getpid */
    int $0x80
image_ready:
    mov $4, %eax                   /* write(1, marker, len) */
    mov $1, %ebx
    mov $marker, %ecx
    mov $(marker_end - marker), %edx
    int $0x80
    cmp $(marker_end - marker), %eax
    setne %bl
    movzbl %bl, %ebx
    mov $1, %eax                   /* exit(write succeeded ? 0 : 1) */
    int $0x80

marker:
    .ascii "RB_REPLACEMENT_OK\n"
marker_end:
.section .note.GNU-stack,"",@progbits
