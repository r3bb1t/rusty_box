# ISOLINUX Boot Debugging Journal

## Overview
Debugging why ISOLINUX 6.04 from Alpine Linux 3.23.3 x86_64 ISO never loads ldlinux.c32.

## Expected Boot Sequence (from screenshots)
1. ISOLINUX displays: `ISOLINUX 6.04 6.04-pre1 ETCD Copyright (C) 1994-2015 H. Peter Anvin et al`
2. `boot:` prompt appears (auto-boot or user presses Enter)
3. Kernel loads, OpenRC init runs, reaches `localhost login:`
4. Alpine Linux 3.23 with kernel 6.18.7-0-virt (x86_64)

## Actual Behavior (Session 22 — 2026-03-06, after channel fix)

**CD-ROM channel fix:** Changed `attach_cdrom(0, 1, ...)` (primary slave) to `attach_cdrom(1, 0, ...)`
(secondary master) to match Bochs config `ata1-master: type=cdrom`. This fixed ATAPI reads.

**Current behavior:**
1. BIOS POST completes (256MB RAM, PCI, ACPI, VGA BIOS)
2. El Torito boot reads boot catalog + boot image from CD-ROM
3. ISOLINUX stub loads itself (20 sectors: LBA 57×15 + LBA 72×5 = 40K)
4. Stub enters PM, runs 175K instructions of initialization
5. Stub prints partial banner via VGA INT 10h (AH=09 char writes): " Copyri 2tel" visible
6. Stub enters keyboard polling loop (INT 16h AH=11) — never exits
7. **NO additional ATAPI reads after initial stub load** — ldlinux.c32 never loaded

**Key difference from previous session:** ATAPI reads now WORK (ATA_rd=23942), stub PRINTS
characters via VGA, but still fails to issue disk reads for ldlinux.c32.

## ISO Structure (verified)
- `/boot/syslinux/isolinux.bin` — LBA 56, 43008 bytes (21 sectors)
- `/boot/syslinux/ldlinux.c32` — LBA 24335, 115128 bytes
- `/boot/syslinux/syslinux.cfg` — LBA 24499, 215 bytes
- `/boot/vmlinuz_virt.` — LBA 27542, 12465152 bytes

## Boot Info Table (verified valid)
At offset 8 of isolinux.bin (also at physical 0x7C08 in RAM):
- PVD LBA: 16 (0x10) — correct, verified CD001 signature
- Boot file LBA: 56 (0x38) — correct
- Boot file len: 43008 (0xA800) — correct
- Checksum: 0x22818AEB

## SYSLINUX 6.x Architecture
In SYSLINUX 6.x, `isolinux.bin` is a stub + PM core (~43KB). It does NOT contain
the full bootloader — `ldlinux.c32` (~115KB) is the real engine. The stub:
1. Loads itself from CD via real-mode INT 13h
2. Enters protected mode
3. Parses boot info table to find PVD
4. Reads ISO9660 filesystem to find ldlinux.c32
5. Loads and executes ldlinux.c32

ldlinux.c32 then:
- Displays the banner ("ISOLINUX 6.04 ...")
- Reads syslinux.cfg
- Shows boot: prompt
- Loads kernel + initrd
- Boots kernel

## PM→RM Bounce Architecture (fully decoded)

### Three bounce targets (EBX values at PM→RM transition)

**0x84A7 — `__intcall` (software INT calls)**
```asm
0x84A7: POP GS; POP FS; POP ES; POP DS; POPAD; POPFD; RET 4
```
Entry at 0x89A0: `PUSH EBP; MOV BX, 0x84A7; JMP 0x89C8`
- Loads pre-configured registers from RM stack
- RET 4 jumps to INT nn trampoline
- Used for INT 12h, INT 15h, etc.

**0x8662 — `__farcall` (arbitrary RM function calls)**
```asm
0x8662: MOV AX, SP; ADD AX, 0x2C; MOV [0x3AD4], AX  ; save recovery point
0x866A: POP GS; POP FS; POP ES; POP DS; POPAD; POPFD
0x8674: RETF                                           ; far-return to target
; After BIOS handler returns:
0x8675: MOV SP, CS:[0x3AD4]     ; restore stack
0x867A: PUSHFD; PUSHAD; PUSH DS/ES/FS/GS
0x8684: MOV EBX, 0x00100065     ; PM return address
0x868A: JMP 0x84B4              ; re-enter PM
```
- Calls any RM function by address (via RETF to stack-supplied CS:IP)
- The dominant operation: ~19,000 calls

