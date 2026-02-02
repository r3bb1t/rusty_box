# Rusty Box

A Rust port of the Bochs x86 emulator - a complete CPU/system emulator targeting 32/64-bit x86 architecture with virtualization support (VMX/SVM).

## Project Status

**Current State:** Active Development - Protected Mode BIOS Execution

- **Mode:** Protected Mode (32-bit)
- **Instructions Executed:** 80,000+ in protected mode
- **Last Known RIP:** 0xff9e (CPU halted with IF=0, awaiting interrupts)
- **BIOS Stage:** GDT/IDT setup complete, entering protected mode execution
- **Recent Achievements:**
  - Fixed segment register validation in decoder (invalid indices 6-7 now rejected)
  - Fixed HLT instruction async_event handling (CPU now properly halts)
  - Protected mode entry working correctly

See [PROGRESS_STATUS.md](PROGRESS_STATUS.md) for detailed current status.

## Documentation

### Quick Reference
- **[PROGRESS_STATUS.md](PROGRESS_STATUS.md)** - Current execution status, what's working, recent fixes, next milestones, and how-to guide for continuing development. Start here for a quick overview of where we are.

### Technical Deep Dive
- **[DECODER_BUG_FIX_SUMMARY.md](DECODER_BUG_FIX_SUMMARY.md)** - Comprehensive technical analysis of the critical Group opcode decoder bug that was causing stack corruption. Includes root cause analysis, the fix, verification, and lessons learned. Read this to understand the major debugging effort that enabled BIOS execution.

### Complete Knowledge Base
- **[RUSTY_BOX_KNOWLEDGE_BASE.md](RUSTY_BOX_KNOWLEDGE_BASE.md)** - Complete architectural overview covering the entire rusty_box emulator. Includes:
  - Architecture and design principles
  - Current implementation state
  - Known issues and workarounds (10+ documented hacks)
  - Prioritized roadmap (P0-P3 tasks)
  - Implementation guide for new instructions
  - Comparison with original Bochs
  - Testing and debugging strategies
  - Performance considerations

  **Start here** if you're new to the codebase or need to understand how everything fits together.

### Project Instructions
- **[CLAUDE.md](CLAUDE.md)** - Build commands, workspace structure, architecture overview, and guidance for working with the codebase.

## Quick Start

### Build and Run

```bash
# Build with optimizations (required for performance)
cargo build --release --all-features

# Run BIOS emulation example
cargo run --release --example dlxlinux --features std

# Run with GUI
cargo run --release --example dlxlinux --features "std,gui-egui"

# Run tests
cargo test
```

### Requirements

- Rust 1.70+ (stable)
- For fuzzing: nightly Rust
- Large stack space for examples (spawned on dedicated thread)

## Project Structure

```
rusty_box/
├── rusty_box/              # Main emulator library
│   ├── src/cpu/           # CPU implementation (organized by instruction category)
│   ├── src/memory/        # Memory subsystem
│   ├── src/iodev/         # I/O devices (PIC, PIT, CMOS, VGA, etc.)
│   └── examples/          # Runnable examples
├── rusty_box_decoder/      # x86 instruction decoder (separate crate for fuzzing)
└── cpp_orig/bochs/         # Original C++ Bochs source (reference)
```

## Architecture Highlights

### No Global State
Each `Emulator<I>` instance is completely self-contained, allowing multiple independent emulator instances to run concurrently.

### Type-Safe CPU Models
CPU behavior is parameterized using traits like `BxCpuIdTrait`. Different CPU models (Corei7SkylakeX, etc.) are compile-time types, not runtime switches.

### no_std Compatible
Core emulator works without standard library. Feature flags enable std functionality (file I/O, terminal GUI, etc.).

### Block-Based Memory
Memory subsystem uses block-based architecture supporting >4GB physical addresses when `bx_phy_address_long` feature is enabled.

## Recent Achievements

### Segment Register Validation & HLT Fix (2026-02-02)

