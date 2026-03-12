# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rusty Box is a Rust port of the Bochs x86 emulator - a complete CPU/system emulator targeting 32/64-bit x86 architecture with virtualization support (VMX/SVM). The original C++ Bochs source is in `cpp_orig/bochs/` for reference during porting.

## Current BIOS Execution Status

### ✅ MAJOR BREAKTHROUGH (2026-02-16): BIOS-bochs-latest Runs Successfully!

**DISCOVERY**: The "corrupted BIOS symbol addresses" bug was NOT in the BIOS ROM files - it was caused by two emulator bugs:

1. **Segment default bug**: `[BP+disp]` addressing modes were defaulting to DS instead of SS. Fixed by adding proper segment override lookup tables (`SREG_MOD00_RM16`, `SREG_MOD01OR10_RM16`, `SREG_MOD0_BASE32`, `SREG_MOD1OR2_BASE32`) in `fetchdecode32.rs`.

2. **execute1/execute2 mismatch**: 18 opcodes in `opcodes_table.rs` had memory-form (`_M`) and register-form (`_R`) handlers swapped, causing memory operands to be read from registers and vice versa.

**Current Status (2026-03-12):**
- ✅ **DLX Linux boots to interactive bash shell!** Full boot: BIOS POST → LILO → kernel → init → `dlx login: root` → `dlx:~#` (200M instructions sufficient)
- ⚠️ **Alpine Linux boots through full kernel init, loads cdrom/sr_mod, detects sr0** — Kernel 6.18.7-0-virt fully initializes: e820 memory map, APIC/IOAPIC, PCI (i440FX/PIIX3/PIIX4), ACPI, ata_piix, serial 16550A, i8042, RTC, IPv6, TCP/UDP, squashfs. Alpine Init 3.13.0-r0 loads cdrom+sr_mod+isofs modules (via `modules=` cmdline). CD-ROM detected as `sr 1:0:0:0: [sr0] scsi3-mmc drive`. **Stalls after sr0 detection**: 12 ATAPI probe commands complete (TUR, INQUIRY, MODE_SENSE, READ_TOC, READ_CAPACITY, READ_DISC_INFO) but zero READ(10) — kernel never reads CD data. `debug_init=1` confirmed the stall occurs inside `nlplug-findfs` (init script line 848). Alpine Init calls `nlplug-findfs -p /sbin/mdev -b /tmp/repositories -a /tmp/apkovls` which uses netlink sockets to receive kernel uevents for device discovery. Without uevent delivery, nlplug-findfs blocks forever waiting for device events. **Next**: investigate netlink/uevent socket support — nlplug-findfs opens `AF_NETLINK`/`NETLINK_KOBJECT_UEVENT` socket via `socket()` syscall. The kernel needs to deliver uevents through this socket for sr0 discovery.
- ✅ **F3 0F 7E decoder convention fix (session 41)**: 0x17E was in Ed,Gd branch for ALL SSE prefixes. F3 0F 7E (MOVQ Vq,Wq) is a LOAD needing Gd,Ed. Swapped operands made musl fdopen() store __stdio_read into wrong XMM register → NULL f->read → modprobe segfault at IP=0. Fixed in both decode64.rs and decode32.rs.
- ✅ **MONITOR asize_mask fix (session 40)**: MONITOR instruction truncated RAX to 32 bits in 64-bit mode. Fixed with Bochs's `asize_mask()` lookup table. Unblocked mwait_idle — Alpine now enters idle cleanly.
- ✅ **AVX/AVX-512/BMI2 instructions (session 39-40)**: blake2s_compress_avx512 and sha256_transform_rorx fully working. VPRORD, VPROLD, VEXTRACTI128, VPERM2I128, VPSHUFB 256-bit, RORX all implemented.
- ✅ **Session 28-29 audit RESOLVED**: All 6 CRITICAL bugs resolved (3 already fixed, 1 not-a-bug, 2 confirmed fixed). Most HIGH bugs also confirmed fixed. Remaining unfixed: ~95 SSE opcode tables, decmask NNN/RRR fields, SRC_EQ_DST bit, UD64 opcodes, 64-bit TLB fast path (performance only).
- ✅ **64-bit canonical address checks (session 30)**: All 10 `read/write_virtual_*_64` functions now check canonical addresses (Bochs access.cc:339/444). Non-canonical raises #GP(0) or #SS(0). Cross-page paths check last byte too.
- ✅ **LPF mask 64-bit fix (session 30)**: `translate_data_access` used `0xFFFF_F000` (32-bit truncating) for lpf/ppf. Fixed to `LPF_MASK` (`0xFFFF_FFFF_FFFF_F000`). Was causing incorrect TLB lookups for 64-bit kernel virtual addresses.
- ✅ **NOP/PAUSE wired in dispatcher**: Both decoded by decoder AND dispatched as no-ops.
- ✅ **64-bit paging bypass FIXED (session 26)**: ALL 64-bit memory access functions were bypassing page table walks, passing linear addresses directly to physical memory. Since long mode requires paging (CR0.PG=1), once the kernel sets up its own page tables, every 64-bit memory access hit wrong physical addresses. Fixed ~137 call sites across 9 files. See detailed fix list below.
- ✅ **REX byte register mapping fix**: Two-bug fix enabling correct SPL/BPL/SIL/DIL access in 64-bit mode. Decoder stored `rex_prefix = b & 0x0F` (bare REX 0x40 → 0 → Extend8bit never set). Register accessor mapped indices 4-7 to AH/CH/DH/BH regardless of REX. Both fixed — Alpine kernel memset now writes correct values to page tables.
- ✅ **Full 64-bit instruction set**: arith64, logical64, shift64, mult64 all implemented and dispatched (~110 new opcodes)
- ✅ **Long mode activation**: EFER MSR handling, handle_cpu_mode_change for Long64/LongCompat, CR0/CR3 long mode writes
- ✅ **64-bit MSRs**: STAR, LSTAR, CSTAR, FMASK, FSBASE, GSBASE, KERNELGSBASE, TSC_AUX + SWAPGS instruction
- ✅ **64-bit data transfer**: MOV/MOVSX/MOVSXD/MOVZX/XCHG/LEA/CMOV all 64-bit forms
- ✅ **64-bit stack ops**: PUSH/POP r/m64, PUSHFQ/POPFQ, PUSH imm64
- ✅ **Decoder audit Ed,Gd convention fix (session 28)**: `(b1 & 0x0F) == 0x01/0x09` pattern for single-byte opcodes also caught two-byte opcodes (0F xx) — swapped DST/SRC for CMOVno/ns, SQRTPS, MULPS, PUNPCK*, PSRLW, PSRAW, PSLLW, PSUBW, etc. Fixed with `b1 < 0x100` guard + explicit two-byte matches. Also fixed 64-bit LEAVE/INS/OUTS missing from no-ModRM list.
- ✅ **SHRD/SHLD decoder fix**: Opcodes 0F A4/A5/AC/AD were in wrong decoder branch (ELSE instead of Ed,Gd). Operands were swapped — result went to wrong register. Fixed ext2 "directory #12 contains a hole" errors that blocked login.
- ✅ All control/debug register types converted to bitflags (BxCr0, BxCr4, BxEfer, BxDr6, BxDr7)
- ✅ CPU-level feature gates removed — APIC, MONITOR/MWAIT, MEMTYPE, AMX, PKEYS, EVEX, SSE, AVX, VMX, SVM always compiled
- ✅ CR4/EFER/XCR0/XSS allow-masks computed from CPUID features (matching Bochs crregs.cc)
- ✅ PAE + long mode paging infrastructure added (page_walk_for_dtlb_pae, page_walk_for_dtlb_long_mode)
- ✅ 64-bit control transfers: call_protected_64, return_protected_64, long_iret, call_gate64
- ✅ L bit parsing in segment descriptors (bit 21 of dword2) for 64-bit code segments
- ✅ BIOS-bochs-latest (128 KB) is now the primary BIOS
- ✅ Full BIOS POST completes: rombios32_init, VGA BIOS, ATA detection, boot
- ✅ VGA text output working! Clean headless text dump:
  ```
  Console: 16 point font, 400 scans
  Console: colour VGA+ 80x25, 1 virtual console (max 63)
  Calibrating delay loop.. ok - 9.98 BogoMIPS
  Memory: 31140k/32768k available (612k kernel code, 384k reserved, 632k data)
  ...
  Linux version 1.3.89 (root@merlin) (gcc version 2.7.2)
  Serial driver version 4.11a with no serial options enabled
  PS/2 auxiliary pointing device detected -- driver installed.
  loop: registered device at major 7
  hda: RUSTY_BOX HARDDISK, 10MB w/256kB Cache, LBA, CHS=306/4/17
  ide0 at 0x1f0-0x1f7,0x3f6 on irq 14
  Started kswapd v 1.4.2.2
  Partition check: hda: hda1
  VFS: Mounted root (ext2 filesystem) readonly.
  Jan  1 12:00:13 init[1]: version 2.4 booting
  EXT2-fs warning: mounting unchecked fs, running e2fsck is recommended
  Mounting remote file systems...
  INET: syslogd
  Jan  1 12:00:14 init[1]: Entering runlevel: 4
  Welcome to DLX V1.0 (C) 1995-96 Erich Boehm
                      (C) 1995    Hannes Boehm
  dlx login:
  ```
