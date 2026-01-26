---
name: DLX Linux Boot Plan
overview: A comprehensive plan to make rusty_box emulator fully boot DLX Linux by implementing missing CPU instructions, device emulation, protected mode support, and VGA display. Based on original Bochs C++ source code as the source of truth.
todos:
  - id: protected-mode
    content: "Implement protected mode: CR0 write, LGDT, LIDT, segment descriptor loading"
    status: completed
  - id: 32bit-instrs
    content: "Add 32-bit instruction variants: PUSHAD/POPAD, arithmetic, data transfer"
    status: completed
  - id: mul-div
    content: Implement MUL/IMUL/DIV/IDIV for 8/16/32-bit operands
    status: completed
  - id: protected-int
    content: Implement protected mode interrupt handling via IDT gates
    status: completed
  - id: vga-text
    content: Create VGA text mode emulation (80x25, 0xB8000 memory)
    status: completed
  - id: paging
    content: Implement CR3 paging with 2-level page tables
    status: completed
  - id: string-32bit
    content: "Complete string instructions: MOVSD, STOSD, CMPS, SCAS, INS, OUTS"
    status: completed
---

# DLX Linux Full Boot Implementation Plan

## Overview

To boot DLX Linux to a login prompt, the emulator must execute through BIOS POST, real-mode initialization, protected mode transition, and Linux kernel boot. This requires completing CPU instruction emulation, device I/O, interrupt handling, and display output.---

## Phase 1: Core CPU Instructions (Critical)

### 1.1 Protected Mode Support

Linux transitions from real mode to protected mode. This is **mandatory** for kernel boot.**Original Source:**

- [`cpp_orig/bochs/cpu/segment_ctrl_pro.cc`](cpp_orig/bochs/cpu/segment_ctrl_pro.cc) - `load_seg_reg()` (lines 30-200)
- [`cpp_orig/bochs/cpu/protect_ctrl.cc`](cpp_orig/bochs/cpu/protect_ctrl.cc) - LGDT, LIDT, LLDT, LTR
- [`cpp_orig/bochs/cpu/crregs.cc`](cpp_orig/bochs/cpu/crregs.cc) - CR0/CR3/CR4 access

**Key Instructions:**

| Instruction | Bochs Source | Purpose |

|-------------|--------------|---------|

| `MOV CRn, reg` | `crregs.cc:49-150` | Set CR0.PE=1 for protected mode |

| `LGDT` | `protect_ctrl.cc:140-180` | Load Global Descriptor Table |

| `LIDT` | `protect_ctrl.cc:181-220` | Load Interrupt Descriptor Table |

| `LLDT` | `protect_ctrl.cc:221-260` | Load LDT selector |

| `LTR` | `protect_ctrl.cc:261-310` | Load Task Register |**Rust implementation:** Expand [`rusty_box/src/cpu/segment_ctrl_pro.rs`](rusty_box/src/cpu/segment_ctrl_pro.rs) with full descriptor validation.

### 1.2 32-bit Instruction Variants

Many instructions have 16/32-bit versions. BIOS uses 16-bit, Linux uses 32-bit.**Original Source:**

- [`cpp_orig/bochs/cpu/arith32.cc`](cpp_orig/bochs/cpu/arith32.cc) - 32-bit arithmetic
- [`cpp_orig/bochs/cpu/logical32.cc`](cpp_orig/bochs/cpu/logical32.cc) - 32-bit logic
- [`cpp_orig/bochs/cpu/data_xfer32.cc`](cpp_orig/bochs/cpu/data_xfer32.cc) - 32-bit data transfer
- [`cpp_orig/bochs/cpu/stack32.cc`](cpp_orig/bochs/cpu/stack32.cc) - 32-bit stack ops
- [`cpp_orig/bochs/cpu/ctrl_xfer32.cc`](cpp_orig/bochs/cpu/ctrl_xfer32.cc) - 32-bit control flow

**Key Instructions needed:**

- `PUSH/POP r32`, `PUSHAD`, `POPAD`
- `ADD/SUB/AND/OR/XOR Ed, Gd` and `Gd, Ed`
- `MOV Ed, Id`, `MOV Gd, Ed`
- `CALL/RET/JMP` 32-bit variants
- `MOVZX Gd, Eb/Ew`, `MOVSX Gd, Eb/Ew`

### 1.3 Multiplication and Division

BIOS POST performs memory sizing using MUL/DIV.**Original Source:**

- [`cpp_orig/bochs/cpu/mult16.cc`](cpp_orig/bochs/cpu/mult16.cc) - MUL, IMUL, DIV, IDIV 16-bit
- [`cpp_orig/bochs/cpu/mult32.cc`](cpp_orig/bochs/cpu/mult32.cc) - MUL, IMUL, DIV, IDIV 32-bit

