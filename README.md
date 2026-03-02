# Rusty Box

A Rust port of the Bochs x86 emulator — a complete CPU/system emulator targeting 32/64-bit x86 architecture with virtualization support (VMX/SVM).

## Project Status

**Current State:** Linux 1.3.89 boots to driver initialization

- **Mode:** Protected mode with paging (CR0=0x80000013, kernel 3G/1G split)
- **Instructions Executed:** 1B+ clean (no crashes, no errors)
- **Boot Stage:** Kernel stalls after "loop: registered device at major 7" — waiting for ATA disk I/O
- **Performance:** 14–29 MIPS active execution (windowed, by phase):
  - BIOS real-mode: ~22 MIPS
  - Kernel decompressor: ~29 MIPS *(exceeds Bochs target of ~14.7 MIPS)*
  - Kernel init: ~14 MIPS
  - Idle (HLT): ~0 MIPS (waiting for IRQ14)

See [docs/DLXLINUX_BOOT_SUMMARY.md](docs/DLXLINUX_BOOT_SUMMARY.md) for the full boot timeline.

## Documentation

- **[CLAUDE.md](CLAUDE.md)** — Build commands, architecture, known issues, and development guidance. Start here.
- **[docs/DLXLINUX_BOOT_SUMMARY.md](docs/DLXLINUX_BOOT_SUMMARY.md)** — Full boot sequence timeline, critical bug fixes, and remaining issues.

## Quick Start

```bash
# Build with optimizations (required for performance)
cargo build --release --features std

# Run headless (fast)
RUSTY_BOX_HEADLESS=1 cargo run --release --example dlxlinux --features std

# Run with egui GUI
cargo run --release --example dlxlinux_egui --features "std,gui-egui"

# Run tests
cargo test
```

## Architecture

```
Emulator<'a, I: BxCpuIdTrait>
├── BxCpuC<I>       CPU (generic over CPUID model)
├── BxMemC          Memory subsystem (block-based, >4GB support)
├── BxDevicesC      I/O port dispatch
├── DeviceManager   Hardware (PIC, PIT, CMOS, DMA, VGA, Keyboard, IDE)
├── BxPcSystemC     Timers and A20 line
└── GUI             Display (NoGui, TermGui, or EguiGui)
```

### Key Design Principles

- **No global state** — each `Emulator<I>` is fully self-contained; multiple instances can run concurrently
- **Bochs parity** — all logic must match Bochs C++ source exactly; deviations are bugs
- **no_std compatible** — core library works without std; `std` feature enables file I/O and terminal GUI
- **Type-safe CPU models** — `BxCpuIdTrait` makes CPU model a compile-time type (Corei7SkylakeX, etc.)

## Performance Optimizations (2026-03-02)

| Optimization | Impact |
|---|---|
| `fetch_decode32_inplace` — decoder writes directly into icache mpool | Eliminates 24-byte copy per instruction |
| Raw pointer execute loop | Eliminates bounds-check overhead per dispatch |
| Trace allocation guard | `format!` / `collect::<String>()` gated on `tracing::enabled!(TRACE)` |
| **Result** | **14–29 MIPS active** (up from ~3–4 MIPS full-run average) |

## What Works

- Full BIOS POST: rombios32_init, VGA BIOS, ATA detection, LILO boot
- LILO boot loader: reads map file, loads linux image via INT 13h
- Linux 1.3.89 kernel: decompresses, enables paging, initializes all drivers listed below:
  ```
  Linux version 1.3.89 (root@merlin) (gcc version 2.7.2)
  Console: colour VGA+ 80x25, 1 virtual console (max 63)
  Calibrating delay loop.. ok - 9.98 BogoMIPS
  Memory: 31140k/32768k available
  NET3.034, TCP/IP, ICMP/UDP/TCP
  Checking 386/387 coupling... Ok, fpu using old IRQ13 error reporting
  Checking 'hlt' instruction... Ok.
  Serial driver version 4.11a
  PS/2 auxiliary pointing device detected -- driver installed.
  loop: registered device at major 7
  ```
- Complete x86 instruction set coverage (all instruction categories audited against Bochs)
- Full x87 FPU with Berkeley SoftFloat 3e (80-bit extended precision, Float128 transcendentals)
- VGA text mode output (egui and headless terminal)
- Comprehensive Bochs parity audit complete across all CPU, device, and memory files

## What's Next

1. **ATA disk read for rootfs** — kernel stalls waiting for disk I/O after driver init
2. **Init process startup** — `/sbin/init` needs to run after rootfs mounts
3. **DLX login prompt** — full boot goal: `dlx login:`

## Project Structure

```
rusty_box/
├── rusty_box/              # Main emulator library
│   ├── src/cpu/            # CPU implementation (by instruction category, mirrors Bochs)
│   ├── src/memory/         # Memory subsystem
│   ├── src/iodev/          # I/O devices (PIC, PIT, CMOS, VGA, IDE, etc.)
│   └── examples/           # Runnable examples
├── rusty_box_decoder/      # x86 instruction decoder (separate crate, fuzzing-friendly)
└── cpp_orig/bochs/         # Original C++ Bochs source (reference for parity audit)
```

## Testing

```bash
# Run all tests
cargo test

# Fuzz the decoder
cd rusty_box_decoder && cargo +nightly fuzz run fuzz_target_1

# WASM build
cd examples/rusty_box_web && cargo build --target wasm32-unknown-unknown
```

## References

- Original Bochs: [bochs.sourceforge.io](http://bochs.sourceforge.io/)
- Intel Manual: Volume 2 (Instruction Set Reference)
- x86 Opcode Map: sandpile.org

## License

See original Bochs licensing in `cpp_orig/bochs/`. This Rust port follows the same terms.

---

**Last Updated:** 2026-03-02
**Current Focus:** ATA disk I/O to unblock kernel rootfs mount
**Status:** 🟢 Active Development — Linux kernel boots to driver init
