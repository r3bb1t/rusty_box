# Upstream PR — NOT YET FILED (prepared 2026-10-09)

Everything after the `---` below is the PR body, ready to paste. The notes above
it are for whoever files it and are not part of the PR.

**Files that go with it** (all in `docs/`):
- `bochs-vex-shift-imm-memory-fix.patch` — the fix as a `git am`-ready patch.
  - Author is `r3bb1t`, the identity configured in the `cpp_orig/bochs`
    checkout.
  - It touches two files: twenty `ATTR_MODC0` additions in
    `bochs/cpu/decoder/fetchdecode_opmap_avx.cc`, and a `bochs/cpu/HISTORY`
    line in upstream's own style.
  - Checked with `git apply --check` against upstream master `f87c5e226`
    (2026-10-03).
  - Drop the HISTORY hunk if the maintainer prefers to write HISTORY himself.
- `bochs-vex-shift-imm-memory-probe.S` — the reproduction, a 512-byte boot
  sector built with GNU binutils alone. Attach it to the PR, or paste it into a
  collapsed section.

**How to file** (on the day):
1. In the fork checkout, branch from current upstream master. Re-run
   `git apply --check` first, because upstream moves daily. The hunks are
   anchored on the three group arrays, not on line numbers, so an offset is
   harmless.
2. Run `git am docs/bochs-vex-shift-imm-memory-fix.patch`, adjusting the commit
   message or the Date header if wanted. Push the branch to `r3bb1t/Bochs`, then
   open the PR against `bochs-emu/Bochs` master with the body below.
3. Update the "Filing status" of the entry in `docs/bochs-upstream-bugs.md`
   with the PR link.

**What was verified, and how** (2026-10-09):
- **Hardware.**
  - The probe image ran on the host processor through the Windows Hypervisor
    Platform. rusty_box's `rusty_box_whp_engine` test harness
    (`machine_running_on` + `FastMachine` in `fast_machine.rs`) loaded the 512
    bytes at 0x7C00, stepped the partition, and read port 0xE9. The test was
    temporary and has been deleted; the image was the one this `.S` builds.
  - The guest's instructions execute on the CPU. The engine intercepts only
    #GP (`trapped_exceptions`), so the #UD reached the guest's own IDT from the
    processor.
  - The register-form controls (`R`, `YR`) retire, which shows that AVX and
    AVX2 state were enabled in the partition.
- **Upstream Bochs.**
  - Master `f87c5e226`, built with `--enable-cpu-level=6 --enable-x86-64
    --enable-vmx=2 --enable-svm --enable-avx --enable-evex --enable-pci
    --enable-cdrom --enable-all-optimizations --enable-static-link
    --enable-show-ips --disable-readline --with-win32 --with-nogui
    --enable-debugger` (MinGW). It ran headless with
    `cpu: model=corei7_skylake_x`, `port_e9_hack: enabled=1` and
    `display_library: nogui`, booting the probe from a 1.44 MB floppy image.
  - Then a copy of the same tree was built with the patch applied. Only
    `fetchdecode_opmap_avx.o` recompiled. Both outputs are in the table below.
- **rusty_box.** Its interpreter already raises #UD for these forms, because
  its legacy SSE table row is register-only and VEX shares it. It prints the
  hardware row. The port registers this as divergence D19 in
  `docs/bochs-parity-divergences.md`.
- **Scope check.**
  - Only the VEX groups change. The EVEX forms (`BxOpcodeGroup_EVEX_0F71/72/73`)
    legitimately take `xmm2/m128` and are untouched.
  - The legacy SSE groups already carry `ATTR_MODC0`.
- **Duplicate search:** none done yet. Before filing, search the
  bochs-emu/Bochs issues and PRs for "VPSRLW", "UdqIb", "shift immediate",
  "ATTR_MODC0" and "0F71".

---

## VEX shift-by-immediate instructions must #UD with a memory operand

### Summary

The VEX opcode groups for `66 0F 71`, `66 0F 72` and `66 0F 73`
(`BxOpcodeGroup_VEX_0F71`, `_0F72`, `_0F73` in
`cpu/decoder/fetchdecode_opmap_avx.cc`) carry no `ATTR_MODC0`. Bochs therefore
decodes and executes a memory form of VPSRLW, VPSRAW, VPSLLW, VPSRLD, VPSRAD,
VPSLLD, VPSRLQ, VPSRLDQ, VPSLLQ and VPSLLDQ: it reads 16 or 32 bytes and shifts
them. The processor raises #UD for every one of these encodings. This PR adds
`ATTR_MODC0` to the twenty rows.

