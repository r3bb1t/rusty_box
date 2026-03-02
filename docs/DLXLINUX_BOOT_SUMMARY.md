# DLX Linux Boot Summary

## Current State (2026-03-02)

The Rusty Box emulator successfully boots DLX Linux from BIOS POST through LILO to kernel execution. The kernel completes all driver initialization and stalls at an idle HLT loop waiting for ATA disk I/O (IRQ14) after 132M+ instructions.

### VGA Text Output (headless mode)

```
Bochs VGABios (PCI) 0.9c 08 Jan 2025
This VGA/VBE Bios is released under the GNU LGPL
Please visit :
 . https://bochs.sourceforge.io
 . https://www.nongnu.org/vgabios
NO Bochs VBE Support available!
Bochs 3.0.devel BIOS - build: 05/15/25
Options: apmbios pcibios pnpbios eltorito rombios32
Press F12 for boot menu.
Booting from Hard Disk...
LILO boot:
Loading linux......
```

This matches the reference Bochs output except for "NO Bochs VBE Support available!" (our VGA device doesn't implement VBE extensions) and missing the ATA disk details line.

---

## Boot Sequence Timeline

### Phase 1: BIOS POST (0 — ~680k instructions)

| Stage | Instructions | Description |
|-------|-------------|-------------|
| Reset vector | 0-10 | CPU starts at F000:FFF0, jumps to F000:E05B |
| Real-mode init | 10-345k | Keyboard init, memory probe, CMOS reads, PCI scan |
| PM entry | 345k | Far jump to CS=0x10:0xF9E5F (protected mode) |
| rombios32_init | 345k-395k | BSS clear, ram_probe, cpu_probe, setup_mtrr, smp_probe, PCI init |
| PM return | 395k | Far jump back to real mode F000:9EB2 |
| VGA BIOS | 395k-500k | ROM scan finds VGA BIOS at C000:0003, initializes text mode |
| ATA detection | 500k-600k | IDENTIFY DEVICE on ata0-0, reads geometry PCHS=306/4/17 |
| Boot attempt | 600k-680k | INT 19h selects boot device, INT 13h reads boot sector |

**Key reasoning**: The instruction count milestones were determined by tracing far jump transitions (logged by `JmpfAp` in cpu.rs) and BIOS output strings on port 0x402. The PM entry/return pair wraps rombios32_init which handles 32-bit PCI and ACPI table setup.

### Phase 2: LILO Boot Loader (680k — ~1.3M instructions)

| Stage | Instructions | Description |
|-------|-------------|-------------|
| Boot sector | 680k-691k | JMP to 8A00:0098, relocates to 8B00:0000 |
| LILO first stage | 691k-1.28M | Reads map file, loads second stage, reads kernel sectors |
| LILO second stage | 1.28M | Prompts "LILO boot:", loads linux image via INT 13h |

**Key reasoning**: LILO's two-stage loading was confirmed by watching far jump sequences. The first jump to 8A00:0098 is the boot sector code, then 8B00:0000 is the relocated first stage. LILO's "Loading linux......" dots correspond to batches of INT 13h sector reads.

### Phase 3: Linux Kernel Boot (1.3M — 132M+ instructions)

| Stage | Instructions | Description |
|-------|-------------|-------------|
| Setup code | 1.28M-1.29M | Real-mode setup at 9020:0000, A20 enable, memory detect |
| PM switch | 1.29M | Far jump to CS=0x10:0x1000 (32-bit flat PM) |
| Decompressor | 1.29M-90M | Decompress bzImage at 0x1000, extracts to 0x100000 (~29 MIPS) |
| Kernel start | 90M | Far jump to CS=0x10:0x100000 (kernel entry) |
| Paging enable | 90M | CR0 set to 0x80000013 (PE+WP+PG), CR3 points to page tables |
| Kernel init | 90M-132M | Memory init, device probing, scheduler setup, driver init (~14 MIPS) |
| Idle HLT | 132M+ | Waiting for ATA disk IRQ14 — nIEN=1 set by kernel, polls status instead |

**Key reasoning**: The decompressor phase was identified by watching the RIP range (0x1000-0x5000 area) and the final far jump to 0x100000. Paging enable was confirmed by CR0 value in the `JmpfAp` log. The kernel uses segmentation with base=0xC0000000, limit=0x3FFFFFFF (3G/1G split) — this was diagnosed when string instructions failed because they used linear addresses as physical (fixed by adding paging translation to all 32-bit string ops).

---

## Critical Bug Fixes That Enabled Boot

### 1. VGA Word-Wide I/O (2026-02-25)

**Symptom**: VGA text dump was completely empty (all `0x26 0x0A` = green attribute + newline). VGA probe showed `mapped_writes=0, unmapped_writes=37468`.

**Root cause**: VGA I/O ports were registered with mask `0x1` (byte-only), but the VGA BIOS programs registers using word-sized OUT instructions (`OUT DX, AX` where AL=index, AH=data to port 0x3CE). Word writes (`io_len=2`) failed the mask check and were silently dropped by the I/O subsystem.

**Evidence**: Added eprintln to `VGA_GRAPHICS_DATA` write handler — zero calls received. Then checked Bochs `vgacore.cc:208-235`: all VGA write handlers registered with mask `3` (byte+word). Line 806-809 shows word writes are split into two byte writes: `write(addr, val & 0xff, 1); write(addr+1, (val >> 8) & 0xff, 1);`.

**Fix**: Changed all VGA port registration masks from `0x1` to `0x3`. Added word-write splitting at the top of `write_port()`. Result: `graphics_regs[6]` changed from 0x08 (mono window 0xB0000) to 0x0E (color window 0xB8000), enabling the correct memory_mapping=3.

### 2. VGA Memory Plane Filtering (2026-02-25)

**Symptom**: Text output correct but followed by garbage font bitmap data on each line.

**Root cause**: The VGA BIOS loads character fonts by writing to VGA memory with sequencer map mask=0x04 (plane 2 only). Our flat `text_memory` array stored ALL writes regardless of which plane was selected. Font data at 0xA0000 with `addr & 0x7FFF = 0` overwrote text data at 0xB8000 with `addr & 0x7FFF = 0`.

**Fix**: Check `seq_regs[2]` (map mask) in the write handler. Only update `text_memory` when planes 0/1 are selected (`map_mask & 0x03 != 0`). Font writes (plane 2/3) are consumed but don't touch the text buffer. Also changed offset calculation from `addr & 0x7FFF` to `addr - window_base` to prevent different memory_mapping windows from aliasing.

### 3. 32-bit String Ops Paging Translation (2026-02-25)

**Symptom**: Kernel stuck at RIP=0x10911d executing LODSB in an infinite loop.

**Root cause**: All 32-bit string instructions (MOVSB, STOSB, LODSB, CMPS, SCAS) used `mem_read_byte(linear_address)` which treats the linear address as a physical address. With paging enabled and DS base=0xC0000000, the linear address 0xC019CF33 is not a valid physical address — paging should translate it to physical 0x0019CF33 via page table walk.

**Evidence**: The kernel uses segmentation with base=0xC0000000. DS:ESI=0x0019CF33 becomes linear address 0xC019CF33. Without paging translation, the memory read goes to physical 0xC019CF33 which is beyond RAM, returning garbage. With translation, page tables map 0xC019CF33 → 0x0019CF33.

**Fix**: Rewrote all 32-bit string ops to use `read_virtual_byte(BxSegregs::Ds, esi)` / `write_virtual_byte(BxSegregs::Es, edi, val)` which chain through segment limit check → base addition → page table translation → physical read/write. Added all missing 32-bit variants and REP/REPE/REPNE forms.

### 4. BT/BTS/BTR/BTC Instructions (2026-02-25)

**Symptom**: `UnimplementedOpcode: BtsEdIb` at RIP=0x17D912.

**Root cause**: The Linux kernel uses BTS extensively for bit manipulation in page allocation bitmaps, IRQ handling, and feature flags. None of the 8 bit-test variants were implemented.

**Fix**: Implemented all 8 variants in logical32.rs: 4 with immediate operand (BT/BTS/BTR/BTC r/m32,imm8) and 4 with register operand (BT/BTS/BTR/BTC r/m32,r32). Memory form with register operand uses signed bit displacement for testing bits beyond the dword boundary.

---

## Performance (as of 2026-03-02)

Measured using windowed per-second MIPS output (`tracing::error!(target: "mips")`) in headless release mode:

| Phase | Instructions | MIPS | Notes |
|-------|-------------|------|-------|
| BIOS real-mode | 0–67M | ~22 | Keyboard init, memory probe, PCI scan |
| BIOS→PM / rombios32 | 67–90M | ~22 | PCI BIOS, ACPI tables, return to real mode |
| Kernel decompressor | 90–118M | ~29 | **Exceeds Bochs target (~14.7 MIPS)** |
| Kernel initialization | 118–132M | ~14 | Matches Bochs target |
| Idle HLT | 132M+ | ~0 | Waiting for IRQ14 disk interrupt |

### Key optimizations enabling this throughput

1. **`fetch_decode32_inplace`** — decoder writes directly into the icache mpool, eliminating a 24-byte struct copy per instruction
2. **Raw pointer execute loop** — dispatch uses a raw slice pointer, eliminating bounds-check overhead per instruction
3. **Trace allocation guard** — `format!` / `collect::<String>()` in icache serve path gated on `tracing::enabled!(TRACE)`

Previous full-run average (~3–4 MIPS) was misleadingly low because it included the HLT idle phase. Active execution rates have always been higher; the windowed measurement makes this visible.

## Remaining Issues

### Kernel Stalls After Driver Init

The kernel prints through "loop: registered device at major 7" then enters HLT. It needs to:
1. Read the root filesystem from the ATA disk (IRQ14 / DRQ polling)
2. Mount the rootfs and start `/sbin/init`
3. Continue to the `dlx login:` prompt

**Root cause**: The kernel sets nIEN=1 (disabling disk interrupts) and polls the ATA status register. The polling loop should complete after the disk completes its command, but the drive state machine is not progressing. This is the primary remaining issue.
