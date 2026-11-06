# Semantic correctness

How Leanbox states that it computes what the Arm architecture says, and how
that is proved. `proofs.md` lists the theorems; this page is the method
they share. It is the method of CompCert and seL4: a specification, an
abstraction function, and a commuting square.

## Three levels, two refinements

```
  Spec        ArchState ──Spec.step──▶ ArchState      the Arm ARM, as written
                 ▲                         ▲
                abs                       abs
                 │                         │
  Reference      St ─────execInstr─────▶  St           Cpu.Step: every instruction, plainly
                 ▲                         ▲
                 │ (same state)            │
  Engine         St ────── run ─────────▶ St           Engine.Run: the same, fast
```

- **`Spec`** is the architecture as the Arm ARM states it. Registers are a
  function `Fin 31 → BitVec 64` with XZR reading zero, memory is a function
  from addresses to bytes, PSTATE and the system registers are plain fields,
  and each instruction's *Operation* pseudocode is a Lean function over
  `BitVec`. It is written to be read against the Arm ARM, not to be fast.
- **`abs : St → ArchState`** says which architectural state an emulator
  state stands for. The register file's 34 slots map to `X[0..30]` and SP,
  and the zero register and the discard slot map to nothing. RAM's
  `ByteArray` maps to the memory function. The micro-TLBs and the trace
  cache map to nothing: they are the engine's, not the architecture's.
- **An invariant `Inv : St → Prop`** is what every emulator state the
  machine reaches satisfies: the zero register's slot is zero, the
  register file has its size, the trace cache agrees with memory. Each L4
  and L6 type puts one of these into a type, and every invariant that lives
  in a type is one the proofs never have to carry.

The two refinements:
1. **The reference refines the specification**: for every instruction,
   `abs (execInstr i st) = Spec.step i (abs st)` when `Inv st` holds, and
   `execInstr` preserves `Inv`. One commuting square per instruction form.
2. **The engine refines the reference**: `run` reaches the state that
   folding `execInstr` over the trace reaches (L9). Composed with (1),
   every run of the engine is a run of the architecture.

## The first square, proved

In `docs/sketches/Refinement.lean`, checked on Lean 4.35 (2026-11-06) and kernel-checked: `#print axioms`
reports `propext` and `Quot.sound` only, so no `bv_decide`, no
`native_decide` and no `sorry`. The emulator's representation is the real
one: 34 slots, the zero register a slot whose value is zero by the type,
writes to XZR going to the discard slot.

```lean
structure Impl where
  rf : Vector UInt64 34
  zr : rf[32] = 0                       -- XZR reads zero: in the type, not in a test

def abs (s : Impl) : ArchState :=
  { x := fun i => s.rf[i.val].toBitVec, sp := s.rf[31].toBitVec }

/-- Running the emulator's ADD, then abstracting, is the Arm ARM's ADD. -/
theorem add_refines (d n m : SpecReg) (s : Impl) :
    abs (implAdd (Slot.ofSpecW d) (Slot.ofSpec n) (Slot.ofSpec m) _ s)
      = specAdd d n m (abs s)
```

The proof is a case split on the destination (a register or XZR), and then
the same three steps every square takes: unfold both sides, compare
register by register, and settle the index arithmetic with `omega`. That
regularity is what makes a few hundred squares a mechanical job: one
`simp` set (`@[arm_spec]`) and one tactic that does those steps, with each
instruction form's theorem a line that calls it.

## Where the architecture allows a choice

The Arm ARM does not always name one outcome, so the specification is a
relation where the architecture is one: `Spec.step i s s'` holds for every
`s'` the architecture allows. The emulator proves that its outcome is one
of them.
- **IMPLEMENTATION DEFINED** choices are a parameter record, `ImpDef`. The
  emulator proves refinement for the Cortex-A57's record, with each field
  cited to the A57 TRM (`cpp_orig/arm-docs/ddi0488/`).
- **CONSTRAINED UNPREDICTABLE** behaviours are the set the Arm ARM lists,
  and the emulator's choice (rusty_box's) is a member of it.
- **TLB caching**: the architecture lets a translation be used from a TLB
  until it is invalidated, even after the page tables change. The
  specification holds the set of translations a TLB may still hold, and the
  emulator's TLB is one allowed content of it. That is why a missing TLBI in
  a guest is a guest bug, and the specification says so.
- **The exclusive monitor** may be cleared for reasons the architecture
  leaves open, and the specification allows any of them.

## The machine: devices, interrupts, time

An instruction's specification acts on memory through an interface (read,
write, fetch), and what a device does with an access is the device's own
specification. Each device is refined separately: its implementation state,
its abstraction to its manual's state, its square per register access. The
bus composes them, so a GIC proof never has to mention the UART.

The whole machine is a labelled transition system: an instruction retires,
an interrupt is taken where PSTATE allows it, a device's deadline passes.
The architecture allows many interleavings. The emulator picks one
deterministically, at one tick per instruction, and its correctness is
**trace inclusion**: every sequence of events the emulator produces is one
the specification allows. The final statement is:

```lean
theorem leanbox_refines (m : Mach) (h : Inv m) (budget : Nat) :
    Spec.Allows (abs m) (events (drive m budget))
```

That is: every run of Leanbox, from any state it can reach, is a behaviour
of an Armv8.0-A processor on this board.

## The specification is the trusted part

Types and proofs show that the emulator agrees with the specification, so
the specification is the one thing taken on trust, and it is built to need
as little trust as possible:
- **Small and readable**, each function next to the Arm ARM pseudocode it
  transcribes, cited by symbol (`AddWithCarry`, `AArch64.TakeException`).
- **Executable**: `Spec.step` is a Lean function. It runs the
  single-instruction tests against rusty_box *by itself*, so a mistake in
  the specification is caught without the emulator. The emulator and its
  specification must then agree with each other and both with rusty_box.
- **Checked against Arm's data** (L17): its encodings against the A64 ISA
  XML. Its pseudocode is to be checked against Arm's ASL through Sail when
  Sail's Lean backend reaches Arm.

## What is tractable

| part | the squares | effort |
|---|---|---|
| integer data processing, branches | one per form, mechanical | small |
| loads and stores to RAM | one per form, over the memory lemmas (`ld64 (st64 m i v) i = v` and the frame lemma) | small |
| exceptions, ERET, PSTATE | a few, each with its PSTATE rules | medium |
| system registers | one per register, mechanical | medium |
| stage 1 translation | the walk against the Arm ARM's `AArch64.TranslateAddress`, the TLB as allowed caching | large |
| SIMD and floating point | needs a specification of IEEE 754 arithmetic as the Arm ARM does it (`FPAdd`, `FPRound`) over `BitVec` | large |
| each device | its registers against its manual | medium each |
| the engine (`run`) | needs `run` total (L11) and the trace-cache invariant | large |

The order follows the table: the small rows first, each proof landing with
the type work that makes it short (L4 before the register squares, L15's
`Instr` before the per-form theorems).
