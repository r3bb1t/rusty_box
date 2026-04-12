# Rusty Box - AI Coding Agent Instructions

**Project**: A Rust port of the Bochs x86 emulator - a complete CPU/system emulator targeting 32/64-bit x86 architecture with virtualization support (VMX/SVM). Boots Alpine Linux to login prompt.

## Architecture Overview

### Core Components

**CPU Emulation** ([src/cpu/](src/cpu/))
- `BxCpuC<I: BxCpuIdTrait>`: The main CPU struct parameterized by CPUID implementation
- Features: Instruction decoding, execution loop, virtualization (VMX/SVM), exception/interrupt handling
- Instruction execution: Fetch -> Decode (I-cache) -> Execute -> Handle async events (interrupts/traps)
- Diagnostic fields gated behind `#[cfg(debug_assertions)]` for release performance

**Memory System** ([src/memory/](src/memory/))
- `BxMemoryStubC`: Manages physical memory (supports >4GB via optional file swapping)
- `BxMemC`: Higher-level memory interface with handler chains
- Block-based architecture with BIOS ROM (512K) + expansion ROM (128K) at high addresses
- MMIO handlers: `MemoryDeviceId` enum (Vga, IoApic) replaces old fn-pointer dispatch

**PC System** ([src/pc_system.rs](src/pc_system.rs))
- `BxPcSystemC`: Instance-based (no global state), manages timers, A20 line, ticks
- Timer dispatch via `TimerOwner` enum (PciIdeCh0, PciIdeCh1, Lapic) - no fn pointers
- `countdown_event()` records fired timers; emulator drains via `dispatch_timer_fires()`

**Emulator** ([src/emulator.rs](src/emulator.rs))
- `Emulator<'a, I>`: Top-level struct owning CPU, memory, devices, pc_system
- `run_interactive()`: GUI mode with wall-clock throttling
- `run_headless()`: Batch mode for testing
- `tick_devices()`: Advances PIT, CMOS, keyboard, VGA, serial, ACPI timing
- `sync_event_flags()`: Propagates PIC/LAPIC/pc_system/IOAPIC interrupt flags to CPU

**Device Manager** ([src/iodev/devices.rs](src/iodev/devices.rs))
- `DeviceManager`: Owns all hardware devices (pic, pit, cmos, dma, keyboard, harddrv, vga, ioapic, pci_bridge, pci2isa, pci_ide, serial, acpi, port92)
- I/O dispatch via `DeviceId` enum in [src/iodev/mod.rs](src/iodev/mod.rs)
- Split-borrow pattern at dispatch sites for cross-device references

### Key Patterns

**No stored device-to-device pointers.** Cross-device communication uses:
1. **Parameter passing**: Device methods take `&mut OtherDevice` when needed. I/O dispatch split-borrows `DeviceManager` fields.
2. **Forwarding queues**: PIC enqueues IOAPIC forwarding requests (`ioapic_forwards`). IOAPIC enqueues LAPIC delivery requests (`pending_deliveries`). Emulator drains these after I/O operations.
3. **Boolean flags + sync**: PIC sets `irq_pending`/`irq_cleared`. CPU's `sync_pic_flags()` propagates after every port_in/port_out. Emulator's `sync_event_flags()` propagates between batches.
4. **Immediate apply**: PAM register changes applied immediately in `outp()` via `NonNull<BxMemC>` wired per-execution.

**Descriptor enum** ([src/cpu/descriptor.rs](src/cpu/descriptor.rs))
- `Descriptor` is a safe enum with `Segment`/`Gate`/`TaskGate` variants (not a union)
- Default is `Segment` (not `TaskGate`) - critical: setters are no-ops on wrong variant
- ~200 callsites use accessor methods (`segment_base()`, `set_gate_dest_offset()`, etc.)

**Feature Gates** (Cargo.toml)
- `bx_full`: Enables all CPU/memory features
- `std`: Enables OnceLock, wall-clock throttling, env vars. Without it: `no_std` + `alloc`
- CPU-specific: `bx_support_apic`, `bx_support_pci`, `bx_configure_msrs`
- `#![allow(...)]` removed from lib.rs - per-file/per-item allows only

**Error Handling**
- `thiserror` crate with transparent error propagation
- Root `Error` enum in [src/error.rs](src/error.rs) aggregates `CpuError` and `MemoryError`

## Running the Emulator

```bash
# Alpine Linux GUI (gold standard test)
cargo build --release --example rusty_box_egui --features "std gui-egui"
target/release/examples/rusty_box_egui.exe

# Alpine Linux headless
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=200000000 cargo run --release --example alpine --features std

# DLX Linux GUI
RUSTY_BOX_BOOT=dlx cargo run --release --example rusty_box_egui --features "std gui-egui"
```

