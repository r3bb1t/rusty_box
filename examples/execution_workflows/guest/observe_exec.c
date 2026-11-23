#include "i386_syscall.h"

enum {
    TRACEME = 0, PEEKDATA = 2, CONT = 7, GETREGS = 12, SETOPTIONS = 0x4200,
    TRACEEXEC = 1 << 4, EXITKILL = 1 << 20, EXEC_EVENT = 4,
    SIGSTOP = 19, SIGTRAP = 5, STOPPED = 0x7f,
    AT_NULL = 0, AT_PHDR = 3, AT_PHENT = 4, AT_PHNUM = 5, AT_ENTRY = 9,
    /* Slots of eip and esp in Linux i386's PTRACE_GETREGS user_regs_struct. */
    EIP = 12, ESP = 15, REGISTER_WORDS = 17,
};

static int trace(int request, int pid, unsigned address, int data)
{
    return syscall4(SYS_PTRACE, request, pid, address, data);
}

static int wait_child(int pid, unsigned *status)
{
    return syscall4(SYS_WAITPID, pid, (int)status, 0, 0) == pid ? 0 : -1;
}

static int fail(const char *operation, unsigned len)
{
    if (WRITE_LITERAL("RB_ERROR ") || write_all(operation, len) || WRITE_LITERAL("\n"))
        return 126;
    return 125;
}

#define FAIL(text) fail(text, sizeof(text) - 1)

static int hex(unsigned value)
{
    char digits[9] = " 00000000";
    for (unsigned i = 8; i > 0; --i, value >>= 4)
        digits[i] = "0123456789abcdef"[value & 15];
    return write_all(digits, sizeof(digits));
}

static void proc_path(char path[32], unsigned pid, const char *leaf)
{
    char digits[10];
    unsigned len = 0;
    do {
        digits[len++] = '0' + pid % 10;
        pid /= 10;
    } while (pid);
    char *out = path;
    for (const char *p = "/proc/"; *p; ++p)
        *out++ = *p;
    while (len)
        *out++ = digits[--len];
    *out++ = '/';
    while ((*out++ = *leaf++)) {}
}

static int report_exec(unsigned pid, const unsigned *regs, unsigned *phdr)
{
    char path[32];
    char target[128];
    proc_path(path, pid, "exe");
    int len = syscall4(SYS_READLINK, (int)path, (int)target, sizeof(target), 0);
    if (len <= 0 || (unsigned)len >= sizeof(target))
        return FAIL("readlink /proc/pid/exe");

    proc_path(path, pid, "auxv");
    int fd = syscall4(SYS_OPEN, (int)path, 0, 0, 0);
    if (fd < 0)
        return FAIL("open /proc/pid/auxv");
    unsigned auxv[128];
    int bytes = read_bytes(fd, auxv, sizeof(auxv));
    if (syscall4(SYS_CLOSE, fd, 0, 0, 0) || bytes <= 0 || bytes % 8)
        return FAIL("read /proc/pid/auxv");
    unsigned entry = 0, phent = 0, phnum = 0;
    int terminated = 0;
    for (unsigned i = 0; i < (unsigned)bytes / 4; i += 2) {
        switch (auxv[i]) {
        case AT_NULL: terminated = 1; break;
        case AT_ENTRY: entry = auxv[i + 1]; break;
        case AT_PHDR: *phdr = auxv[i + 1]; break;
        case AT_PHENT: phent = auxv[i + 1]; break;
        case AT_PHNUM: phnum = auxv[i + 1]; break;
        }
        if (terminated)
            break;
    }
    if (!terminated || entry != regs[EIP] || !entry)
        return FAIL("expected auxv and static entry registers");

    if (WRITE_LITERAL("RB_EXEC") || hex(pid) || hex(entry) || hex(regs[ESP]) ||
        hex(*phdr) || hex(phent) || hex(phnum) || WRITE_LITERAL("\nRB_EXE ") ||
        write_all(target, len) || WRITE_LITERAL("\nRB_MAPS\n"))
        return FAIL("write exec record");

    proc_path(path, pid, "maps");
    fd = syscall4(SYS_OPEN, (int)path, 0, 0, 0);
    if (fd < 0)
        return FAIL("open /proc/pid/maps");
    char chunk[512];
    unsigned budget = 16 * 1024;
    while ((bytes = read_bytes(fd, chunk, sizeof(chunk))) > 0) {
        if ((unsigned)bytes > budget || write_all(chunk, bytes))
            return FAIL("maps transcript bound");
        budget -= bytes;
    }
    if (syscall4(SYS_CLOSE, fd, 0, 0, 0) || bytes < 0 || WRITE_LITERAL("RB_WAIT\n"))
        return FAIL("read /proc/pid/maps");
    return 0;
}

int guest_main(const unsigned *stack)
{
    if (stack[0] != 2)
        return FAIL("observe needs one executable path");
    int pid = syscall4(SYS_FORK, 0, 0, 0, 0);
    if (pid < 0)
        return FAIL("fork");
    if (!pid) {
        int self = syscall4(SYS_GETPID, 0, 0, 0, 0);
        if (trace(TRACEME, 0, 0, 0) || syscall4(SYS_KILL, self, SIGSTOP, 0, 0))
            return FAIL("child trace stop");
        unsigned argv[] = {stack[2], 0};
        unsigned envp[] = {0};
        syscall4(SYS_EXECVE, stack[2], (int)argv, (int)envp, 0);
        return FAIL("execve");
    }

    unsigned status;
    if (wait_child(pid, &status) || status != ((SIGSTOP << 8) | STOPPED) ||
        trace(SETOPTIONS, pid, 0, TRACEEXEC | EXITKILL) || trace(CONT, pid, 0, 0) ||
        wait_child(pid, &status) || status != ((EXEC_EVENT << 16) | (SIGTRAP << 8) | STOPPED))
        return FAIL("successful exec stop");
    unsigned regs[REGISTER_WORDS];
    unsigned phdr = 0;
    if (trace(GETREGS, pid, 0, (int)regs))
        return FAIL("entry registers");
    int error = report_exec(pid, regs, &phdr);
    if (error)
        return error;

    char command[4];
    if (read_bytes(0, command, sizeof(command)) != 2 || command[1] != '\n' ||
        (command[0] != 'r' && command[0] != 'm'))
        return FAIL("resume command");
    if (command[0] == 'm') {
        /* Fault in the header page for the optional host read. The host
           treats a failed translation as unavailable even after this hint. */
        unsigned word;
        if ((!phdr || trace(PEEKDATA, pid, phdr, (int)&word)) &&
            WRITE_LITERAL("RB_HEADER_UNAVAILABLE\n"))
            return FAIL("write residency result");
    }
    if (WRITE_LITERAL("RB_RUNNING\n") || trace(CONT, pid, 0, 0) || wait_child(pid, &status))
        return FAIL("resume/wait");
    if ((status & 0xff) == STOPPED)
        return FAIL("unexpected ptrace stop");
    if (WRITE_LITERAL("RB_STATUS") || hex(pid) || hex(status) || WRITE_LITERAL("\n"))
        return FAIL("write exit status");
    return 0;
}