- ✅ LILO boot loader runs, loads compressed Linux kernel
- ✅ Kernel decompresses and starts executing (paging enabled, CR0=0x80000013)
- ✅ Kernel runs 1B+ instructions cleanly (no crashes, no errors)
- ✅ Kernel initializes console, calibrates BogoMIPS, loads serial/PS2/loop drivers
- ✅ Software interrupt (INT) dispatch: unified interrupt() method matches Bochs
  - INT/INT3/INTO/INT1 now correctly dispatch through IDT in protected mode
  - Previously always used real-mode IVT, causing kernel re-execution from startup_32
- ✅ XCHG r32, r/m32: mod_c0() dispatch fix — memory form was treated as register form
  - Caused `XCHG EAX, [ESP+offset]` to become `XCHG EAX, ESP` → ESP=0xFFFFFFFF → triple fault
- ✅ BOUND uses exception(Br, 0) instead of interrupt_real_mode(5) — matches Bochs
- ✅ CpuLoopRestart leak fixed: InsertedOpcode path in cpu_loop_n now catches CpuLoopRestart
- ✅ Paging: system_write_byte/word/dword now translate linear→physical via page walk (Bug 1 fix)
- ✅ Paging: user_pl updated in load_cs() — CPL=3 pages now properly permission-checked (Bug 2 fix)
- ✅ Paging: 4MB PSE pages in translate_linear_legacy get permission checks + A/D bit updates (Bug 3 fix)
- ✅ Paging translation: all 32-bit string ops use read/write_virtual_byte/word/dword
- ✅ Segment limit checks in virtual memory access functions
- ✅ Protected mode: segment loading, descriptor parsing, privilege checks
- ✅ BT/BTS/BTR/BTC r/m32 (all 8 variants: imm8 and register forms)
- ✅ MOVSX GdEw unified dispatch (memory + register forms)
- ✅ LEAVE instruction decoder fix (0xC9 added to no-ModRM list)
- ✅ TLB flush on CR0/CR3/CR4 writes, INVLPG handler
- ✅ VGA word-wide I/O: port mask 0x3 (byte+word) matching Bochs vgacore.cc
- ✅ VGA memory plane filtering: map mask check prevents font data corrupting text buffer
- ✅ Full x87 FPU implementation with Berkeley SoftFloat 3e (true 80-bit extended precision)
  - All ~80 FPU opcodes: load/store, arithmetic, compare, transcendentals, constants, FCMOV
  - FNSAVE/FRSTOR for kernel context switching, FNSTENV/FLDENV, FBLD/FBSTP (packed BCD)
  - Float128 polynomial evaluation for sin/cos/atan/log/exp transcendentals
  - File structure mirrors Bochs `cpu/fpu/` 1:1 (ferr.rs, fpu.rs, fpu_arith.rs, etc.)
- ✅ Exception delivery: protected_mode_int BadVector → recursive exception() for double/triple fault
- ✅ Complete task_switch: TSS state save/restore (286+386), CR3+TLB, CR0.TS, segment validation
- ✅ Protected mode far CALL/RET: call_protected() with call gates (same/inner-priv), return_protected()
- ✅ Far JMP system descriptors: TSS, task gates, call gates in jump_protected()
- ✅ IRET NT nested task return: reads back-link from TSS, validates busy TSS, task_switch
- ✅ validate_seg_regs(): nulls ES/DS/FS/GS on outer-priv return if DPL < CPL
- ✅ MOV SS / POP SS interrupt inhibition (inhibit_mask + inhibit_icount)
- ✅ Enhanced RDMSR/WRMSR: actual MSR field storage (sysenter, PAT, MTRR)
- ✅ handleCpuModeChange: updates cpu_mode from CR0.PE + EFLAGS.VM
- ✅ Zero compiler warnings (crate-level allows for Bochs naming conventions and dead code)
- ✅ Paging: system_write_byte/word/dword translate linear→physical via page walk with A/D bits
- ✅ Paging: user_pl tracks CPL==3 in load_cs() — user-mode page permissions now enforced
- ✅ Paging: 4MB PSE path in translate_linear_legacy has full permission checks + A/D updates
- ✅ Fixed handle_alignment_check: reads CPL from CS.selector.rpl instead of CL register
- ✅ Clean output: diagnostic warn!/error! downgraded to debug!/trace! (~925 → 0 error messages)
- ✅ Triple fault FIXED: was caused by INT always using IVT (real-mode) + XCHG mod_c0 bug
- ✅ vsprintf FIXED: ADD AL,Ib (opcode 0x04) was operating on AH instead of AL — jump table index wrong
- ✅ "Trying to free nonexistent swap-page" RESOLVED: caused by triple-fault-induced IDT corruption
- ✅ PUSH imm8 (0x6A) sign-extension: was zero-extending → wait4(-1) became wait4(255) → init couldn't reap children
- ✅ Prefetch page fault propagation: translate_linear errors in prefetch() were silently swallowed
- ✅ Fetch buffer 4K page bounding: get_host_mem_addr returned unbounded slice, decoder read past page boundary into wrong physical memory → stale CALL displacement → wild jumps
- ✅ Icache SMC boundary check: page-spanning instructions with only partial first_bytes match forced to re-decode
- ✅ Empty ATA drive status returns 0xFF (floating bus) instead of 0x00
- ✅ CMOS floppy registers set for DLX config (two 1.44M drives)
- ✅ HLT busy-wait for small sleeps (<2ms) avoids Windows 15.6ms timer granularity

**What Fixed the "Corrupted Symbols":**
The previous investigation concluded BIOS ROM had wrong symbol addresses. In reality, the segment default bug caused stack reads via `[BP+offset]` to use DS (base=0) instead of SS, and the execute1/execute2 swap caused memory reads to return register values. Together, these made the BIOS load wrong values for `_end`, `__data_start`, etc. With both bugs fixed, the BIOS reads correct values from the stack and memory.

### Investigation History: Protected Mode Init (2026-02-17 to 2026-02-19)

**Execution timeline (measured by instruction count):**
- 0-10: Real mode BIOS at F000:E0xx (initial setup)
- 10-100: Drops into low-address subroutines (F000:0Cxx area = keyboard/PCI init)
- ~360k: Real-mode init completes, BIOS enters protected mode
- At 362k: RIP=0xE08C0, CS=0x0010, mode=protected - rombios32 executing
- At ~363k+: Continues executing in protected mode

**Log flooding bug found and fixed (2026-02-17):**
The apparent "hang" at 363k instructions was caused by `tracing::debug!` in `misc_mem.rs` and `memory_stub.rs` logging every byte written beyond 32MB RAM. Changed to `tracing::trace!`.

**I/O port tracking added (2026-02-17):**
`BxDevicesC::inp()` now tracks the last I/O read port/value (`last_io_read_port`, `last_io_read_value`). The stuck-loop detector in `emulator.rs` reports this info. Signature changed from `&self` to `&mut self`.

**BIOS ROM shadow mapping bug found (partially fixed, 2026-02-17):**
The `get_host_mem_addr` PCI path for addresses 0xE0000-0xFFFFF was using the expansion ROM formula instead of `bios_map_last128k()`. Fixed.

**Root cause of "no BIOS output" found (2026-02-19): Port 0x61 delay_ms() infinite loop**

The Bochs BIOS `rombios32_init()` calls `smp_probe()` at line 2589 (after `BX_INFO` at 2576, `ram_probe` at 2583, `cpu_probe`, `setup_mtrr`). `smp_probe()` calls `delay_ms(10)`, which polls port 0x61 bit 4 (PIT channel 2 output) waiting for 66 edge transitions. Our emulator returned fixed `0x10` from port 0x61 — bit 4 never toggled → `delay_ms()` looped forever.

The two-part explanation for "no BIOS output":
1. **Performance**: Before logging fixes, debug flood made the emulator too slow to execute enough instructions to reach rombios32_init at all
2. **Correctness**: After logging fixes made it fast enough, the emulator reached rombios32_init and its BX_INFO calls, but then got stuck in `smp_probe()` → `delay_ms()` — the BIOS couldn't continue to print more output or do any useful work

**Fix**: `keyboard.rs` `SYSTEM_CONTROL_B` read handler now XORs bit 4 on each read:
```rust
self.system_control_b ^= 0x10;
```

**Hot-path logging fixed (2026-02-19):**
Multiple `debug!`/`info!` calls on hot paths were causing I/O-bound slowdowns:
- `cpu.rs`: `get_icache_entry` (every instruction) changed from `debug!` → `trace!`
- `cpu.rs`: Two `prefetch` messages changed from `info!` → `debug!`
- `stack.rs`: `PUSH16` message changed from `info!` → `debug!`
- `dlxlinux.rs`: Hardcoded `Level::DEBUG` replaced with `RUST_LOG` env var (default WARN)

