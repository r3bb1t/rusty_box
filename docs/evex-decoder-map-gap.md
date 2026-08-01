# The EVEX opcode maps are the real blocker for advertising AVX-512

**Found 2026-08-01**, while verifying the Skylake-X CPUID flip against an
Ubuntu 26.04 live-server boot. The flip was reverted; this is what has to
land first.

## Symptom

With AVX512F/DQ/CD/BW/VL advertised, Ubuntu's kernel boots normally and then
panics a few seconds in:

```
exc_invalid_op+0x5d/0x80
Attempted to kill init! exit code=0x00000004
RIP: 0033:0x7b4e55d82b46
Code: ... f3 0f 1e fa 89 f8 <62> e2 7d 28 7a c6 25 ff 0f 00 00 ...
```

The faulting byte is `62` — the EVEX prefix. `62 E2 7D 28 7A C6` decodes as
**VPBROADCASTB ymm0, esi**, EVEX.256.66.0F38.W0 7A /r, which glibc's
AVX-512 `strlen`/`memchr` IFUNC emits. RIP is userspace, so this is the
dynamic loader or early init taking #UD on the first AVX-512 string routine
it selects.

## Root cause

`fetch_decode64` rejects that encoding outright with `BxIllegalOpcode`: the
opcode-map slot for EVEX.66.0F38.W0 7A is empty.

This is *not* a missing handler. `evex_vpbroadcastb_gpr` exists and is
dispatched. The gap is one layer earlier — nothing in the decoder tables
produces the opcode, so it never reaches the ISA gate or the dispatcher.

## Scale

    EVEX opcodes in the Opcode enum         1333
    ...referenced by a decoder table          268
    ...unreachable from the decoder          1065

Measured by matching every `Evex*` enum variant against the contents of
`rusty_box_decoder/src/decoder/*.rs`.

Note what this means for the earlier "all 898 Skylake-X EVEX opcodes are
dispatched" result: that was measured enum-to-dispatcher and is still true,
but it is not sufficient. An opcode can have a correct, tested handler and
still be unreachable because no decode path yields it. Any future
completeness claim about EVEX has to measure *decoder → dispatcher*, not
just enum → dispatcher.

## What has to happen

Build the EVEX opcode maps, the counterpart of Bochs's
`cpu/decoder/fetchdecode_evex.cc`. The def file
(`cpu/decoder/ia_opcodes_evex.def`) already carries every entry with its
map, prefix, W bit, ISA feature and operand form, and `evex_scope.py` in the
session scratchpad already parses it — so the tables can be generated rather
than hand-written, the same way `scripts/gen_opcode_isa.py` generates the
ISA gate.

Until then the CPUID flip must stay reverted: advertising AVX-512 without
the decode paths turns a working Ubuntu boot into a kernel panic.

## Repro

`rusty_box_decoder`'s `evex_vpbroadcastb_from_gpr_decodes` is the exact
one-instruction repro, marked `#[ignore]` with a pointer here. Un-ignore it
when the maps land; it should pass.