### Root cause

```cpp
// cpu/decoder/fetchdecode_opmap_avx.cc
static const Bit64u BxOpcodeGroup_VEX_0F71[] = {
  form_opcode(ATTR_SSE_PREFIX_66 | ATTR_NNN2 | ATTR_VL128, BX_IA_V128_VPSRLW_UdqIb),
  ...                                     // no ATTR_MODC0 on any row
```

The operand is declared `OP_Wdq` and the memory handler is `LOADU_Wdq` /
`LOAD_Vector` (`ia_opcodes.def`, `BX_IA_V128_VPSRLW_UdqIb` and its siblings),
so the decoder admits a memory operand, and the handler reads one.

Bochs's own legacy table already gets this right. `BxOpcodeTable0F71/72/73` in
`fetchdecode_opmap.h` mark the SSE forms `ATTR_MODC0`:

```cpp
form_opcode(ATTR_NNN2 | ATTR_SSE_PREFIX_66 | ATTR_MODC0, BX_IA_PSRLW_UdqIb),
```

The opcodes' own names say `Udq`, which in Intel's operand notation is a vector
register selected by ModRM.r/m, never memory.

### Architecture

The SDM lists the VEX forms with a register operand only, for example
`VEX.128.66.0F.WIG 71 /2 ib  VPSRLW xmm1, xmm2, imm8` and
`VEX.256.66.0F.WIG 71 /2 ib  VPSRLW ymm1, ymm2, imm8`. Only the EVEX forms take
`xmm2/m128`, and Bochs decodes those through separate EVEX groups, which this PR
does not touch.

### Measured

A boot-sector probe enters 32-bit protected mode and enables SSE and AVX state
(CR4.OSFXSR, OSXMMEXCPT and OSXSAVE; XCR0 = 7). It then runs each instruction
with #UD, #NM, #GP and #PF handlers installed. `+` means the instruction
retired; `#U` means #UD. Every memory operand is `[0x6000]`, which is ordinary
RAM.

| Test | Encoding | Intel i5-12450H | Bochs `f87c5e226` | Bochs + this PR |
|---|---|---|---|---|
| `R` VEX.128 VPSRLW xmm1, xmm2, 4 | `C5 F1 71 D2 04` | + | + | + |
| `S` PSRLW xmm, [mem], 4 (legacy) | `66 0F 71 15 m32 04` | #U | #U | #U |
| `71` VEX.128 VPSRLW xmm1, [mem], 4 | `C5 F1 71 15 m32 04` | #U | **+** | #U |
| `72` VEX.128 VPSRLD xmm1, [mem], 4 | `C5 F1 72 15 m32 04` | #U | **+** | #U |
| `73/2` VEX.128 VPSRLQ xmm1, [mem], 4 | `C5 F1 73 15 m32 04` | #U | **+** | #U |
| `73/3` VEX.128 VPSRLDQ xmm1, [mem], 4 | `C5 F1 73 1D m32 04` | #U | **+** | #U |
| `YR` VEX.256 VPSRLW ymm1, ymm2, 4 | `C5 F5 71 D2 04` | + | + | + |
| `Y71` VEX.256 VPSRLW ymm1, [mem], 4 | `C5 F5 71 15 m32 04` | #U | **+** | #U |

The hardware is an Intel Core i5-12450H (family 6, model 154, stepping 3),
reached through the Windows Hypervisor Platform, which runs the guest's code on
the processor. Bochs ran with `cpu: model=corei7_skylake_x`.

### Fix

The patch adds `| ATTR_MODC0` to every row of `BxOpcodeGroup_VEX_0F71`, `_0F72`
and `_0F73`: twenty rows, VL128 and VL256. It also adds a HISTORY line. A memory
form then finds no row and decodes as #UD, as the legacy forms already do.

### Reproduction

The attached `bochs-vex-shift-imm-memory-probe.S` is a 512-byte boot sector:

```
as -o probe.o bochs-vex-shift-imm-memory-probe.S
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

It prints one line on port 0xE9, then writes `Shutdown` to port 0x8900.
