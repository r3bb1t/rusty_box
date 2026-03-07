# Emulator Audit Status

Tracks all audit findings against Bochs C++ source. Prevents duplicate work across sessions.

**Last updated**: 2026-03-07 (Sessions 27-29)

---

## Audit Rounds

### Session 27: Device & Subsystem Audit (17 agents, 10 completed)
**Scope**: All I/O devices, CPU subsystems, paging, interrupt routing
**Method**: 17 parallel Opus agents comparing Rust vs Bochs C++

### Session 28: Decoder & CPU Accessor Audit (10 agents + manual)
**Scope**: fetchdecode32.rs, fetchdecode64.rs, CPU accessor methods, instruction struct
**Method**: 10 parallel Opus agents + manual decoder audit

### Session 29: Fix Application
**Scope**: Apply fixes from session 27-28 findings
**Method**: Sequential fix + DLX regression after each

---

## Bug Tracker

### Legend
- ✅ FIXED — verified with DLX regression
- 🔲 UNFIXED — confirmed bug, not yet fixed
- ⬜ LOW — minor/latent, not blocking

### CRITICAL Bugs

| ID | Description | Status | Session | Files |
|----|------------|--------|---------|-------|
| C1 | SSE prefix F2/F3 values SWAPPED in both decoders | ✅ FIXED (S29) | 28 | fetchdecode32.rs, fetchdecode64.rs |
| C2 | get_gpr32() returns 0 for R8D-R15D (indices 8-15) | ✅ FIXED (S29) | 28 | cpu.rs |
| C3 | REX.B not applied for non-ModRM opcodes in 64-bit decoder | ✅ FIXED (S29) | 28 | fetchdecode64.rs |
| C4 | Ed,Gd convention mask too broad (b1 & 0x0F catches two-byte) | ✅ FIXED (S28) | 28 | fetchdecode32.rs, fetchdecode64.rs |
| C5 | msr_fsbase()/msr_gsbase() read GPR array instead of segment base | ✅ FIXED (S29) | 28 | cpu_getters_and_setters.rs |

### HIGH Bugs

| ID | Description | Status | Session | Files |
|----|------------|--------|---------|-------|
| H1 | 0x67 prefix clears both As32 AND As64 in 64-bit decoder | ✅ FIXED (S29) | 28 | fetchdecode64.rs |
| H2 | Missing ~95 two-byte opcode tables (0F 50-7F, 0F D0-FE) | 🔲 UNFIXED | 28 | fetchdecode32.rs, fetchdecode64.rs |
| H3 | System read/write cross-page addresses truncated to 32 bits | ✅ FIXED (S29) | 28 | access.rs |
| H4 | 64-bit virtual write functions skip SMC write stamp check | 🔲 UNFIXED | 28 | access.rs |
| H5 | 64-bit virtual read/write skip TLB fast path (PERF) | 🔲 UNFIXED | 28 | access.rs |
| H6 | 8-bit register handlers use get_gpr8() not read_8bit_regx() | ✅ FIXED (S29) | 28 | logical8.rs, data_xfer32.rs |
| H7 | XOP prefix check missing mod field validation (32-bit decoder) | ✅ FIXED (S29) | 28 | fetchdecode32.rs |
| H8 | LEAVE (0xC9) missing from 64-bit no-ModRM list | ✅ FIXED (S28) | 28 | fetchdecode64.rs |
| H9 | INS/OUTS (0x6C-0x6F) missing from 64-bit no-ModRM list | ✅ FIXED (S28) | 28 | fetchdecode64.rs |
| H10 | FEMMS (0F 0E) and UD0 (0F FF) missing from no-ModRM lists | ✅ FIXED (S29) | 28 | fetchdecode32.rs, fetchdecode64.rs |
| H11 | NOP (0x90) not distinguished from XCHG EAX,EAX; PAUSE missing | 🔲 UNFIXED | 28 | Both decoders |
| H12 | LAPIC timer current count register returns stale value | 🔲 UNFIXED | 27 | apic.rs |
| H13 | linaddr_width never updated after CR0/CR4 writes | 🔲 UNFIXED | 27 | crregs.rs |
| H14 | 184 missing 64-bit opcodes in dispatcher | 🔲 UNFIXED | 27 | dispatcher.rs |
| H15 | MovCr2rq uses 32-bit handler — truncates 64-bit CR2 | 🔲 UNFIXED | 27 | dispatcher.rs |

