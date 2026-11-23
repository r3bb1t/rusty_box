# User-mode execution foundation

Milestone 9 adds the optional `rusty_box_linux_user` crate on top of `e0638c4`. A static Linux/i386 process runs through the x86 interpreter in ring 3 without a Linux kernel or PC. The two independently compiled test executables run from ordinary ELF files. [Run and test them](../../rusty_box_linux_user/README.md)

## Architecture review

| Layer | Existing API and responsibility | Use in this integration |
| --- | --- | --- |
| x86 instruction execution | `MemorySession` lends a CPU and `FlatMemory` or `MappedMemory`. `CpuEmulator` owns a dense backing and retains cache state across short borrows. The interpreter handles typed registers, decode, dispatch budgets, fault delivery, and restart. | Use `CpuEmulator` with compact physical storage and ordinary x86 paging. Reuse its scalar I/O continuation. |
| Process address space | `MappedMemory` describes caller-owned **physical** regions. Its holes are open bus for data. Virtual memory faults and privileges come from the CPU's page-table walker. | Populate PAE tables with supervisor-only bootstrap pages, individual user segment permissions and a guarded stack. Check syscall buffers against the same fixed mapping. |
| OS ABI and external I/O | The CPU and full-system machine have no Linux syscall policy. The full system's devices and scheduling serve a guest OS. Earlier Linux examples use a real OS or inspect resident ELF data. | Put ELF loading, stack construction, syscall numbers and errno in the adapter. Leave stdin, stdout and stderr with a generic application-owned `Io` provider. |
| Execution control and events | The CPU-only API reports dispatch progress and architecture stops. `App`, `ReadyApp` and `Emit` compose application handlers and borrow state synchronously. | Translate processor boundaries into process results. Let the embedding application publish owned lifecycle values as needed. |

The choices for syscall entry were host-side opcode inspection and IP advance, a new interpreter exit after decoding, and an actual protected-mode interrupt gate bridged through an existing CPU-only port continuation. Opcode inspection would duplicate fetch and prefix validation and invent a retirement contract. A new engine exit would need a distinct validation and restart policy at the boundary where the engine currently delivers the interrupt. The interrupt gate uses the ordinary privilege check, TSS stack switch, flags and return frame, and works with the existing ports and budget API. The tradeoff is measurable work for the supervisor hop and `IRET`. [CPU-only control](cpu-only-control.md) · [Scalar I/O continuation](../../rusty_box/src/cpu/memory_session/scalar_io.rs) · [Bochs `INT_Ib` port](../../rusty_box/src/cpu/soft_int.rs)

The existing `pre_syscall` hook covers the fast `SYSENTER` and `SYSCALL` instructions. On those paths the interpreter checks the advertised architectural entry, then lets an instrumenter observe, skip, or stop the transition. `INT imm8` instead goes through normal interrupt delivery. The adapter leaves the fast-entry MSRs at reset values and uses the ordinary `INT` path for its Linux ABI. The existing pre-delivery fault exit belongs to the guarded unpaged external-memory MOV step. Process faults here use normal IDT delivery and the saved architectural frame. [Fast syscall entry and instrumentation](../../rusty_box/src/cpu/proc_ctrl.rs) · [Guarded scalar execution](../../rusty_box/src/cpu/cpu.rs)

## Interrupt, handoff and return

The integration initializes a minimal GDT, TSS, IDT, and PAE tables. The x86 interpreter runs the supervisor bootstrap with real `LGDT`, `LIDT`, `LTR`, control-register writes, and `IRET`. A fixed bootstrap dispatch count and typed register checks establish the initial user instruction and stack without running the application first. PAE and NX use features of the default emulated CPU. The i386 label describes the guest ABI. [Supervisor setup](../../rusty_box_linux_user/src/i386.rs) · [Intel SDM, Vol. 3](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html)

1. A real guest `INT 0x80` goes through the engine's decode and IDT delivery. Its DPL-3 gate enters ring 0 and saves the next user IP, CS, flags, and old stack.
2. The supervisor handler executes an ordinary scalar `OUT`. The CPU-only API stops before that port instruction retires. The adapter recognizes the supervisor port and checks the current CS. Ring-3 port I/O fails its own privilege check. This private channel belongs to the supervisor handler and requires no guest executable checkpoint.
3. `service_scalar_io` completes the pending instruction once, before the host changes guest RAM or EAX. The adapter takes the Linux call number from EAX and reads the first three arguments from EBX, ECX, and EDX. It validates the arguments and invokes the application provider at most once. The result is placed in EAX. For a returning syscall the engine then executes the handler's `IRET`, restoring the guest frame. `exit` is terminal.

