# Upstream PR — NOT YET FILED (prepared 2026-10-04)

Everything after the `---` below is the PR body, ready to paste. Above it are
the notes for whoever files it; they are not part of the PR.

**Files that go with it** (all in `docs/`):
- `bochs-retf16-imm16-fix.patch` — the fix as a `git am`-ready patch
  (author `r3bb1t`, the identity configured in the `cpp_orig/bochs` checkout).
  Two files: the one-line type change in `bochs/cpu/ctrl_xfer16.cc` and a
  `bochs/cpu/HISTORY` line in upstream's own style. Checked with
  `git apply --check` against upstream master `f87c5e226` (2026-10-03); the
  code hunk also applies to the vendored snapshot. Drop the HISTORY hunk if the
  maintainer prefers to write HISTORY himself.
- `bochs-retf16-imm16-probe.S` — the reproduction: a 512-byte boot sector,
  built with GNU binutils alone. Attach it to the PR, or paste it into a
  collapsed section.

**How to file** (on the day):
1. In the fork checkout, branch from current upstream master (re-run
   `git apply --check` first — upstream moves daily; the hunk is anchored on
   `RETfar16_Iw`, not on a line number, so an offset is harmless).
2. `git am docs/bochs-retf16-imm16-fix.patch` (adjust the commit message or
   the Date header if wanted), push the branch to `r3bb1t/Bochs`, open the
   PR against `bochs-emu/Bochs` master with the body below.
3. Then update the "Filing status" of the entry in
   `docs/bochs-upstream-bugs.md` with the PR link.

**What was verified, and how** (2026-10-04):
- *Hardware.* The probe image was run on the host processor through the
  Windows Hypervisor Platform: rusty_box's `rusty_box_whp_engine` test harness
  (`machine_running_on` + `FastMachine` in `fast_machine.rs`) loaded the 512
  bytes at 0x7C00 and stepped the partition, and read port 0xE9. The test was
  temporary and is deleted; the image was the one this `.S` builds. Hyper-V
  runs real-mode guests natively (unrestricted guest), so the RETF executes on
  the CPU — and the port's own interpreter, which copies Bochs, would have
  printed the Bochs answer, so the hardware answer cannot have come from
  emulation.
- *Upstream Bochs.* Master `f87c5e226` from a clean GitHub tarball, built with
  the configure line of the local `build-mingw` tree (`--enable-cpu-level=6
  --enable-x86-64 --enable-vmx=2 --enable-svm --enable-avx --enable-evex
  --enable-pci --enable-all-optimizations --enable-static-link
  --enable-show-ips --disable-readline --with-win32 --with-nogui`, MinGW
  g++ 15.1.0), run headless with `cpu: model=corei7_skylake_x`, then the patch
  applied and rebuilt, then reversed and rebuilt: the two outputs in the table
  below. The older local `build-mingw` binary (3.0.devel, 2026-07-13) prints
  the same unpatched answer.
- *rusty_box.* Its interpreter prints the unpatched Bochs answer — it
  reproduces the bug for parity (`cpu/ctrl_xfer16.rs` `retfar16_iw`). Whether
  the port takes the hardware behaviour is open; see the entry in
  `docs/bochs-upstream-bugs.md`.
- *A first probe variant was not enough.* With only ESP = 0x20000 the
  zero-extended add and a 16-bit SP update give the same ESP; Intel's real-mode
  far-return pseudocode is written as `SP := SP + (SRC AND FFFFH)`, so the
  second starting point (0x29000, where the add carries out of SP) is what
  shows the processor does a 32-bit add. Keep both rows in the PR.
- *Duplicate search:* none done yet. Before filing, search bochs-emu/Bochs
  issues and PRs for "RETF", "retfar16", "imm16", "Bit16s".

---

## RETF imm16 in real mode sign-extends the immediate on a 32-bit stack

### Summary

`BX_CPU_C::RETfar16_Iw` (`cpu/ctrl_xfer16.cc`) declares its immediate as
`Bit16s`. In real mode with a 32-bit stack — SS cached with B = 1, "big real
mode" — the release step `ESP += imm16` therefore sign-extends it, and
`RETF 0x8000` moves ESP down by 0x7FFC instead of up by 0x8000. The
immediate is a byte count; the processor zero-extends it. This PR makes it
`Bit16u`, like every other RET handler in Bochs.

### Reproduction

