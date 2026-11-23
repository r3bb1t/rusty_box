# Downstream execution examples

This is a standalone downstream Cargo workspace. Its lockfile pins the example dependencies. The main rusty_box workspace has no dependency on this package, Linux userland, or LibAFL.

## Route commands to application-owned x86 guests

```sh
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin typed_events
cargo test --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin typed_events
```

The same `RunRequested` route follows changes to the selected guest and also accepts an exact process identity. The demo executes `mov` and `hlt` on two firmware-free x86 emulators, then recovers the application state and prints an audit record for each run request. The application assigns a new generation when replacing an instance that reuses a process ID. The Linux examples below demonstrate identification inside a guest OS.

Start at `compose` to see `App::on()` route `SelectProcess` and `RunRequested` by type before `with_state` supplies the catalog. The run route wraps `Subscriptions` in two example-local `Emit` layers. Audit surrounds the budget guard so a refused request is recorded without a lookup or a CPU dispatch. The `TargetProcess` extractor resolves the requested identity on first use and shares that result across the before, execution, and after handlers. Each handler borrows the guest for its own call. The tests cover selection changes, resuming execution, stale IDs, cached lookup errors, and observation after a stop request. [Composition](src/bin/typed_events/main.rs) · [Layers and extraction](src/bin/typed_events/run.rs) · [Application event guide](../../docs/vnext/application-events.md)

## Lend custom RAM to the x86 interpreter

Run `external_ram --direct` for a downstream provider that owns separate code, data, and stack allocations. The guest uses all three and the caller verifies its stores after the session ends. The provider constructs a `DirectMemory` map by lending the existing slices and one stamp per 4 KiB page. [Direct memory contract](../../docs/vnext/direct-x86-backing.md)

Run `external_ram` without arguments to see indirect data RAM implemented with two independent byte lanes. A real paged x86 workload writes and reads those bytes through `MemorySession`. The demonstration replaces warm direct backing, checks that a host code write invalidates a cached instruction, reads across two provider pages, and verifies that instruction fetch requires direct backing.

With an initialized CPU, a flat or mapped `direct` backing, and a provider `ram` that implements `IndirectRam`, attach the provider for one session.

```rust
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::memory_session::MemorySession;
use rusty_box::memory::indirect::IndirectRamRange;

let range = IndirectRamRange { start: BxPhyAddress::new(0x5000), len: 8192 };
let loan = direct.loan().with_indirect_ram(range, &mut ram)?;
let mut session = MemorySession::new(&mut cpu, loan);
let run = session.run_controlled(16);
```

```sh
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin external_ram
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin external_ram -- --direct
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin external_ram -- --bench
```

The benchmark rotates dense, mapped, multi-backing direct, indirect-copy, and byte-lane memory through the same paged guest loop. `RB_MEMORY_COUNT` sets dispatches per sample. [Memory contracts and implementation comparison](../../docs/vnext/direct-x86-backing.md)

## Measure CPU-only execution and instrumentation

```sh
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml --bin cpu_perf
```

`cpu_perf` reports the ordinary ALU loop with the same tracer's execution mask disabled or enabled. It also compares short warm runs with checked code writes followed by refetch. `RB_CPU_COUNTS` and `RB_CACHE_BATCHES` set the per-sample budgets. [Performance review procedure](../../docs/vnext/api-consolidation.md#performance-review-procedure)

## Sample Linux tasks from the emulator host

Download the pinned Debian installer image and unpack the BusyBox package as described below. Use `--host-only` to build a guest with an ordinary shell and two executables. COM1 launches each command and checks its output and exit code. The observer selects the active task at a kernel breakpoint and reads the task, page-directory root, executable mappings, and entry from emulator-visible registers and RAM. [Design and scope](../../docs/vnext/linux-host-observation.md)

```sh
sh examples/execution_workflows/guest/build_observation.sh \
  "$RB_WORK/busybox/bin/busybox" "$RB_WORK/host-only" --host-only
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --bin linux_host_observe -- "$RB_WORK/linux" "$RB_WORK/host-only/observation.cpio.gz"
RB_OBSERVATION_KERNEL="$RB_WORK/linux" \
RB_HOST_OBSERVATION_INITRAMFS="$RB_WORK/host-only/observation.cpio.gz" \
cargo test --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --test linux_host_observation -- --ignored --nocapture
```

The example reports a pending userspace return and the executable backing it for each of `program_sum` and `program_text`. Add `--benchmark` to time paused task reads, the mapping walk with auxiliary-vector parsing, and a separate optional ELF probe. The probe reports unavailable header bytes if the read-only header page is absent.

The independent oracle test uses the **supervised** initramfs built in the next section. It compares a new host snapshot at a ring-3 pause against the original supervisor's PID, entry and executable ranges, and checks the live CR3 and resident ELF metadata.

```sh
RB_OBSERVATION_KERNEL="$RB_WORK/linux" \
RB_OBSERVATION_INITRAMFS="$RB_WORK/observation/observation.cpio.gz" \
cargo test --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --lib host_snapshot_matches_independent_ptrace_procfs_oracle -- --ignored --nocapture
```

## Observe two Linux programs at user entry

Use the Debian i386 kernel and unpacked BusyBox described in the next section. Build a separate initramfs with the supervisor, a fixed-address static executable, and a static PIE. The build uses GCC with i386 code generation, GNU binutils with `elf_i386` support, and Python 3. The three fixtures link without a host or guest C library.