### Building & Testing

```bash
# Full build
cargo build --release --all-features

# Unit tests (167 tests, 0 failures)
cargo test -p rusty_box --lib -- --test-threads=1

# no_std build check
cargo build -p rusty_box --no-default-features --features bx_full

# Clippy (0 warnings)
cargo clippy --all-features
```

## Interrupt Delivery Architecture

### PIC (Legacy 8259) Path
1. Device calls `pic.raise_irq(N)` -> PIC evaluates priority -> `raise_intr()` sets `irq_pending = true`
2. PIC also enqueues `(N, true)` in `ioapic_forwards` queue for IOAPIC
3. CPU's `sync_pic_flags()` (called after every I/O op) reads `irq_pending` -> sets `cpu.async_event = 1`
4. I/O dispatch drains `pic.ioapic_forwards` -> forwards to `ioapic.set_irq_level()`
5. CPU's `handle_async_event()` calls `pic.iac()` to get vector and delivers interrupt

### APIC (IOAPIC -> LAPIC) Path
1. IOAPIC receives IRQ via `set_irq_level()` -> evaluates redirection table
2. If LAPIC reference available: delivers directly via `lapic.deliver()`
3. If LAPIC unavailable (MMIO path): enqueues in `pending_deliveries`
4. Emulator's `sync_event_flags()` drains IOAPIC pending deliveries to LAPIC

### Timer Path
1. `pc_system.tickn()` -> `countdown_event()` -> records fired `TimerOwner`s
2. Emulator calls `dispatch_timer_fires()` -> dispatches by owner enum
3. `sync_event_flags()` propagates any interrupt state changes to CPU

## Important Implementation Details

### Memory Address Mapping
- BIOS ROM: 0xE0000-0xFFFFF (128KB) - ROM mapped area at boot
- Expansion ROM: 0xC0000-0xDFFFF (128KB)
- PAM registers (PCI bridge 0x59-0x5F) control ROM/RAM shadowing
- **PAM changes must be immediate**: applied in `outp()` after PCI writes, not deferred

### CPU State Access

**Read-only getters** (suffixed with `()`):
```rust
cpu.rax()        // u64 register value
cpu.rip()        // instruction pointer
cpu.eflags()     // flags register
```

**Setters** (prefixed with `set_`):
```rust
cpu.set_rax(0x777)
cpu.set_rip(0)
cpu.reset(ResetReason::Hardware)
```

See [src/cpu/cpu_getters_and_setters.rs](src/cpu/cpu_getters_and_setters.rs) for full list.

### Conditional Compilation
- `#[cfg(feature = "std")]`: OnceLock, wall-clock throttling, env vars
- `#[cfg(not(feature = "std"))]`: `spin::once::Once`, no throttling
- `#[cfg(debug_assertions)]`: Diagnostic fields on CPU, detailed logging in emulator loop
- Watch endianness: `#[cfg(target_endian = "little")]` affects register layout

## Common Modification Points

1. **Adding instructions**: Decode in `decoder/` -> Add handler in `data_xfer/`, `arith/`, etc. -> Wire in executor dispatch
2. **Adding devices**: Add struct in `iodev/`, add variant to `DeviceId` enum, register ports in `DeviceManager`, add to I/O dispatch
3. **Cross-device signaling**: Use forwarding queues (PIC `ioapic_forwards`, IOAPIC `pending_deliveries`) or parameter passing. Never stored pointers.
4. **Adding timers**: Add variant to `TimerOwner` enum, register in emulator, dispatch in `dispatch_timer_fires()`
5. **Debugging**: Set `RUST_LOG=debug` or `RUST_LOG=trace`. Release builds omit diagnostic fields.

## Known Issues

- **mkdir + echo reboot**: Creating a directory and echoing to a file inside it causes the guest to reboot. The keyboard controller 0xFE reset and port92 reset mechanisms are wired but the root cause (likely a filesystem/IDE timing issue) persists.
- **Emulator struct size**: Too large for default test stack. Tests use `std::thread::Builder::new().stack_size(64MB)`.

## References

- **Bochs Original**: [cpp_orig/bochs/](cpp_orig/bochs/) - C++ reference implementation
- **Memory Analysis**: [BIOS_ROM_MEMORY_MAPPING_ANALYSIS.md](BIOS_ROM_MEMORY_MAPPING_ANALYSIS.md)
- **Device Initialization**: [BOCHS_DEVICE_INITIALIZATION_ANALYSIS.md](BOCHS_DEVICE_INITIALIZATION_ANALYSIS.md)
