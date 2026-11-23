#ifndef RB_I386_SYSCALL_H
#define RB_I386_SYSCALL_H

/* Linux i386 syscall ABI. These fixtures link without a C library. */
enum {
    SYS_FORK = 2, SYS_READ = 3, SYS_WRITE = 4, SYS_OPEN = 5, SYS_CLOSE = 6,
    SYS_WAITPID = 7, SYS_EXECVE = 11, SYS_GETPID = 20, SYS_PTRACE = 26,
    SYS_KILL = 37, SYS_READLINK = 85,
};

static inline int syscall4(int number, int a, int b, int c, int d)
{
    int result;
    __asm__ volatile ("int $0x80"
        : "=a" (result)
        : "0" (number), "b" (a), "c" (b), "d" (c), "S" (d)
        : "cc", "memory");
    return result;
}

static inline int read_bytes(int fd, void *bytes, unsigned size)
{
    return syscall4(SYS_READ, fd, (int)bytes, size, 0);
}

static inline int write_all(const char *bytes, unsigned size)
{
    while (size) {
        int count = syscall4(SYS_WRITE, 1, (int)bytes, size, 0);
        if (count <= 0)
            return -1;
        bytes += count;
        size -= count;
    }
    return 0;
}

#define WRITE_LITERAL(text) write_all(text, sizeof(text) - 1)

#endif