```sh
sh examples/execution_workflows/guest/build_observation.sh \
  "$RB_WORK/busybox/bin/busybox" "$RB_WORK/observation"
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --bin linux_observe -- "$RB_WORK/linux" "$RB_WORK/observation/observation.cpio.gz"
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --bin linux_observe -- "$RB_WORK/linux" "$RB_WORK/observation/observation.cpio.gz" --elf
```

Expect `sum=22` and `argv0=/bin/program_text` from the two successful guest executions. Both modes report the PID, CR3, image generation, runtime entry and executable VMAs. `--elf` also reports the resident ELF header and program headers. The PIE's reported runtime entry includes its load bias. Basic mode requests zero ELF inspections.

Run the automated two-mode guest regression with the same assets.

```sh
RB_OBSERVATION_KERNEL="$RB_WORK/linux" \
RB_OBSERVATION_INITRAMFS="$RB_WORK/observation/observation.cpio.gz" \
cargo test --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --test linux_observation -- --ignored
```

The observer relies on ptrace and procfs in the controlled Linux/i386 guest. Its typed events describe the two supervised execs. [Design and boundaries](../../docs/vnext/linux-semantic-observation.md)

## Boot Linux and replace `/bin/cat`

The example uses a Debian 12 i386 kernel and a static i386 BusyBox. The initramfs starts a shell on COM1 and contains the original BusyBox `cat`. The replacement travels over COM1 after the guest reports readiness. The host invokes the guest's own `printf`, `chmod`, `mv`, `exec`, and `wait` through that shell.

The verified installer kernel reports `6.1.0-50-686`. Its SHA-256 is `c77cd9c37609b8b1b1cdfbb91b644a44d606d3d75da8656a0c79635be0b89c21`. The BusyBox package SHA-256 is `725ef9e6fbac4ab79b353165b05005e8c140d6e42177ebbc202ff1e13ded8b7c`.

Run from the rusty_box repository root with GNU `as` and `ld`, Python 3, and `dpkg-deb` available. Set `RB_WORK` to an absolute scratch directory.

```sh
mkdir -p "$RB_WORK"
curl -fL 'https://deb.debian.org/debian/dists/bookworm/main/installer-i386/20230607+deb12u15/images/netboot/debian-installer/i386/linux' -o "$RB_WORK/linux"
curl -fL 'https://deb.debian.org/debian/pool/main/b/busybox/busybox-static_1.35.0-4+deb12u1+b1_i386.deb' -o "$RB_WORK/busybox.deb"
dpkg-deb -x "$RB_WORK/busybox.deb" "$RB_WORK/busybox"
python3 examples/execution_workflows/guest/mkinitramfs.py "$RB_WORK/busybox/bin/busybox" "$RB_WORK/initramfs.cpio.gz"
as --32 examples/execution_workflows/guest/replacement.S -o "$RB_WORK/replacement.o"
ld -m elf_i386 -s -T examples/execution_workflows/guest/replacement.ld "$RB_WORK/replacement.o" -o "$RB_WORK/replacement.elf"
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --bin linux_cat -- "$RB_WORK/linux" "$RB_WORK/initramfs.cpio.gz" "$RB_WORK/replacement.elf"
```

The sample expects the ELF32/i386 fixture with a `getpid` instruction sequence at entry. Its linker script places the ELF metadata and executable bytes in one file-backed executable segment. The shell reports a real child PID while a FIFO holds the child before `exec`. A hardware-neutral debugger breakpoint catches it after Linux returns from the fixture's `getpid` call. The host checks that PID against both the shell's launch and its eventual wait status.

`ImageObservers::on` receives the event. `on_binary` also receives `Resolved<'_, AnalyzedBinary>`. One stopped-machine preparation reads the mapped ELF for all binary handlers. The example compares a digest of mapped code and headers with the supplied file's corresponding bytes. It then verifies the marker, restores the original `cat`, and checks that the original reads back a verification file.

## Run the LibAFL fixture

```sh
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --features libafl --bin libafl_fixture
cargo run --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --features libafl --bin libafl_fixture -- --replay 54
```

The example uses LibAFL's `Executor`, `OwnedMapObserver`, `MaxMapFeedback`, `StdFuzzer`, and in-memory corpus. It evaluates a fixed, benign byte classifier with three known paths. The guest accepts ASCII `A`, rejects other ordinary bytes, and reaches its dispatch budget on `T`. Both normal returns are `ExitKind::Ok`. The budget maps to `ExitKind::Timeout`. Unexpected engine exits stay host errors with input, PC, and dispatch count in the diagnostic.

A replay starts with a complete copy of 16 KiB of initial RAM and a protected-mode CPU reset. The map uses exact conditional edges and a known loop edge. No hashing or count bucketing is required for this fixed image. `--replay` prints the case outcome, final PC, dispatch count, guest tally, and edge map. `--bench` compares the same cases with and without branch hooks.

## Checks

```sh
cargo test --locked --release --manifest-path examples/execution_workflows/Cargo.toml
cargo test --locked --release --manifest-path examples/execution_workflows/Cargo.toml --features libafl
cargo test --locked --release --manifest-path examples/execution_workflows/Cargo.toml \
  --lib measure_resolution_and_delivery -- --ignored --nocapture
```