Other IDT entries report architectural faults with the saved EIP and error code where applicable. Page faults include CR2. Invalid instruction encodings, absent gates and direct user port access follow the CPU's normal validation. There is no change to full-system delivery or the interpreter hot loop. A process either reaches the next budget boundary, exits, reports a fault or unsupported call, or ends with a typed engine/provider error. Completed I/O is never replayed by another `run`. [Run loop](../../rusty_box_linux_user/src/lib.rs) · [Boundary regression tests](../../rusty_box_linux_user/src/tests.rs)

## Loader, memory and Linux ABI scope

- Validate little-endian `ELFCLASS32`, `EM_386`, fixed-address `ET_EXEC`, up to 64 program headers, readable nonoverlapping `PT_LOAD` pages, offset/address congruence, and an entry inside executable file bytes. The program header table must be part of a loaded segment. Segment memory is limited to 32 MiB. `PT_INTERP`, `PT_DYNAMIC`, `PT_TLS`, executable stacks and writable/executable segments are refused. Memory outside copied file bytes starts at zero, including BSS. [ELF loader](../../rusty_box_linux_user/src/elf.rs)
- Start at the ELF entry with zeroed general-purpose registers except ESP. The 16-byte-aligned initial stack contains `argc`, `argv`, `envp`, and a minimal auxiliary vector with a mapped `AT_PHDR`, `AT_ENTRY`, and `AT_PAGESZ`. No VDSO, TLS or loader entropy is supplied. The stack has 64 KiB of writable, non-executable memory with a guard below it. [Stack builder](../../rusty_box_linux_user/src/stack.rs)
- Use normal x86 page checks for unmapped pages, user/supervisor access, writes, and instruction fetch. The host's static region map checks user spans for syscalls and debug reads. It does not implement a second CPU virtual memory engine. Physical loading and host writes go through the existing tracked `CpuEmulator` memory access. [Process memory](../../rusty_box_linux_user/src/memory.rs)
- Support Linux i386 `exit(1)`, `read(3)` on fd 0 and `write(4)` on fd 1 or 2. Return the low eight exit-status bits, actual transfer counts, zero at EOF, or negative errno in EAX. A single call offers up to 4 KiB to the host provider. The adapter checks that whole offered span before I/O and returns `EFAULT` for an invalid span. Guest code can retry short transfers with adjusted pointers. The built-in provider has sequential input and bounded, independent output streams. Unsupported syscall numbers return a terminal `UnsupportedSystemCall` result. [I/O contract](../../rusty_box_linux_user/src/io.rs)

The API takes an immutable parsed `Image`, a borrowed `Launch` description and an `Io` value by ownership. Passing bytes for every launch would simplify one-shot use but repeat validation during reconstruction. A builder that owns copied argument strings would allocate before the fixed stack copy. The chosen API accepts caller-owned byte slices, supports non-UTF-8 strings and keeps one parse reusable across launches. Lifecycle events are explicit so an application can run without event subscriptions or per-instruction routing.

## Determinism and portability

Fresh launches create fresh CPU state, zeroed physical backing, stack, page tables, and I/O state supplied by the caller. The same inputs, budgets, and deterministic provider yield equivalent status, stream bytes and register/memory results for the supported workload. A budget stop preserves the machine and pending supervisor continuation through the next call. Guest `REP` and exception restarts retain the CPU's dispatch and count conventions. The reported engine count includes supervisor instructions and can exceed the dispatch budget for `REP`. It is separate from guest-visible output and status. `run` takes the CPU-only dispatch budget directly. The full-system `RunBudget` also offers guest ticks, which this environment does not schedule. The budget supplies no host deadline for a blocking provider.

AArch64 has typed registers, EL0 state, and its own execution, translation, and `SVC` delivery in the `virt` machine. A future adapter can follow the same ownership pattern for I/O and lifecycle, with an AArch64-specific ELF loader, process entry, syscall registers and exception return. That work needs a standalone AArch64 execution and memory boundary suited to EL0. This milestone implements Linux/i386 only. [AArch64 `SVC` path](../../rusty_box_aarch64/src/cpu/system.rs) · [Shared machine budget vocabulary](../../rusty_box_core/src/run.rs)

## Validation

