# Rusty Box

A Rust port of the [Bochs](https://bochs.sourceforge.io/) x86 emulator — a complete CPU/system emulator targeting 32/64-bit x86 architecture with virtualization support.

## Status

- **DLX Linux** boots to an interactive bash shell (BIOS POST, LILO, kernel, init, login)
- **Alpine Linux** fully boots through OpenRC with all packages installed
- Full x87 FPU with Berkeley SoftFloat 3e (80-bit extended precision)
- AVX-512 Foundation (320 handlers), AVX2, SSE4.2, AES-NI, SHA, BMI1/BMI2
- Bus Master DMA, ATAPI CD-ROM, PCI IDE
- Runs in the browser via WASM (egui frontend)

## Quick Start

```bash
# Build with optimizations (required for acceptable performance)
cargo build --release --features std

# DLX Linux — headless (boots to login prompt)
RUSTY_BOX_HEADLESS=1 cargo run --release --example dlxlinux --features std

# DLX Linux — with GUI
cargo run --release --example rusty_box_egui --features "std,gui-egui"

# Alpine Linux — headless BIOS boot
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=3500000000 cargo run --release --example alpine_direct --features std

# Run tests
cargo test

# WASM build
cd examples/rusty_box_web && trunk serve
```

## Getting Alpine Linux ISO

To run Alpine Linux in the emulator:

1. Visit [alpinelinux.org/downloads](https://alpinelinux.org/downloads/)
2. Download the **Virtual** x86 ISO (e.g., `alpine-virt-3.21.3-x86.iso`)
3. Place it in the project root or set `ALPINE_ISO=/path/to/alpine.iso`

The web version supports uploading the ISO directly from the browser.

## Architecture

```
Emulator<'a, I: BxCpuIdTrait>
+-- BxCpuC<I>         CPU (generic over CPUID model like Corei7SkylakeX)
+-- BxMemC            Memory subsystem (block-based, supports >4GB)
+-- BxDevicesC        I/O port handler manager
+-- DeviceManager     Hardware (PIC, PIT, CMOS, DMA, VGA, Keyboard, IDE)
+-- BxPcSystemC       Timers and A20 line control
+-- GUI               Display (NoGui, TermGui, or EguiGui)
```

### Key Design Principles

- **No global state** — each `Emulator<I>` is fully self-contained; multiple instances can run concurrently
- **Bochs parity** — all logic matches the Bochs C++ source; deviations are bugs
- **no_std compatible** — core library works without std; `std` feature enables file I/O and terminal GUI
- **Type-safe CPU models** — `BxCpuIdTrait` makes CPU model a compile-time type parameter

## Project Structure

```
rusty_box/
+-- rusty_box/              # Main emulator library
|   +-- src/cpu/            # CPU (instruction handlers, mirrors Bochs cpu/)
|   +-- src/memory/         # Memory subsystem
|   +-- src/iodev/          # I/O devices (PIC, PIT, CMOS, VGA, IDE, etc.)
|   +-- examples/           # Desktop examples (DLX, Alpine, egui GUI)
+-- rusty_box_decoder/      # x86 instruction decoder (separate crate)
+-- examples/rusty_box_web/ # WASM web frontend
```

## Web Demo

The WASM frontend provides a browser-based emulator with two boot options:

- **DLX Linux** — embedded 10 MB disk image, boots instantly
- **Alpine Linux** — upload your own ISO via file picker

Build and run locally:

```bash
cd examples/rusty_box_web
trunk serve
```

Then open `http://localhost:8080` in your browser.

## Testing

```bash
# Run all tests
cargo test

# Fuzz the decoder
cd rusty_box_decoder && cargo +nightly fuzz run fuzz_target_1
```

## Performance

Release build on modern hardware: ~22-40 MIPS depending on workload phase.

| Phase | MIPS |
|-------|------|
| BIOS real-mode | ~22 |
| Kernel decompressor | ~25-50 |
| Kernel init | ~22-27 |
| Alpine steady-state | ~40 |

## References

- [Bochs x86 Emulator](https://bochs.sourceforge.io/)
- [Intel Software Developer Manual, Volume 2](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html) (Instruction Set Reference)
- [Sandpile.org](https://www.sandpile.org/) (x86 opcode maps)

## License

This project is a derivative work of the [Bochs](https://bochs.sourceforge.io/) x86 emulator
and is licensed under the [GNU Lesser General Public License v2.1](LICENSE) (LGPL-2.1-or-later).

See [THIRD-PARTY-LICENSES](THIRD-PARTY-LICENSES) for bundled third-party code (Berkeley SoftFloat 3e, Hauser FPU transcendentals).