**Note**: `tracing_subscriber::EnvFilter` requires the `env-filter` feature (not enabled).
Use `std::env::var("RUST_LOG").parse::<tracing::Level>()` instead.

**For headless testing on Windows**: Set `RUSTY_BOX_HEADLESS=1` to skip TermGUI repaint.
Performance (windowed MIPS, release build, 2026-03-03): BIOS real-mode ~22 MIPS, kernel decompressor ~25-50 MIPS, kernel init ~22-27 MIPS.
Monitor per-phase throughput with: `RUST_LOG=error cargo run --release --example dlxlinux --features std` (output lines tagged `[mips]`, fire every 5M instructions).

**Interactive/egui HLT sync (2026-03-03)**: When a GUI is attached and the CPU is in protected mode, the emulator sleeps once per HLT batch to sync virtual time to wall-clock time (analogous to Bochs `clock: sync=slowdown`). This prevents the Linux console blank timer from firing ~360x too early. BIOS (real mode) runs at full speed. Implemented as `self.gui.is_some() && cpu_mode != 0` — no per-example config needed.

**New fixes (2026-02-19): Short jumps, CLC/STC/CMC, RDMSR/WRMSR, Jbd dispatch**

These bugs were causing an infinite loop at ~363k instructions and crashes in the first few hundred instructions of protected-mode execution:

1. **Short jump sign-extension** (`fetchdecode32.rs:586`): byte immediates for opcodes 0x70-0x7F, 0xEB, 0xE0-0xE3 were zero-extended. `jmp_jd` uses `instr.id() as i32` so 0xFE → 254 instead of -2. Fixed by sign-extending for branch opcodes only.
2. **Missing Jbd dispatch** (`cpu.rs`): Only `JmpJbd`, `JzJbd`, `JnzJbd`, `JecxzJbd` were handled. Added JoJbd, JnoJbd, JbJbd, JnbJbd, JbeJbd, JnbeJbd, JsJbd, JnsJbd, JpJbd, JnpJbd, JlJbd, JnlJbd, JleJbd, JnleJbd, LoopJbd, LoopeJbd, LoopneJbd.
3. **CLC/STC/CMC** (`cpu.rs`): Clear/Set/Complement CF flag — first crash after short-jump fix (opcode 0xF8 at protected mode entry). Added near Hlt/Cpuid.
4. **RDMSR/WRMSR stubs** (`cpu.rs`): Called by `setup_mtrr()` in rombios32_init. Return 0/ignore writes.
5. **mpool_start_idx fallback removed** (`cpu.rs`): Was emitting `warn!` on every first-trace icache lookup (index 0 is valid for the first cached trace). Removing the false-error code improved performance.

**Result**: BIOS now runs 1M instructions at ~1.08 MIPS without crashing. Final RIP=0xE1D81 (still in protected mode). Still no BIOS output — need to trace why `BX_INFO("Starting rombios32\n")` hasn't fired.

**Next investigation**: The BIOS spends 1M instructions in protected mode but never reaches `rombios32_init()` BX_INFO at the start. Possible causes:
- Long setup_mtrr/pci init before rombios32_init is called
- Some loop/spin consuming instructions before the BX_INFO point
- Run with RUST_LOG=debug to see RDMSR/WRMSR calls and trace what's happening

### BIOS Binary Analysis (2026-02-23)

**Confirmed BIOS layout** (128KB = file 0x0000-0x1FFFF = physical 0xFFFE0000-0xFFFFFFFF):
- File 0x0000 = phys 0xE0000: rombios32 _start (BSS clear, .data copy, JMP to rombios32_init)
- File 0x2980 = phys 0xE2980: `rombios32_init()` — first function called in 32-bit PM
- File 0x0B98 = phys 0xE0B98: `bios_printf()` — writes ALL formatted bytes to port 0x402
- File 0x075C = phys 0xE075C: `vsnprintf()` — called by bios_printf to format strings
- File 0x17F4 = phys 0xE17F4: `delay_ms()` — polls port 0x61 bit 4 (66 transitions/ms)
- File 0x1D3A = phys 0xE1D3A: `smp_probe()` — APIC check + AP trampoline copy + IPI + delay_ms
- File 0x1D74 = phys 0xE1D74: smp_probe copy loop (74 bytes, 0xE0028 → 0x9F000)
- Real-mode code: file 0x8000-0x1FFFF (16-bit code segment)

**True PM entry sequence** (not 0xE08C0 as previously thought):
```
Real-mode BIOS (~362K instr):
  → F000:XXXX: LGDT [rombios32_gdt_48]; MOV CR0, EAX; FAR JMP 0x10:0xF9E5F
  → phys 0xF9E5F (file 0x19E5F): PM setup (MOV DS/ES/SS=0x18, FS/GS=0; set stack)
  → PUSH 0x4B0; PUSH 0x4B2; MOV EAX, 0xE0000; CALL EAX (_start)
  → phys 0xE0000 (_start): XOR EAX; REP STOSB (BSS 88B); REP MOVSB (.data 12B)
  → JMP 0xE2980 (rombios32_init)
rombios32_init (0xE2980):
  1. bios_printf(4, "Starting rombios32\n")    ← first ASCII to port 0x402
  2. bios_printf(4, "Shutdown flag %x\n", ...)
  3. ram_probe() — CMOS reads for memory size
  4. cpu_probe() — CPUID
  5. setup_mtrr() — RDMSR/WRMSR (wrmsr stubs in emulator)
  6. smp_probe() — APIC check, 74-byte AP copy, INIT/SIPI IPI, delay_ms(10)
     → bios_printf("Found %d cpu(s)\n", num_cpus)
  7. find_bios_table_area()
  8. pci_bios_init()
```

**GDT (rombios32_gdt at line 10698 of rombios.c)**:
```c
// selector 0x10: 32-bit flat code  (base=0, limit=4GB, D=1, G=1)
dw 0xffff, 0, 0x9b00, 0x00cf
// selector 0x18: 32-bit flat data  (base=0, limit=4GB, D=1, G=1)
dw 0xffff, 0, 0x9300, 0x00cf
```
D=1 confirmed in bit 22 of dword2 (0x00CF... → bit 22 = 1). Decoder correctly reads CS.d_b.

**bios_printf port 0x402 behavior**: Port 0x402 is ONLY written from a single loop at file 0x0BE9. bios_printf(rombios32.c) ALWAYS writes ALL formatted chars to port 0x402, regardless of the `flags` argument. No flag gate before the output loop.

**smp_probe loop analysis** (ending at RIP=0xE1D81):
```asm
; EAX starts at 0x9F000, ECX = 0x9F04A (end), 74 iterations
0xE1D74: LEA EDX, [EAX+1]
0xE1D77: MOV BL, [EAX + 0x41028]    ; read from ROM (0xE0028..0xE0071)
0xE1D7D: MOV [EAX], BL              ; write to RAM (0x9F000..0x9F049)
0xE1D7F: MOV EAX, EDX
0xE1D81: CMP EDX, ECX
0xE1D83: JNZ → 0xE1D74
```
This copies the AP startup trampoline from ROM to RAM. After the copy, smp_probe sends INIT IPI + SIPI + delay_ms(10) + reports CPU count.

**2026-02-23 session findings:**
- Port 0x402 writes confirmed at **RIP=0xE0BEA** (PM bios_printf output loop, file 0x0BE9) — NOT real-mode
- Only 0xB2 and 0xFF observed — not ASCII "Starting rombios32\n" — vsnprintf corruption suspected
- RDMSR MSR=0xFE observed → `setup_mtrr()` ran before `smp_probe()`
- PM prefetch at 0xE1D38 → `smp_probe()` entered
- smp_probe copy loop at 0xE1D74 confirmed: 74 iterations copying AP trampoline 0xE0028→0x9F000
- APIC path taken in smp_probe (bit 9 of EDX=0x0783_fbff is set)
- CPUID Leaf 1 currently reports Family 15 (Pentium 4) — wrong for Core i7 Skylake-X (should be Family 6, Model 0x55)
- bios_printf/vsnprintf corruption hypothesis: wrong CPUID or register state causes vsnprintf to produce garbage

**bios_printf corruption (possible causes):**
- CPUID Leaf 0 max=1 causes `cpu_probe()` to skip extended leaves, wrong feature bits set
- Stack state or calling convention mismatch in vsnprintf could produce wrong chars
- Wrong Leaf 1 EAX (Family 15) doesn't affect format strings directly, but wrong feature flags might alter codepath

## Known Issues & Next Steps