**Decoder Validation:** Fixed decoder bug where invalid segment register indices (6-7) were passed to CPU, violating x86 spec. Added validation in `fetchdecode32.rs` and `fetchdecode64.rs` to reject invalid encodings at decode time.

**HLT Instruction Fix:** Fixed critical bug where HLT instruction wasn't properly halting CPU execution. Two locations in `cpu.rs` were unconditionally clearing `BX_ASYNC_EVENT_STOP_TRACE` flag, preventing the CPU from detecting halt state. Modified to preserve the flag when `activity_state != Active`.

**Impact:**
- Protected mode BIOS execution now works correctly
- CPU properly halts and returns control to emulator
- 80,000+ instructions executed in protected mode
- BIOS progresses to RIP=0xff9e (halted awaiting interrupts)

See [docs/DECODER_BUGS.md](docs/DECODER_BUGS.md) for decoder validation details.

### Critical Decoder Bug Fix (2026-01-30)

Fixed a critical bug in x86 instruction decoding affecting all Group 2 instructions (shift/rotate operations). The decoder was using the ModR/M `nnn` field (opcode extension) as the destination register instead of the `rm` field (actual operand).

**Impact:**
- Before fix: BIOS crashed at ~40K instructions (RIP 0xFFEA)
- After fix: BIOS executes 100K+ instructions (RIP 0x9E4F)
- **60x improvement** in execution progress

See [DECODER_BUG_FIX_SUMMARY.md](DECODER_BUG_FIX_SUMMARY.md) for complete technical analysis.

## What Works

- CPU instruction decoding (Group opcodes fixed, segment register validation)
- Protected mode execution (GDT/IDT setup, segment loading)
- Basic arithmetic and logical operations (8/16/32-bit variants)
- Shift operations (SHL, SHR, SAR, ROL, ROR)
- Multiply operations (IMUL variants)
- Stack operations (PUSH, POP, CALL, RET) - 16/32-bit modes
- String operations (REP STOSD, MOVSB)
- Memory operations (MOV variants, MOVSX)
- Control flow (conditional jumps, loops)
- Descriptor table loading (LIDT, LGDT)
- HLT instruction (properly halts CPU)
- BIOS initialization through protected mode entry

## What's Next

### Immediate (P0)
1. Investigate why BIOS halts with IF=0 (interrupts disabled) - may need interrupt delivery implementation
2. Implement missing I/O device responses (keyboard, timer, etc.) that BIOS expects
3. Continue implementing missing instructions as BIOS encounters them
4. Add execution bounds checking for memory regions

### Short-term (P1)
- Complete interrupt handling (hardware interrupts, IRQ delivery)
- Implement remaining I/O devices for BIOS interaction
- Complete Group opcode implementations (RCL, RCR variants)
- Add comprehensive unit tests for decoder edge cases
- Performance profiling and optimization

### Long-term (P3)
- Boot DLX Linux
- Full BIOS POST completion
- JIT compilation
- Multi-core emulation

## Contributing

This is a learning/research project porting Bochs to Rust. When adding new code:

- Use `Result<>` instead of `panic!()`
- Guard `println!()` with `#[cfg(feature = "std")]`
- Use `tracing::trace!()` for debug logging
- Organize instructions in appropriate files (match cpp_orig/bochs/ structure)
- Test both with and without std feature
- Follow existing code patterns

## Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture

# Verify no_std compatibility
cargo build --release --lib

# Fuzz the decoder
cd rusty_box_decoder && cargo +nightly fuzz run fuzz_target_1
```

## References

- Original Bochs: [bochs.sourceforge.io](http://bochs.sourceforge.io/)
- Intel Manual: Volume 2 (Instruction Set Reference)
- x86 Opcode Map: sandpile.org

## License

See original Bochs licensing in `cpp_orig/bochs/`. This Rust port follows the same terms.

---

**Last Updated:** 2026-02-02
**Current Focus:** Interrupt handling and I/O device responses
**Status:** 🟢 Active Development - Protected Mode Working