**0x8523 — Hardware IRQ handler**
```asm
0x8523: PUSHF; CALL FAR [IVT[vector]]
```
Entry at 0x89D7: `PUSHAD; MOVZX ESI, [ESP+0x20]; INC [0x3AD8]; MOV EBX, 0x8523; JMP 0x89C8`
- Called for timer, keyboard, and other hardware interrupts
- Uses PUSHF + CALL FAR to simulate INT instruction

### Common PM→RM transition code
```asm
0x89C8: CLI; CLD; MOV [0x8E78], ESP    ; save PM stack
        JMP FAR 0x0010:0x84EE           ; to 16-bit code segment
0x84EE: Load 16-bit segment regs; LIDT for RM; clear CR0.PE
        JMP FAR 0:0x850F               ; to real mode
0x850F: LSS SP, [0x38B8]; JMP BX       ; load RM stack, jump to handler
```

### PM re-entry code (from RM)
```asm
0x84B4: CLI; LSS SP, [0x38B8]; set up GDT/IDT; set CR0.PE
        JMP FAR 0x0020:0x89A8          ; to 32-bit PM
0x89A8: XOR EAX; MOV FS/GS, AX; LLDT AX; MOV ES/DS/SS=0x28; LTR 0x08
        MOV ESP, [0x8E78]; JMP EBX     ; restore PM stack, return to caller
```

## Diagnostic Data (at 20M instructions)

### PM↔RM Transitions
```
PM→RM transitions: 71,437
RM→PM transitions: 71,438
```

### Transition Timeline (from first 30 transitions)
```
#1:  EBX=0x0A00 icount=1,406,280   — BIOS rombios32 RM return
#2:  EBX=0x84A7 icount=2,331,895   — __intcall
#3:  EBX=0x84A7 icount=2,332,002   — __intcall (107 instr later)
#4:  EBX=0x8662 icount=2,333,209   — __farcall begins
#5:  EBX=0x8662 icount=2,333,612   — (403 instr/iter)
#6:  EBX=0x8662 icount=2,334,011   — (399 instr/iter)
#7:  EBX=0x8662 icount=2,334,244   — (233 instr/iter)
                                    [175K instruction gap - pure PM code]
#8:  EBX=0x84A7 icount=2,509,174   — __intcall
#9:  EBX=0x8662 icount=2,509,375   — __farcall resumes
#10: EBX=0x8662 icount=2,509,591

#1K-#19K: EBX=0x8662 (steady ~263 instr/iter)
#20K+:    EBX=0x8523 (hardware IRQ bounces start at ~9.9M icount)
```

### Interrupt Delivery
```
IRQ0 (timer): ~26,309 via inject, ~8 via iac
IRQ1 (keyboard): 1 delivery (our Enter injection at 5M instructions)
PIC NOT remapped: still at default vectors 0x08-0x0F
Zero exceptions (#GP, #PF, #UD, etc.)
Zero IaErrors (decoder failures)
INT 10h calls: ZERO (no display output)
```

### ATAPI Commands (all from initial load)
```
First batch (~1.77M icount):
  AH=4Bh (Get El Torito Status)
  READ(10) LBA=57, 15 sectors → buffer 0x0880:0000
  READ(10) LBA=72, 5 sectors  → buffer 0x1000:0000

Second batch (~6.47M icount): IDENTICAL reads
  (both from CS:IP=0000:7F0F = RM stub)

ZERO additional ATAPI commands after initial load
```

### Memory Search Results
```
"isolinux" found at:
  0x813C: "ISOLINUX 6.04 6.04-pre1" (stub string)
  0x8185: "isolinux: No boot info table..." (error string, NOT triggered)
  0x10962E: "isolinux.cfg.syslinux.cfg./boot/isolinux./boot/syslinux." (search paths)

"SERIAL" — NOT FOUND (config never loaded)
"TIMEOUT" — NOT FOUND (config never loaded)
"vmlinuz" — NOT FOUND (kernel path never loaded)
```

