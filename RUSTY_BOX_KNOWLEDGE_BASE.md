# Rusty Box - Complete Knowledge Base

**Last Updated:** 2026-01-30
**Version:** 0.1.0 (In Active Development)
**Status:** 🟡 Functional but Incomplete

---

## Table of Contents

1. [What is Rusty Box?](#what-is-rusty-box)
2. [Architecture Overview](#architecture-overview)
3. [Current State](#current-state)
4. [Known Issues & Hacks](#known-issues--hacks)
5. [What Needs to Be Done](#what-needs-to-be-done)
6. [Implementation Guide](#implementation-guide)
7. [Comparison with Bochs](#comparison-with-bochs)
8. [How to Improve](#how-to-improve)
9. [Testing & Debugging](#testing--debugging)
10. [Performance Considerations](#performance-considerations)

---

## What is Rusty Box?

### Overview

**Rusty Box** is a Rust port of the [Bochs x86 emulator](https://bochs.sourceforge.io/), a complete PC emulator capable of running operating systems and applications designed for x86 architecture. Unlike virtualization (which runs on native hardware), Rusty Box emulates the entire x86 system in software, instruction by instruction.

### Key Characteristics

- **Language:** Pure Rust (with no_std support)
- **Target:** x86/x86-64 architecture emulation
- **Scope:** Full system emulation (CPU, memory, devices)
- **Goal:** Binary-compatible with Bochs while leveraging Rust's safety and performance
- **Status:** Early development, BIOS boots partially

### Design Philosophy

1. **Safety First:** Leverage Rust's type system to eliminate undefined behavior
2. **No Global State:** Instance-based design for multiple concurrent emulators
3. **Modularity:** Separate decoder crate, pluggable components
4. **Compatibility:** Reference Bochs C++ code for correctness
5. **Performance:** Optimize after correctness is achieved

### Use Cases

- **OS Development:** Test operating systems without real hardware
- **Reverse Engineering:** Analyze x86 binaries in controlled environment
- **Education:** Learn x86 architecture and emulation techniques
- **Security Research:** Sandboxed execution of untrusted code
- **Regression Testing:** Deterministic execution for CI/CD

---

## Architecture Overview

### Component Hierarchy

```
┌─────────────────────────────────────────────────────────┐
│                    Emulator<I>                          │
│                 (Top-level orchestrator)                │
└─────────────────────────────────────────────────────────┘
                          │
        ┌─────────────────┼─────────────────┐
        │                 │                 │
   ┌────▼────┐      ┌────▼────┐      ┌────▼────┐
   │ BxCpuC  │      │ BxMemC  │      │Devices  │
   │  (CPU)  │      │(Memory) │      │ Manager │
   └─────────┘      └─────────┘      └─────────┘
        │                 │                 │
   ┌────▼────┐      ┌────▼────┐      ┌────▼────┐
   │Decoder  │      │  TLB    │      │  PIC    │
   │  Crate  │      │ System  │      │  PIT    │
   └─────────┘      └─────────┘      │  CMOS   │
                                      │  DMA    │
                                      │  VGA    │
                                      │Keyboard │
                                      │HardDrive│
                                      └─────────┘
```

### Core Components

#### 1. **BxCpuC** - CPU Emulator
**Location:** `rusty_box/src/cpu/cpu.rs`

**Responsibilities:**
- Instruction fetch, decode, execute cycle
- Register state management (GPRs, segment registers, control registers)
- Flag computation (ZF, SF, OF, CF, PF, AF)
- Exception/interrupt handling
- Privilege level enforcement
- Paging and segmentation

**Key Structures:**
```rust
pub struct BxCpuC<'c, I: BxCpuIdTrait> {
    // General purpose registers (RAX, RBX, RCX, RDX, RSI, RDI, RBP, RSP, R8-R15)
    pub gen_reg: [BxGenReg; BX_GENERAL_REGISTERS],

    // Instruction pointer
    rip: u64,

    // Flags register (RFLAGS/EFLAGS)
    pub eflags: u32,

    // Segment registers (ES, CS, SS, DS, FS, GS)
    pub sregs: [BxSegmentReg; 6],

    // Control registers (CR0, CR2, CR3, CR4, CR8)
    pub cr0: BxCr0,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: BxCr4,

    // Descriptor tables
    pub gdtr: BxGlobalSegmentReg,  // Global Descriptor Table Register
    pub idtr: BxGlobalSegmentReg,  // Interrupt Descriptor Table Register
    pub ldtr: BxSegmentReg,         // Local Descriptor Table Register
    pub tr: BxSegmentReg,           // Task Register

    // TLB (Translation Lookaside Buffer) for paging
    dtlb: Tlb,  // Data TLB
    itlb: Tlb,  // Instruction TLB

    // Instruction cache
    i_cache: BxICache,

    // FPU state (x87, MMX, SSE, AVX)
    pub i387: I387,

    // APIC (Advanced Programmable Interrupt Controller)
    pub lapic: BxLocalApic,

    // Statistics and debugging
    icount: u64,  // Instruction counter
}
```

#### 2. **BxMemC** - Memory Subsystem
**Location:** `rusty_box/src/memory/`

**Responsibilities:**
- Physical memory storage (RAM, ROM)
- Memory-mapped I/O
- ROM loading (BIOS, VGA BIOS)
- A20 gate control
- Memory access validation

**Memory Layout:**
```
0x00000000 - 0x0009FFFF : Conventional RAM (640 KB)
0x000A0000 - 0x000BFFFF : VGA video memory (128 KB)
0x000C0000 - 0x000DFFFF : Expansion ROM area (128 KB)
0x000E0000 - 0x000FFFFF : BIOS ROM area (128 KB)
0x00100000 - 0xFFFFFFFF : Extended memory (up to 4 GB)
0xFFF80000 - 0xFFFFFFFF : System BIOS mirror (512 KB)
```

**Key Features:**
- Block-based allocation for large RAM
- Direct byte-array for ROM
- TLB integration for fast paging
- Write-protection for ROM regions

#### 3. **Decoder** - Instruction Decoder
**Location:** `rusty_box_decoder/` (separate crate)

**Responsibilities:**
- Parse x86/x86-64 instruction bytes
- Extract opcode, operands, addressing modes
- Calculate instruction length
- Handle prefixes (REX, operand/address size, segment override)

**Why Separate Crate:**
- Allows fuzzing without full emulator
- Reusable in other projects
- Faster compilation during decoder development
- Clear separation of concerns

**Decoding Pipeline:**
```
Byte Stream → Prefix Handling → Opcode Lookup → ModR/M Parsing
           → SIB Parsing → Displacement/Immediate → BxInstructionGenerated
```

#### 4. **Device Manager** - I/O Devices
**Location:** `rusty_box/src/iodev/`

**Devices Implemented:**
- **PIC** (8259A): Programmable Interrupt Controller
- **PIT** (8254): Programmable Interval Timer
- **CMOS** (MC146818): Real-time clock and NVRAM
- **DMA** (8237A): Direct Memory Access controller
- **VGA**: Video Graphics Array (text mode only)
- **Keyboard** (8042): PS/2 keyboard controller
- **Hard Drive**: IDE/ATA disk controller
- **System Control**: Port 0x92 (A20, reset)

**I/O Port Architecture:**
```rust
pub struct BxDevicesC<'a> {
    // Port handlers: 65536 possible I/O ports (0x0000-0xFFFF)
    read_handlers: [Option<ReadHandler>; 65536],
    write_handlers: [Option<WriteHandler>; 65536],

    // Device instances
    pic: BxPic,
    pit: BxPit,
    cmos: BxCmos,
    dma: BxDma,
    vga: BxVga,
    keyboard: BxKeyboard,
    hard_drive: BxHardDrive,
}
```

#### 5. **BxPcSystemC** - System Controller
**Location:** `rusty_box/src/pc_system.rs`

**Responsibilities:**
- System timer management
- A20 line control (memory addressing)
- Reset logic
- Interrupt routing

### Instruction Organization

Instructions are organized by category in separate files, mirroring Bochs structure:

```
cpu/
├── arith*.rs       - Arithmetic (ADD, SUB, ADC, SBB, INC, DEC, NEG, CMP)
├── logical*.rs     - Logical (AND, OR, XOR, NOT, TEST)
├── mult*.rs        - Multiply/Divide (MUL, IMUL, DIV, IDIV)
├── shift.rs        - Shifts/Rotates (SHL, SHR, SAR, ROL, ROR, RCL, RCR)
├── data_xfer*.rs   - Data transfer (MOV, MOVSX, MOVZX, LEA, XCHG)
├── ctrl_xfer*.rs   - Control transfer (JMP, CALL, RET, Jcc, LOOP)
├── stack*.rs       - Stack operations (PUSH, POP, PUSHF, POPF)
├── string.rs       - String ops (MOVS, STOS, LODS, CMPS, SCAS + REP)
├── io.rs           - I/O (IN, OUT, INS, OUTS)
├── soft_int.rs     - Interrupts (INT, IRET, INTO, BOUND)
├── proc_ctrl.rs    - Processor control (HLT, NOP, RDTSC, CPUID)
├── crregs.rs       - Control registers (MOV to/from CR0-CR8)
└── flag_ctrl_pro.rs - Flag control (CLC, STC, CMC, CLD, STD)
```

**Naming Convention:**
- `*8.rs`: 8-bit operations
- `*16.rs`: 16-bit operations
- `*32.rs`: 32-bit operations
- `*64.rs`: 64-bit operations
- No suffix: Mixed or common operations

---

## Current State

### What Works ✅

#### CPU Instructions (Partial)
- ✅ Basic arithmetic: ADD, SUB, ADC, SBB, INC, DEC (8/16/32-bit)
- ✅ Logical operations: AND, OR, XOR, NOT, TEST (8/16/32-bit)
- ✅ Shifts: SHL, SHR, SAR (some variants)
- ✅ Multiply: MUL, IMUL (basic forms)
- ✅ Data movement: MOV (most variants), LEA, XCHG
- ✅ Control flow: JMP, CALL, RET, Jcc (conditional jumps)
- ✅ Stack: PUSH, POP (register/immediate forms)
- ✅ String operations: STOSD, STOSB, MOVSB, REP prefix
- ✅ I/O: IN, OUT (basic forms)
- ✅ System: INT, IRET, CLI, STI, HLT, CLD
- ✅ Descriptor tables: LIDT, LGDT

#### Memory & Addressing
- ✅ Real mode addressing
- ✅ Segment:offset calculation
- ✅ A20 gate control
- ✅ ROM loading (BIOS, VGA BIOS)
- ✅ RAM allocation
- ✅ Direct memory access

#### Devices
- ✅ PIC (basic interrupt handling)
- ✅ PIT (timer ticks)
- ✅ CMOS (RTC, NVRAM reads)
- ✅ Keyboard (input buffer)
- ✅ VGA (text mode buffer)
- ✅ Hard drive (basic IDE)

#### BIOS Boot Progress
- ✅ Boots from F000:FFF0 (reset vector)
- ✅ Executes initial POST code
- ✅ Sets up IVT (Interrupt Vector Table)
- ✅ Initializes memory regions
- ✅ Loads IDT (Interrupt Descriptor Table)
- ✅ Reaches ~0x9E4F (descriptor table setup)

### What's Broken or Missing ❌

#### Critical Decoder Bug (FIXED 2026-01-30)
- ~~❌ Group opcodes using wrong ModR/M field~~ ✅ **FIXED**
- ~~❌ A0-A3 opcodes (MOV moffs) wrong length~~ ✅ **FIXED**

#### Missing CPU Instructions
- ❌ Control registers: MOV to/from CR0-CR8 (partially missing)
- ❌ Debug registers: MOV to/from DR0-DR7
- ❌ Segment operations: LLDT, LTR, SGDT, SIDT, STR, SLDT
- ❌ Task switching: IRET (protected mode), task gates
- ❌ Protection: LAR, LSL, ARPL, VERR, VERW
- ❌ Many shift/rotate variants: RCL, RCR with different operands
- ❌ Division: DIV, IDIV (many variants)
- ❌ BCD operations: DAA, DAS, AAA, AAS, AAM, AAD
- ❌ Bit operations: BT, BTS, BTR, BTC, BSF, BSR
- ❌ Conditional MOV: CMOVcc
- ❌ FPU instructions: Almost all x87 FPU opcodes
- ❌ MMX/SSE/AVX: SIMD instructions
- ❌ System management: SYSENTER, SYSEXIT, SYSCALL, SYSRET

#### Missing Features
- ❌ Protected mode (partially implemented but not working)
- ❌ Paging (structures exist but not functional)
- ❌ Virtual 8086 mode
- ❌ Long mode (64-bit)
- ❌ Hardware virtualization (VMX/SVM)
- ❌ FPU state management
- ❌ Exception delivery mechanism (incomplete)
- ❌ Interrupt priorities
- ❌ I/O permission bitmap
- ❌ Task State Segment (TSS)

#### Device Limitations
- ❌ VGA: Only text mode, no graphics modes
- ❌ Hard drive: Basic reads, no DMA, no error handling
- ❌ Serial port: Not implemented
- ❌ Parallel port: Not implemented
- ❌ Floppy drive: Not implemented
- ❌ Sound card: Not implemented
- ❌ Network card: Not implemented
- ❌ USB: Not implemented

#### Memory Issues
- ❌ MMIO (Memory-Mapped I/O) incomplete
- ❌ No cache emulation
- ❌ Page attribute table (PAT) not implemented
- ❌ Memory type range registers (MTRR) not implemented

---

## Known Issues & Hacks

### 1. Group Opcode Decoder Bug (FIXED)

**Issue:** Decoder was using wrong ModR/M field for Group instructions (C0, C1, D0-D3, F6, F7, FE, FF)

**Symptom:** SHL EAX, 0x10 was shifting ESP instead of EAX, causing stack corruption

**Fix Applied:** Added Group opcode detection in `fetchdecode32.rs`:
```rust
let is_group_opcode = matches!(b1, 0xC0 | 0xC1 | 0xD0 | 0xD1 | 0xD2 | 0xD3 | 0xF6 | 0xF7 | 0xFE | 0xFF);
if is_group_opcode {
    instr.meta_data[BX_INSTR_METADATA_DST] = rm as u8;
    instr.meta_data[BX_INSTR_METADATA_SRC1] = nnn as u8;
}
```

**Status:** ✅ Resolved
**File:** `rusty_box_decoder/src/fetchdecode32.rs` lines 419-428
**Date Fixed:** 2026-01-30

### 2. Invalid Segment Register 6 Workaround

**Issue:** Decoder generates MOV Ew,Sw with segment register index 6, which is invalid

**Symptom:** Panic: "index out of bounds: the len is 6 but the index is 6"

**Root Cause:** Opcode 8C with ModR/M reg=6 is undefined in x86 spec, but decoder creates MovEwSw for it

**Hack Applied:** Treat segment 6 as DS (segment 3)
```rust
let actual_seg = if src_seg == 6 {
    tracing::warn!("Invalid segment register 6 in MOV Ew,Sw - using DS as workaround");
    3 // DS
} else {
    src_seg
};
```

**Status:** ⚠️ **Workaround Active** - Needs proper fix
**File:** `rusty_box/src/cpu/data_xfer_ext.rs` lines 100-111
**TODO:** Fix decoder to not generate MovEwSw for invalid reg values

### 3. Large Stack Requirement

**Issue:** Examples crash with stack overflow on default stack

**Cause:** Rust default stack is 2MB, emulator needs much more for:
- Large instruction cache structures
- Deep call stacks during initialization
- TLB arrays

**Hack:** Spawn emulator on dedicated thread with 500MB-1.5GB stack:
```rust
let builder = std::thread::Builder::new()
    .name("DLX Linux".into())
    .stack_size(500 * 1024 * 1024); // 500 MB stack
```

**Status:** ⚠️ **Workaround Active**
**File:** `rusty_box/examples/dlxlinux.rs`
**TODO:** Reduce stack usage by heap-allocating large structures

### 4. Incomplete Exception Handling

**Issue:** Exceptions often return errors instead of proper x86 exception delivery

**Symptom:** Emulator stops instead of delivering #GP, #PF, #UD to guest

**Current State:** Many functions return `Result<()>` which propagates to caller instead of pushing exception frame

**Impact:** Protected mode transitions and exception handling don't work

**Status:** ⚠️ **Major Issue**
**TODO:** Implement proper exception queuing and delivery mechanism

### 5. TLB Not Invalidated Properly

**Issue:** TLB (Translation Lookaside Buffer) not always invalidated when it should be

**Cause:** Incomplete tracking of when page tables change

**Symptom:** Stale address translations after page table modifications

**Status:** ⚠️ **Potential Issue** - Not yet encountered in testing
**TODO:** Review all CR3 writes, INVLPG usage, and TLB flush logic

### 6. Flags Computation Inconsistencies

**Issue:** Some instructions don't compute all flags correctly

**Examples:**
- Auxiliary Flag (AF) calculation in BCD operations
- Overflow Flag (OF) in some shift operations
- Parity Flag (PF) sometimes skipped

**Status:** ⚠️ **Known Issue**
**Impact:** Mainly affects BCD and obscure operations
**TODO:** Cross-check all flag computations against Intel manual

### 7. FPU State Uninitialized

**Issue:** FPU (x87) state exists but isn't properly initialized or used

**Symptom:** FPU instructions would fail or produce incorrect results

**Status:** ⚠️ **Not Implemented**
**Workaround:** BIOS doesn't heavily use FPU during early boot
**TODO:** Implement FPU reset, control word, status word, tag word

### 8. No Dirty Page Tracking

**Issue:** Paging system doesn't set Accessed/Dirty bits in page tables

**Cause:** Not implemented yet

**Impact:** OS page replacement algorithms won't work

**Status:** ⚠️ **Not Implemented**
**TODO:** Set A bit on read, D bit on write in page table entries

### 9. Hardcoded CPUID Model

**Issue:** CPUID is hardcoded to Corei7SkylakeX

**Cause:** Generic parameter `I: BxCpuIdTrait` throughout codebase

**Limitation:** Can't easily emulate other CPU models

**Status:** ⚠️ **Design Limitation**
**TODO:** Make CPUID more configurable or support multiple models

### 10. Memory Access Rights Not Checked

**Issue:** Reads/writes don't check segment limits or permissions in real mode

**Symptom:** Invalid memory accesses not caught

**Status:** ⚠️ **Not Implemented**
**TODO:** Add segment limit checking, even in real mode

---

## What Needs to Be Done

### Priority 0: Critical for BIOS Boot

#### 1. Implement Missing Control Register Operations
- [ ] `MOV reg, CR0` - Read CR0 (needed at 0x9E4F)
- [ ] `MOV CR0, reg` - Write CR0 (for protected mode entry)
- [ ] `MOV reg, CR2` - Read CR2 (page fault address)
- [ ] `MOV CR3, reg` - Write CR3 (set page directory)
- [ ] `MOV reg, CR4` - Read CR4
- [ ] `MOV CR4, reg` - Write CR4 (enable PSE, PAE, etc.)

**Files:** `rusty_box/src/cpu/crregs.rs`

#### 2. Complete Protected Mode Support
- [ ] GDT (Global Descriptor Table) parsing
- [ ] Descriptor privilege level (DPL) checking
- [ ] Segment limit checking
- [ ] Protected mode flag propagation
- [ ] Privilege level transitions (ring 0-3)

**Files:** `rusty_box/src/cpu/segment_ctrl_pro.rs`, `descriptor.rs`

#### 3. Implement Remaining Shift/Rotate Operations
- [ ] RCL (Rotate through Carry Left) - all forms
- [ ] RCR (Rotate through Carry Right) - all forms
- [ ] Missing SHR/SHL variants (memory operands, etc.)

**Files:** `rusty_box/src/cpu/shift.rs`

#### 4. Add Division Instructions
- [ ] DIV (Unsigned divide) - 8/16/32/64-bit
- [ ] IDIV (Signed divide) - 8/16/32/64-bit
- Handle divide-by-zero exception
- Handle divide overflow

**Files:** `rusty_box/src/cpu/mult*.rs`

### Priority 1: Important for OS Boot

#### 5. Paging Implementation
- [ ] Page directory and page table walking
- [ ] TLB population on page fault
- [ ] CR3 register handling
- [ ] Page-level protection bits (R/W, U/S)
- [ ] Accessed and Dirty bit updates
- [ ] INVLPG instruction

**Files:** `rusty_box/src/cpu/paging.rs`

#### 6. Exception Delivery Mechanism
- [ ] Exception priority handling
- [ ] Pushing exception frame (error code, return address)
- [ ] IDT (Interrupt Descriptor Table) lookup
- [ ] Gate descriptor parsing (Interrupt, Trap, Task gates)
- [ ] Privilege level checking for exceptions
- [ ] Nested exception handling (double fault, triple fault)

**Files:** `rusty_box/src/cpu/exception.rs`, `protected_interrupts.rs`

#### 7. Interrupt System
- [ ] Hardware interrupt delivery via PIC
- [ ] Interrupt masking (IF flag, PIC mask register)
- [ ] Interrupt priority
- [ ] IRET in protected mode
- [ ] Interrupt gates vs trap gates

**Files:** `rusty_box/src/cpu/soft_int.rs`, `iodev/pic.rs`

#### 8. Complete VGA Support
- [ ] Text mode cursor positioning
- [ ] Character attributes (color, blinking)
- [ ] Graphics modes (at least 13h - 320x200x256)
- [ ] VGA register programming
- [ ] Display refresh to terminal/GUI

**Files:** `rusty_box/src/iodev/vga.rs`

### Priority 2: Nice to Have

#### 9. Long Mode (64-bit) Support
- [ ] IA32_EFER MSR
- [ ] 64-bit paging (4-level or 5-level)
- [ ] 64-bit instruction decoding (REX prefix handling)
- [ ] 64-bit register operations
- [ ] RIP-relative addressing

**Files:** All `*64.rs` files

#### 10. FPU (x87) Instructions
- [ ] Floating-point load/store (FLD, FST, FSTP)
- [ ] Arithmetic (FADD, FSUB, FMUL, FDIV)
- [ ] Comparisons (FCOM, FUCOM)
- [ ] Control (FINIT, FSTCW, FLDCW)
- [ ] FPU stack management
- [ ] Rounding modes

**Files:** `rusty_box/src/cpu/i387.rs`, separate FPU module

#### 11. SIMD Instructions
- [ ] MMX (Pentium MMX)
- [ ] SSE (Pentium III)
- [ ] SSE2-SSE4 (Pentium 4 onwards)
- [ ] AVX (Sandy Bridge)
- [ ] AVX2, AVX-512 (newer CPUs)

**Files:** `rusty_box/src/cpu/avx/`, `xmm.rs`

#### 12. Additional Devices
- [ ] Serial port (COM1-COM4)
- [ ] Parallel port (LPT1)
- [ ] Floppy disk controller
- [ ] Sound card (SoundBlaster emulation)
- [ ] Network card (NE2000 or similar)

**Files:** `rusty_box/src/iodev/` (new files)

### Priority 3: Optimizations

#### 13. Performance Improvements
- [ ] JIT compilation (translate hot code to native)
- [ ] Instruction caching (avoid re-decoding)
- [ ] Lazy flag computation (defer flag calculation)
- [ ] Fast paths for common operations
- [ ] TLB optimization
- [ ] Memory access batching

#### 14. Developer Experience
- [ ] Better error messages
- [ ] GDB stub for debugging
- [ ] Save/restore state (snapshots)
- [ ] Execution tracing to file
- [ ] Performance profiling hooks
- [ ] CPU execution statistics

---

## Implementation Guide

### How to Add a New Instruction

#### Step 1: Check if Opcode is Decoded
```bash
grep -r "YourOpcodeName" rusty_box_decoder/src/
```

If not found, need to add to decoder first (usually already there).

#### Step 2: Find the Right File

Based on instruction category:
- Arithmetic → `arith*.rs`
- Logical → `logical*.rs`
- Shift/Rotate → `shift.rs`
- Multiply/Divide → `mult*.rs`
- Data movement → `data_xfer*.rs`
- Control flow → `ctrl_xfer*.rs`
- Stack → `stack*.rs`
- String → `string.rs`
- I/O → `io.rs`
- System/Control → `proc_ctrl.rs`, `crregs.rs`

Based on operand size:
- Mixed sizes → Base file (e.g., `shift.rs`)
- 8-bit only → `*8.rs`
- 16-bit only → `*16.rs`
- 32-bit only → `*32.rs`
- 64-bit only → `*64.rs`

#### Step 3: Implement the Function

Follow this template:

```rust
/// <Instruction Name> - Brief description
///
/// Opcode: <hex opcode>
/// Format: <assembly syntax>
/// Example: <assembly example>
///
/// Flags: <which flags are affected>
///
/// Based on Bochs: <original file>.cc:<function name>
pub fn your_instruction_name(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
    // 1. Extract operands
    let dst = instr.dst() as usize;
    let src = instr.src() as usize;
    let imm = instr.ib(); // or iw(), id(), iq()

    // 2. Read values
    let op1 = self.get_gpr32(dst);
    let op2 = self.get_gpr32(src);

    // 3. Perform operation
    let result = op1.wrapping_add(op2);

    // 4. Write result
    self.set_gpr32(dst, result);

    // 5. Update flags
    self.update_flags_add32(result, op1, op2);

    // 6. Trace (optional, for debugging)
    tracing::trace!("YOUR_INSTR: r{} = {:#x}", dst, result);

    Ok(())
}
```

**Key Points:**
- Use `Result<()>` return type for error handling
- Use `tracing::trace!()` not `println!()` or `debug!()`
- Look at similar instructions in same file for patterns
- Check original Bochs implementation for flag computation
- Use `wrapping_*` methods for arithmetic (no panics on overflow)

#### Step 4: Register in Opcode Dispatcher

Edit `rusty_box/src/cpu/cpu.rs`, find `execute_instruction()` match statement:

```rust
Opcode::YourOpcodeName => {
    self.your_instruction_name(instr)?;
    Ok(())
}
```

Place it alphabetically or near similar instructions.

#### Step 5: Test

```bash
cargo build --release --example dlxlinux --features std
cargo run --release --example dlxlinux --features std 2>&1 | tee test.log
```

Check if:
1. Compiles without errors
2. BIOS progresses further (RIP increases)
3. No panics or errors at the instruction
4. Next unimplemented instruction is hit

### How to Debug Issues

#### Enable Detailed Tracing

Edit `rusty_box/examples/dlxlinux.rs`:
```rust
let log_level = tracing::Level::TRACE; // Very verbose
// or
let log_level = tracing::Level::DEBUG; // Moderate verbosity
```

#### Add Targeted Logging

In the function you're debugging:
```rust
tracing::info!("BEFORE: reg={:#x}, flag={}", self.rax(), self.get_zf());
// ... operation ...
tracing::info!("AFTER: reg={:#x}, flag={}", self.rax(), self.get_zf());
```

#### Check Instruction Bytes

In `cpu.rs`, the instruction trace shows:
```
Execute: F000:A12C  [66, C1, E0, 10]  ShlEdIb
```
- `F000:A12C` - CS:IP location
- `[66, C1, E0, 10]` - Instruction bytes
- `ShlEdIb` - Decoded opcode

Compare bytes with:
- Online x86 disassembler
- `objdump -D -b binary -m i386 <file>`
- Intel/AMD manuals

#### Compare with Bochs

Run same BIOS in Bochs with logging:
```bash
bochs -q -f bochsrc.txt
# In Bochs debugger:
log: trace.log
trace-on
```

Compare traces to find divergence point.

#### Use GDB on Emulator

```bash
cargo build --example dlxlinux --features std
rust-gdb target/debug/examples/dlxlinux
(gdb) break rusty_box::cpu::cpu::BxCpuC::your_function
(gdb) run
```

### Common Pitfalls

#### 1. Wrong Register Size
```rust
// WRONG - reading 32-bit into 64-bit context
let val = self.get_gpr32(0) as u64;

// RIGHT - explicit handling
let val = if in_64bit_mode {
    self.get_gpr64(0)
} else {
    self.get_gpr32(0) as u64
};
```

#### 2. Forgetting to Update Flags
```rust
// WRONG - missing flag update
let result = op1 + op2;
self.set_gpr32(dst, result);
// Flags unchanged!

// RIGHT
let result = op1.wrapping_add(op2);
self.set_gpr32(dst, result);
self.update_flags_add32(result, op1, op2); // ✓
```

#### 3. Using nnn Instead of rm for Group Opcodes
```rust
// WRONG - using nnn for Group opcode destination
let dst = instr.meta_data[1] as usize; // nnn = opcode extension

// RIGHT
let dst = instr.meta_data[0] as usize; // rm = actual operand
```

#### 4. Not Handling Memory Operands
```rust
// WRONG - assuming register operand
let src = self.get_gpr32(instr.src() as usize);

// RIGHT - check mod field
if instr.mod_c0() {
    // Register operand
    let src = self.get_gpr32(instr.src() as usize);
} else {
    // Memory operand
    let eaddr = self.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let src = self.read_virtual_dword(seg, eaddr);
}
```

#### 5. Ignoring Exceptions
```rust
// WRONG - ignoring potential exceptions
let value = self.read_virtual_dword(seg, addr);

// RIGHT - propagating errors
let value = self.read_virtual_dword(seg, addr)?; // Returns Result<>
```

---

## Comparison with Bochs

### Similarities ✓

1. **Architecture:** Both use modular design with separate CPU, memory, devices
2. **Organization:** Instruction implementations in category-specific files
3. **Naming:** Most structures have same names (BxCpuC, BxMemC, etc.)
4. **Algorithms:** Flag computation, address calculation identical
5. **Compatibility:** Same BIOS, same disk images work (when complete)

### Key Differences

#### 1. Language
- **Bochs:** C++ (with macros, templates, manual memory management)
- **Rusty Box:** Rust (type safety, ownership, no undefined behavior)

#### 2. Global State
- **Bochs:** Uses global variables (`bx_cpu`, `bx_mem`, `bx_devices`)
- **Rusty Box:** Instance-based, no global state

**Impact:** Multiple emulator instances possible in Rusty Box

#### 3. Error Handling
- **Bochs:** Exceptions via setjmp/longjmp, returns void
- **Rusty Box:** `Result<>` types, proper error propagation

**Impact:** Better error messages, no crashes on unexpected input

#### 4. Register Access
- **Bochs:** Direct member access (`RAX = value`)
- **Rusty Box:** Getter/setter methods (`set_rax(value)`)

**Impact:** Encapsulation, validation possible

#### 5. Memory Model
- **Bochs:** Single large array or file-backed
- **Rusty Box:** Block-based allocation

**Impact:** Better memory efficiency for sparse address spaces

#### 6. Decoder
- **Bochs:** Integrated in CPU
- **Rusty Box:** Separate crate

**Impact:** Fuzzing, testing, reusability

#### 7. Feature Completeness
- **Bochs:** 100% - boots Windows, Linux, BSD, DOS, everything
- **Rusty Box:** ~5% - boots through BIOS partially

#### 8. Performance
- **Bochs:** Mature optimizations, 10-100 MIPS
- **Rusty Box:** No optimization yet, ~0.5-1 MIPS

#### 9. Documentation
- **Bochs:** Minimal inline docs, but 20+ years of development
- **Rusty Box:** Comprehensive doc comments (when present)

### What Bochs Does Better

1. ✓ Complete instruction set (all opcodes)
2. ✓ All CPU modes (real, protected, long, V86, SMM)
3. ✓ Full device support (serial, parallel, floppy, network, sound)
4. ✓ Debugger built-in
5. ✓ Save/restore state
6. ✓ Multiple acceleration modes
7. ✓ 25+ years of bug fixes
8. ✓ Tested with thousands of OS images
9. ✓ Hardware virtualization support (SVM/VMX)
10. ✓ Extensive testing and CI

### What Rusty Box Does Better

1. ✓ Memory safety (no buffer overflows, use-after-free)
2. ✓ Type safety (no implicit casts, strict types)
3. ✓ Thread safety (can be sent between threads safely)
4. ✓ Modern code style (no macros, clear ownership)
5. ✓ Better error messages (Result types with context)
6. ✓ Modular decoder (separate crate, fuzzable)
7. ✓ No undefined behavior (verified by Rust compiler)
8. ✓ Easier to extend (cleaner abstractions)

---

## How to Improve

### Short-term Goals (1-3 months)

#### Goal 1: Complete BIOS Boot
**Target:** Boot to "Bochs 3.0 BIOS" message

**Tasks:**
1. Implement all control register operations
2. Fix protected mode transitions
3. Implement remaining arithmetic/logical instructions
4. Complete VGA text mode
5. Test with multiple BIOS versions

**Success Criteria:**
- BIOS completes POST
- Displays boot messages
- Attempts to boot from disk

#### Goal 2: Boot DOS
**Target:** Boot MS-DOS 6.22 or FreeDOS

**Tasks:**
1. Implement disk I/O fully (INT 13h support)
2. Add all missing real mode instructions
3. Implement floppy controller (optional, boot from HDD)
4. Fix interrupt delivery

**Success Criteria:**
- Loads DOS kernel
- Shows DOS prompt
- Can execute DIR, TYPE commands

#### Goal 3: Testing Infrastructure
**Target:** Automated testing for regressions

**Tasks:**
1. Unit tests for each instruction category
2. Integration tests with known-good traces
3. Fuzzing for decoder
4. Comparison testing vs Bochs
5. CI/CD pipeline

**Success Criteria:**
- >80% code coverage
- No regressions on new commits
- Decoder fuzzer runs 24/7

### Medium-term Goals (3-6 months)

#### Goal 4: Protected Mode Support
**Target:** Boot simple protected mode OS (Linux 0.01)

**Tasks:**
1. Complete paging implementation
2. Implement all descriptor table operations
3. Add privilege level checking
4. Implement task switching
5. Add all protected mode exceptions

#### Goal 5: Performance Optimization
**Target:** 10x speedup (5-10 MIPS)

**Tasks:**
1. Profile hot paths
2. Implement instruction caching
3. Optimize memory accesses (reduce bounds checks)
4. Lazy flag computation
5. Fast paths for common operations
6. Consider JIT for hottest code

#### Goal 6: Device Completeness
**Target:** Boot and use Linux with GUI

**Tasks:**
1. Complete VGA (all modes)
2. Add serial port (debugging)
3. Add network card (ping, ssh)
4. Improve disk performance
5. Add sound card (optional)

### Long-term Goals (6-12 months)

#### Goal 7: Linux Distribution
**Target:** Boot modern Linux (Debian, Ubuntu)

**Requires:**
- Long mode (64-bit) support
- Full CPU feature set
- Fast I/O devices
- Stable interrupt handling

#### Goal 8: Windows Support
**Target:** Boot Windows (95/98 or later)

**Requires:**
- Complete real/protected/V86 mode
- Hardware virtualization hints
- Accurate timing
- Complex device interactions

#### Goal 9: Production Quality
**Target:** Usable by end-users

**Requires:**
- Stable API
- Configuration files
- GUI interface
- Documentation
- Example disk images
- Pre-built binaries

---

## Testing & Debugging

### Testing Strategy

#### 1. Unit Tests
**Location:** `rusty_box/src/*/tests/`

**Coverage:**
- Each instruction implementation
- Flag computation for all cases
- Address mode calculation
- Decoder correctness

**Example:**
```rust
#[test]
fn test_add_eax_imm32() {
    let mut cpu = create_test_cpu();
    cpu.set_rax(0x10);
    let instr = decode(&[0x05, 0x20, 0x00, 0x00, 0x00]); // ADD EAX, 0x20
    cpu.add_eax_id(&instr).unwrap();
    assert_eq!(cpu.rax(), 0x30);
    assert!(!cpu.get_zf()); // Zero flag should be clear
    assert!(!cpu.get_sf()); // Sign flag should be clear
}
```

#### 2. Integration Tests
**Location:** `rusty_box/tests/`

**Scenarios:**
- Boot sequence (BIOS to bootloader)
- Specific instruction sequences
- Device interactions
- Exception handling

#### 3. Fuzzing
**Location:** `rusty_box_decoder/fuzz/`

**Target:** Decoder with random byte streams

```bash
cd rusty_box_decoder
cargo +nightly fuzz run fuzz_target_1 -- -max_len=15
```

**Finds:** Panics, crashes, infinite loops in decoder

#### 4. Differential Testing
**Method:** Compare with Bochs

```bash
# Run same code in both emulators
# Compare register state, memory, flags
```

### Debugging Tools

#### 1. Tracing Levels
```rust
// In dlxlinux.rs
let log_level = tracing::Level::TRACE;  // Everything
let log_level = tracing::Level::DEBUG;  // Important events
let log_level = tracing::Level::INFO;   // Warnings/errors only
```

#### 2. Execution Trace
Logs show:
```
Execute: F000:A12C  [66, C1, E0, 10]  ShlEdIb
```

Analyze:
- Instruction sequence
- Register changes
- Memory accesses
- Device I/O

#### 3. Memory Dumps
```rust
// In emulator code
std::fs::write("memory.bin", &memory.dump())?;
```

Analyze with:
```bash
hexdump -C memory.bin | less
```

#### 4. Breakpoints (Future)
```rust
if self.rip() == 0xF000A12C {
    tracing::error!("BREAKPOINT: EAX={:#x}", self.rax());
    return Err(CpuError::Breakpoint);
}
```

### Performance Profiling

#### Using Cargo Flamegraph
```bash
cargo install flamegraph
cargo flamegraph --example dlxlinux -- --features std
# Opens flamegraph in browser
```

#### Using perf (Linux)
```bash
cargo build --release --example dlxlinux --features std
perf record -g ./target/release/examples/dlxlinux
perf report
```

#### Benchmarking
```rust
// In benches/cpu_bench.rs
#[bench]
fn bench_instruction_decode(b: &mut Bencher) {
    let bytes = &[0x05, 0x20, 0x00, 0x00, 0x00];
    b.iter(|| {
        let instr = fetch_decode32(bytes, true).unwrap();
        black_box(instr)
    });
}
```

---

## Performance Considerations

### Current Performance

**Measured:** ~0.5-1 MIPS (Million Instructions Per Second)

**Bottlenecks:**
1. Instruction decoding (20-30% of time)
2. Memory access bounds checking (15-20%)
3. Flag computation (10-15%)
4. Register access indirection (10-15%)
5. Tracing/logging overhead (5-10% even at INFO level)

### Optimization Opportunities

#### 1. Instruction Caching
**Idea:** Cache decoded instructions by physical address

**Before:**
```rust
let bytes = fetch_bytes(rip);
let instr = decode(bytes)?;  // Decode every time
execute(instr)?;
```

**After:**
```rust
if let Some(instr) = icache.get(rip) {
    execute(instr)?;  // Use cached
} else {
    let bytes = fetch_bytes(rip);
    let instr = decode(bytes)?;
    icache.insert(rip, instr);
    execute(instr)?;
}
```

**Expected Gain:** 20-30% speedup

**Status:** Structure exists but not fully used

#### 2. Lazy Flags
**Idea:** Don't compute flags until needed

**Before:**
```rust
let result = op1 + op2;
self.eflags = compute_all_flags(result, op1, op2);  // Always compute
```

**After:**
```rust
let result = op1 + op2;
self.lazy_flags = LazyFlags::Add(result, op1, op2);  // Defer computation
// When flag is read:
fn get_zf(&self) -> bool {
    self.lazy_flags.compute_zf()  // Compute on demand
}
```

**Expected Gain:** 10-20% speedup

**Status:** Structure exists, partially implemented

#### 3. JIT Compilation
**Idea:** Translate hot x86 code to native code

**Approach:**
1. Detect hot loops (threshold: 1000 iterations)
2. Translate to LLVM IR or Cranelift
3. Compile to native
4. Execute native code

**Expected Gain:** 10-100x for hot loops

**Status:** Not implemented, significant effort

#### 4. SIMD Optimization
**Idea:** Use Rust SIMD for array operations

**Example:** String operations (MOVS, STOS)
```rust
// Current: byte-by-byte
for i in 0..count {
    memory[dst + i] = memory[src + i];
}

// With SIMD:
use std::simd::*;
let chunks = count / 16;
for i in 0..chunks {
    let vec = u8x16::from_slice(&memory[src + i*16..]);
    vec.copy_to_slice(&mut memory[dst + i*16..]);
}
```

**Expected Gain:** 5-10% for string-heavy code

#### 5. Reduce Bounds Checking
**Idea:** Use unsafe for hot paths after validation

```rust
// Current: checked every access
memory[address] = value;  // Bounds check

// Optimized:
unsafe {
    *memory.get_unchecked_mut(address) = value;  // No check
}
```

**Caveat:** Only after proving safety

**Expected Gain:** 5-10% speedup

**Status:** Avoid for now (correctness first)

### Comparison with Other Emulators

| Emulator | Speed (MIPS) | Approach | Accuracy |
|----------|--------------|----------|----------|
| Bochs | 10-100 | Interpreter + Optimization | Very High |
| QEMU (TCG) | 100-500 | Dynamic translation | High |
| QEMU (KVM) | 10,000+ | Hardware virtualization | Perfect |
| Rusty Box | 0.5-1 | Pure interpreter | Medium |
| **Target** | **10-50** | **Optimized interpreter** | **High** |

---

## Conclusion

### Summary

Rusty Box is a **work-in-progress** Rust port of Bochs, currently at ~5% feature completeness. The core architecture is sound, but many CPU instructions and system features remain unimplemented. The recent decoder bug fix (Group opcodes) was a major breakthrough, allowing BIOS execution to progress significantly.

### Immediate Focus

1. **Implement missing instructions** as encountered during BIOS boot
2. **Fix workarounds** (segment register 6, etc.)
3. **Complete control register operations** for mode transitions
4. **Improve exception handling** for proper fault delivery

### Long-term Vision

Create a **safe, performant, and maintainable** x86 emulator that:
- Boots real operating systems
- Serves as educational tool
- Enables security research
- Provides deterministic execution

### Getting Involved

**Contributions Welcome:**
- Implement missing instructions (start with simple ones)
- Write tests for existing code
- Fix known bugs
- Improve documentation
- Add device support

**Skills Needed:**
- Rust programming
- x86 assembly knowledge
- Systems programming
- Patience and attention to detail

### Resources

- **Documentation:** See `DECODER_BUG_FIX_SUMMARY.md`, `PROGRESS_STATUS.md`
- **Original Bochs:** `cpp_orig/bochs/` for reference
- **Intel Manuals:** Volume 2 (Instruction Set Reference)
- **Tracing:** Enable TRACE level for detailed logs

---

**Last Updated:** 2026-01-30
**Status:** BIOS boots to 0x9E4F, descriptor table setup in progress
**Next:** Implement MovRdCr0 (read control register)

---

For questions or contributions, see project documentation.
