#include "i386_syscall.h"

int guest_main(const unsigned *stack)
{
    if (stack[0] != 1)
        return 1;
    volatile unsigned values[] = {4, 5, 6, 7};
    unsigned sum = 0;
    for (unsigned i = 0; i < sizeof(values) / sizeof(values[0]); ++i)
        sum += values[i];
    if (sum != 22)
        return 2;
    return WRITE_LITERAL("sum=22\n") != 0;
}