### MEDIUM Bugs

| ID | Description | Status | Session | Files |
|----|------------|--------|---------|-------|
| M1 | Non-ModRM opcodes excluded from decmask NNN/RRR fields (64-bit) | 🔲 UNFIXED | 28 | fetchdecode64.rs |
| M2 | Missing SRC_EQ_DST bit in decmask (both decoders) | 🔲 UNFIXED | 28 | Both decoders |
| M3 | LOCK prefix validation missing (post-decode) | 🔲 UNFIXED | 28 | Both decoders |
| M4 | Logic operations don't clear AF | ✅ FIXED (S29) | 28 | eflags.rs |
| M5 | Group 3 (F6/F7) immediate check uses REX-extended nnn | ✅ FIXED (S29) | 28 | fetchdecode64.rs |
| M6 | LOCK prefix overwritten by subsequent F2/F3 | 🔲 UNFIXED | 28 | fetchdecode64.rs |
| M7 | Segment default not set from base register in 64-bit decoder | 🔲 UNFIXED | 28 | fetchdecode64.rs |
| M8 | MOVNTI (0F C3) in wrong operand convention | ✅ FIXED (S29) | 28 | fetchdecode32.rs, fetchdecode64.rs |
| M9 | UD64 opcodes decode as valid in 64-bit decoder | 🔲 UNFIXED | 28 | fetchdecode64.rs |
| M10 | DisplacementData not a union — struct 4 bytes too large | 🔲 UNFIXED | 28 | instr_generated.rs |
| M11 | Missing canonical address checks in 64-bit access functions | 🔲 UNFIXED | 28 | access.rs |
| M12 | PIT fractional tick accumulation (16% drift) | 🔲 UNFIXED | 27 | pit.rs |
| M13 | CMOS reset doesn't clear PIE/AIE/UIE | 🔲 UNFIXED | 27 | cmos.rs |
| M14 | iret64 NMI unblocking wrong | 🔲 UNFIXED | 27 | ctrl_xfer64.rs |
| M15 | handle_cpu_mode_change missing update_fetch_mode_mask | 🔲 UNFIXED | 27 | crregs.rs |

### LOW Bugs

| ID | Description | Status | Session |
|----|------------|--------|---------|
| L1 | rep_used_l() dead code | ⬜ LOW | 28 |
| L2 | from_u16_const() unsound for large values | ⬜ LOW | 28 |
| L3 | Magic number 19 instead of BX_NIL_REGISTER | ⬜ LOW | 28 |
| L4 | Duplicate SsePrefix enum definitions | ⬜ LOW | 28 |
| L5 | BxDecodeError default returns wrong variant | ⬜ LOW | 28 |
| L6 | BxInstruction→Instruction loses modrm_form | ⬜ LOW | 28 |
| L7 | .to_owned() on Copy types in register getters | ⬜ LOW | 28 |
| L8 | Missing 0F 24/0F 26 (MOV test registers) | ⬜ LOW | 28 |
| L9 | Duplicate flag update functions | ⬜ LOW | 28 |
| L10 | FPU escape (0xD8-0xDF) missing from 64-bit table | ⬜ LOW | 28 |
| L11 | VEX detection 64-bit checks mod field | ⬜ LOW | 28 |
| L12 | INTO (Int0) missing from dispatcher | ⬜ LOW | 28 |
| L13 | DMA transfer engine entirely missing | ⬜ LOW | 27 |
| L14 | PIC spurious IRQ7 not detected | ⬜ LOW | 27 |
| L15 | ATA HOB register readback missing | ⬜ LOW | 27 |

---

## Files Audited (complete list)

