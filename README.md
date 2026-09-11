# Rusty Box

A Rust port of the [Bochs](https://bochs.sourceforge.io/) x86 emulator -- a complete 32/64-bit x86 PC emulator with a software interpreter, an optional Windows Hypervisor Platform engine, and a VMware-style desktop and browser GUI.

## Status

- **DLX Linux** boots to an interactive bash shell (BIOS POST, LILO, kernel, init, login)
- **Alpine Linux** (x86_64 Virtual ISO) boots to a login prompt
- Two CPU models: Intel Core i7 Skylake-X (the default) and AMD Ryzen
- Full x87 FPU on Berkeley SoftFloat 3e (80-bit extended precision)
- AVX-512 (F, DQ, CD, BW, VL), AVX2, SSE4.2, AES-NI, SHA, BMI1/BMI2
- Intel VMX and AMD SVM virtualization extensions
- SMP guests (`--cpus`, or `--cpu-sockets` / `--cpu-cores` / `--cpu-threads`)
- PCI (i440FX/PIIX3), IDE with bus-master DMA, ATAPI CD-ROM, HPET, ACPI
- Machine snapshots (save and resume, see the `snapshot_resume` example)
- Runs in the browser via WASM
- The core compiles without `alloc` (no_std, no heap) for bare-metal and UEFI targets
- [UEFI application](examples/rusty_box_uefi/) -- runs on UEFI firmware with no allocator; it completes BIOS POST and reaches the boot sector

## Firmware and disk images

The repository does not ship the BIOS ROMs or the DLX disk image. Both locations are gitignored:

| File | Expected path | Source |
|------|---------------|--------|
| System BIOS | `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest` | a Bochs checkout: `git clone https://github.com/bochs-emu/Bochs cpp_orig/bochs` |
| VGA BIOS | `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin` | the same checkout |
| DLX Linux disk | `dlxlinux/hd10meg.img` | [Bochs disk images](https://bochs.sourceforge.io/diskimages.html) |

The Bochs checkout is also the C++ reference source that every port decision is checked against. The browser builds, the UEFI application and `cargo xtask ci` embed or read these files, so they fail without them. [rusty_box/examples/README.md](rusty_box/examples/README.md) lists the other locations the desktop examples search.

## Quick start

Always build with `--release`. Debug builds are far too slow to boot a guest.

```bash
# The GUI: a VMware-style shell (sidebar tree, VM bar, Summary/Console/Hardware/Images pages)
cargo run --release -p rusty_box_gui -- \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom alpine-virt-3.23.3-x86_64.iso --boot cdrom --memory-mib 256

# DLX Linux, headless (boots to the login prompt)
RUSTY_BOX_HEADLESS=1 cargo run --release --example dlxlinux --features std

# Alpine Linux, headless BIOS boot
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=3500000000 cargo run --release --example alpine_direct --features std

# The full local gate
cargo xtask ci
```

The GUI starts powered off; press **Power on** in the VM bar. It reads a config file only when `--config PATH` names one, and needs a BIOS path from either `--bios` or `rom.bios` in that file. Nothing is picked up from the current directory or its parent. `--no-config` is accepted and changes nothing. [docs/getting-started.md](docs/getting-started.md) walks through the config file and the settings whose meaning is not obvious; [rusty_box_gui/README.md](rusty_box_gui/README.md) covers the shell itself.

The repository's `.cargo/config.toml` sets `MAX_INSTRUCTIONS=20000000000` for every cargo command, so the DLX and Alpine examples, which read it, stop after 20 billion instructions unless you override it.

On Linux, the egui GUI needs the display-server development packages that CI installs:

```bash
sudo apt-get install -y pkg-config libgl1-mesa-dev libx11-dev libxi-dev libxcursor-dev \
  libxrandr-dev libxinerama-dev libxkbcommon-dev libwayland-dev
```

### Android APK quick start

```bash
cargo xtask android build
cargo xtask android run
cargo xtask android screenshot rustybox_android.png
```

The APK runs the same `rusty_box_gui` shell as the desktop, laid out for a phone. Its Browse buttons open an on-device file browser, a Keys pad sends the keys a soft keyboard lacks, and the Bochs ROMs and an Alpine ISO travel inside the APK. `cargo xtask android` installs the Android SDK components, the Rust target and `cargo-apk` as needed; copies the ISO (`--iso PATH`, or `~/Downloads/alpine-virt-3.23.3-x86_64.iso`) to the ignored `rusty_box_gui/assets/alpine.iso`; signs with a generated local dev keystore under your home directory; and uses `adb_client` for install, launch and screenshots. See [xtask/README.md](xtask/README.md#android-commands).

## Execution engines

`rusty_box_gui` runs a guest on one of two engines:

- **Interpreter** (the default, `--engine interpreter`) -- this port of Bochs, on every host and target.
- **Windows Hypervisor Platform** (`--engine whp`, or the Engine setting on the Processors page) -- runs the guest on the host CPU through `WinHvPlatform`. It needs a Windows build of `rusty_box_gui` that carries the hypervisor path (the `hv-whp` feature, on by default, and no `guest-trace`) and a Windows host with the Hypervisor Platform enabled. When the build lacks the path or the host lacks the platform, `--engine whp` is refused instead of falling back to the interpreter. A process can hold only one hypervisor partition at a time.

`--cpu-capabilities host-shared` narrows the guest's CPU model to what the host can also carry, which a machine that may run on the hypervisor needs. The default, `preset`, offers this port's own model, the same on every host. [docs/whp-guest-capabilities.md](docs/whp-guest-capabilities.md) explains why.

The engine is part of the machine's type: `MachineBuilder::new(config).build()` builds an interpreter machine, and `build_on::<WhpEngine>()` builds a hypervisor one.

## Examples

Developer harnesses in `rusty_box/examples/`, each run with `cargo run --release --example <name> --features <features>`:

| Example | Features | What it does |
|---------|----------|--------------|
| `dlxlinux` | `std` | DLX Linux in the terminal, or headless with `RUSTY_BOX_HEADLESS=1` (the CI boot gate) |
| `dlxlinux_egui` | `std,gui-egui` | DLX Linux in an egui window |
| `rusty_box_egui` | `std,gui-egui` | One window for DLX or Alpine (`RUSTY_BOX_BOOT=dlx\|alpine\|alpine-direct`; picks Alpine when it finds an ISO) |
| `alpine_direct` | `std` | Alpine Linux from the ISO, BIOS or direct kernel boot |
| `alpine` | `std` | Alpine Linux from a disk image or ISO |
| `alpine_strace` | `std,gui-egui` | Alpine with guest syscall tracing |
| `shellcode_trace` | `std` | Runs a Linux x86-64 shellcode in flat long mode and intercepts its syscalls with a `pre_syscall` hook |
| `snapshot_resume` | `std` | Boots a guest from CD, saves a snapshot, and restores it in a fresh process (set `RB_ISO` to a bootable ISO) |
| `perfbench` | `std` | CPU-core microbenchmark, needing no BIOS or disk (`cargo xtask perf-baseline` archives a build of it) |

The WHP engine's harnesses are in `rusty_box_whp_engine/examples/` (`dlx_whp`, `alpine_probe`, `alpine_bench`, `compute_bench`, `step_bench`). [rusty_box/examples/README.md](rusty_box/examples/README.md) documents the environment variables.

## Getting an Alpine Linux ISO

1. Visit [alpinelinux.org/downloads](https://alpinelinux.org/downloads/).
2. Download the **Virtual** x86_64 ISO (e.g. `alpine-virt-3.23.3-x86_64.iso`, the name `alpine_direct` looks for by default).
3. Set `ALPINE_ISO=/path/to/alpine.iso`, or place the file in the project root as `alpine-virt-3.23.3-x86_64.iso` and run from the root. The `rusty_box_egui` example also picks up any `.iso` whose name contains `alpine` in the workspace root, its parent, or the current directory.

The browser builds upload the ISO from the page instead.

## Browser builds

Two front ends build for `wasm32-unknown-unknown` with [trunk](https://trunkrs.dev/) (`rustup target add wasm32-unknown-unknown`, `cargo install --locked trunk`). Both compile the BIOS ROMs in, so the Bochs checkout above must be present:

```bash
# The GUI's browser shell
cd rusty_box_gui && trunk serve --release --port 8080

# The standalone web demo: boots the embedded DLX image (needs dlxlinux/hd10meg.img) or an uploaded Alpine ISO
cd examples/rusty_box_web && trunk serve --release --port 8080
```

Then open `http://localhost:8080`.

## Architecture

```
Emulator<T: Instrumentation = (), E = SoftwareEngine>
+-- cpus            one BxCpuC<T> per processor, boot processor at index 0
|                   (the model is runtime data: CpuModel::Corei7SkylakeX | CpuModel::AmdRyzen)
+-- engine: E       what retires guest instructions: SoftwareEngine (the interpreter)
|                   or rusty_box_whp_engine::WhpEngine
+-- BxMemC          memory (block-based, supports >4 GB)
+-- BxDevicesC      I/O port dispatch (65536 ports, fixed arrays)
+-- DeviceManager   devices: interrupt fabric, PIT, CMOS, DMA, keyboard, HPET, IDE,
|                   VGA, ACPI, PCI host and ISA bridges, serial, fw_cfg
+-- BxPcSystemC     timers and the A20 line
+-- gui             NoGui, TermGui or EguiGui [alloc only]
```

`T` is the instrumentation hook, a type rather than a feature: `()` observes nothing, and its empty hook mask makes the CPU skip every dispatch. Instructions execute on an `ExecCtx`, which holds disjoint borrows of one CPU and the machine parts it touches for one scheduler slice.

## Feature flags (`rusty_box`)

| Flag | Default | Description |
|------|---------|-------------|
| `std` | yes | Standard library: terminal display, file-backed disks, tempfile. Implies `alloc`. |
| `alloc` | yes (via `std`) | Heap allocation. Enables `MachineBuilder::build()`, the display front ends, diagnostics and `StopHandle`. |
| `gui-egui` | yes | egui/eframe display (`EguiGui`). |
| `profiling` | no | Profiling counters in the CPU loop. Implies `std`. |
| `bench` | no | Criterion benchmarks (`benches/cpu_bench.rs`). Implies `std`. |

`rusty_box_gui` has its own features: `gui-egui` and `hv-whp` (both default), `guest-trace` and `windows-gui-subsystem`.

### Build configurations

```bash
# The emulator library with its default features
cargo build --release

# no_std + no_alloc: core emulation only, no heap
cargo check --release -p rusty_box --no-default-features

# no_std + alloc: adds MachineBuilder::build(), display front ends, diagnostics
cargo check --release -p rusty_box --no-default-features --features alloc

# UEFI application: no allocator, placement construction
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi
```

### Key design principles

- **No global state** -- each `Emulator` is fully self-contained. Interpreter machines can run side by side; a process can hold only one WHP partition at a time.
- **Bochs parity** -- every behavioural divergence from the Bochs C++ source is a bug. The deliberate exceptions are registered, with evidence, in [docs/bochs-parity-divergences.md](docs/bochs-parity-divergences.md).
- **no_std + no_alloc core** -- CPU, memory, decoder, devices and the machine compile without `alloc`; fixed-size arrays and ring buffers replace `Vec`/`VecDeque`.
- **CPU models as data** -- `CpuModel` is a closed runtime enum (Skylake-X, Ryzen); the model answers CPUID leaves and supplies the ISA, VMX and SVM extension bitmasks cached at init.
- **`Send` by derivation** -- the tree has no `unsafe impl Send`; the [Safety Doctrine](docs/safety-doctrine.md) (R0-R9) governs every API and safety seam.

## Project structure

```
rusty_box/
+-- rusty_box/                 # Emulator library: CPU, memory, machine, I/O devices, examples
|   +-- src/cpu/               # CPU (instruction handlers, mirrors Bochs cpu/)
|   +-- src/memory/            # Memory subsystem
|   +-- src/iodev/             # Machine-wired devices (PIT, CMOS, IDE, serial, ACPI, HPET, ...)
|   +-- src/emulator/          # Emulator, MachineBuilder, execution engines
|   +-- examples/              # Developer harnesses (DLX, Alpine, egui, tracing, benchmarks)
+-- rusty_box_core/            # no_std, forbid(unsafe) foundations (ring buffer, GPA plan, time units)
+-- rusty_box_devices/         # no_std, forbid(unsafe) device models with no CPU or bus access (VGA, PCI)
+-- rusty_box_decoder/         # x86 instruction decoder
+-- rusty_box_whp_sys/         # Windows Hypervisor Platform FFI leaf (every hypervisor call is confined to its windows.rs)
+-- rusty_box_whp/             # Safe wrapper over the WHP leaf
+-- rusty_box_whp_engine/      # Runs a rusty_box machine's guest on WHP
+-- rusty_box_gui/             # Desktop, Android and browser VM shell (egui): CLI/TOML config, interpreter or WHP engine, disk image creation
+-- rusty_box_bximage/         # bximage-compatible disk image creation
+-- xtask/                     # Local CI gate (cargo xtask ci) and automation
+-- examples/rusty_box_web/    # Standalone WASM web demo
+-- examples/rusty_box_uefi/   # UEFI application (no allocator)
+-- examples/no_alloc_smoke/   # no_alloc link smoke test for x86_64-unknown-none
+-- scripts/                   # Bochs table generators (gen_*.py) and developer tooling
+-- docs/                      # Doctrine, parity registry, user guide, WHP notes
+-- cpp_orig/bochs/            # Bochs checkout: reference source and ROMs (gitignored)
+-- dlxlinux/                  # DLX Linux disk image (gitignored)
```

## Testing

```bash
# The full local gate; run it before every commit
cargo xtask ci
cargo xtask ci --skip-boot   # without the DLX boot gate
cargo xtask ci --full        # plus the integration tests and the GUI release build

# The fast loop
cargo test --release -p rusty_box --lib --features std

# One crate
cargo test --release -p rusty_box_decoder

# Fuzz the decoder (targets: fuzz_fetchdecode64, fuzz_fetchdecode32_32, fuzz_fetchdecode32_16)
cd rusty_box_decoder && cargo +nightly fuzz run fuzz_fetchdecode64
```

A bare `cargo test` tests only the default workspace member (`rusty_box`), in a debug build. `cargo xtask ci` first runs the doctrine ratchets (unsafe-token baselines, no `unsafe impl Send/Sync`, blanket `dead_code` allows, no production `unwrap`/`expect`). It then tests `rusty_box_core`, `rusty_box_devices`, the three WHP crates, the decoder and `rusty_box`; checks `rusty_box` in its no_std, bare-metal and wasm configurations and `rusty_box_gui` for the host and for wasm; and builds the UEFI application. It also runs the public-API and doc examples and the doctrine compile-fail fixtures, and does a debug-assertions check. Last, it boots DLX headlessly to the login prompt. It does not run the tests of `rusty_box_gui` or `rusty_box_bximage`; run those with `cargo test --release -p <crate>`. It needs the rustup targets `x86_64-unknown-none`, `x86_64-unknown-uefi` and `wasm32-unknown-unknown`, plus the firmware and disk image above. [CONTRIBUTING.md](CONTRIBUTING.md) has the details.

## Performance

Throughput depends on the host CPU and on the guest's instruction mix: BIOS real-mode code runs slower than a long-mode kernel or userspace. The GUI's status bar shows the live instruction rate. The `ips` setting calibrates the guest clock against it without limiting speed; [docs/getting-started.md](docs/getting-started.md) explains how to choose it.

## Documentation

- [docs/getting-started.md](docs/getting-started.md) -- using the GUI: config file, pacing, CPU topology, disks, display
- [CONTRIBUTING.md](CONTRIBUTING.md) -- build, test, and the rules every change follows
- [docs/safety-doctrine.md](docs/safety-doctrine.md) -- the Safety Doctrine, rules R0-R9
- [docs/bochs-parity-divergences.md](docs/bochs-parity-divergences.md) -- the registry of deliberate divergences from Bochs
- [docs/bochs-upstream-bugs.md](docs/bochs-upstream-bugs.md) -- upstream Bochs bugs this port does not reproduce
- [docs/whp-guest-capabilities.md](docs/whp-guest-capabilities.md) -- what a machine on the hypervisor may advertise to its guest
- Crate READMEs: [rusty_box_gui](rusty_box_gui/README.md), [examples](rusty_box/examples/README.md), [rusty_box_web](examples/rusty_box_web/README.md), [rusty_box_uefi](examples/rusty_box_uefi/README.md), [rusty_box_bximage](rusty_box_bximage/README.md), [xtask](xtask/README.md)

## References

- [Bochs x86 Emulator](https://bochs.sourceforge.io/)
- [Intel Software Developer Manual, Volume 2](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html) (Instruction Set Reference)
- [Sandpile.org](https://www.sandpile.org/) (x86 opcode maps)

## License

This project is a derivative work of the [Bochs](https://bochs.sourceforge.io/) x86 emulator
and is licensed under the [GNU Lesser General Public License v2.1](LICENSE) (LGPL-2.1-or-later).

See [THIRD-PARTY-LICENSES](THIRD-PARTY-LICENSES) for bundled third-party code (Berkeley SoftFloat 3e, Hauser FPU transcendentals).
