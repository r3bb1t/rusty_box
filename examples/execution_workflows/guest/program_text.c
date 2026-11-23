#include "i386_syscall.h"

int guest_main(const unsigned *stack)
{
    if (stack[0] != 1)
        return 1;
    const char *name = (const char *)stack[1];
    unsigned len = 0;
    while (name[len])
        ++len;
    if (len != sizeof("/bin/program_text") - 1)
        return 2;
    return WRITE_LITERAL("argv0=") || write_all(name, len) || WRITE_LITERAL("\n");
}
