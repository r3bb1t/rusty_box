---
name: BIOS VGA output + no_std IO wiring
overview: Wire CPU port I/O to the existing device bus so Bochs BIOS/VGABIOS can initialize VGA and produce visible terminal output, while keeping `std` optional and the core crate `no_std + alloc`.
todos:
  - id: wire-cpu-io
    content: Replace CPU IN/OUT stub with calls into BxDevicesC (set/clear io bus pointer around cpu_loop_n).
    status: completed
  - id: plumb-emulator-io
    content: Update Emulator execution path to provide the I/O bus during CPU batches; keep A20 syncing workable.
    status: completed
  - id: port-e9-hack
    content: Implement always-on port 0xE9 read/write behavior in BxDevicesC default handlers with a drainable buffer.
    status: completed
  - id: no-std-gating
    content: Gate TermGui/other std-only GUI code so the core crate stays `no_std + alloc` and `std` remains optional.
    status: completed
  - id: remove-static-mut-debug
    content: Remove/replace `static mut` debug counters in VGA/memory paths with per-instance state.
    status: completed
  - id: validate-dlxlinux
    content: Adjust dlxlinux example paths/limits and verify VGA/BIOS output appears; also verify core crate builds without requiring `std`.
    status: completed
isProject: false
---

## What’s broken today (root cause)

- **CPU `IN/OUT` is still stubbed**, so BIOS can’t talk to VGA/PIC/CMOS/keyboard/etc.
- Current stub: [`rusty_box/src/cpu/io.rs`](rusty_box/src/cpu/io.rs)
- Your device bus is already implemented and populated with handlers (`BxDevicesC::inp/outp`), but never called from the CPU.

## Target behavior (matching Bochs logic)

- BIOS ROM mapped at `0xfffe0000` and visible also via the legacy `0xE0000-0xFFFFF` window (already implemented in memory).
- BIOS uses port I/O to configure VGA, then writes to VGA text memory `0xB8000..`.
- Your `TermGui` renders VGA text memory to the host terminal.
- Also implement Bochs-style **port `0xE9` hack** (always-on per your choice):
- read from `0xE9` returns `0xE9`
- write to `0xE9` appends a byte to a host-drainable buffer (no `std` I/O in core).

## Implementation outline

### 1) Connect CPU `port_in/port_out` to `BxDevicesC`

- **Add an optional I/O-bus pointer inside the CPU** (raw pointer/`NonNull`), set only for the duration of a CPU run call.
- Files:
- [`rusty_box/src/cpu/cpu.rs`](rusty_box/src/cpu/cpu.rs) (add a field + small helper methods)
- [`rusty_box/src/cpu/io.rs`](rusty_box/src/cpu/io.rs) (replace stub logic)
- **Change the helpers to use `&mut self`**, so reads can mutate devices if needed (VGA flip-flops etc.).
- Example shape:
- `port_in(&mut self, port, len) -> u32` calls `devices.inp(port, len)`
- `port_out(&mut self, port, val, len)` calls `devices.outp(port, val, len)`
- Ensure the pointer is **set/cleared** around `cpu_loop_n` so it can’t be used accidentally outside execution.

### 2) Plumb the bus through emulator execution

- Update the execution entrypoints to ensure the CPU has access to the I/O bus.
- Files:
- [`rusty_box/src/emulator.rs`](rusty_box/src/emulator.rs)
- optionally [`rusty_box/src/cpu/cpu.rs`](rusty_box/src/cpu/cpu.rs) if we add a `cpu_loop_n_with_io(...)` wrapper.
- In `run_interactive`, pass `&mut self.devices` into the CPU loop batches.

### 3) Always-on port `0xE9` hack (no_std compliant)

- Implement in the **unhandled-port path** of `BxDevicesC` so it works even without a dedicated device.
- File: [`rusty_box/src/iodev/mod.rs`](rusty_box/src/iodev/mod.rs)
- Behavior (mirrors Bochs `unmapped.cc`):
- `inp(0xE9, 1)` returns `0xE9`.
- `outp(0xE9, byte, 1)` pushes `byte` to an internal ring buffer.
- Expose a small drain API (alloc-based, no `std`):
- e.g. `BxDevicesC::take_port_e9_output() -> Vec<u8>`.

### 4) Keep `std` optional (core stays `no_std + alloc`)

- Gate `std`-dependent GUI code behind `#[cfg(feature = "std")]`.
- Files likely involved:
- [`rusty_box/src/gui/mod.rs`](rusty_box/src/gui/mod.rs)
- [`rusty_box/src/gui/term.rs`](rusty_box/src/gui/term.rs)
- any other GUI files pulling in `std`.
- Keep **core emulation (CPU/memory/iodev)** `no_std + alloc`.

### 5) Remove non-thread-safe `static mut` debug counters

- Replace `static mut` counters used for logging with either:
- per-instance fields (preferred, zero cross-thread issues)
- or remove the counters entirely.
- Files:
- [`rusty_box/src/iodev/vga.rs`](rusty_box/src/iodev/vga.rs)
- [`rusty_box/src/memory/misc_mem.rs`](rusty_box/src/memory/misc_mem.rs)

### 6) Validate: BIOS/VGABIOS text visible in `dlxlinux` example

- Update [`rusty_box/examples/dlxlinux.rs`](rusty_box/examples/dlxlinux.rs) to:
- prefer BIOS/VGABIOS from `binaries/bios` (your canonical location)
- increase the instruction budget enough to reach VGA output
- optionally print/drain port `0xE9` buffer alongside VGA (useful for early debug)
- Run:
- `cargo run --example dlxlinux` (std)
- Build-check the core crate in a `no_std + alloc` configuration (examples and `TermGui` remain `std`-only).

## Expected outcome

- On `cargo run --example dlxlinux`, you should see **VGA text-mode BIOS output** rendered by `TermGui` (and optionally any port-`0xE9` debug bytes).
- The `rusty_box` core remains `no_std + alloc`, with `std`-only code confined to optional features and examples.