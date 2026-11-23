typedef unsigned int u32;

static int io(u32 number, u32 fd, void *buffer, u32 len) {
    int result;
    __asm__ volatile ("int $0x80" : "=a" (result) : "0" (number), "b" (fd), "c" (buffer), "d" (len) : "memory", "cc");
    return result;
}

int fixture_main(const u32 *initial) {
    (void)initial;
    unsigned char buffer[3];
    for (;;) {
        int count = io(3, 0, buffer, sizeof(buffer));
        if (count <= 0)
            return count < 0 ? 91 : 0;
        for (int i = 0; i < count; i++) {
            if (buffer[i] >= 'a' && buffer[i] <= 'z')
                buffer[i] -= 'a' - 'A';
        }
        for (int at = 0; at < count;) {
            int written = io(4, 1, buffer + at, (u32)(count - at));
            if (written <= 0)
                return 92;
            at += written;
        }
    }
}