**Key Instructions:**

| Instruction | Bochs Source | Notes |

|-------------|--------------|-------|

| `MUL AL/AX/EAX` | `mult{8,16,32}.cc` | Unsigned multiply |

| `IMUL` | `mult{8,16,32}.cc` | Signed multiply (1, 2, 3 operand forms) |

| `DIV` | `mult{8,16,32}.cc` | Unsigned divide |

| `IDIV` | `mult{8,16,32}.cc` | Signed divide |

### 1.4 String Instructions (Complete)

REP-prefixed string operations are heavily used.**Original Source:**

- [`cpp_orig/bochs/cpu/string.cc`](cpp_orig/bochs/cpu/string.cc) - All string operations

**Existing:** Basic `MOVSB`, `STOSB`, `LODSB` in [`rusty_box/src/cpu/string.rs`](rusty_box/src/cpu/string.rs)**Missing:**

- `MOVSD`, `STOSD`, `LODSD` (32-bit variants)
- `CMPSB/W/D`, `SCASB/W/D` (comparison variants)
- `REP/REPE/REPNE` prefix handling for all variants
- `INSB/W/D`, `OUTSB/W/D` (port string I/O)

---

## Phase 2: Exception and Interrupt Handling

### 2.1 Real Mode Interrupts (Mostly Complete)

**Current:** [`rusty_box/src/cpu/soft_int.rs`](rusty_box/src/cpu/soft_int.rs) has basic IVT handling.**Verify against:** [`cpp_orig/bochs/cpu/exception.cc`](cpp_orig/bochs/cpu/exception.cc) lines 731-760 (`real_mode_int`)

### 2.2 Protected Mode Interrupts (Required for Linux)

Linux uses protected mode IDT with interrupt gates.**Original Source:**

- [`cpp_orig/bochs/cpu/exception.cc`](cpp_orig/bochs/cpu/exception.cc) lines 269-580 (`protected_mode_int`)
- [`cpp_orig/bochs/cpu/iret.cc`](cpp_orig/bochs/cpu/iret.cc) - IRET protected mode

**Implementation:**

1. Parse IDT gate descriptors (interrupt gate, trap gate)
2. Privilege level checks (DPL, CPL, RPL)
3. Stack switching on ring transition
4. Proper IRET with stack restoration

### 2.3 Hardware Interrupt Delivery

Timer (IRQ0) and keyboard (IRQ1) interrupts must work.**Integration points:**

- PIC EOI handling in [`rusty_box/src/iodev/pic.rs`](rusty_box/src/iodev/pic.rs)
- CPU INTR pin checking in main loop
- IF flag respect

---

## Phase 3: Device Emulation

### 3.1 VGA Display (Critical for User Interaction)

Without VGA, there's no visible output.**Original Source:**

- [`cpp_orig/bochs/iodev/display/vgacore.cc`](cpp_orig/bochs/iodev/display/vgacore.cc) - Core VGA logic
- [`cpp_orig/bochs/iodev/display/vga.cc`](cpp_orig/bochs/iodev/display/vga.cc) - VGA adapter

**Required I/O ports:**

| Port Range | Purpose | Priority |

|------------|---------|----------|

| 0x3C0-0x3CF | VGA attribute/sequencer/graphics | High |

| 0x3D4-0x3D5 | CRTC controller | High |

| 0x3DA | Status register | High |

| 0xA0000-0xBFFFF | Video memory | High |**Minimum Implementation:**

1. Text mode (80x25) with character/attribute memory at 0xB8000
2. CRTC cursor position registers
3. Basic attribute controller

**Create:** `rusty_box/src/iodev/vga.rs`

### 3.2 ATA/IDE Hard Drive (Existing but Verify)

**Current:** [`rusty_box/src/iodev/harddrv.rs`](rusty_box/src/iodev/harddrv.rs)**Original Source:**

- [`cpp_orig/bochs/iodev/hdimage/hdimage.cc`](cpp_orig/bochs/iodev/hdimage/hdimage.cc)
- [`cpp_orig/bochs/iodev/hd/harddrv.cc`](cpp_orig/bochs/iodev/hd/harddrv.cc)

**Verify:**

- ATA IDENTIFY DEVICE command
- READ SECTORS command (PIO mode)
- Proper status register handling
- IRQ14 generation

### 3.3 PIT Timer (Existing but Verify)

**Current:** [`rusty_box/src/iodev/pit.rs`](rusty_box/src/iodev/pit.rs)**Original Source:**

