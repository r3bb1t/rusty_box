/*
 * The guest side of gen_qemu_sve.py: a static AArch64 Linux program run
 * under QEMU's user-mode emulator (`qemu-aarch64 -cpu max,sme_fa64=off,
 * sme-default-vector-length=64`), which runs each word in Streaming SVE
 * mode at a 512-bit vector length, with ZA storage enabled.
 *
 * It maps the 64 KiB data region at DATA, then for each case read from
 * stdin (a record: the instruction word, the register state as `struct
 * state` lays it out, then the region's bytes) loads the region, patches
 * the word into `insn_slot` and runs it with qemu_sve_trampoline.S. It
 * writes back a status (0 executed, 1 raised a signal), the register state
 * after the word and the region's bytes, and stops at end of input.
 */
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <sys/mman.h>
#include <ucontext.h>
#include <unistd.h>

#define DATA 0x100000UL
#define DATA_SIZE 0x10000UL
/* The Streaming SVE vector length in bytes, and the predicate length. */
#define VLB 64
#define PLB (VLB / 8)

struct state {
	uint64_t x[31];
	uint64_t sp;
	uint64_t nzcv;
	uint64_t fpcr;
	uint64_t fpsr;
	uint64_t pad;
	uint8_t z[32][VLB];
	uint8_t p[16][PLB];
	/* ZA's horizontal vectors, ZA[0] to ZA[VLB - 1]. */
	uint8_t za[VLB][VLB];
};

_Static_assert(offsetof(struct state, sp) == 248, "OFF_SP");
_Static_assert(offsetof(struct state, nzcv) == 256, "OFF_NZCV");
_Static_assert(offsetof(struct state, fpcr) == 264, "OFF_FPCR");
_Static_assert(offsetof(struct state, fpsr) == 272, "OFF_FPSR");
_Static_assert(offsetof(struct state, z) == 288, "OFF_Z");
_Static_assert(offsetof(struct state, p) == 2336, "OFF_P");
_Static_assert(offsetof(struct state, za) == 2464, "OFF_ZA");
_Static_assert(sizeof(struct state) == 6560, "state size");

struct request {
	uint32_t insn;
	uint32_t pad[3];
	struct state in;
};

struct reply {
	uint32_t status;
	uint32_t pad[3];
	struct state out;
};

_Static_assert(offsetof(struct request, in) == 16, "request header");
_Static_assert(offsetof(struct reply, out) == 16, "reply header");

extern int run_one(const struct state *in, struct state *out);
extern uint32_t insn_slot[];
extern char fault_return[];

static char signal_stack[1 << 16];

/* Resume a word that raised a signal at `fault_return`, which leaves
 * Streaming SVE mode, disables ZA and restores the caller's registers. */
static void on_signal(int sig, siginfo_t *info, void *context)
{
	ucontext_t *uc = context;
	(void)sig;
	(void)info;
	uc->uc_mcontext.pc = (uintptr_t)fault_return;
}

/* Read exactly `n` bytes: 1 if they arrived, 0 at end of input. */
static int read_exact(void *buf, size_t n)
{
	char *p = buf;
	while (n > 0) {
		ssize_t got = read(0, p, n);
		if (got <= 0)
			return 0;
		p += got;
		n -= (size_t)got;
	}
	return 1;
}

static int write_exact(const void *buf, size_t n)
{
	const char *p = buf;
	while (n > 0) {
		ssize_t put = write(1, p, n);
		if (put <= 0)
			return 0;
		p += put;
		n -= (size_t)put;
	}
	return 1;
}

int main(void)
{
	void *data = mmap((void *)DATA, DATA_SIZE, PROT_READ | PROT_WRITE,
			  MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
	if (data != (void *)DATA)
		return 2;
	stack_t ss = { .ss_sp = signal_stack, .ss_size = sizeof signal_stack };
	if (sigaltstack(&ss, NULL) != 0)
		return 3;
	struct sigaction sa;
	memset(&sa, 0, sizeof sa);
	sa.sa_sigaction = on_signal;
	sa.sa_flags = SA_SIGINFO | SA_ONSTACK;
	const int signals[] = { SIGILL, SIGSEGV, SIGBUS, SIGFPE, SIGTRAP };
	for (size_t i = 0; i < sizeof signals / sizeof signals[0]; i++)
		if (sigaction(signals[i], &sa, NULL) != 0)
			return 4;

	static struct request req;
	static struct reply rep;
	while (read_exact(&req, sizeof req) && read_exact(data, DATA_SIZE)) {
		insn_slot[0] = req.insn;
		__builtin___clear_cache((char *)insn_slot, (char *)(insn_slot + 1));
		memset(&rep, 0, sizeof rep);
		rep.status = (uint32_t)run_one(&req.in, &rep.out);
		if (!write_exact(&rep, sizeof rep) || !write_exact(data, DATA_SIZE))
			return 5;
	}
	return 0;
}