### Next Steps
1. ~~**Reach DLX Linux `dlx login:` prompt**~~ — **DONE** (2026-03-03). Full boot to login prompt achieved.
2. ~~**Interactive login**~~ — **DONE** (2026-03-04). SHRD/SHLD decoder fix resolved ext2 "directory #12 contains a hole" errors. `root` login → `dlx:~#` bash shell works.
3. **Boot Alpine Linux** — **IN PROGRESS** (2026-03-12). Kernel boots to Alpine Init, loads CD-ROM modules, detects sr0:
   - **Session 42 FIX (egui cmdline + ATAPI error handling)**: Added `modules=cdrom,sr_mod,isofs` to egui AlpineDirect cmdline (was only in headless). Fixed ATAPI lazy-load error handling — `read_cdrom_block()` failure now aborts transfer with MEDIUM_ERROR sense instead of returning stale buffer data. Added `MediumError` sense key and `UnrecoveredReadError` ASC.
   - **Session 41 FIX (F3 0F 7E decoder)**: MOVQ Vq,Wq (F3 0F 7E) had swapped operands in both decoders — 0x17E was unconditionally in Ed,Gd branch. F3 prefix form is a LOAD (Gd,Ed). This caused musl fdopen() to store __stdio_read into wrong XMM → NULL f->read → modprobe segfault at IP=0. Fixed by excluding 0x17E from Ed,Gd when sse_prefix==PrefixF3.
   - **Session 40 FIX (MONITOR asize_mask)**: MONITOR truncated RAX to 32 bits in 64-bit mode. Fixed with Bochs `asize_mask()` lookup table. Unblocked mwait_idle.
   - **Result**: `sr 1:0:0:0: [sr0] scsi3-mmc drive: 16x/16x cd/rw xa/form2 tray` detected. Alpine Init loads boot drivers (cdrom, sr_mod, isofs). Stalls inside `nlplug-findfs` which uses `AF_NETLINK`/`NETLINK_KOBJECT_UEVENT` socket to discover devices. Without uevent delivery over netlink, nlplug-findfs blocks forever and never attempts to mount sr0 as boot media.
   - **Session 43 investigation**: Extracted Alpine initramfs `/init` script. Two paths: (1) `root=` cmdline → "Mounting root" via nlplug-findfs, (2) no root → "Mounting boot media" via nlplug-findfs with `-b repofile -a /tmp/apkovls`. Both call `nlplug-findfs -p /sbin/mdev` which opens netlink socket and waits for uevents. `debug_init=1` cmdline enables shell `set -x` tracing but output stalls before any trace appears — confirming the block is inside the nlplug-findfs C binary, not the shell script.
   - **nlplug-findfs full flow** (from source analysis): Uses two-phase discovery: (1) **Coldplug**: a trigger thread recursively walks `/sys/bus/` and `/sys/class/`, writing `"add"` to each device's `uevent` file — this makes the kernel re-emit uevents for already-registered devices (including sr0). (2) **Hotplug**: listens on `PF_NETLINK`/`NETLINK_KOBJECT_UEVENT` socket (512KB buffer) via `poll()` for new device events. For each uevent with `DEVNAME`, spawns `mdev` to create `/dev/` node, then calls `blkid` to identify filesystem type. If `iso9660` found on sr0, mounts `/dev/sr0` on `/media/sr0` read-only and scans for `.boot_repository` marker. Timeouts: MAX_EVENT_TIMEOUT=5000ms (initial), DEFAULT_EVENT_TIMEOUT=250ms (after finding boot repo). All 6 steps must work: (1) netlink socket, (2) coldplug `/sys` trigger, (3) mdev creates `/dev/sr0`, (4) blkid reads device, (5) `mount(2)` iso9660, (6) directory scan for `.boot_repository`.
   - **Next step**: investigate kernel netlink/uevent socket delivery mechanism — the stall is in `poll()` waiting for uevents that never arrive.
   - **ISOLINUX path**: El Torito boot works, ISOLINUX 6.04 loads + enters PM, but init_func table is empty → iso_init never called. See `docs/ISOLINUX_DEBUG.md`.
4. **Fix `ide2 at 0x1e8` phantom** — PCI PIIX IDE BAR misconfiguration causes kernel to probe a non-existent 3rd IDE channel
5. ~~**Fix LDT triple fault**~~ — FIXED: root cause was INT using IVT in PM + XCHG mod_c0 bug
6. ~~**Fix vsprintf**~~ — FIXED: ADD AL,Ib (opcode 0x04) operated on AH, breaking vsprintf's jump table index computation
7. ~~**Fix swap init loop**~~ — RESOLVED: "Trying to free nonexistent swap-page" was caused by IDT corruption from the INT dispatch bug
8. ~~**Fix ext2 "directory #12 contains a hole"**~~ — **FIXED** (2026-03-04): SHRD/SHLD decoder convention bug. Opcodes 0F A4/A5/AC/AD were in ELSE branch instead of Ed,Gd. Operands swapped → shift never applied to destination register.

### Exact Instruction Threshold: 132,865,700

The kernel first enters HLT idle at **exactly instruction 132,865,700**. Key notes:

- Instruction 132,865,701 is the next instruction after HLT (part of the idle loop or an ISR return). Changing MAX_INSTRUCTIONS by ±1 changes which instruction is last-executed, which can affect whether a critical ATA command write or status check runs.
- **cpu_loop_n `>=` fix (2026-03-02)**: Changed `if iteration > max_instructions` to `if iteration >= max_instructions` at both check sites in `cpu.rs`. This makes each batch run **exactly** `batch_size` instructions instead of `batch_size + 1`. Previously one extra instruction executed per batch — this off-by-one caused the exact threshold to depend on batch alignment.
- The `>=` fix makes MAX_INSTRUCTIONS an exact count. Use `132_865_700` to stop just as the kernel enters HLT idle; use `132_865_701` to execute one more instruction.

### Quick Debug Commands

**Key instruction count milestones — use the minimum needed:**
- `200_000_000` — full boot to `dlx login:` prompt (default). Takes ~8-10 seconds at ~20 MIPS.
- `132_865_710` — kernel first HLT threshold + 10; diagnostics print at run end. Use for ATA/IRQ debugging.
- `2_000_000` — BIOS POST only (fast check, VGA text visible)

```bash
# Build release binary
cargo build --release --example dlxlinux --features std

# Run headless — full boot to login prompt (default 200M instructions)
RUSTY_BOX_HEADLESS=1 ./target/release/examples/dlxlinux.exe

# Run with GUI (egui)
cargo run --release --example dlxlinux --features "std,gui-egui"

# Run with debug logging to see port 0x402 writes (and port 0x80 POST codes)
RUST_LOG=debug RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=500000 ./target/release/examples/dlxlinux.exe 2>&1 | grep -E "0x0402|0x0080|port_out.*402|BIOS output"
```

