# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Rules

- Do NOT commit unless the user explicitly tells you to.
- Do NOT push unless the user explicitly tells you to.
- Prefer LSP tools (definition, references, hover, diagnostics) over grep for code navigation.
- Use `lsp references` before modifying any function, type, or exported symbol to find all consumers.

## Project Overview

Rusty Box is a Rust port of the Bochs x86 emulator — a complete CPU/system emulator targeting 32/64-bit x86 architecture. The original C++ Bochs source is in `cpp_orig/bochs/` for reference.

**Status:** DLX Linux boots to interactive bash shell. Alpine Linux fully boots.

## Build Commands

```bash
cargo build --release --all-features          # Full build
cargo test                                    # Run tests
cargo run --release --example dlxlinux --features std            # DLX headless
cargo run --release --example rusty_box_egui --features "std,gui-egui"  # GUI
cd examples/rusty_box_web && trunk serve      # WASM dev server
cargo check --no-default-features -p rusty_box  # no_std + no_alloc build
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi  # UEFI app
```

## Architecture

```
Emulator<'a, I: BxCpuIdTrait>
├── BxCpuC<I>         CPU (generic over CPUID model like Corei7SkylakeX)
├── BxMemC            Memory subsystem (block-based, supports >4GB)
├── BxDevicesC        I/O port handler manager
├── DeviceManager     Hardware devices (PIC, PIT, CMOS, DMA, VGA, Keyboard, HardDrive)
├── BxPcSystemC       Timers and A20 line control
└── GUI               Display (NoGui, TermGui, or EguiGui)
```

### Key Design Principles

- **No global state** — each `Emulator<I>` is fully self-contained
- **Bochs parity** — all logic must match Bochs C++ source exactly; deviations are bugs
- **no_std compatible** — core emulation (CPU, memory, decoder, params, pc_system) compiles without alloc. Only iodev, emulator, gui, snapshot require alloc/std.

## Workspace Structure

- **rusty_box/** -- Main emulator library
- **rusty_box_decoder/** -- Separate crate for x86 instruction decoding
- **examples/rusty_box_web/** -- WASM web frontend
- **examples/rusty_box_uefi/** -- UEFI bootable emulator application
- **cpp_orig/bochs/** -- Original C++ Bochs source for reference

## Key Files for Common Tasks

| Task | Files |
|------|-------|
| Add new instruction | `rusty_box_decoder/src/fetchdecode*.rs`, `rusty_box/src/cpu/<category>/` |
| Add new I/O device | `rusty_box/src/iodev/` (new file), `iodev/devices.rs` (registration) |
| Modify memory mapping | `rusty_box/src/memory/misc_mem.rs`, `memory/mod.rs` |
| Add/modify FPU instruction | `rusty_box/src/cpu/fpu/` (handlers), `cpu/softfloat3e/` (math) |

## Feature Flags

- `std` -- Standard library support (terminal, file I/O, tempfile). Implies `alloc`.
- `alloc` -- Heap allocation. Required for Emulator wrapper, iodev, GUI.
- `gui-egui` -- Graphical UI using egui
- `instrumentation` -- Closure-based CPU hooks. Implies `alloc`.
- `bx_debugger` -- Built-in debugger
- `bx_gdb_stub` -- GDB remote stub
- `profiling` -- Profiling support. Implies `std`.
