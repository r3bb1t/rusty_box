# rusty_box examples

These are developer harnesses for the `rusty_box` library: boot tests, tracing
demos and benchmarks. The launcher meant for users is `rusty_box_gui`.
`cargo run --release -p rusty_box_gui` opens its VMware-style egui shell, which
takes typed CLI flags, an optional `rusty_box.toml`, and a choice of engine:
`--engine interpreter` (the default) or `--engine whp`. See
[rusty_box_gui/README.md](../../rusty_box_gui/README.md).

Run every command below from the workspace root; the examples look for their
files relative to the current directory.

## Required files

None of these are in the repository (`/cpp_orig`, `/binaries`, `/dlxlinux` and
`/*.iso` are gitignored).

| File | Needed by | Where to get it |
|------|-----------|-----------------|
| `BIOS-bochs-latest` — system BIOS, 128 KB | every example that boots firmware | a [Bochs](https://bochs.sourceforge.io/) source checkout, `bios/BIOS-bochs-latest` |
| `VGABIOS-lgpl-latest.bin` — VGA BIOS, 32 KB | optional, except for `snapshot_resume` | a Bochs source checkout, `bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin` |
| `hd10meg.img` — DLX Linux disk, 10.2 MiB | `dlxlinux`, `dlxlinux_egui`, `rusty_box_egui` (DLX) | [Bochs DLX Linux](https://bochs.sourceforge.io/diskimages.html) |
| `alpine-virt-*-x86_64.iso` | the Alpine examples, `rusty_box_egui` (Alpine) | [alpinelinux.org/downloads](https://alpinelinux.org/downloads/), the **Virtual** x86_64 image |

`shellcode_trace` and `perfbench` need no files at all.

Where the examples look:

- **System BIOS:** every example that boots firmware reads
  `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest`. `dlxlinux`, `dlxlinux_egui`,
  `rusty_box_egui` and `alpine` also accept `BIOS-bochs-latest` in the workspace
  root.
- **VGA BIOS:** a Bochs checkout is enough. `dlxlinux`, `dlxlinux_egui`,
  `rusty_box_egui` and `alpine`, and `alpine_direct` and `alpine_strace` in
  their BIOS boot, try `binaries/bios/VGABIOS-lgpl-latest.bin` first and then
  the Bochs tree's
  `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin`; a copy in
  `binaries/bios/` therefore overrides the checkout's. The direct kernel boot
  of `alpine_direct` and `alpine_strace` loads no VGA BIOS. `snapshot_resume`
  reads only the Bochs path, and panics when the file is missing.
- **DLX disk:** `dlxlinux/hd10meg.img`.
- **Alpine image:** each example has its own rule; see `ALPINE_ISO` and
  `ALPINE_DISK` below.

## Examples

Each is a `[[example]]` in `rusty_box/Cargo.toml`; the features column is its
`required-features`. `rusty_box`'s default features are `std` and `gui-egui`,
so passing `--features` only restates what is already on.

| Example | Features | What it does |
|---------|----------|--------------|
| `dlxlinux` | `std` | Boots DLX Linux on the terminal display. With `RUSTY_BOX_HEADLESS` set it runs with no display, presses Enter at `LILO boot:`, types `root` at `login:` and prints `*** LOGIN DETECTED ***`. |
| `dlxlinux_egui` | `std,gui-egui` | Boots DLX Linux in an egui window, with the emulator on a background thread. |
| `rusty_box_egui` | `std,gui-egui` | Boots DLX Linux or Alpine in an egui window, chosen by `RUSTY_BOX_BOOT`. |
| `alpine` | `std` | Boots Alpine on the terminal display, from a raw disk image or an ISO. |
| `alpine_direct` | `std` | Boots Alpine from an ISO, either through BIOS and ISOLINUX (the default) or by loading the kernel and initramfs straight out of the ISO. |
| `alpine_strace` | `std,gui-egui` | Boots Alpine in an egui window and decodes every SYSCALL through the `pre_syscall` instrumentation hook, logging each one to `strace.log`. |
| `shellcode_trace` | `std` | Runs a 74-byte Linux x86-64 reverse-TCP shellcode in flat long mode. A `pre_syscall` hook logs each syscall, spoofs its result and stops at `execve`. |
| `perfbench` | `std` | A CPU hot-loop microbenchmark in paging-on long mode, for profiling. `cargo xtask perf-baseline` builds it and archives the binary under `target/perf-baselines/<rev>/`. |
| `snapshot_resume` | `std` | Boots a guest from CD, saves a snapshot, restores it in a new process and keeps running. |

Instrumentation is not a Cargo feature. It is the machine's type parameter, an
implementation of `rusty_box::cpu::Instrumentation`. `alpine_strace` installs
one with `MachineBuilder::tracer`, and `shellcode_trace` with
`Emulator::new_with_mode_and_instrumentation`.

## Running

```bash
# DLX Linux, headless: the same run as the CI boot gate
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=450000000 cargo run --release --example dlxlinux --features std

# DLX Linux in an egui window
cargo run --release --example dlxlinux_egui --features "std,gui-egui"
RUSTY_BOX_BOOT=dlx cargo run --release --example rusty_box_egui --features "std,gui-egui"

# Alpine in an egui window
ALPINE_ISO=/path/to/alpine-virt-<version>-x86_64.iso RUSTY_BOX_BOOT=alpine \
  cargo run --release --example rusty_box_egui --features "std,gui-egui"

# Alpine, headless, BIOS boot
ALPINE_ISO=/path/to/alpine-virt-<version>-x86_64.iso RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=3500000000 \
  cargo run --release --example alpine_direct --features std

# Alpine, headless, direct kernel boot
ALPINE_ISO=/path/to/alpine-virt-<version>-x86_64.iso RUSTY_BOX_HEADLESS=1 RUSTY_BOX_BOOT=direct \
  cargo run --release --example alpine_direct --features std

# Syscall tracing
ALPINE_ISO=/path/to/alpine-virt-<version>-x86_64.iso \
  cargo run --release --example alpine_strace --features "std,gui-egui"
cargo run --release --example shellcode_trace --features std

# CPU benchmark
PERFBENCH_INSN=500000000 cargo run --release --example perfbench --features std

# Snapshot round trip: `full` (the default) runs `boot-save`, then
# `restore-run`, as two separate processes; either phase can be run alone.
RB_ISO=/path/to/image.iso cargo run --release --example snapshot_resume --features std
RB_ISO=/path/to/image.iso cargo run --release --example snapshot_resume --features std -- boot-save
```

`rusty_box_egui` runs until its window closes. Without `RUSTY_BOX_BOOT` it boots
Alpine if it finds an Alpine ISO and DLX otherwise, so name the guest when both
are present.

### Environment variables

| Variable | Read by | Effect |
|----------|---------|--------|
| `MAX_INSTRUCTIONS` | all six boot examples | Instruction budget. Defaults when unset: `dlxlinux` 450,000,000, the budget of the CI boot gate; `alpine` 1,000,000,000; `alpine_direct` 4,000,000,000; `dlxlinux_egui`, `rusty_box_egui` and `alpine_strace` unlimited, running until the window closes. `cargo run` always sets it: the repository's `.cargo/config.toml` supplies `20000000000` unless your environment already sets a value, so these defaults apply only to an example binary run outside cargo. |
| `RUSTY_BOX_HEADLESS` | `dlxlinux`, `alpine`, `alpine_direct` | Any value replaces the terminal display with no display. In `dlxlinux` it also turns on the scripted boot described above. In `alpine` it switches to a scripted loop of 100,000-instruction phases: it prints RIP after every phase and flags a RIP unchanged over four consecutive phases, presses Enter at the ISOLINUX prompt once 18M instructions have run, and from 100M on prints the VGA text every phase, adding `*** LOGIN DETECTED ***` when it shows `login:`. It types `root` the first time `login:` appears, and in each of the other phases from 100M on it presses Left Shift as a keep-alive. The egui examples ignore it. |
| `RUSTY_BOX_BOOT` | `rusty_box_egui` | `dlx`, `alpine` or `alpine-direct` (case-insensitive; any other value boots DLX). When unset: `alpine` if an Alpine ISO is found, otherwise `dlx`. |
| | `alpine_direct` | `direct` loads the kernel and initramfs from the ISO. Anything else, or unset, is BIOS boot. |
| | `alpine_strace` | `bios` is BIOS/ISOLINUX boot. The ISO's default console is serial, so the window's VGA console stays empty and output appears only in the serial panel. Anything else, or unset, is direct kernel boot with `console=tty0`. |
| `ALPINE_ISO` | `rusty_box_egui`, `alpine_direct`, `alpine_strace` | Path to the Alpine ISO. `alpine_direct` defaults to `alpine-virt-3.23.3-x86_64.iso` and `alpine_strace` to `alpine-virt-3.24.1-x86_64.iso`, both in the current directory. `rusty_box_egui` falls back to `ALPINE_DISK`, then to the first `.iso` whose name contains `alpine` (in any case) in the workspace root, its parent, or the current directory. `alpine` does not read it. |
| `ALPINE_DISK` | `alpine`, `rusty_box_egui` | `alpine`: a raw disk image or an ISO; a `.iso` extension attaches it as a CD-ROM. When unset, `alpine` tries `alpine/alpine.img`, `alpine/alpine-virt.img` and `alpine.img`, then the first `*alpine*.iso` in the workspace root. `rusty_box_egui` treats it as a second ISO path, checked after `ALPINE_ISO`. |
| `ALPINE_CHS` | `alpine` | Geometry `C,H,S` (e.g. `1024,16,63`) for a raw disk image. Detected from the image size when unset. |
| `ALPINE_RAM_MB` | `alpine`, `alpine_direct`, `alpine_strace`, `rusty_box_egui` (Alpine) | Guest RAM in MB (default 256). |
| `CMDLINE` | `alpine_direct`, `alpine_strace`, `rusty_box_egui` (`alpine-direct`) | The kernel command line for direct kernel boot, replacing the built-in one. |
| `RUSTY_BOX_NOSYNC` | `rusty_box_egui`, `alpine_strace` | `1` turns off the wall-clock slowdown (`sync_slowdown`), which is on by default. |
| `BIOS_OUTPUT_FILE` | `dlxlinux`, `alpine` | Write the BIOS's port-0xE9 debug output to this file. |
| `BIOS_QUIET_MODE` | `dlxlinux` | Only prints a heading above the BIOS output section; changes nothing else. |
| `RUSTY_BOX_DEBUG` | `alpine` | Headless only, and non-release builds only (`cfg(debug_assertions)`): prints boot diagnostics in each phase between 2.8M and 3.1M instructions. |
| `STRACE_LOG` | `alpine_strace` | The syscall log file (default `strace.log`, in the current directory). |
| `PERFBENCH_MODE` | `perfbench` | Loop shape: `mixed` (default), `alu`, `branch`, `straight` or `string`. |
| `PERFBENCH_INSN` | `perfbench` | Instruction budget (default 500,000,000). |
| `PERFBENCH_CPUS` | `perfbench` | Logical CPUs (default 1). The loop runs on the BSP; the other CPUs wait for a SIPI. |
| `PERFBENCH_QUANTUM` | `perfbench` | SMP scheduling quantum (default 16, range 1–32). |
| `RB_ISO` | `snapshot_resume` | The ISO to boot. Required, with no default: when it is unset, or not valid Unicode, the harness exits with status 2 and an error naming it. |
| `RB_MEM_MIB` | `snapshot_resume` | Guest and host RAM in MiB (default 2048). |
| `RB_BOOT_INSNS` | `snapshot_resume` | Instructions before the snapshot (default 1,000,000,000). |
| `RB_RESUME_INSNS` | `snapshot_resume` | Instructions after the restore (default 200,000,000). |
| `RB_SNAPSHOT` | `snapshot_resume` | Snapshot file (default `target/snapshot_resume.rbx`). |
| `RUST_LOG` | see below | Log level. |

**Logging.** The workspace builds `tracing` with `release_max_level_info`, so
`debug!` and `trace!` are compiled out of every `--release` build. `RUST_LOG`
chooses among `error`, `warn` (the default) and `info`. `dlxlinux` parses it as
a `tracing_subscriber` filter, so targets work (`RUST_LOG=rusty_box::iodev=info`).
`dlxlinux_egui`, `rusty_box_egui`, `alpine` and `alpine_direct` accept only a
bare level. `alpine_strace` and `shellcode_trace` always log at `info`
(`alpine_strace` to its `STRACE_LOG` file). `perfbench` and `snapshot_resume`
install no logger.

## Windows Hypervisor Platform harnesses

The WHP engine's harnesses are examples of the `rusty_box_whp_engine` crate:

```bash
cargo run --release -p rusty_box_whp_engine --example <name>
```

They compile on any host. Their hypervisor arm needs Windows with the Windows
Hypervisor Platform feature enabled; without it, each prints `skipped`. The ones
that boot firmware read `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest` and
`cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin`, relative to
the workspace root. `dlx_whp`, `alpine_bench` and `alpine_probe` read
`RUST_LOG` as filter directives. `compute_bench` and `step_bench` install no
logger.

| Example | What it does | Environment |
|---------|--------------|-------------|
| `dlx_whp` | Boots DLX Linux (`dlxlinux/hd10meg.img`) with the guest on the hypervisor and reports which screen milestones it reached: `BIOS`, `LILO`, `login:`. Exits 77, read as "skipped", when there is no hypervisor. | `DLX_WHP_PATIENCE_SECS`: host-time limit (default 360). `RUST_LOG=rusty_box_whp_engine=trace` shows the engine's memory-exit, park and local-APIC-mode lines, and adding `irq=debug` shows its ExtINT line, only in a debug build: `--release` compiles out `trace!` and `debug!`, which leaves the engine's `warn` and `error` lines. |
| `alpine_bench` | Boots the same Alpine ISO on the interpreter and on the hypervisor and reports the host time each took to reach each milestone. Runs the interpreter arm, then exits 77 if there is no hypervisor. | `ALPINE_ISO` (default `alpine-virt-3.24.1-x86_64.iso` in the workspace root); `ALPINE_PATIENCE_SECS` (default 300); `ALPINE_ENGINE=interpreter` or `whp` runs only that arm. |
| `alpine_probe` | Boots Alpine on one engine a slice at a time and prints `RESULT` lines: progress, the engine's exit census, the share of wall time spent inside the hypervisor run call, milestone times and fatal screen signatures. | `ALPINE_ISO` (required); `ALPINE_ENGINE=interpreter` selects the interpreter, and any other value, or none, the hypervisor; `ALPINE_PROBE_PATIENCE_SECS` (default 300). The hypervisor arm exits 77 when there is no hypervisor. |
| `compute_bench` | Times the same device-free counted loop on both engines. | None. Without a hypervisor it reports the interpreter only and exits 0. |
| `step_bench` | Times per-instruction observation: the interpreter running free, with a hook per instruction, and driven one instruction per call, then the hypervisor single-stepping. | None. Without a hypervisor it reports the interpreter only and exits 0. |

## Continuous integration

`cargo xtask ci` runs `dlxlinux` as its DLX boot gate, with
`RUSTY_BOX_HEADLESS=1` and `MAX_INSTRUCTIONS=450000000`. The gate passes only if
the output contains `*** LOGIN DETECTED ***`. `cargo xtask ci --skip-boot`
leaves it out.

## Linux GUI dependencies

On Ubuntu/Debian, install the display server libraries before building with
`gui-egui`:

```bash
sudo apt install -y libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxinerama-dev libxcursor-dev libxi-dev libgl-dev
```