### Progress Metrics
- ✅ REX byte register mapping: bare REX (0x40) now correctly enables SPL/BPL/SIL/DIL — Alpine kernel enters 64-bit mode
- ✅ All major decoder bugs fixed (Group 1 opcodes, segment defaults, execute1/execute2)
- ✅ Protected mode transition works (far jump, GDT, segment loading)
- ✅ rombios32_init completes fully (ram_probe, cpu_probe, setup_mtrr, smp_probe, PCI init)
- ✅ BIOS output working: "Starting rombios32", "Found 1 cpu(s)", MP/SMBIOS tables
- ✅ MOV [mem], sreg / MOV sreg, [mem] memory forms (fixed IVT corruption after PM return)
- ✅ JMP/CALL r/m memory forms (vsnprintf jump table fix — was THE cause of output corruption)
- ✅ Store-direction register fixes complete (16-bit logical, 8-bit XCHG)
- ✅ LES/LDS (Load Far Pointer) 16/32-bit forms
- ✅ Complete 16-bit arithmetic: ADD/SUB/SBB/CMP in both GwEw and EwGw directions
- ✅ Complete 8-bit Group 1 immediate: ADD/SUB/ADC/SBB EbIb with R/M forms
- ✅ Extensive instruction set coverage (arithmetic, logical, shift, rotate, control flow, data transfer, string ops)
- ✅ REP string instructions fixed (check prefix before looping — was ~1000x slowdown)
- ✅ SCAS/CMPS with REPE/REPNE (ZF-based termination), INS/OUTS string I/O
- ✅ #DE exception delivery via IVT (DIV/IDIV handlers call self.exception())
- ✅ Proper Skylake-X CPUID implemented (Leaf 0 max=0x16, Leaf 1 Family 6 Model 0x55, extended leaves)
- ✅ APIC MMIO scratch buffer, CPU shutdown detection, WBINVD
- ✅ PIT→PIC→CPU interrupt delivery infrastructure complete
- ✅ PIT RateGenerator mode fixed (output transition detection was broken)
- ✅ VGA BIOS initializes and runs (vgabios.c + vbe.c)
- ✅ ATA disk detected: "ata0-0: PCHS=306/4/17 translation=none LCHS=306/4/17"
- ✅ BIOS completes full POST and reaches boot attempt stage (~1.1M instructions)
- ✅ Returns to real-mode BIOS with correct SS:SP, IVT intact after PM return
- ✅ INT 13h Read Sectors works — "Booting from 0000:7c00"
- ✅ LILO runs, loads compressed Linux kernel into memory
- ✅ Shift/rotate Ib dispatch: 6 opcodes fixed (SarEdIb, RolEbIb, etc. were using CL not imm8)
- ✅ Two-operand IMUL (0F AF) for kernel decompressor
- ✅ Icache SMC detection: first_bytes[8] fingerprint on each trace entry
- ✅ Kernel decompresses, enables paging, reaches idle HLT loop (~100M instructions)
- ✅ BT/BTS/BTR/BTC all 8 variants, MOVSX GdEw, LEAVE decoder fix
- ✅ VGA text output: word-wide I/O (mask 0x3), map mask plane filtering, window-base offsets
- ✅ Linux "Linux version 1.3.89" visible in VGA output — kernel console IS working
- ✅ Exception delivery: protected_mode_int BadVector → recursive exception() (double/triple fault chain)
- ✅ task_switch: TSS GPR load now writes EAX-EDI to CPU (compiler warning revealed dead assignment)
- ✅ Full x87 FPU: SoftFloat3e + all ~80 opcodes + Float128 transcendentals (Bochs-mirrored file structure)
- ✅ Complete task_switch: 286+386 TSS save, CR3+TLB flush, CR0.TS, LDTR/CS/SS/DS/ES/FS/GS validation
- ✅ Protected mode far CALL: call_protected() with code segs, call gates (same+inner priv), task gates
- ✅ Protected mode far RET: return_protected() with same-priv and outer-priv paths + validate_seg_regs()
- ✅ Far JMP system descriptors: TSS direct, task gates, call gates in jump_protected()
- ✅ IRET NT nested task return: back-link selector from TSS, validates busy TSS, task_switch
- ✅ MOV SS / POP SS interrupt inhibition (inhibit_mask + inhibit_icount checked before IRQ delivery)
- ✅ Enhanced RDMSR/WRMSR with actual MSR fields (sysenter CS/ESP/EIP, PAT, MTRR, EFER, STAR/LSTAR/CSTAR/FMASK, FS/GS/KernelGS base)
- ✅ handleCpuModeChange: cpu_mode updated from CR0.PE + EFLAGS.VM + EFER.LMA + CS.L (Long64/LongCompat)
- ✅ Long mode activation: EFER.LME + CR0.PG + CR4.PAE → sets EFER.LMA, CR3 64-bit in long mode
- ✅ Full 64-bit ALU: arith64 (ADD/ADC/SUB/SBB/CMP/NEG/INC/DEC/XADD/CMPXCHG), logical64 (XOR/OR/AND/NOT/TEST), shift64 (SHL/SHR/SAR/ROL/ROR/RCL/RCR/SHLD/SHRD), mult64 (MUL/IMUL/DIV/IDIV)
- ✅ 64-bit data transfer: MOV/MOVSX/MOVSXD/MOVZX/XCHG/LEA/BSWAP/CMOVcc all Eq/Gq forms
- ✅ 64-bit stack: PUSH/POP r/m64, PUSHFQ/POPFQ, PUSH imm64
- ✅ SWAPGS instruction for kernel/user GS base switching
- ✅ Zero compiler warnings (crate-level allows for intentional Bochs naming + dead code)
- ✅ Paging: system_write_byte/word/dword translate via page walk (was bypassing paging entirely)
- ✅ Paging: user_pl updated in load_cs() (was always false — no user-mode page protection)
- ✅ Paging: 4MB PSE path permission checks + A/D bit updates (was skipping both)
- ✅ handle_alignment_check: CPL from CS.selector.rpl not CL register
- ✅ Clean output: diagnostic prints downgraded from warn!/error! to debug!/trace!
- ✅ Triple fault FIXED: two root causes found and fixed:
  1. INT/INT3/INTO/INT1 always used interrupt_real_mode() even in protected mode — fixed with unified interrupt() dispatch
  2. XCHG r32, r/m32 (XchgEdGd) missing mod_c0() dispatch — memory form treated as register form
- ✅ vsprintf FIXED: accumulator-immediate 8-bit opcodes (ADD/XOR/ADC/SBB AL,Ib) used AH instead of AL
- ✅ "Trying to free nonexistent swap-page" RESOLVED: was caused by IDT corruption from INT dispatch bug
- ✅ BOUND uses exception(Br, 0) instead of interrupt_real_mode(5) — matches Bochs
- ✅ CpuLoopRestart leak fixed: InsertedOpcode path in cpu_loop_n properly catches CpuLoopRestart
- ✅ Kernel boots to driver init: console, BogoMIPS, networking, serial, PS/2, loop device
- ✅ **DLX Linux boots to `dlx login:` prompt** — init runs sysinit scripts, mounts ext2, starts syslogd, enters runlevel 4, launches agetty
- ✅ Fetch buffer 4K page bounding: prevented decoder from reading past page boundaries (THE root cause of agetty crashes)
- ✅ PUSH imm8 sign-extension: wait4(-1) fixed (was wait4(255) due to zero-extended 0xFF)
- ✅ Prefetch page fault propagation: CpuLoopRestart from translate_linear now propagated correctly
- ✅ Empty ATA drive returns 0xFF (floating bus), CMOS floppy registers configured, HLT busy-wait for Windows

## Build Commands

```bash
# Build everything
cargo build --all-features

# Build with release optimizations (needed for acceptable performance)
cargo build --release --all-features

# Run tests
cargo test

# Run a single test
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run examples (require --release for large stack)
cargo run --release --example init_and_run
cargo run --release --example dlxlinux --features std
cargo run --release --example dlxlinux --features "std,gui-egui"

# Fuzz the decoder
cd rusty_box_decoder && cargo +nightly fuzz run fuzz_target_1
```

## Workspace Structure

- **rusty_box/**: Main emulator library
- **rusty_box_decoder/**: Separate crate for x86 instruction decoding (allows fuzzing and reuse)
- **cpp_orig/bochs/**: Original C++ Bochs source for reference

## Architecture

### No Global State
The emulator uses instance-based architecture. Each `Emulator<I>` is completely self-contained, allowing multiple independent emulator instances to run concurrently.

### Core Components

```
Emulator<'a, I: BxCpuIdTrait>
├── BxCpuC<I>         CPU (generic over CPUID model like Corei7SkylakeX)
├── BxMemC            Memory subsystem (block-based, supports >4GB)
├── BxDevicesC        I/O port handler manager
├── DeviceManager     Hardware devices (PIC, PIT, CMOS, DMA, VGA, Keyboard, HardDrive)
├── BxPcSystemC       Timers and A20 line control
└── GUI               Display (NoGui, TermGui, or EguiGui)
```

### Initialization Sequence

```rust
let config = EmulatorConfig::default();
let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;
emu.initialize()?;                    // Init PC system, memory, devices, CPU
emu.load_bios(&bios_data, 0xfffe0000)?;
emu.load_optional_rom(&vga_bios, 0xc0000)?;
emu.reset(ResetReason::Hardware)?;
emu.prepare_run();
emu.cpu.cpu_loop(&mut emu.memory, &[])?;
```

### CPU Module Organization

Instructions are organized by category (matching original Bochs cpp_orig/bochs/cpu/ structure):
- `cpu/arith8.rs`, `cpu/arith16.rs`, `cpu/arith32.rs`: ADD, SUB, ADC, SBB, DEC, INC
- `cpu/logical*/`: AND, OR, XOR, NOT (8/16/32/64-bit variants)
- `cpu/mult*/`: MUL, IMUL (8/16/32/64-bit variants)
- `cpu/shift8.rs`, `cpu/shift16.rs`, `cpu/shift32.rs`: SHL, SHR, SAR, ROR, ROL
- `cpu/ctrl_xfer*/`: JMP, CALL, RET, loops (ctrl_xfer16.rs, ctrl_xfer32.rs, ctrl_xfer64.rs)
- `cpu/data_xfer8.rs`, `cpu/data_xfer16.rs`, `cpu/data_xfer32.rs`, `cpu/data_xfer64.rs`: MOV, LEA, XCHG
- `cpu/stack.rs`: Common stack primitives (push_16/32, pop_16/32, stack memory access)
- `cpu/stack16.rs`: 16-bit stack ops (PUSH/POP r16, PUSHA16, POPA16, PUSHF, POPF)
- `cpu/stack32.rs`: 32-bit stack ops (PUSH/POP r32, PUSHAD, POPAD, PUSHFD, POPFD)
- `cpu/stack64.rs`: 64-bit stack ops (PUSH/POP r64, PUSHFQ, POPFQ)
- `cpu/string.rs`: MOVSB, STOSB, LODSB, REP string operations
- `cpu/io.rs`: IN, OUT, INS, OUTS
- `cpu/soft_int.rs`: INT, IRET, INTO, BOUND, HLT
- `cpu/fpu/`: Full x87 FPU (mirrors Bochs `cpu/fpu/` file structure):
  - `fpu.rs` (FNINIT, FNSAVE, FRSTOR, FLDCW — mirrors `fpu.cc`)
  - `ferr.rs` (exception handling, stack overflow/underflow — mirrors `ferr.cc`)
  - `fpu_arith.rs` (FADD, FSUB, FMUL, FDIV + memory/integer variants)
  - `fpu_load_store.rs` (FLD, FST, FILD, FIST, FBLD, FBSTP, FISTTP)
  - `fpu_compare.rs` (FCOM, FUCOM, FCOMI, FTST, FXAM)
  - `fpu_trans.rs` (transcendental CPU handlers — mirrors `fpu_trans.cc`)
  - `fpu_misc.rs` (FXCH, FCHS, FABS, FDECSTP, FINCSTP, FFREE)
  - `fpu_const.rs` (FLD1, FLDPI, FLDL2T, etc.)
  - `fpu_cmov.rs` (FCMOV conditional moves)
  - `fsincos.rs`, `fpatan.rs`, `fyl2x.rs`, `f2xm1.rs`, `fprem.rs` (transcendental implementations)
  - `poly.rs` (Float128 polynomial evaluation)
  - `constants.rs` (CW/SW/TW bit definitions)
- `cpu/softfloat3e/`: Berkeley SoftFloat 3e port (true 80-bit extended precision):
  - `internals.rs` (round-and-pack core), `primitives.rs` (128-bit math), `specialize.rs` (NaN handling)
  - `extf80_*.rs` (add/sub, mul, div, sqrt, compare, conversions, misc)
  - `f128_*.rs` (Float128 operations for polynomial evaluation)

### CPU State Access

```rust
// Read-only getters
cpu.rax()      // u64 register value
cpu.rip()      // instruction pointer
cpu.eflags()   // flags register

