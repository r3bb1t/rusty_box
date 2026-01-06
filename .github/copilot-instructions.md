# Rusty Box - AI Coding Agent Instructions

**Project**: A Rust port of the Bochs x86 emulator - a complete CPU/system emulator targeting 32/64-bit x86 architecture with virtualization support (VMX/SVM).

## Architecture Overview

### Core Components

**CPU Emulation** ([src/cpu/](src/cpu/))
- `BxCpuC<I: BxCpuIdTrait>`: The main CPU struct parameterized by CPUID implementation
- Features: Instruction decoding, execution loop, virtualization (VMX/SVM), exception/interrupt handling
- Instruction execution: Fetch → Decode (I-cache) → Execute → Handle async events (interrupts/traps)
- Two execution paths: full decoder for complex instructions + simple executor for mov/add/sub

**Memory System** ([src/memory/](src/memory/))
- `BxMemoryStubC`: Manages physical memory (supports >4GB via optional file swapping)
- `BxMemC`: Higher-level memory interface with handler chains
- Block-based architecture with BIOS ROM (512K) + expansion ROM (128K) at high addresses
- Memory handlers: Chained linked-list for I/O device emulation

**PC System** ([src/pc_system.rs](src/pc_system.rs))
- `BxPcSystemC`: Global singleton managing timers, A20 line, ticks
- Provides monotonic time source (`time_ticks()`) and A20 address masking

### Key Patterns

**Builder Pattern** ([src/cpu/builder.rs](src/cpu/builder.rs))
- `BxCpuBuilder<I>` constructs `BxCpuC` with CPUID model (e.g., `Corei7SkylakeX`)
- Initializes all CPU state fields to defaults in `build()`

**Feature Gates** (Cargo.toml)
- `bx_full`: Enables all CPU/memory features
- CPU-specific: `bx_support_apic`, `bx_support_pci`, `bx_configure_msrs`, `bx_support_handlers_chaining_speedups`
- Memory: `bx_large_ram_file` (swap to disk), `bx_phy_address_long` (>32-bit addressing)
- Endianness: `bx_little_endian` (default) vs `bx_big_endian` (mutually exclusive)

**Generic Register Layout** (Endianness-aware)
- `BxGenReg`: Union supporting 64-bit (`rrx`), 32-bit (`erx`), and 16-bit (`rx`) access
- Layout differs by endianness feature flag - see [src/cpu/cpu.rs#L70-L140](src/cpu/cpu.rs#L70-L140)

**Error Handling**
- `thiserror` crate with transparent error propagation
- Root `Error` enum in [src/error.rs](src/error.rs) aggregates `CpuError` and `MemoryError`

## Critical Workflows

### Running the Emulator

```bash
cargo run --release --example init_and_run
```

**What it does** ([examples/init_and_run.rs](examples/init_and_run.rs)):
1. Spawns emulator on a large stack (500MB-1.5GB depending on debug/release)
2. Builds CPU with `Corei7SkylakeX` CPUID model
3. Creates 32MB guest memory + stub
4. Resets CPU (hardware reset reason)
5. Enters `cpu_loop()` - currently limited to 10 iterations (prevents infinite loops)
6. Tracing output at DEBUG level shows iteration count and register values

**Current Limitations**:
- Timeout at 2 seconds (enforced in example)
- Only simple mov/add/sub execute - most instructions return `UnimplementedInstruction`
- `cpu_loop()` has hard limit of 10 iterations

### Building & Testing

```bash
cargo build --release --all-features
cargo test
```

Note: Tests exist but are sparse. Use `tracing` macros to debug execution flow (see [tracing crate docs](https://docs.rs/tracing/)).

## Code Organization Conventions

### Decoder Module Organization
- [src/cpu/decoder/mod.rs](src/cpu/decoder/mod.rs): Defines `BxInstructionGenerated` - the central instruction representation
- `fetchdecode32.rs` + generated code: Maps x86 opcodes to instruction struct fields
- `simple_decoder.rs`: Minimal decoder for bootstrap/testing
- `ia_opcodes.rs`: x86 opcode definitions

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

### Instruction Execution Flow

1. **get_icache_entry()** → Fetch & decode instruction → Returns `BxIcacheEntry`
2. **before_execution()** → Pre-execution hooks
3. **Simple executor** → Execute mov/add/sub inline
4. **Full executor** → Dispatch to decoder-generated handler (currently unimplemented)
5. **handle_async_event()** → Process interrupts/exceptions/traps after each instruction

## Important Implementation Details

### Memory Address Mapping
- BIOS ROM: 0xE0000-0xFFFFF (128KB) - ROM mapped area at boot
- Expansion ROM: 0xC0000-0xDFFFF (128KB)
- A20 masking: `a20_addr(addr) & bx_pc_system().a20_mask` (0xFFFFFFFFFFFFFFFF when enabled)

### Lifetime Management
- `EmulatorContext<'c, I>` holds references to memory & CPU
- CPU holds `&'c` lifetime reference to memory for the execution loop
- `unsafe` transmute in `cpu_loop()` to work around borrow checker (see [src/cpu/cpu.rs#L885-L890](src/cpu/cpu.rs#L885-L890))

### Conditional Compilation
- `#[cfg(feature = "std")]`: Uses `OnceLock` for singletons (pc_system)
- `#[cfg(not(feature = "std"))]`: Uses `spin::once::Once` for no_std support
- Watch endianness: `#[cfg(feature = "bx_little_endian")]` affects register layout

## Common Modification Points

1. **Adding instructions**: Decode in `decoder/` → Add handler in `data_xfer/`, `arith/`, etc. → Wire in executor dispatch
2. **Adding MSRs/CPUID**: Extend `cpuid::BxCpuIdTrait` impl, update `init_fetch_decode_tables()`
3. **Memory handlers**: Chain new handler in memory system (I/O device setup)
4. **Debugging**: Increase tracing level in `init_and_run.rs` from `Level::DEBUG` to `Level::TRACE`

## References

- **Bochs Original**: [cpp_orig/bochs/](cpp_orig/bochs/) - C++ reference implementation
- **Memory Analysis**: [BIOS_ROM_MEMORY_MAPPING_ANALYSIS.md](BIOS_ROM_MEMORY_MAPPING_ANALYSIS.md)
- **Device Initialization**: [BOCHS_DEVICE_INITIALIZATION_ANALYSIS.md](BOCHS_DEVICE_INITIALIZATION_ANALYSIS.md)
