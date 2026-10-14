# Rusty Box

Rusty Box is a Rust port of the [Bochs](https://bochs.sourceforge.io/) x86 emulator: a whole 32/64-bit x86 PC. It runs guests on its own software interpreter on every host, or on the Windows Hypervisor Platform (WHP) on Windows. Its front end, Rusty Box Workstation, is a VMware-style VM shell that runs on the desktop, on Android and in the browser, and runs several VMs at once.

## What runs today

| Guest | What works | Engine |
|-------|------------|--------|
| Windows XP | Installs and runs. A full install took about five hours on a phone. | Interpreter |
| Windows 10 22H2 | Setup runs but has not finished: on a phone it stayed at "Please wait", its animation still moving, after 15 hours. | Interpreter |
| Windows 7 SP1 | Setup starts and reaches the edition picker. | Interpreter |
| Alpine Linux 3.24.1 | Boots to `login:` and a working root shell. | Interpreter and WHP |
| Ubuntu Server 26.04 | Boots to its installer. | Interpreter and WHP |
| DLX Linux | Boots to a bash shell. `cargo xtask ci` boots it to its login prompt on every run. | Interpreter |

No guest has a network: the machine has no network adapter. No Windows guest has been tried on WHP. The settings each guest runs with are in [Running guests](docs/getting-started.md#running-guests).

## Quick start

Always build with `--release`. Debug builds are far too slow to boot a guest.

1. Get the Bochs ROMs (see [Firmware and disk images](#firmware-and-disk-images)).
2. Run the shell:

   ```bash
   cargo run --release -p rusty_box_gui
   ```

3. It opens on the VM library, powered off. A new VM starts at 32 MiB of memory and an IPS target of 4,000,000, too little for any guest above. For Alpine, set the BIOS and VGA BIOS paths under Hardware › Display, the memory to 256 MiB under Hardware › Memory, the IPS target to 300,000,000 under Hardware › Processors, and attach the ISO under Hardware › CD/DVD. Then press `▶ Power on` in the VM bar.

The same machine can be given as flags:

```bash
cargo run --release -p rusty_box_gui -- \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom alpine-virt-3.24.1-x86_64.iso --boot cdrom --memory-mib 256 --ips 300000000
```

On Windows, with the Hypervisor Platform enabled, add `--engine whp` to run it on the hypervisor. A VM given by flags is a temporary VM, marked "(unsaved)", until `Keep in library` on its Summary page keeps it.

[docs/getting-started.md](docs/getting-started.md) walks through the shell, the VM file and the settings for each guest. On Linux, the shell needs the display-server development packages that CI installs:

```bash
sudo apt-get install -y pkg-config libgl1-mesa-dev libx11-dev libxi-dev libxcursor-dev \
  libxrandr-dev libxinerama-dev libxkbcommon-dev libwayland-dev
```

## Firmware and disk images

The repository does not ship the BIOS ROMs or the DLX disk image. Both locations are gitignored:

| File | Expected path | Source |
|------|---------------|--------|
| System BIOS | `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest` | a Bochs checkout: `git clone https://github.com/bochs-emu/Bochs cpp_orig/bochs` |
| VGA BIOS | `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin` | the same checkout |
| DLX Linux disk | `dlxlinux/hd10meg.img` | [Bochs disk images](https://bochs.sourceforge.io/diskimages.html) |

The Bochs checkout is also the C++ reference source that every port decision is checked against. The browser builds, the Android APK, the UEFI application and `cargo xtask ci` embed or read these files, so they fail without them. For Alpine, download the **Virtual** x86_64 ISO from [alpinelinux.org/downloads](https://alpinelinux.org/downloads/); this page and the harnesses use `alpine-virt-3.24.1-x86_64.iso`.

## Where it runs

- **Desktop.** `rusty_box_gui` on Windows, Linux and macOS: a VM library that saves itself, several VMs running at once, and the interpreter or, on Windows, WHP. [rusty_box_gui/README.md](rusty_box_gui/README.md) has the detail.
- **Android.** The same shell as an APK, built and installed by `cargo xtask android build` and `cargo xtask android run`. A finger on the guest's screen works as a trackpad, a Keys pad sends the keys a soft keyboard lacks, and while a VM runs the app keeps running with the screen off, under a notification. It runs the interpreter.
- **Browser.** The shell in a web page, built with [trunk](https://trunkrs.dev/): `cd rusty_box_gui && trunk serve --release --port 8080`. It boots an uploaded ISO and runs the interpreter. `examples/rusty_box_web` is a smaller standalone demo.
- **UEFI.** [examples/rusty_box_uefi](examples/rusty_box_uefi/) runs the emulator on UEFI firmware with no allocator. It completes BIOS POST and reaches the boot sector.

## Execution engines

A VM runs on one of two engines, chosen under Hardware › Processors › Engine or with `--engine`.

- **The interpreter** (`interpreter`, the default) is this port of Bochs. It runs on every host and target, counts every instruction, and can stop a run at `max_instructions`.
- **WHP** (`whp`) runs the guest's instructions on the host processor through the Windows Hypervisor Platform. The devices stay this port's own models, run on host time, except the local APIC, which is the partition's own. It needs a Windows build with the `hv-whp` feature (on by default) and the Hypervisor Platform enabled; when either is missing, `--engine whp` is refused rather than falling back.

Measured with the `alpine_probe` harness at commit `4f46a11` (Intel Core i5-12450H, median of three interleaved runs): Alpine 3.24.1 reaches `login:` in 27.1 s on WHP against 65.0 s on the interpreter, 2.40× faster. BIOS POST is slower on WHP; the gain comes after the boot loader starts. DLX Linux does not reach `login:` on WHP today.

On WHP one processor runs, nothing counts instructions, device timers run on host time, the guest is always offered `host-shared` CPU capabilities, and a process holds one partition, so one VM at a time runs on it. [Execution engines](docs/getting-started.md#execution-engines) in the getting-started guide says which engine suits which guest, and [docs/whp-guest-capabilities.md](docs/whp-guest-capabilities.md) what the guest is offered.

## The emulated PC

**CPU**

- Intel Core i7 Skylake-X is the model every machine runs. An AMD Ryzen model, with AMD SVM, is ported and unit-tested, but `MachineBuilder` and the shell cannot select it.
- Full x87 FPU on Berkeley SoftFloat 3e (80-bit extended precision).
- AVX-512 (F, DQ, CD, BW, VL), AVX2, SSE4.2, AES-NI, SHA, BMI1/BMI2.
- Intel VMX, implemented and unit-tested. No run of a hypervisor inside a guest is recorded.
- SMP guests on the interpreter (`--cpus`, or `--cpu-sockets` / `--cpu-cores` / `--cpu-threads`). The WHP engine runs one processor.
- System Management Mode.

**Chipset and devices**

- the i440FX PCI host bridge and the PIIX3 PCI-to-ISA bridge; PCI can be switched off;
- the 8259 PIC pair and an I/O APIC, and a local APIC per processor, with x2APIC (on WHP the local APIC is the hypervisor partition's own);
- the 8254 PIT, the CMOS real-time clock, the 8237 DMA controller and the HPET;
- PIIX3 IDE with bus-master DMA, for hard disks (32 GB and larger) and an ATAPI CD-ROM;
- the 8042 keyboard controller, with a PS/2 keyboard and a PS/2 mouse;
- VGA with the Bochs VBE extensions. It can also be registered on PCI as 1234:1111, for Linux `bochs-drm`; that option is experimental;
- a 16550A UART on COM1 (ports `0x3F8`–`0x3FF`); there is no COM2 to COM4;
- PIIX4 ACPI power management, with S5 soft-off and S3 suspend, and SMBus ports;
- QEMU `fw_cfg`;
- port 92h, the A20 gate and fast reset.

There is no network adapter, no sound card (not even the PC speaker), no USB controller and no floppy controller.

**Memory and firmware**

- Guest memory is block-based and can exceed 4 GB. A guest larger than its host-memory budget (`host_memory_mib`) keeps the rest in an overflow file.
- The firmware is the Bochs BIOS and the LGPL VGA BIOS, from a Bochs checkout.

**Snapshots**

- An interpreter machine can be saved and restored in a fresh process; see the `snapshot_resume` example.

## Using it as a library

[docs/automation.md](docs/automation.md) is the task-based guide: boot a machine headless, read the screen, type at it, run and stop it, read and write its memory and registers, watch what it does, and what differs on the hypervisor engine. Its examples are compiled by the gate.

- **Building a machine.** `MachineBuilder::new(config).build()` builds an interpreter machine, and `build_on::<WhpEngine>()` builds a hypervisor one. The engine is part of the machine's type.
- **Instrumentation.** A machine's type parameter `T: Instrumentation` observes the CPU. `()` observes nothing, and its empty hook mask makes the CPU skip every dispatch. A hook such as `pre_syscall` sees each system call: `shellcode_trace` uses it to intercept a shellcode's syscalls, and `alpine_strace` to trace Alpine's.
- **No allocator.** The core compiles without `alloc` (no_std, no heap) for bare-metal and UEFI targets. The [UEFI application](examples/rusty_box_uefi/) places the whole machine in caller-provided memory.

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

```bash
# DLX Linux, headless (boots to the login prompt)
RUSTY_BOX_HEADLESS=1 cargo run --release --example dlxlinux --features std

# Alpine Linux, headless BIOS boot
ALPINE_ISO=alpine-virt-3.24.1-x86_64.iso RUSTY_BOX_HEADLESS=1 cargo run --release --example alpine_direct --features std
```

`alpine_direct` prints the guest's serial console and does not stop at `login:`. The repository's `.cargo/config.toml` sets `MAX_INSTRUCTIONS=20000000000` for every cargo command, so the boot examples stop after 20 billion instructions unless you override it. `alpine_direct` defaults to an older ISO name, `alpine-virt-3.23.3-x86_64.iso`, hence `ALPINE_ISO` above.

The WHP engine's harnesses are in `rusty_box_whp_engine/examples/`: `dlx_whp`, `alpine_probe`, `alpine_bench`, `compute_bench` and `step_bench`. [rusty_box/examples/README.md](rusty_box/examples/README.md) documents the environment variables and where the harnesses look for their images.

## Architecture

```
Emulator<T: Instrumentation = (), E = SoftwareEngine>
+-- cpus            one BxCpuC<T> per processor, boot processor at index 0
|                   (the model is runtime data: CpuModel::Corei7SkylakeX | CpuModel::AmdRyzen;
|                   MachineBuilder always builds Corei7SkylakeX)
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

### Key design principles

- **No global state** -- each `Emulator` is fully self-contained. Interpreter machines run side by side, as the shell's VMs do; a process can hold only one WHP partition at a time.
- **Bochs parity** -- every behavioural divergence from the Bochs C++ source is a bug. The deliberate exceptions are registered, with evidence, in [docs/bochs-parity-divergences.md](docs/bochs-parity-divergences.md).
- **no_std + no_alloc core** -- CPU, memory, decoder, devices and the machine compile without `alloc`; fixed-size arrays and ring buffers replace `Vec`/`VecDeque`.
- **CPU models as data** -- `CpuModel` is a closed runtime enum (Skylake-X, Ryzen); the model answers CPUID leaves and supplies the ISA, VMX and SVM extension bitmasks cached at init. `MachineBuilder` builds every processor as Skylake-X.
- **`Send` by derivation** -- the tree has no `unsafe impl Send`; the [Safety Doctrine](docs/safety-doctrine.md) (R0-R9) governs every API and safety seam.

## Feature flags (`rusty_box`)

| Flag | Default | Description |
|------|---------|-------------|
| `std` | yes | Standard library: terminal display, file-backed disks, tempfile. Implies `alloc`. |
| `alloc` | yes (via `std`) | Heap allocation. Enables `MachineBuilder::build()`, the display front ends, diagnostics and `StopHandle`. |
| `gui-egui` | yes | egui/eframe display (`EguiGui`). |
| `profiling` | no | Profiling counters in the CPU loop. Implies `std`. |
| `bench` | no | Criterion benchmarks (`benches/cpu_bench.rs`). Implies `std`. |

`rusty_box_gui` has its own features: `gui-egui` and `hv-whp` (both default), `guest-trace` and `windows-gui-subsystem`.

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

## Project structure

```
rusty_box/
+-- rusty_box/                 # Emulator library: CPU, memory, machine, I/O devices, examples
|   +-- src/cpu/               # CPU (instruction handlers, mirrors Bochs cpu/)
|   +-- src/memory/            # Memory subsystem
|   +-- src/iodev/             # Most machine-wired devices (PIT, CMOS, IDE, serial, ACPI, HPET, ...)
|   +-- src/pic.rs, src/dma.rs # The 8259 PIC pair and the 8237 DMA controller
|   +-- src/emulator/          # Emulator, MachineBuilder, execution engines
|   +-- examples/              # Developer harnesses (DLX, Alpine, egui, tracing, benchmarks)
+-- rusty_box_core/            # no_std, forbid(unsafe) foundations (the engine seam and GPA plan, ring buffer, time units, snapshot sections)
+-- rusty_box_devices/         # no_std, forbid(unsafe) device models with no CPU or bus access (VGA, PCI)
+-- rusty_box_decoder/         # x86 instruction decoder
+-- rusty_box_whp_sys/         # Windows Hypervisor Platform FFI leaf (every hypervisor call is confined to its windows.rs)
+-- rusty_box_whp/             # Safe wrapper over the WHP leaf
+-- rusty_box_whp_engine/      # Runs a rusty_box machine's guest on WHP
+-- rusty_box_gui/             # Desktop, Android and browser VM shell (egui); android/ holds the APK's manifest and service
+-- rusty_box_bximage/         # bximage-compatible disk image creation
+-- xtask/                     # Local CI gate (cargo xtask ci) and the Android build
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

A bare `cargo test` tests only the default workspace member (`rusty_box`), in a debug build.

`cargo xtask ci` runs 29 steps; [xtask/README.md](xtask/README.md) lists every one.

1. The doctrine ratchets, over eight source trees, the GUI's included. They check the unsafe-token baselines, that there is no `unsafe impl Send/Sync`, the blanket `dead_code` allows, and that there is no production `unwrap`/`expect`.
2. A 27-step matrix:
   - It tests `rusty_box_core`, `rusty_box_devices`, the three WHP crates, the decoder and `rusty_box`.
   - It checks `rusty_box_core` and `rusty_box_devices` for bare metal, and builds the WHP probe examples.
   - It checks `rusty_box` without std (with and without alloc), for bare metal, for wasm, with all features, and with debug assertions.
   - It checks `rusty_box_gui` for the host, with its tests, for wasm, and with its `guest-trace` feature, whose tracer tests it runs.
   - It builds the UEFI application.
   - It runs the public-API and doc examples and the doctrine compile-fail fixtures.
3. The DLX boot gate: DLX boots headlessly to its login prompt, on the interpreter. It is the only guest boot in the gate.

The gate does not run the tests of `rusty_box_bximage` or `xtask`, nor those of `rusty_box_gui` beyond the guest-trace tracer's, and does not build the Android APK; run those with `cargo test --release -p <crate>` and `cargo xtask android build`.

It needs the rustup targets `x86_64-unknown-none`, `x86_64-unknown-uefi` and `wasm32-unknown-unknown`, plus the firmware and disk image above. [CONTRIBUTING.md](CONTRIBUTING.md) has the details.

## Documentation

**Using Rusty Box**

- [docs/getting-started.md](docs/getting-started.md) -- using the shell: the guests and their settings, the VM file, pacing, CPU topology, disks, display
- [rusty_box_gui/README.md](rusty_box_gui/README.md) -- the shell in full: the VM library, pages, engines, disk images, Android, browser
- [docs/automation.md](docs/automation.md) -- driving a guest from Rust: boot it headless, read the screen, type at it, watch what it does
- [docs/whp-guest-capabilities.md](docs/whp-guest-capabilities.md) -- what a machine on the hypervisor may advertise to its guest

**Contributing**

- [CONTRIBUTING.md](CONTRIBUTING.md) -- build, test, and the rules every change follows
- [docs/safety-doctrine.md](docs/safety-doctrine.md) -- the Safety Doctrine: rules R0-R9 for contributors, each with a code sample
- [docs/bochs-parity-divergences.md](docs/bochs-parity-divergences.md) -- the registry of deliberate divergences from Bochs
- [docs/bochs-upstream-bugs.md](docs/bochs-upstream-bugs.md) -- upstream Bochs bugs this port does not reproduce
- Crate READMEs: [examples](rusty_box/examples/README.md), [rusty_box_web](examples/rusty_box_web/README.md), [rusty_box_uefi](examples/rusty_box_uefi/README.md), [rusty_box_bximage](rusty_box_bximage/README.md), [xtask](xtask/README.md)

## References

- [Bochs x86 Emulator](https://bochs.sourceforge.io/)
- [Intel Software Developer Manual, Volume 2](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html) (Instruction Set Reference)
- [Sandpile.org](https://www.sandpile.org/) (x86 opcode maps)

## License

This project is a derivative work of the [Bochs](https://bochs.sourceforge.io/) x86 emulator
and is licensed under the [GNU Lesser General Public License v2.1](LICENSE) (LGPL-2.1-or-later).

See [THIRD-PARTY-LICENSES](THIRD-PARTY-LICENSES) for bundled third-party code (Berkeley SoftFloat 3e, Hauser FPU transcendentals).