The attached `bochs-retf16-imm16-probe.S` is a 512-byte boot sector. It loads
SS from a B = 1 descriptor in protected mode, drops back to real mode (SS stays
32-bit), and executes `RETF 0x8000` and `RET 0x8000` with a 16-bit operand
size from two starting ESP values, printing ESP after each on port 0xE9 and on
the VGA text screen, then writes `Shutdown` to port 0x8900.

```
as -o probe.o bochs-retf16-imm16-probe.S
objcopy -O binary -j .text probe.o probe.img
# pad to 1474560 bytes for a 1.44 MB floppy, then:
```

```
cpu: model=corei7_skylake_x
megs: 32
romimage: file=bios/BIOS-bochs-latest
vgaromimage: file=bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin
floppya: 1_44=probe_floppy.img, status=inserted
boot: floppy
port_e9_hack: enabled=1
display_library: nogui
```

### Results

Expected ESP for each candidate behaviour, and what was observed:

| Instruction, start ESP | ESP + zext(imm16) | ESP + sext(imm16) | SP += imm16 only |
|---|---|---|---|
| RETF 0x8000, 0x00020000 | 0x00028004 | 0x00018004 | 0x00028004 |
| RETF 0x8000, 0x00029000 | 0x00031004 | 0x00021004 | 0x00021004 |
| RET 0x8000, 0x00020000 | 0x00028002 | 0x00018002 | 0x00028002 |
| RET 0x8000, 0x00029000 | 0x00031002 | 0x00021002 | 0x00021002 |

| Machine | Output |
|---|---|
| Intel Core i5-12450H (family 6, model 154, stepping 3), real hardware | `RETF 00028004 00031004 RET 00028002 00031002` |
| Bochs master `f87c5e226` | `RETF 00018004 00021004 RET 00028002 00031002` |
| Bochs master `f87c5e226` + this PR | `RETF 00028004 00031004 RET 00028002 00031002` |

The processor adds the zero-extended immediate to the full 32-bit ESP for
both instructions. Bochs agrees for near RET and sign-extends for far RET.

### What the architecture says

- AMD64 Architecture Programmer's Manual, vol. 3, RET (Far), real and
  virtual-8086 mode: "temp_IMM = word-sized immediate specified in the
  instruction, zero-extended to 64 bits", then `RSP.s = RSP + temp_IMM`.
  RET (Near) describes the immediate as the value "it adds to the rSP after it
  pops the target rIP".
- Intel SDM, RET: "Far return to calling procedure and pop imm16 bytes from
  stack"; "The optional source operand specifies the number of stack bytes to
  be released after the return address is popped". The real-address-mode far
  return pseudocode writes the release as `SP := SP + (SRC AND FFFFH)`, which
  assumes a 16-bit stack; the second row of the table shows the processor
  updates all of ESP when SS.B = 1, without sign extension.

### Root cause

```cpp
// cpu/ctrl_xfer16.cc  BX_CPU_C::RETfar16_Iw
Bit16s imm16 = (Bit16s) i->Iw();
...
  if (BX_CPU_THIS_PTR sregs[BX_SEG_REG_SS].cache.u.segment.d_b)
    ESP += imm16;      // sign-extended
  else
     SP += imm16;
```

Every other RET path reads the immediate unsigned: `RETnear16_Iw`,
`RETnear32_Iw` and `RETfar32_Iw` declare `Bit16u imm16`, `RETnear64_Iw` adds
`i->Iw()`, `RETfar64_Iw` passes it to `return_protected(bxInstruction_c *,
Bit16u pop_bytes)` — and so does the protected-mode branch of
`RETfar16_Iw` itself, converting the `Bit16s` back. The `Bit16s` is in the
original 2000 snapshot (`bochs-2000_0325a`) and was carried through the 2008
handler rework.

### Scope

Only the real-mode branch of `RETfar16_Iw` is affected, and only when SS is
32-bit and imm16 >= 0x8000. Virtual-8086 segments are always 16-bit, and the
16-bit-stack branch (`SP += imm16`) is the same either way.

### Other implementations

VirtualBox's IEM takes the count as `uint16_t cbPop` (`iemCImpl_retf`). QEMU's
TCG front end reads every RET/RETF immediate as `int16_t`
(`target/i386/tcg/emit.c.inc`, `gen_RET` / `gen_RETF`), so it sign-extends on
32-bit stacks too and is not a reference for this case.

### Environment

Windows 11 (10.0.26300), MinGW g++ 15.1.0, GNU binutils 2.44. The hardware
run used the Windows Hypervisor Platform, which executes real-mode guest code
natively on the CPU.