- [`cpp_orig/bochs/iodev/pit.cc`](cpp_orig/bochs/iodev/pit.cc)

**Verify:**

- Mode 2 (rate generator) and Mode 3 (square wave) for timer
- IRQ0 generation at proper intervals
- Counter read-back

### 3.4 PIC (Existing but Verify)

**Current:** [`rusty_box/src/iodev/pic.rs`](rusty_box/src/iodev/pic.rs)**Verify:**

- ICW1-ICW4 initialization sequence
- IRQ masking via OCW1
- EOI handling (OCW2)
- Cascade mode for slave PIC

### 3.5 CMOS/RTC (Existing)

**Current:** [`rusty_box/src/iodev/cmos.rs`](rusty_box/src/iodev/cmos.rs)**Verify CMOS values for:**

- Base/extended memory size (registers 0x15-0x18, 0x30-0x31)
- Boot device (register 0x2D)
- Hard disk type (registers 0x12, 0x19, 0x1A)

### 3.6 Keyboard Controller (Existing)

**Current:** [`rusty_box/src/iodev/keyboard.rs`](rusty_box/src/iodev/keyboard.rs)**Verify:**

- Self-test (0xAA command)
- A20 gate control
- Key scan code generation for login input

---

## Phase 4: Memory Management

### 4.1 A20 Gate

**Current:** [`rusty_box/src/pc_system.rs`](rusty_box/src/pc_system.rs) and [`rusty_box/src/iodev/devices.rs`](rusty_box/src/iodev/devices.rs) (Port 0x92)**Verify:** Memory accesses apply A20 mask correctly in [`rusty_box/src/memory/mod.rs`](rusty_box/src/memory/mod.rs).

### 4.2 Paging (Required for Linux)

Linux enables paging after protected mode.**Original Source:**

- [`cpp_orig/bochs/cpu/paging.cc`](cpp_orig/bochs/cpu/paging.cc) - Page table walking

**Implementation:**

1. CR3 page directory base
2. Two-level page table walk (PDE → PTE)
3. TLB caching (optional for correctness)
4. Page fault (#PF) exception handling

---

## Phase 5: Integration and Testing

### 5.1 Instruction Execution Tracing

Add opcode hit counters to identify remaining gaps:

```rust
// In cpu_loop, track which opcodes are hit
match opcode {
    Opcode::Unknown => { unimplemented_count += 1; }
    _ => { /* execute */ }
}
```

### 5.2 BIOS Checkpoints

BIOS outputs diagnostic codes to port 0x80. Add handler:

```rust
// Port 0x80 handler
fn post_code_write(_: *mut c_void, _: u16, value: u32, _: u8) {
    tracing::info!("POST code: {:#04x}", value);
}
```

### 5.3 Boot Sequence Validation

1. **BIOS POST** → Memory check, device init
2. **Boot sector load** → Read MBR from HD
3. **Linux boot** → Protected mode, kernel decompression
4. **Login prompt** → VGA text output

---

## Priority Order

| Priority | Component | Effort | Impact |

|----------|-----------|--------|--------|

| 1 | Protected mode (CR0, LGDT, segment loading) | High | Blocks Linux |

| 2 | VGA text mode | Medium | No visible output |

| 3 | 32-bit instruction variants | Medium | Blocks kernel |

| 4 | MUL/DIV instructions | Low | BIOS crashes |

| 5 | Protected mode interrupts | Medium | Linux crashes |

| 6 | Paging | Medium | Linux crashes |

| 7 | Complete string instructions | Low | Performance |---

## File Structure Mapping

| Bochs File | Rust File | Status |

|------------|-----------|--------|

| `cpu/arith16.cc` | `cpu/arith.rs` | Partial |

| `cpu/arith32.cc` | `cpu/arith.rs` | Missing |

| `cpu/ctrl_xfer16.cc` | `cpu/ctrl_xfer.rs` | Partial |

| `cpu/ctrl_xfer32.cc` | `cpu/ctrl_xfer.rs` | Missing |

| `cpu/stack16.cc` | `cpu/stack.rs` | Partial |

| `cpu/stack32.cc` | `cpu/stack.rs` | Missing |

| `cpu/exception.cc` | `cpu/soft_int.rs` + NEW | Partial |

| `cpu/segment_ctrl_pro.cc` | `cpu/segment_ctrl_pro.rs` | Minimal |

| `cpu/protect_ctrl.cc` | NEW | Missing |

| `cpu/crregs.cc` | `cpu/crregs.rs` | Exists, verify |

| `cpu/paging.cc` | `cpu/paging.rs` | Exists, verify |

| `cpu/mult16.cc` | NEW | Missing |

| `cpu/mult32.cc` | NEW | Missing |