The two test-owned C programs were compiled separately with GCC 14.2. The compute program verified its initial arguments, environment, auxiliary-vector headers, BSS and checksum, then exited with status 0. The filter produced `HELLO FROM RUSTY_BOX` from application-supplied input and exited with status 0. The independent integration test imposed two-byte reads and one-byte writes, checked EOF and complete output, and replayed the same input at the same budget. It also ran different input one dispatch at a time and recovered typed application state after five program exits.

The adapter tests cover ELF bounds and layout, initial registers, physical loading, register/flag restoration, syscall return values, unsupported calls, I/O errors, and the boundary between interrupt delivery and one-time I/O completion. They checked page-fault error bits for unmapped, read-only, supervisor and non-executable pages. The ownership compile-fail test confirmed that a borrowed I/O view prevents another run. Adapter Clippy passed with `-D warnings`.

The existing x86 library suite passed 1,065 tests. The CPU-only, typed-register, flat and external memory, and composed hardware regressions passed, along with the 18 compiler fixtures. The full-system headless DLX guest reached its login marker around the 383-million-instruction progress report. Standard AArch64 suites passed 1,180 tests without default features and 1,286 with `std`, with eight ignored in each suite. A separate AArch64 `no_std` unit run with the AARCHMRS dataset passed 980 tests with two ignored and one filtered out. The long register-access sweep was filtered and the exhaustive decoder sweep remained ignored. [Independent guest integration](../../rusty_box_linux_user/tests/programs.rs) · [Reproduction commands](../../rusty_box_linux_user/README.md)

The x86 interpreter checked with and without `alloc` under `no_std`, and without allocation on `x86_64-unknown-none`. The freestanding smoke binary linked without Rust allocator symbols. The interpreter with `alloc` checked on WebAssembly, the AArch64 bare-metal check passed, and the new crate checked on both freestanding x86 and WebAssembly.

The repository gate reached `symbols clippy` with `--skip-aarchmrs` and stopped on six existing `chunks_exact_to_as_chunks` diagnostics in `rusty_box_symbols` with Rust 1.100.0. The symbol-engine sources match the milestone base. The adapter Clippy and the targeted x86, AArch64, and guest checks above ran independently.

## Performance

These are two release benchmark runs with Rust 1.100.0 and GCC 14.2 on an AMD EPYC 9V74 Linux worker. Each median comes from seven batches with host CPU affinity. The shared worker showed run-to-run variation. The launch measurement includes process allocation, ELF installation, bootstrap and destruction. The ELF installation measurement copies the fixture's load segments into already allocated RAM. The compute and filter timings begin at a ready user entry and exclude construction and event delivery.

| Stage | First median | Repeat median |
| --- | ---: | ---: |
| Validate `compute` ELF | 43.75 ns | 43.91 ns |
| Install `compute` segments into allocated RAM | 25.46 ns | 29.33 ns |
| Fresh launch from parsed `compute` ELF | 1.33 ms | 1.02 ms |
| Execute `compute` | 31.92 µs | 33.78 µs |
| Execute `filter`, per input byte over 4 KiB | 859 ns | 855 ns |
| Zero-byte `read` including `INT`, scalar handoff, `IRET`, and the guest loop | 1.74 µs | 1.55 µs |
| Checked physical read of 64 bytes | 7.84 ns | 6.53 ns |
| Checked process read of 64 bytes | 15.22 ns | 13.57 ns |
| Optional typed start and exit event pair | 1.36 ns | 1.35 ns |
| CPU-only unpaged increment-and-jump loop, per engine count | 38.86 ns | 41.58 ns |
| User-mode paged increment-and-jump loop, per engine count | 29.09 ns | 13.34 ns |

The 32-bit loop comparison used identical bytes and guest addresses, alternating the order of the two modes. Each batch ran two million counts and both final memory values agreed. The two runs observed 23.89–47.73 ns per count for CPU-only and 12.97–41.01 ns for user mode. These are measurements of two specific memory and privilege configurations. The variability and the small loop limit what they establish about other workloads. The full-system `perfbench` measured a separate 64-bit mixed workload using 50 million counts per sample. Seven samples gave a median of 14.44 ns per count with a 10.85–16.12 ns range. [User-mode benchmark](../../rusty_box_linux_user/src/tests.rs) · [Full-system benchmark](../../rusty_box/examples/perfbench/perfbench.rs)

```sh
PERFBENCH_MODE=mixed PERFBENCH_INSN=50000000 \
  cargo run --locked --release -p rusty_box --no-default-features --features std --example perfbench
```