## Key Observations (updated Session 22)

1. **ldlinux.c32 was NEVER loaded** — zero ATAPI reads to LBA 24335
2. **syslinux.cfg was NEVER read** — zero ATAPI reads to LBA 24499
3. **VGA output IS happening** — INT 10h AH=09 (write char) via __farcall PM→RM bounce
4. **Banner text partially visible** — " Copyri 2tel" (copyright banner, sampled every 10th farcall)
5. **The stub IS running** — PM→RM transitions for VGA + keyboard BIOS calls
6. **Keyboard polling loop** — INT 16h AH=11 from farcall #5000+ (steady ~4.5M instructions onward)
7. **Only 3 __intcall calls** (#2, #3, #8) — #2/#3 are 107 instructions apart (very fast return)
8. **175K instruction gap** between VGA init (#4-#7) and character printing (#8+) — PM-only code
9. **ATAPI reads work** — READ CAPACITY + READ(10) all succeed, ATA_rd=23942

### Boot Timeline (Session 22, 10M instructions)
```
icount 0-1.4M:     BIOS POST (real mode + rombios32 PM)
icount 1.407M:      PM→RM #1 (BIOS returns to real mode)
icount 1.628M:      VGA BIOS text output
icount 1.770M:      El Torito boot: READ(10) LBAs 17,55,56 (boot catalog + image)
                    "Booting from 07c0:0000"
                    ISOLINUX stub self-loads: LBA 57×15 + LBA 72×5
icount 1.77M:       ATA_rd=23942 (final value, never increases)
icount 2.0-2.8M:    ISOLINUX PM code (CS=0020) initialization
icount 2.887M:      __intcall #2 (first PM BIOS call)
icount 2.887M:      __intcall #3 (107 instr later — very fast!)
icount 2.889M:      __farcall #4-#7 (VGA init: INT 10h AH=0F, 00, 02, 02)
icount 2.890-3.065M: Pure PM code (175K instructions — initialization)
icount 3.065M:      __intcall #8 → __farcall #9+ (banner printing begins)
icount 3.065-3.13M: Character output: " Copyri..." (AH=09 writes)
icount 3.13M+:      Keyboard polling: INT 16h AH=11 in loop
icount 4.5M+:       Still polling (farcall #5000)
icount 10M:         Still polling (farcall #15000)
```

## Session 16 Deep Dive: El Torito + INT 13h Tracing (2026-03-05)

### El Torito Boot Flow (verified working)

The BIOS El Torito boot sequence works correctly:
1. BIOS reads Boot Record Volume Descriptor at LBA 17 (0x11)
2. Reads Boot Catalog at LBA specified in BRVD
3. Boot Catalog entry: media=0 (no-emulation), load_seg=0x07C0, sectors=4, LBA=56
4. BIOS loads 4 sectors (2048 bytes) from LBA 56 to 0x7C00
5. Jumps to 0x07C0:0x0000 (normalized to 0x0000:0x7C00)

### EBDA cdemu_t Structure Location

**Key finding:** The EBDA cdemu_t structure is at EBDA+0x25A, NOT EBDA+0x1DB.

**Investigation path:**
- Calculated offset 0x1DB from rombios.c struct definitions (cdemu_t follows mouse, hard_drive, ata structs)
- Memory at EBDA+0x1DB was all zeros — wrong offset
- Scanned EBDA memory for signature pattern (emulated_drive=0xE0 near ilba=56)
- Found correct data at EBDA+0x25A (127 bytes later than calculated)
- **Root cause:** Bochs BIOS was compiled with different struct padding/alignment than visible in the C source

**EBDA+0x25A contents (verified):**
```
Offset  Value   Field
+0x00   0x01    active flag (emulation active)
+0x01   0x00    media type (0 = no emulation)
+0x02   0xE0    emulated drive number
+0x03   0x00    controller index
+0x04-7 0x38    ilba (initial LBA = 56, matches boot catalog)
+0x08-9 0x0004  sector count (4 sectors)
+0x0A-B 0x07C0  load segment
```

### INT 13h AH=4Bh (Get Boot Status) — Verified Working

ISOLINUX calls INT 13h AH=4Bh to retrieve the El Torito specification packet:
- **Call:** DL=0xE0, AH=0x4B, DS:SI=0x3030
- **BIOS returns:** CF=0, AH=0x00 (success)
- **Spec packet at 0x3030 (19 bytes):**
  ```
  13 00 E0 00 00 00 00 00  38 00 00 00 C0 07 04 00
  00 00 00
  ```
  - packet_size=0x13 (19), media=0, drive=0xE0, ilba=56(0x38), load_seg=0x07C0, sectors=4

**Verification method:** Added IRET trace in `soft_int.rs` for CS=0xF000 returns during ISOLINUX window. BIOS INT 13h handler returns with CF=0, AH=0x00 — the call succeeds.

### __intcall Mechanism (critical discovery)

ISOLINUX's `__intcall` does NOT use the x86 INT instruction. It uses a PM→RM trampoline that:
1. Switches from PM to RM (clear CR0.PE)
2. Sets up registers from a pre-built context
3. Uses `PUSHF; CALL FAR [IVT_entry]` to invoke the BIOS handler
4. The BIOS handler runs in RM and returns via RETF (or IRET)
5. Captures return registers and CF flag
6. Switches back to PM

**Implication:** Our `int_ib()` tracing in `soft_int.rs` misses ALL PM-originated BIOS calls. The INT instruction is never executed for ISOLINUX's BIOS calls — they go through FAR CALL to the IVT handler address.

**INT wrapper at 0x7F05:**
```asm
7F05: PUSHF
7F06: PUSH AX/CX/DX/BX/BP/SI/DI
7F0D: INT 13h        ; actual INT instruction
7F0F: SETC [BP+0x0A] ; write CF into FLAGS word on stack
7F13: POP DI/SI/BP/BX/DX/CX/AX
7F1A: POPF
7F1B: RET
```

The SETC trick writes CF=1/0 into the saved FLAGS word at [BP+0x0A], propagating the INT 13h return CF through the POPF.

### Boot Info Table Two-Phase Check

The boot info table is checked TWICE — once in 16-bit boot sector code, once in 32-bit PM core:

**Phase 1 (16-bit, address 0x7D10):**
```asm
66 83 3E 0C 7C 00   CMP DWORD [0x7C0C], 0    ; bi_file at offset 0x0C
75 27                JNZ +0x27                 ; if non-zero → continue boot
```
Result: **PASSES** — `[0x7C0C]` = 56 (boot file LBA) ≠ 0

**Phase 2 (32-bit PM, at relocatable address in isolinux core):**
The PM code checks the boot info table again at a compile-time address. The string "No boot info table, assuming single session disk" exists at 0x8185 in the stub, suggesting the PM code reports this error. Evidence: VGA text shows no banner, no error messages, no disk reads after initial load.

### Non-Contiguous Memory Layout (key finding)

El Torito loads only 2048 bytes (1 CD sector) to 0x7C00-0x83FF. The boot sector then loads the rest:

```
Step 1 (El Torito):  0x7C00-0x83FF  (2048 bytes from LBA 56)
Step 2 (boot sector): 0x8800-0xFFFF  (15 sectors from LBA 57 to 0x0880:0000)
Step 3 (boot sector): 0x10000-0x127FF (5 sectors from LBA 72 to 0x1000:0000)
Step 4 (boot sector): 0x8400-0x87FF  (boot sector code copies data to close gap)
```

**Gap at 0x8400-0x87FF:** The boot sector fills this by copying data (confirmed: memory at 0x8500 matches file offset 0x900 of isolinux.bin). After this copy, the binary is contiguous from 0x7C00 to 0x127FF.

### PM Code Execution Pattern

The PM code runs at addresses like 0x8507, 0x84A7, 0x8662 (in the "gap" area) and at 0x100000+ (relocated core). Key observations:
- 175K instruction gap (pure PM code) between transitions #7 and #8 — initialization
- ~19,000 __farcall iterations at ~263-400 instructions each — polling loop
- All __farcall calls are VGA/keyboard (ATA_rd counter doesn't increment)
- 9,506 __intcall bounces during full run — all VGA/keyboard, NOT disk I/O
- Zero INT 10h calls via our int_ib() handler (because __intcall uses FAR CALL)
- Zero exceptions during ISOLINUX window

### Session 23 Findings (2026-03-06) — Deep Per-Instruction Tracing

**__intcall #2/#3 are NOT disk reads — they are trampoline self-tests.**

Per-instruction trace of __intcall #2 (icount 2,887,772-2,887,853):
1. PM code saves ESP, JMP FAR to 16-bit segment (0x84EE)
2. Loads 16-bit seg regs, LIDT for RM, clears CR0.PE → enters real mode
3. LSS to load RM stack, JMP BX → 0x84A7 (__intcall handler)
4. POP GS/FS/ES/DS, POPAD → **EAX=0x0000F2F0** (magic signature)
5. POPFD → **CF=true**
6. RET 4 → jumps to 0x840C
7. At 0x840C: `CMP EAX, 0x0000F2F0` → ZF=1 (matches!)
8. JNZ not taken → falls through to 0x8416
9. PUSH 0x0010199A (PM return address)
10. CALL 0x848F → PUSHFD, PUSHAD, PUSH segs, re-enter PM at 0x84B4

**The code at 0x840C checks if EAX == 0xF2F0** (intcall interface ready signature).
When it matches, the trampoline self-test passes and returns to PM at 0x10199A.
When it doesn't match, it jumps to 0x80BD (error handler).

Both __intcall #2 and #3 complete successfully with EAX=0xF2F0.

**Revised Boot Timeline (Session 23, verified per-instruction):**
```
icount 0-1.407M:     BIOS POST (real mode + rombios32 PM)
icount 1.407M:       PM→RM #1 — rombios32 returns to real mode
icount 1.628M:       VGA BIOS text output via RETF16 #0 (INT 13h)
icount 1.770M:       El Torito boot: READ(10) LBAs 17,55,56
                     Boot sector self-loads: LBA 57×15 + LBA 72×5
                     ATA_rd=23942 (final — never increases again)
icount 1.857M:       ISOLINUX stub enters PM (CS=0x0020)
icount 1.857-2.888M: ~1M instructions of RELOCATION FIXUPS
                     (loop at 0x8AE8-0x8C43, processing 0x100000+ addresses)
icount 2.888M:       __intcall self-test #2 (EAX=0xF2F0 ✓)
icount 2.888M:       __intcall self-test #3 (EAX=0xF2F0 ✓)
icount 2.889M:       VGA init: INT 10h AH=0F (Get Mode) via __farcall
icount 2.889-2.890M: 3 more VGA __farcall calls (Set Mode, Cursor)
icount 2.890-3.065M: 175K instruction PM gap = ROM MEMORY SCAN
                     Scans 0xF0000-0xFA200 for SMBIOS/ACPI/PnP structures
                     Then processes tables at 0xFA2D0 area
icount 3.065M:       __intcall #8, then __farcall dispatches begin
icount 3.065-3.13M:  Banner printing: " Copyright (C) 1994-2015 H. Peter Anvin et al"
                     Characters: 0x20(' '), 0x43('C'), 0x6F('o'), 0x70('p'), 0x79('y')...
icount 3.13M+:       Keyboard polling: INT 16h AH=11 in loop (boot: prompt)
```

**CRITICAL GAP: Between ROM scan end (3.065M) and banner printing, there are only
~5000 instructions. No disk read calls happen. iso_init() was NEVER called or
returned without reading disk.**

Expected SYSLINUX 6.x init sequence (from source):
```
1. Relocation fixups                    ✅ (1M instructions)
2. __intcall self-test                  ✅
3. openconsole (VGA init)               ✅
4. mem_init (includes ROM scan)         ✅
5. fs_init → iso_init (READ PVD!)       ❌ MISSING — zero disk reads
6. writestr(copyright)                  ✅ (but only stub copyright)
7. load_config → read syslinux.cfg      ❌ never reached
8. Load ldlinux.c32                     ❌ never reached
```

**Real Bochs output (from screenshot):**
```
ISOLINUX 6.04 6.04-pre1 ETCD Copyright (C) 1994-2015 H. Peter Anvin et al
boot:
 21% ################
```
"ISOLINUX 6.04..." prefix comes from ldlinux.c32; "Copyright..." from stub.
"66MB medium detected" comes from ldlinux.c32 after reading ISO.

### Session 24 Findings (2026-03-06) — Init Function Table Analysis

**MAJOR DISCOVERY: The SYSLINUX init_func table at 0x110DC0 is EMPTY.**

Per-instruction tracing of the init table iterator at 0x10430A-0x1043EE revealed:

**Init function table structure:**
- Base address: 0x110DC0 (in BSS region — beyond the 43KB binary file)
- Entry stride: 0x38 (56 bytes), confirmed by `IMUL EDX, EDX, 0x38`
- Sentinel: first dword of entry == computed entry address (self-referential)
- Iterator: `entry_addr = index * 0x38 + 0x110DC0; if *entry_addr == entry_addr → skip`

**The IMUL at 0x10433A is correct** — verified decoder convention:
- Opcode 0x6B falls in ELSE branch: dst=nnn, src=rm (Gd,Ed format)
- `IMUL EDX, EDX, 0x38` correctly computes index * stride
- Values verified: index=0x2C (44), 0*0x38=0, 0+0x110DC0=0x110DC0 ✓

**Last iteration (icount ~3,113,001) with index=0 (EDX cleared before IMUL):**
```
IMUL EDX, EDX, 0x38  → EDX = 0 * 0x38 = 0
ADD  EDX, 0x110DC0   → EDX = 0x110DC0
TEST EDX, EDX        → NZ (non-zero, so not null)
[load EBX from [EDX]] → EBX = 0x110DC0 (value at 0x110DC0 = its own address!)
CMP  EBX, EDX        → EQUAL (sentinel detected)
JZ   0x1043E3        → TAKEN → skip, return 0
```

**The sentinel value at 0x110DC0 is the address itself (0x00110DC0).** This is the SYSLINUX convention for "end of init function list". Since it's the FIRST entry, the table has ZERO registered init functions.

**Earlier iterations (indices 0x14=20, 0x2C=44) also find empty/sentinel entries.**
The iterator is called multiple times with different starting indices (seen: 20, 28, 44).
All return 0 — the entire table is empty.

**Why the table is empty:**
Address 0x110DC0 is in the BSS region (file is only 43008=0xA800 bytes, but 0x110DC0 - 0x100000 = 0x10DC0 = 69,056 > 43,008). BSS is zero-initialized during the relocation phase. The init_func table entries should be populated by:
1. The linker embedding function pointers into `.ctors` / `.init_array`
2. These being processed during SYSLINUX core's early init to populate the table
3. OR the table being part of the `.data` section with pre-filled entries

**Since the table is in BSS (zero-initialized), entries need to be populated at runtime.**
But the code that populates them never runs — or runs and fails.

**Post-init-table execution flow (verified per-instruction):**
```
After init_func returns 0 (empty table):
  0x102173: Store result (0) to [0x10FB6C]
  0x102178-0x10217F: Zero a flag byte, load EAX=0x10FB64
  0x102184: RET
  0x10203F-0x102052: Setup EDX=0x108C20 from [memory], CALL EDX
  0x10240B-0x102420: Function prologue, set EAX=0x2C, CALL 0x1043EF (init_func again)
  → Second init_func call also returns 0 (table still empty)
  0x1042B0: DEC counter → JS (skip, counter went negative)
  → XOR EAX, EAX → return 0
  0x104316: Load from 0x10F780, more setup
  0x104323-0x104325: CALL [function pointer] → enters init_func AGAIN with index 0x2C
  → Third call also returns 0
  Eventually falls through to banner printing
```

**The dispatch loop calls init_func up to 3 times with the same index. Each time,
the table is empty, so it gives up and proceeds to print the banner and enter
the keyboard polling loop.**

### Updated Root Cause Hypothesis (Session 24)

**The init_func table at 0x110DC0 is empty because the SYSLINUX constructor
mechanism that populates it was never executed.**

This table is the central dispatch mechanism for SYSLINUX subsystem initialization.
Each SYSLINUX module (filesystem, disk I/O, console, etc.) registers itself via
`__constructor` macros that add entries to this table. Without these entries:
- `iso_init()` is never called → no ISO 9660 filesystem driver
- `disk_init()` is never called → no disk I/O beyond BIOS stub
- Only hardcoded fallback code runs → banner + keyboard poll

**The init_func table lives in BSS (runtime-filled), not .data (file-loaded).**
The 1M-instruction relocation phase (icount 1.857M-2.888M) does:
1. Copy binary from 0x7C00 to 0x100000 (code + rodata + data)
2. Fix up relocation entries (applying base address delta)
3. Zero BSS region (where 0x110DC0 lives)

But somewhere between BSS zeroing and the init_func calls, the constructor
chain that should fill the table is either:
1. **Skipped** — the code jumps past the constructor execution loop
2. **Broken** — the constructor list itself (`.ctors`/`.init_array`) is wrong
3. **CPU bug** — a subtle instruction error in the relocation code

**New Priority Investigation Steps:**

1. **Find the `.ctors`/`.init_array` section in the relocated binary:**
   Search memory at 0x100000-0x10A800 for patterns that look like arrays
   of function pointers (addresses in 0x100000-0x110000 range).

2. **Trace the exact path from relocation end to init_func call:**
   The ~50 instructions between relocation completion (icount 2.888M) and
   the first __intcall self-test might include constructor execution that
   produces no output because constructors just register function pointers.

3. **Compare with Bochs 3.0 execution:**
   Run Alpine ISO on real Bochs and trace the init_func table — verify it
   has entries, identify what fills them.

4. **Check if relocation fixups are correct:**
   If the `.ctors` array at 0x100000+ contains wrong function pointers
   (e.g., unfixed addresses pointing to 0x7C00-based locations instead of
   0x100000-based), the constructors would jump to wrong code.

### Evidence Summary (Updated Session 24)

| Check | Result |
|-------|--------|
| El Torito load | ✅ Working |
| Boot info table at 0x7C08 | ✅ Valid (PVD=16, file=56, len=43008) |
| INT 13h AH=4Bh | ✅ Working (CF=0, correct spec packet) |
| 16-bit boot info check | ✅ Passes |
| Boot sector loads rest | ✅ Working (15+5 sectors) |
| Gap fill (0x8400-0x87FF) | ✅ Working |
| PM entry | ✅ Working (1M+ instructions) |
| __intcall self-test | ✅ Passes (EAX=0xF2F0 matched) |
| VGA init | ✅ Working (4 __farcall calls) |
| ROM memory scan | ✅ Working (0xF0000-0xFA200) |
| PM→RM mechanism | ✅ Verified correct (per-instruction audit) |
| Banner printing | ⚠️ Partial (" Copyright..." only, no "ISOLINUX 6.04") |
| Init func table (0x110DC0) | ❌ **EMPTY** — sentinel at first entry, zero init funcs registered |
| IMUL instruction (0x6B) | ✅ Correct (verified decoder convention + arithmetic) |
| iso_init() disk reads | ❌ ZERO — never dispatched because table empty |
| ldlinux.c32 loaded | ❌ Never read (depends on iso_init) |
| syslinux.cfg loaded | ❌ Never read (depends on iso_init) |
| Boot info at 0x100008 | ❓ Not yet verified (may be irrelevant if init_func broken) |
| INT 13h AH=41h support | ❓ Not yet verified (never reached) |
| Constructor execution | ❓ **KEY UNKNOWN** — where does .ctors/.init_array run? |

### Next Investigation Steps (Session 24)

1. **Find `.ctors`/`.init_array` in memory** — scan 0x100000-0x10A800 for
   arrays of function pointers pointing into the relocated code region.
   These should be 4-byte pointers to addresses in 0x100000-0x110000.

2. **Trace icount 2.880M-2.888M** — the ~8K instructions between relocation
   end and __intcall self-test may include constructor execution.
   Look for: CALL to addresses in 0x100000+ that write to 0x110DC0 area.

3. **Dump 0x110DC0-0x110F00** at relocation end vs at init_func call time
   to confirm the table was never written (not written then overwritten).

4. **Run real Bochs with Alpine** — trace the init_func table on working
   emulator to see what entries should exist.

### IMUL Verification (Session 24)

**IMUL `GdEdsIb` (opcode 0x6B) verified CORRECT:**

- **Decoder convention**: 0x6B is in ELSE branch → `dst()=nnn`, `src()=rm` (Gd,Ed format) ✓
- **Handler reads `ib()` instead of `id()`**: Functionally equivalent because decoder sign-extends
  the byte into `id`, and `ib()` returns LSB of `id`. Manual `as i8 as i32` in handler produces
  same result as reading the pre-sign-extended `id()` value. Verified for both positive and negative immediates.
- **Bochs reference** (`mult32.cc:138`): `IMUL_GdEdIdR` uses `i->Id()` and `i->src()` — same values,
  different access paths. Both `BX_IA_IMUL_GdEdId` and `BX_IA_IMUL_GdEdsIb` share this handler.
- **Arithmetic verified**: Trace shows `IMUL EDX, EDX, 0x38` with EDX=0 → result=0 ✓.
  EDX=1 → result=0x38 ✓. No overflow cases observed.

### SYSLINUX Binary Format Discovery (Session 24)

**The isolinux.bin file is NOT a flat binary — it uses LZO compression.**

- File offset 0x433A (where init_func code appears at runtime 0x10433A) contains
  **compressed data** (`2b 8e f3 04...`), NOT valid x86 instructions.
- The only `6B D2 38` (IMUL EDX,EDX,0x38) in the raw file is at offset 0x1500 (part of compressed stream).
- After the boot sector loads the file, a decompression phase unpacks the PM core to 0x100000+.
- The ~1M instruction "relocation phase" (icount 1.857M-2.888M) includes decompression + fixups.
- The decompression works correctly — the init_func iterator code at 0x10433A is valid and executes properly.
- BSS at 0x110DC0 is correctly zeroed.
- **The missing piece is constructor execution** that should populate the init_func table.

## Key Addresses
```
0x7C00:  El Torito boot image load address
0x7C08:  Boot info table (PVD=16, file=56, len=43008)
0x8000+: ISOLINUX stub (RM code)
0x84A7:  __intcall RM handler (POP regs + RET 4)
0x84B4:  RM→PM re-entry
0x84EE:  PM→RM transition (clear PE)
0x850F:  RM setup (load RM stack, JMP BX)
0x8523:  Hardware IRQ RM handler (PUSHF + CALL FAR)
0x8662:  __farcall RM handler (POP regs + RETF)
0x89A0:  __intcall PM entry point
0x89C8:  Common PM→RM bounce (save ESP, JMP FAR to 16-bit)
0x89D7:  IDT common handler (PUSHAD, INC timer, MOV BX=0x8523)
0x89EA:  PM return handler (DEC timer, POPAD, IRET)
0x100000+: Relocated ISOLINUX core (32-bit PM code)
0x100065: __farcall PM return address
0x100C40: Timer/idle check function
0x100C6F: First HLT address
0x10203F: Post-init-func dispatch setup
0x10240B: Init dispatch wrapper (calls init_func with index)
0x1042B0: Counter-based dispatch (DEC + JS)
0x1042FF: Init_func table iterator entry
0x10430A: Init_func table iterator body
0x10433A: IMUL EDX, EDX, 0x38 (compute entry address)
0x10433D: ADD EDX, 0x110DC0 (add table base)
0x104343: TEST EDX, EDX (null check)
0x104355: MOV EBX, [EDX] (load first dword from entry)
0x104358: CMP EBX, EDX (sentinel check: value == address?)
0x1043E3: Sentinel detected path (XOR EAX, EAX → return 0)
0x1043EF: Init_func alternative entry point (sets ECX=2, XOR EDX)
0x104171: Idle loop caller
0x10962E: Config search path strings
0x110DC0: Init function table base (BSS region — currently EMPTY)
0x10FB6C: Init result storage location
0x3AD4:  __farcall SP recovery
0x3AD8:  Timer nesting counter
0x38B8:  RM stack pointer (SS:SP)
0x8E78:  Saved PM ESP
0x8E84:  Callback pointer (0x89FD)
```
