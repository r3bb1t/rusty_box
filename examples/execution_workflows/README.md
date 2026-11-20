# Linux observation and LibAFL execution examples

This is a standalone downstream Cargo workspace. Its lockfile pins the example dependencies. The main rusty_box workspace has no dependency on this package, Linux userland, or LibAFL.

## Boot Linux and replace `/bin/cat`

The example uses a Debian 12 i386 kernel and a static i386 BusyBox. The initramfs starts a shell on COM1 and contains the original BusyBox `cat`. The replacement travels over COM1 after the guest reports readiness. The host invokes the guest's own `printf`, `chmod`, `mv`, `exec`, and `wait` through that shell.

The verified installer kernel reports `6.1.0-50-686`. Its SHA-256 is `c77cd9c37609b8b1b1cdfbb91b644a44d606d3d75da8656a0c79635be0b89c21`. Debian's `current` URL can move to a newer kernel. The BusyBox package SHA-256 is `725ef9e6fbac4ab79b353165b05005e8c140d6e42177ebbc202ff1e13ded8b7c`.

Run from the rusty_box repository root with GNU `as` and `ld`, Python 3, and `dpkg-deb` available. Set `RB_WORK` to an absolute scratch directory.

```sh
mkdir -p "$RB_WORK"
curl -fL 'https://deb.debian.org/debian/dists/bookworm/main/installer-i386/current/images/netboot/debian-installer/i386/linux' -o "$RB_WORK/linux"
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