// Setters
cpu.set_rax(0x777)
cpu.set_rip(0)
```

### Decoder Usage

```rust
// 32-bit mode
let instr = fetch_decode32_chatgpt_generated_instr(&bytes, is_32_bit_mode)?;

// 64-bit mode
let instr = fetch_decode64(&bytes)?;

// Const (compile-time) decoding
const NOP: BxInstructionGenerated = const_fetch_decode64(&[0x90]).unwrap();

// Access decoded instruction data
instr.dst()           // destination register
instr.src1()          // source register 1
instr.ib()            // 8-bit immediate
instr.id()            // 32-bit immediate
instr.get_ia_opcode() // decoded opcode
instr.ilen()          // instruction length
```

### Decoder Validation

The decoder performs validation to ensure only valid x86 encodings are produced:

- **Segment register indices** must be 0-5 (ES, CS, SS, DS, FS, GS)
- Invalid segment register indices (6-7) cause `DecodeError::InvalidSegmentRegister`
- This prevents undefined behavior and catches decoder bugs early
- See `docs/DECODER_BUGS.md` for historical bug fixes and validation details

### Memory Layout

- **0x00000-0x9FFFF**: Conventional memory (640KB)
- **0xA0000-0xBFFFF**: VGA memory
- **0xC0000-0xDFFFF**: Expansion ROM (128KB)
- **0xE0000-0xFFFFF**: BIOS ROM area (128KB)
- **0xFFF80000-0xFFFFFFFF**: System ROM (512KB BIOS)

### I/O Device Registration

Devices register port handlers during init. Each port (0x0000-0xFFFF) can have read/write handlers:
- PIC: 0x20-0x21, 0xA0-0xA1
- PIT: 0x40-0x43
- Keyboard: 0x60, 0x64
- CMOS: 0x70-0x71
- VGA: 0x3B0-0x3DF
- IDE: 0x1F0-0x1F7, 0x3F6-0x3F7
- System Control (A20/reset): 0x92

## Feature Flags

Key Cargo features in `rusty_box/Cargo.toml`:
- `std`: Standard library support (terminal, file I/O)
- `gui-egui`: Graphical UI using egui
- `bx_full`: Enables all emulation features (default)
- `bx_little_endian` / `bx_big_endian`: Endianness (mutually exclusive)
- `bx_phy_address_long`: >4GB physical address support
- `bx_support_apic`: APIC support
- `bx_support_pci`: PCI bus support

**Feature gate removal (2026-03-04):** Most CPU-level `#[cfg(feature = "bx_support_*")]` gates have been removed from CPU core files (cpu.rs, event.rs, init.rs, builder.rs, proc_ctrl.rs, string.rs, mwait.rs, paging.rs, tlb.rs, icache.rs, opcodes_table.rs). Features like APIC, MONITOR/MWAIT, MEMTYPE, AMX, PKEYS, EVEX, SSE, AVX, VMX, SVM, alignment check, and handler chaining are now always compiled. The I/O device files (devices.rs, ioapic.rs) still retain PCI/APIC gates for structural reasons.

**Register types (2026-03-04):** All control/debug register types now use the `bitflags` crate:
- `BxCr0`: bitflags (PE, MP, EM, TS, ET, NE, WP, AM, NW, CD, PG)
- `BxCr4`: bitflags (VME, PVI, TSD, DE, PSE, PAE, MCE, PGE, PCE, OSFXSR, etc.)
- `BxEfer`: bitflags (SCE, LME, LMA, NXE, SVME, LMSLE, FFXSR, TCE)
- `BxDr6`: bitflags (B0-B3, BD, BS, BT)
- `BxDr7`: bitflags (L0-L3, G0-G3, LE, GE, GD) + multi-bit field accessors (R/W, LEN)

## Key Files for Common Tasks

| Task | Files |
|------|-------|
| Add new instruction | `rusty_box_decoder/src/fetchdecode*.rs`, `cpu/<category>/` |
| Add new I/O device | `iodev/` (new file), `iodev/devices.rs` (registration) |
| Modify memory mapping | `memory/misc_mem.rs`, `memory/mod.rs` |
| Add CPUID model | `cpu/cpuid/` |
| Add/modify FPU instruction | `cpu/fpu/` (handlers), `cpu/softfloat3e/` (math), `cpu/i387.rs` (state) |
| Debug execution | Enable tracing: `Level::DEBUG` or `Level::TRACE` in examples |

## Error Handling

Uses `thiserror` with root `Error` enum in `src/error.rs` aggregating:
- `CpuError`: CPU execution errors, unimplemented instructions
- `MemoryError`: Memory access errors
- `DecodeError`: Instruction decoding errors

## Platform Notes

- Uses `OnceLock` (std) or `spin::once::Once` (no_std) for singletons
- Examples require large stack (500MB-1.5GB) - spawned on dedicated thread
- Register layout in `BxGenReg` union differs by endianness feature flag

## Known Issues

### BIOS Output — VERIFIED WORKING (2026-02-23)

**Status:** RESOLVED - full rombios32_init output confirmed

**Full BIOS output from port 0x402:**
```
Starting rombios32
Shutdown flag 0
ram_size=0x01fa0000
ram_end=33161216MB
Found 1 cpu(s)
bios_table_addr: 0x000fa1d8 end=0x000fcc00
MP table addr=0x000000b0 MPC table addr=0x000000e0 size=0xc8
SMBIOS table addr=0x000000c0
bios_table_cur_addr: 0x000001a3
```

**Three bugs fixed in this session to make timer-driven BIOS waits work:**
1. **PIT RateGenerator mode** (pit.rs): Output pulsed LOW→HIGH in same clock() call, making transition check always see no change. Fixed by separating LOW pulse from HIGH recovery across clock cycles.
2. **async_event not cleared** (event.rs): Bochs event.cc:428-433 clears `async_event=0` at end of `handleAsyncEvent()`. Our version never cleared it → `BX_ASYNC_EVENT_STOP_TRACE` stayed set → inner trace loop broke after every instruction (executed=1 per batch). Fixed to match Bochs.
3. **Minimum usec for tick_devices** (emulator.rs): With executed=1 at IPS=15M, `usec = (1*1M)/15M = 0` → tick_devices never called → PIT starved. Fixed with `usec = usec_from_instr.max(10)`.

**Result:** PIT fires IRQ0 → PIC raises interrupt → CPU injects INT 8 → handler increments BDA[0x046C] → timer wait loops exit → BIOS continues.

### INT 13h Read Sectors Error — RESOLVED (2026-02-24)

**Status:** Fixed — INT 13h Read Sectors works, LILO boots, kernel loads and runs to login prompt.

### BIOS ROM Shadow Mapping (2026-02-17)

**Status:** Partially fixed

Found that `get_host_mem_addr` PCI path for 0xE0000-0xFFFFF used wrong ROM offset formula. Fixed to use `bios_map_last128k()` which maps shadow addresses to the last 128KB of the 4MB ROM array. Three locations in `misc_mem.rs` were corrected. The real-mode BIOS execution was NOT affected (it took an earlier correct code path), but protected-mode code that accesses the BIOS shadow area now gets correct data.

### Decoder Bug: Group 3a/3b Immediate Size (2026-02-02)

**Status:** Identified, not yet fixed (may no longer be hit with current BIOS path)

The decoder fails to account for immediate bytes in TEST instructions (opcodes 0xF6 and 0xF7 with ModRM.nnn=0 or 1). Impact: instruction length miscalculation causes RIP misalignment. This was the original cause of BIOS crashes at 0xe1d59 with the legacy BIOS, but may not be triggered by the current BIOS-bochs-latest execution path.

### Exception Handling (2026-02-02)

**Status:** Fully implemented — real mode IVT, protected mode IDT, double/triple fault chain, recursive exception delivery all working. DLX Linux boots to login with full exception handling.