### Session 28 — Decoder & CPU Core
| File | Audited By | Status | Notes |
|------|-----------|--------|-------|
| fetchdecode32.rs | 2 agents + manual | ✅ Complete | C1,C4,H7,H10,M8 found+fixed |
| fetchdecode64.rs | 2 agents + manual | ✅ Complete | C1,C3,H1,H8,H9,H10,M5 found+fixed |
| cpu.rs (get_gpr32) | 2 agents | ✅ Complete | C2 found+fixed |
| cpu_getters_and_setters.rs | 2 agents | ✅ Complete | C5 found+fixed |
| instr_generated.rs | 2 agents | ✅ Complete | M10 found (unfixed, minor) |
| logical8.rs | 1 agent | ✅ Complete | H6 found+fixed |
| data_xfer32.rs (MOVZX) | 1 agent | ✅ Complete | H6 found+fixed |

### Session 27 — Devices & Subsystems
| File | Audited By | Status | Notes |
|------|-----------|--------|-------|
| access.rs | 1 agent | ✅ Complete | H3,H4,H5,M11 found; H3 fixed |
| paging.rs | 2 agents | ✅ Complete | Minor error code diffs |
| pic.rs | 1 agent | ✅ Complete | L14 (spurious IRQ7) |
| pit.rs | 1 agent | ✅ Complete | M12 (fractional drift) |
| cmos.rs | 1 agent | ✅ Complete | M13 (reset bits) |
| dma.rs | 1 agent | ✅ Complete | L13 (no transfer engine) |
| devices.rs | 1 agent | ✅ Complete | IOAPIC routing FIXED |
| ioapic.rs | 1 agent (S21) | ✅ Complete | Clean |
| harddrv.rs | 1 agent | ✅ Complete | L15 (HOB missing) |
| vga.rs | 1 agent | ✅ Complete | Text-only stub noted |
| keyboard.rs | — | Not audited | — |
| serial.rs | — | Not audited | — |
| ctrl_xfer64.rs | 1 agent | ✅ Complete | M14 (iret64 NMI) |
| dispatcher.rs | 1 agent | ✅ Complete | H14 (184 missing opcodes) |
| crregs.rs | 1 agent | ✅ Complete | H13,M15 found |
| apic.rs | 1 agent | ✅ Complete | H12 found |
| eflags.rs | — | Fixed (M4) | LOGIC_MASK AF added |

### Previously Audited (Sessions 16-25)
| File | Status | Notes |
|------|--------|-------|
| tlb.rs | ✅ Clean (S21) | TLB structure matches Bochs |
| icache.rs | ✅ Clean (S21) | SMC detection present |
| descriptor.rs | ✅ Clean (S21) | L bit, gate types correct |
| string.rs | ✅ Fixed (S21) | REP async_event check noted |
| tasking.rs | ✅ Audited (S20) | Full task_switch present |
| data_xfer64.rs | ✅ Audited (S20) | 963L reviewed |
| smm.rs | ✅ Audited (S20) | 766L reviewed |
| protected_interrupts.rs | ✅ Audited (S20) | 1227L reviewed |
| segment_ctrl_pro.rs | ✅ Audited (S18) | check_cs L+D_B validated |
| exception.rs | ✅ Audited (S18) | DR6 commit, long_mode_int |
| emulator.rs | ✅ Audited (S20) | 2752L, diagnostic bloat |
| arith8/16/32 | ✅ Clean (S16) | All match Bochs |
| logical16/32/64 | ✅ Clean (S16) | All match Bochs |
| shift8/16/32 | ✅ Clean (S16) | All match Bochs |
| mult8/16/32/64 | ✅ Clean (S16) | All match Bochs |
| bit/bcd | ✅ Clean (S16) | All match Bochs |
| fpu/*.rs (9 files) | ✅ Audited (S16) | FXTRACT precision fixed |

---

## Alpine Boot Blockers (Priority Order)

1. **64-bit paging bypass** — FIXED (S26): All 64-bit memory access now page-walks
2. **Decoder bugs** — FIXED (S28-29): SSE prefix, get_gpr32, REX.B, Ed,Gd, etc.
3. **Missing SSE opcode tables (H2)** — ~95 entries needed for any SSE execution
4. **Missing dispatcher entries (H14)** — 184 opcodes decoded but not dispatched
5. **LAPIC timer stale read (H12)** — timer calibration returns wrong value
6. **linaddr_width never updated (H13)** — may affect address masking
