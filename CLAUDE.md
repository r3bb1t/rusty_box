# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Rules

- Do NOT commit unless the user explicitly tells you to.
- Do NOT push unless the user explicitly tells you to.
- Prefer LSP tools (definition, references, hover, diagnostics) over grep for code navigation.
- Use `lsp references` before modifying any function, type, or exported symbol to find all consumers.
- **Never skip an inaccuracy against the original Bochs.** Every divergence from `cpp_orig/bochs/` is a bug, *unless* the divergence is purely an improvement to idiomatic Rust (enum instead of magic-number int, bitflags instead of raw u32, RAII instead of manual lifecycle, etc.) that does not change observable behavior. If a Bochs feature is too large to fix in the current change, **call it out explicitly** in the end-of-work overview so the user can decide whether to defer it — never silently leave it as a stub or `// not implemented yet` comment.
- **Thread safety trumps Bochs literalness.** If a Bochs construct is non-thread-safe (sequenced read+write that says "should be atomic RMW", shared state accessed without a lock, etc.), fix it with Rust atomics / Mutex / lock-free primitives — even when Bochs itself doesn't. Single-threaded-per-CPU Bochs assumptions silently transfer correctness obligations to the caller; don't inherit them. Scope: cross-thread / shared state only; CPU-local state doesn't need atomics.
- Bochs-source comments must cite the file + symbol (e.g. `// Bochs cet.cc INCSSPD`), never the specific line number — the upstream snapshot rebases.

## Project Overview

Rusty Box is a Rust port of the Bochs x86 emulator -- a complete CPU/system emulator targeting 32/64-bit x86 architecture. The original C++ Bochs source is in `cpp_orig/bochs/` for reference.

**Status:** DLX Linux boots to interactive bash shell. Alpine Linux fully boots. UEFI example completes BIOS POST and reaches boot sector.

## Build Commands

```bash
cargo build --release --all-features          # Full build
cargo test                                    # Run tests (187)
cargo run --release --example dlxlinux --features std            # DLX headless
cargo run --release --example rusty_box_egui --features "std,gui-egui"  # GUI
cd examples/rusty_box_web && trunk serve      # WASM dev server
cargo check --no-default-features -p rusty_box  # no_std + no_alloc build
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi  # UEFI app
```

## Architecture

```
Emulator<'a, I: BxCpuIdTrait>
+-- BxCpuC<I>         CPU (generic over CPUID model like Corei7SkylakeX)
+-- BxMemC            Memory subsystem (block-based, supports >4GB)
+-- BxDevicesC        I/O port handler manager (65536 ports, fixed arrays)
+-- DeviceManager     Hardware devices (PIC, PIT, CMOS, DMA, VGA, Keyboard, IDE, Serial)
+-- BxPcSystemC       Timers and A20 line control
+-- GUI               Display (NoGui, TermGui, or EguiGui) [alloc only]
```

### Key Design Principles

- **No global state** -- each `Emulator<I>` is fully self-contained
- **Bochs parity** -- all logic must match Bochs C++ source exactly; deviations are bugs
- **no_std + no_alloc core** -- CPU, memory, decoder, I/O devices, emulator all compile without alloc. Fixed-size arrays and RingBuffer replace Vec/VecDeque. Alloc-dependent features (GUI, diagnostic String returns, StopHandle, hook closures) are behind `#[cfg(feature = "alloc")]`.

### no_alloc Construction (UEFI path)

```rust
// Placement construction -- no Box, no allocator
BxCpuBuilder::<I>::init_cpu_at(cpu_ptr, tracer)     // CPU at raw pointer
BxMemoryStubC::create_from_raw(ptr, len, ...)       // Memory from raw buffer
Emulator::init_at(emu_ptr, cpu, mem_stub, config)   // Emulator at raw pointer
```

## Workspace Structure

- **rusty_box/** -- Main emulator library
- **rusty_box_decoder/** -- Separate crate for x86 instruction decoding
- **examples/rusty_box_web/** -- WASM web frontend
- **examples/rusty_box_uefi/** -- UEFI bootable emulator application (no allocator)
- **cpp_orig/bochs/** -- Original C++ Bochs source for reference

## Key Files for Common Tasks

| Task | Files |
|------|-------|
| Add new instruction | `rusty_box_decoder/src/fetchdecode*.rs`, `rusty_box/src/cpu/<category>/` |
| Add new I/O device | `rusty_box/src/iodev/` (new file), `iodev/devices.rs` (registration) |
| Modify memory mapping | `rusty_box/src/memory/misc_mem.rs`, `memory/mod.rs` |
| Add/modify FPU instruction | `rusty_box/src/cpu/fpu/` (handlers), `cpu/softfloat3e/` (math) |
| Ring buffer (replaces VecDeque) | `rusty_box/src/ring_buffer.rs` |

## Feature Flags

- `std` -- Standard library support (terminal, file I/O, tempfile). Implies `alloc`.
- `alloc` -- Heap allocation. Enables `Emulator::new()`, GUI, diagnostic methods, StopHandle.
- `gui-egui` -- Graphical UI using egui.
- `instrumentation` -- Closure-based CPU hooks. Implies `alloc`.
- `bx_debugger` -- Built-in debugger.
- `bx_gdb_stub` -- GDB remote stub.
- `profiling` -- Profiling support. Implies `std`.