### Major Bug Fixes (Historical)

**Session 28 (2026-03-07) — Full decoder/accessor/struct audit (10 agents + manual)**

*Manual fixes applied:*
0. **FIXED: Ed,Gd convention mask too broad** (`fetchdecode32.rs:565`, `fetchdecode64.rs:431`): `(b1 & 0x0F) == 0x01/0x09` caught two-byte opcodes. Added `b1 < 0x100` guard + explicit two-byte matches.
0. **FIXED: 64-bit LEAVE (0xC9) missing from no-ModRM list** (`fetchdecode64.rs:960`).
0. **FIXED: 64-bit INS/OUTS (0x6C-0x6F) missing from no-ModRM list** (`fetchdecode64.rs:955`).

*Agent-discovered CRITICAL bugs — ALL RESOLVED:*
0. **NOT A BUG: SSE prefix F2/F3** — `PrefixF3=2, PrefixF2=3` matches Bochs `SSE_PREFIX_F3=2, SSE_PREFIX_F2=3` exactly. Original audit was wrong.
0. **FIXED: get_gpr32() R8D-R15D** — already uses direct `gen_reg[idx]` array access.
0. **FIXED: REX.B for non-ModRM** — already applied: `if !needs_modrm && (rex_prefix & 0x01) != 0 { rm |= 8; }`.

*Agent-discovered HIGH bugs — status:*
0. **FIXED: 0x67 prefix** — already clears only As64.
0. **FIXED: msr_fsbase()/msr_gsbase()** — already reads segment base correctly.
0. **Missing ~95 SSE opcode tables** (0F 50-7F, 0F D0-FE): Blocks all SSE instruction execution.
0. **FIXED: System read/write 64-bit addresses** — already uses `long_mode()` mask.
0. **FIXED: 64-bit writes SMC check** — already present in write functions.
0. **FIXED: 8-bit REX handlers** — already use `read_8bit_regx`/`write_8bit_regx`.
0. **FIXED: XOP mod=11 validation** — already checks mod field.
0. **FIXED: FEMMS/UD0 no-ModRM** — already in no-ModRM list.
0. **FIXED: NOP/PAUSE** — decoded by decoder AND wired in dispatcher (Nop → Ok(()), Pause → Ok(())).
0. **Non-ModRM opcodes excluded from decmask NNN/RRR fields** (64-bit decoder): Always include nnn/rm.
0. **Missing SRC_EQ_DST bit in decmask** (both decoders): Zero-idiom opcodes never match.
0. **UD64 opcodes (PUSH ES, DAA, etc.) decode as valid** in 64-bit decoder: 21 opcodes should return IaError.
0. **FIXED: Group 3 (F6/F7) nnn masking** — already masks `nnn & 7`.
0. **FIXED: LOGIC_MASK AF** — already includes AF.
0. **64-bit TLB fast path missing** (`access.rs`): All 64-bit accesses do full page walk (performance).

See `memory/audit-session28-full.md` for complete findings from all 10 agents.

**Session 26 (2026-03-07) — Fix 64-bit paging bypass + missing opcodes**
0. **64-bit paging bypass — ALL memory access (access.rs, 9 files)**: Every 64-bit memory access function passed linear addresses directly to physical memory via `mem_read_*/mem_write_*`, bypassing page table walks entirely. In long mode (CR0.PG=1), linear addresses must be translated through 4-level page tables. Root cause: helper functions `read_linear_qword`, `read_rmw_linear_qword`, `write_rmw_linear_qword`, `stack_read_qword`, `stack_write_qword` were thin wrappers around `mem_read_qword`/`mem_write_qword` (physical access) instead of going through `translate_data_read`/`translate_data_write`. Fix: Added proper paging-aware 64-bit access functions in `access.rs` (`read_virtual_qword_64`, `write_virtual_qword_64`, `read_rmw_virtual_qword_64`, `write_rmw_virtual_qword_back_64`, `stack_read_qword_64`, `stack_write_qword_64`) with cross-page boundary handling and address_xlation caching for RMW. Replaced bypass wrappers with paging-enabled versions that accept pre-computed linear addresses (`read_linear_qword`, `read_rmw_linear_qword`, `write_rmw_linear_qword`, `stack_read_qword`, `stack_write_qword`). Updated ~137 call sites: data_xfer64.rs, ctrl_xfer64.rs, stack64.rs (full rewrite to use new functions), arith64.rs, logical64.rs, shift64.rs, mult64.rs (added `?` propagation), bit64.rs (replaced direct `mem_*` calls with `read/write_virtual_qword_64`), string.rs (qword string ops), segment_ctrl_pro.rs (64-bit stack reads), crc32.rs.
0. **LEA GdM 64-bit mode fix** (`data_xfer_ext.rs`): `lea_gd_m()` always used `resolve_addr32()`. In 64-bit mode with 32-bit operand size (LEA r32, [rip+disp32]), the address should be computed with `resolve_addr64()` then truncated to 32 bits. Fixed to check `long64_mode()` and use the correct resolver.
0. **Missing dispatcher: SubGqEqZeroIdiom** (`dispatcher.rs`): SUB r64,r64 zero idiom had no dispatcher entry. Mapped to existing `sub_gq_eq` handler alongside `SubGqEq`.
0. **XCHG r64, RAX** (`data_xfer64.rs`): Added `xchg_rrx_rax()` handler for opcode 0x90+rd with REX.W (XchgRrxRax). Simple register swap.
0. **Dispatcher return type cascade** (`dispatcher.rs`): ~38 dispatcher entries wrapped functions returning `Result<()>` with `{ self.fn(instr); Ok(()) }`, silently discarding page fault errors. Fixed all to propagate `?` or call directly: 64-bit data xfer (MOV/MOVSX/MOVZX/LEA/XCHG), stack ops (PUSHFQ/POPFQ/PUSH imm), MOV moffs64, and all 16 CMOVcc 64-bit dispatchers.
0. **Investigation: LPF mask latent bug** (`paging.rs`, `access.rs`): `translate_data_access` and 32-bit TLB fast paths use `0xFFFF_F000` as LPF mask, which is only 32-bit wide (`0x0000_0000_FFFF_F000` as u64). In 64-bit mode this zeroes upper 32 address bits, causing TLB aliasing. Not currently blocking because new 64-bit access functions bypass inline TLB and go through `translate_data_read/write` directly. The ITLB (instruction fetch) correctly uses `lpf_of()` with full 64-bit `LPF_MASK`. Fix deferred — will need updating when adding TLB fast path to 64-bit access functions.
0. **Investigation: 64-bit word/dword string ops bypass** (`string.rs`): Word-width (`movsw64`, `stosw64`, etc.) and dword-width (`movsd64`, `stosd64`, etc.) 64-bit string ops also bypass paging via `mem_read_word`/`mem_write_dword`. Only byte-width ops correctly use `read_virtual_byte_at_laddr`. Qword ops fixed in this session. Word/dword fix deferred.
0. **Remaining missing opcodes identified**: LahfLm/SahfLm, MovqVdqEq/MovqEqVq, FSGSBASE (8 opcodes), float↔int64 conversions (6 opcodes), MovntiMqGq. Implementations deferred — will add as Alpine boot hits them.

**Session 25 (2026-03-06) — Alpine kernel enters 64-bit long mode**
0. **REX byte register mapping — decoder** (`fetchdecode64.rs:178`): `rex_prefix = b & 0x0F` stored only the low nibble. For bare REX (0x40, no R/X/B/W bits), this produced 0, so the `if rex_prefix != 0` check failed and `Extend8bit` was never set. Bare REX still enables SPL/BPL/SIL/DIL mapping. Fixed: `rex_prefix = b` (store full byte). REX bit tests (`& 0x04`, `& 0x01`, `& 0x08`) still work since they test low nibble bits.
0. **REX byte register mapping — accessor** (`logical8.rs:131-158`): `read_8bit_regx`/`write_8bit_regx` called `get_gpr8(idx)` which maps indices 4-7 to high bytes (AH/CH/DH/BH) unconditionally. With REX, indices 4-7 should map to SPL/BPL/SIL/DIL (low byte of RSP/RBP/RSI/RDI). Fixed: directly access `gen_reg[reg_idx].word.byte.rl` when `extend8bit_l != 0`. Symptom: Alpine kernel memset wrote 0x20 (DH) instead of 0x00 (SIL) to page table memory, corrupting PDPTE entries → triple fault.

