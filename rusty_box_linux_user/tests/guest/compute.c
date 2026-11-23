typedef unsigned int u32;

static volatile u32 scratch[4];

int fixture_main(const u32 *initial) {
    if (initial[0] != 2 || initial[3] || initial[5])
        return 100;
    const char *name = (const char *)initial[1];
    const char *number = (const char *)initial[2];
    const char *mode = (const char *)initial[4];
    if (name[0] != 'c' || number[0] != '7' || number[1] || mode[0] != 'M')
        return 101;
    const u32 *aux = initial + 6;
    u32 page = 0;
    u32 headers = 0;
    for (; aux[0]; aux += 2) {
        if (aux[0] == 3)
            headers = aux[1];
        if (aux[0] == 6)
            page = aux[1];
    }
    if (page != 4096 || !headers || *(const u32 *)headers != 1)
        return 102;
    for (u32 i = 0; i < 4; i++) {
        if (scratch[i])
            return 103;
        scratch[i] = (number[0] - '0') + i;
    }
    return scratch[0] + scratch[1] + scratch[2] + scratch[3] == 34 ? 0 : 104;
}