**Session 13 (2026-03-04) — DLX Linux boots to interactive bash shell**
0. **SHRD/SHLD decoder convention** (`fetchdecode32.rs`): Opcodes 0F A4/A5 (SHLD) and 0F AC/AD (SHRD) were in the ELSE decoder branch, making `dst()=nnn` and `src1()=rm`. The implementations expected Ed,Gd convention (`dst()=rm`, `src1()=nnn`). This swapped operands: the shift result went to the wrong register. Root cause of ext2 "directory #12 contains a hole" errors — the kernel's `block = f_pos >> block_size_bits` (using SHRD) never actually shifted the destination register, so block was always the raw f_pos value. Fixed by adding SHRD/SHLD opcodes to the Ed,Gd branch.
0. **Diagnostic cleanup** (`dlxlinux.rs`): Removed ~1600 lines of debugging code (inode12 monitoring, RAM scanning, page table walks, kernel code dumps). Replaced with clean 60-line headless boot loop.
0. **Control register bitflags** (`crregs.rs`): CR0, CR4, EFER, DR6, DR7 converted to `bitflags!` types matching Bochs crregs.cc.
0. **Feature gate removal**: Removed CPU-level `#[cfg(feature)]` gates — APIC, MONITOR/MWAIT, SSE, AVX, VMX, SVM always compiled.
0. **PAE + long mode paging** (`paging.rs`): Added page_walk_for_dtlb_pae and page_walk_for_dtlb_long_mode.
0. **64-bit control transfers** (`segment_ctrl_pro.rs`): call_protected_64, return_protected_64, long_iret, call_gate64.

**Session 10 (2026-03-03) — DLX Linux boots to `dlx login:`**
0. **Fetch buffer 4K page bounding** (`cpu.rs`): `get_host_mem_addr()` returned `&actual_vector[start..]` extending to end of all emulator RAM. The decoder could read past a 4K page boundary into physically adjacent but virtually unrelated memory, producing stale CALL displacements → wild jumps to 0xBDED0580. Fixed by bounding to `fetch_ptr[..4096]`. The TLB code path already did this correctly.
0. **PUSH imm8 sign-extension** (`fetchdecode32.rs`): Opcode 0x6A (PUSH imm8) zero-extended its immediate. `PUSH 0xFF` pushed 0x000000FF (255) instead of 0xFFFFFFFF (-1). This broke `wait4(-1)` in init → agetty children weren't reaped → "No more processes left in this runlevel". Fixed by adding 0x6A, 0x6B to sign-extension list.
0. **Prefetch page fault propagation** (`cpu.rs`): `translate_linear` errors during `prefetch()` were silently swallowed (`Err(_) => { None }`). Page fault exceptions pushed a frame and changed RIP, but `CpuLoopRestart` was never returned → `boundary_fetch` continued with stale state → panic. Fixed by propagating the error.
0. **Icache SMC boundary check** (`cpu.rs`): For page-spanning instructions, `first_bytes` only checked bytes on the current page. If the next page was remapped (e.g. `uselib`/`mmap`), the remaining bytes changed but SMC didn't catch it. Added `if avail < ilen { smc_invalid = true }`.
0. **Empty ATA drive status** (`harddrv.rs`): Returned 0x00 for status register when no device attached. Fixed to return 0xFF (floating bus) matching real hardware.
0. **CMOS floppy registers** (`cmos.rs`): CMOS 0x10 (drive types) and 0x14 (equipment byte) now set for DLX's two 1.44M floppy drives.
0. **HLT busy-wait** (`emulator.rs`): Small sleeps (<2ms) now spin instead of calling `thread::sleep`, avoiding Windows 15.6ms timer granularity rounding.

0. **INT/INT3/INTO always used IVT in protected mode (2026-02-28)**: `int_ib()`, `int3()`, `into()`, and `int1()` unconditionally called `interrupt_real_mode(vector)` regardless of CPU mode. In protected mode, this read the IVT at physical `vector*4` instead of dispatching through the IDT. For Linux INT 0x80 (syscall), this caused the kernel to jump to startup_32 (0x100000) with CS=0x0000, re-executing `setup_idt` which overwrote all IDT entries with the default `ignore_int` handler, then any subsequent exception would recursively call `printk` → stack overflow → GDT corruption → triple fault. Fixed by creating a unified `interrupt()` method (matching Bochs `exception.cc:762-839`) that dispatches to `interrupt_real_mode()` or `protected_mode_int()` based on CPU mode. Also fixed BOUND to use `exception(Br, 0)` matching Bochs.
1. **XCHG r32, r/m32 missing mod_c0 dispatch (2026-02-28)**: `XchgEdGd` in the dispatcher always called the register form, never checking `instr.mod_c0()` for memory operands. `XCHG EAX, [ESP+offset]` in the Linux exception handler was treated as `XCHG EAX, ESP`, setting ESP=0xFFFFFFFF. The subsequent `PUSH` caused a page fault, leading to double/triple fault. Fixed by adding mod_c0 dispatch matching the 8-bit and 16-bit XCHG forms.
2. **Accumulator-immediate 8-bit register bug (2026-02-28)**: Opcodes 0x04 (ADD AL,Ib) and 0x34 (XOR AL,Ib) operated on AH instead of AL. The decoder extracts `rm = opcode & 7` which is 4 for these opcodes, and the generic `ADD_EbIb`/`xor_eb_ib_r` handlers used `instr.dst()` (=4=AH) instead of hardcoding register 0 (AL). Fixed by adding dedicated `ADD_ALIb`, `XOR_ALIb`, `ADC_ALIb`, `SBB_ALIb` handlers that hardcode AL. This was the root cause of the vsprintf bug: Linux 1.3.89 vsprintf uses `ADD AL, 0xA8` to compute a jump table index for format conversion characters, but since AH was modified instead of AL, the index was always wrong and the default case ran, outputting raw format specifiers like "%uk/%uk".
1. **Paging: system_write bypass fix (2026-02-28)**: `system_write_byte/word/dword` passed linear addresses directly to `mem_write_*`, bypassing paging. TSS writes, descriptor access-bit updates, and GDT/LDT writes went to wrong physical addresses when paging was enabled. Added `translate_linear_system_write()` with full page walk and A/D bit updates.
2. **Paging: user_pl never updated (2026-02-28)**: `user_pl` was initialized to `false` and never assigned. All paging permission checks treated accesses as supervisor-level, meaning CPL=3 code could read/write kernel pages. Fixed by setting `self.user_pl = (cpl == 3)` in `load_cs()`.
3. **Paging: 4MB PSE permission skip (2026-02-28)**: `translate_linear_legacy` 4MB page path returned immediately without permission checks or A/D bit updates. Added PRIV_CHECK + A/D update matching the 4KB path.
4. **handle_alignment_check CPL (2026-02-28)**: Used `self.cl()` (CL register) instead of `self.sregs[CS].selector.rpl` for CPL check.
5. **REP string prefix fix (2026-02-24)**: REP LODSB/STOSB/MOVSB/etc. always looped CX times even without REP prefix. Non-REP forms should execute once. Caused ~1000x slowdown when VGA BIOS executed single-iteration string ops.
2. **#DE exception delivery fix (2026-02-24)**: DIV/IDIV handlers in mult8/16/32.rs returned `Err(BadVector)` which terminated the CPU loop. Changed to `self.exception(Exception::De, 0)` for proper IVT delivery.
3. **SCAS/CMPS REPE/REPNE semantics (2026-02-24)**: Added ZF-based loop termination for REPE (break if ZF=0) and REPNE (break if ZF=1) string compare/scan ops.
4. **INS/OUTS string I/O (2026-02-24)**: Implemented INSB/INSW/INSD and OUTSB/OUTSW/OUTSD with REP variants for ATA PIO disk access.
5. **PIT RateGenerator mode fix (2026-02-23)**: Mode 2 output pulsed LOW→HIGH in same clock() call. Fixed by separating LOW pulse from HIGH recovery across clock cycles.
6. **async_event clearing fix (2026-02-23)**: Matched Bochs event.cc:428-433 — clears `async_event=0` at end of `handleAsyncEvent()`.
7. **JMP/CALL r/m memory form fix (2026-02-23)**: Added memory-form handlers with `mod_c0()` dispatch for vsnprintf jump table.
8. **Store-direction register fix (2026-02-23)**: 16-bit logical ops and 8-bit XCHG used wrong register fields due to decoder's meta_data swap.
9. **Port 0x61 delay_ms fix (2026-02-19)**: `keyboard.rs` port 0x61 bit 4 now toggles on each read.
10. **Hot-path logging fix (2026-02-19)**: Changed `get_icache_entry` from `debug!` to `trace!`, `prefetch` from `info!` to `debug!`.
11. **REP STOSB/MOVSB 32-bit fix (2026-02-19)**: Dispatch to 32-bit variants when `instr.as32_l() != 0`.
6. **Log flooding fix (2026-02-17)**: Out-of-bounds memory write messages (`misc_mem.rs`, `memory_stub.rs`) downgraded from `debug!` to `trace!`.
7. **Segment default fix (2026-02-16)**: `[BP+disp]` was using DS instead of SS. Fixed with lookup tables in `fetchdecode32.rs`.
8. **execute1/execute2 fix (2026-02-16)**: 18 opcodes had memory/register handler forms swapped in `opcodes_table.rs`.
9. **Group 1 decoder fix (2026-02-02)**: ModRM `reg` field stored instead of `r/m` for opcodes 0x80/0x81/0x83.
10. **BIOS load address fix (2026-02-07)**: Address calculated from BIOS size instead of hardcoded.
11. **Memory allocation fix (2026-02-06)**: `vec![0; size]` instead of loop-based `push()` for large allocations.
