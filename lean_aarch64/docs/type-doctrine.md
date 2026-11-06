# The type doctrine

Leanbox is written so that a wrong program is a program Lean rejects. A
test shows that one run went right; a type shows that no run can go wrong
in that way. These rules say how, adapted to dependent types from
llvmkit's type-safety doctrine (D1–D11) and rusty_box's safety doctrine
(R0–R9). Cite them by id (`L0`–`L18`) in commits and reviews.

The bar is adversarial: a guarantee holds against someone trying to break
it, not only against mistakes. So a guarantee rests on something Lean's
kernel checks (a proof field, an index), never on a convention (a private
constructor, a naming rule, a comment), and the few ways to forge a proof
are themselves detected mechanically (L0).

The rules cost nothing at run time, and that is checked, not assumed (L13):
Lean erases proofs, compiles an enumeration to one byte, and compiles a
structure whose only non-proof field is a machine word to that word.

## The trusted base

**L0. The trusted base is audited, not trusted.** A proof field can be
forged only through an escape hatch. Every one of them is visible in the
elaborated environment, so `lake test` scans the environment of every
`Leanbox` declaration, not the source text, and fails on anything outside
this list:
- **Axioms** (`collectAxioms` of each declaration): only `propext`,
  `Classical.choice` and `Quot.sound`. This shuts out `sorry` (`sorryAx`),
  any user `axiom`, and every proof checked by running compiled code
  instead of by the kernel: `native_decide` (`Lean.ofReduceBool`), and
  `bv_decide`, which checks its certificate natively and adds one
  `_native.bv_decide.ax_*` axiom per proof.
- **Code that replaces its definition**: no `unsafe` declaration, no
  `@[implemented_by]` and no `@[extern]`. A `@[csimp]` replacement is
  allowed, because it carries a kernel-checked proof of equality.
- **Opaque definitions**: exactly the named `partial` list of L11. A
  `partial def` is an `opaque` constant with an `_unsafe_rec` twin, so a
  new one is caught by name.

The environment scan cannot see `set_option debug.skipKernelTC`, which
admits a declaration the kernel never checked, so the build's `.olean`s are
also replayed through the kernel (leanprover's `lean4checker`). The audit
itself is locked (L12): a fixture with a `sorry`, an `axiom`, a
`native_decide` and an `implemented_by` must each make it fail.

A measured gap (an environment scan with `collectAxioms`, 2026-11-06): the
proofs `Isa.Alu.clz_eq_clzFast`, `rbit_eq_rbitFast` and `rev16`/`rev32`/
`rev64_eq_…Fast` use `bv_decide`, so each depends on its native axioms. They
are proofs that trust Lean's compiler, not its kernel, and L0 requires them
reproven by kernel-checked reasoning. Nothing else in the library uses an
axiom outside the three, and there are no `unsafe`, `implemented_by` or
`extern` declarations. The opaque declarations are exactly `run`, `drive`
and `checkLoop`.

**L14. Invariants live in proofs and indices, never in privacy.** Lean's
`private` is a namespacing tool, not a boundary: anonymous constructor
notation, `{ .. }`, `by constructor` and the generated `casesOn` all reach
past a private constructor or field (de Moura and others on the Lean
Zulip). A smart constructor is a convenience, not a guarantee. The type
itself must hold the fact, so anyone may build the value directly, but only
by proving the invariant.

## The rules

**L1. States are types.** Every closed set of states is an inductive type,
and data that exists in only one state lives in that constructor. A
processor that has stopped carries why; a running one has no "why" field
to read.

```lean
inductive Power where
  | running | off | cpuOff | reset
  | unmodelled (why : String)       -- the only state with a reason
inductive Activity where | running | waitInterrupt | waitEvent
inductive Flow where                -- how a step hands back to the loop
  | next | branch | newContext | toLoop
inductive Fault where
  | exception (ec : Ec) (iss : Iss ec)   -- the ISS's shape depends on the class
  | stop (s : Stop)
```

**L2. Privilege is evidence.** A handler defined only in some processor
state takes the proof of that state as an argument. The dispatcher is the
one place that decides it, with `if h : …`, and code that has not decided
it cannot call the handler.

```lean
def eret (st : St) (h : st.sys.el = .el1) : St := …
def simdOp (st : St) (h : fpEnabled st.sys) (op : SimdOp) : St := …
def SysReg.write (r : SysReg) (st : St) (h : r.minEl ≤ st.sys.el) : … := …

-- the dispatcher, and only the dispatcher:
| .eret => if h : st.sys.el = .el1 then eret st h else raise st pc undefinedF
```

**L3. Widths are indices.** A value knows its width, and a 32-bit value
carries the Arm rule that a W-register write zeroes bits 63:32. Every
operation keeps the width of its operands, and the only way back to a raw
word is the explicit projection `.bits`.

```lean
inductive Width where | w32 | w64
structure Val (w : Width) where
  bits : UInt64
  fits : bits.toNat < 2 ^ w.bits
def add : Val w → Val w → Val w
```

Access sizes (`Size := b | h | w | x | q`) and SIMD element shapes
(`Lanes esize`) follow the same rule.

**L4. Operands mean what they say.** A register operand is a slot whose
type says what it may name. A read names X0–X30, SP or XZR and never the
discard slot. A write names X0–X30, SP or the discard slot and never XZR.
The register file's size is in its type, so an access needs no bounds
check. The decoder is the only place that makes a slot.

```lean
structure RSlot where i : UInt8; ok : i.toNat < 33
structure WSlot where i : UInt8; ok : i.toNat < 34 ∧ i ≠ ZR
structure RegFile where bytes : ByteArray; size : bytes.size = RF_BYTES
def RegFile.write (rf : RegFile) (r : WSlot) (v : Val w) : RegFile
```

**L5. Units are types.** A virtual address, a physical address, a RAM
offset, a page number, a count of retired instructions (`Ticks`) and a
value of the system counter (`Count`) are distinct structures over
`UInt64`. Each conversion is a named function: `Ticks.toCount` is
timer.rs `counter_at`, and `translate` is the only path from `VA` to `PA`.
Passing one where the other is meant is a type error (rusty_box R4).

**L6. A check returns its witness.** A check whose answer later code relies
on returns evidence, not a `Bool`:
- `translate` returns `Granted va kind el`: the physical address, with the
  proof that the entry's permissions allow `kind` at `el`.
- The data micro-TLB holds only `Granted` pages.
- The RAM fast path takes `InRam pa bytes` and `Aligned pa bytes`.
- A `Trace` carries the proof that it ends in `endTrace` and holds at most
  33 ops, so `run` indexes it with a position proven in range and has no
  "past the end" branch.

**L7. Matches over closed sets are exhaustive.** No `_ =>` over `Op`,
`El`, `Flow`, `Fault` or `Power`, so a new constructor breaks every
consumer until each says what it does. The hot loop has the one named
exception: its default arm is the reference step (`slowpath!`), which runs
every op completely.

**L8. One choke point per hazard.**
- Every store to RAM goes through the one function that checks the trace
  cache's code marks (a store into code drops its traces).
- Every register write goes through `RegFile.write`.
- Every exception entry goes through `takeException`.
- Every change of translation context goes through `Flow.newContext`,
  which empties the micro-TLBs.

**L9. The reference is the meaning; the engine refines it.**
`Cpu.Step.execOp` defines what an instruction does. Every fast path in
`Engine` either is proven equal to it (a `theorem`), or is checked against
it in lockstep: whole boots, and random single instructions against
rusty_box. The goal is a stated `Refines` proposition for `run`, proven
op family by op family once `run` is total (L11).

**L10. Specifications are propositions over `BitVec`.** `Spec/` writes the
Arm ARM's pseudocode (`AddWithCarry`, `ShiftReg`, `DecodeBitMasks`, the
CRC and SHA functions) over `BitVec n`, the way the Arm ARM states it. The
`UInt64` code is proven to agree (`bv_decide`, `simp`). A fast replacement
is installed only with a proof (`@[csimp]`, as CLZ, RBIT and REV are).

**L11. Total by default.** The project has no `sorry`, `unsafe`,
`@[extern]`, `implemented_by`, `panic!`, `get!` or `!`-indexing. `partial`
is allowed only on a named list, each entry with the measure that will make
it total. Today that list is `run` (fuel: `lim - icnt`), `drive` (the
instruction budget) and `checkLoop` (the checkpoint count).

**L12. Every rejection is locked.** Each rule's guarantee has a
compile-fail lock in `LeanboxTest/Locks/`: a program the rule must reject,
inside `#guard_msgs`, with its error pinned. If a change weakens a rule
until the forbidden program compiles, `lake test` fails. Tests are ported,
not invented: a `#guard`'s expected value comes from a rusty_box unit test
or an Arm document, cited by symbol.

A lock comes with an accepted twin, a program that differs from it only in
the forbidden part and compiles. Without the twin, a lock can pass for an
unrelated error (a typo, a wrong return type) and prove nothing about the
rule.

```lean
/-- error: Application type mismatch: The argument
  rfl
has type
  ?m.2 = ?m.2
but is expected to have type
  s.el = El.el1
in the application
  eret s ⋯ -/
#guard_msgs in
example (s : Sys) : Sys := eret s rfl     -- ERET with no proof of EL1

/-- error: Tactic `decide` proved that the proposition
  32 ≠ 32
is false -/
#guard_msgs in
example : WSlot := ⟨32, by decide, by decide⟩   -- a write that names XZR
```

**L13. Types are free, and that is measured.** A type that adds work at run
time is a regression. Each change that adds types is gated twice: both boots
and the single-instruction tests stay byte-identical, and callgrind's host
instructions over Alpine's first 40M instructions do not rise. What the
compiler does with these types, from `lean -c` on Lean 4.35.0 (the sketch
behind the examples above, 2026-11-06):

| Lean | C |
|---|---|
| `Val .w32`, `Val .w64` | `uint64_t` |
| `WSlot` (a `UInt8` and two proofs) | `uint8_t` |
| `RegFile` (a `ByteArray` and a proof) | the `ByteArray` itself |
| `structure Sys where el : El` | `uint8_t` |
| `RegFile.write` with the bound proven | `lean_byte_array_fset` (no bounds check) |
| a proof argument of a function that is not inlined | one placeholder word (`lean_box(0)`) |

## Instructions

**L15. Instructions are intrinsically typed.** The interpreter's types are
its safety statement (intrinsically typed interpreters: Rouvoet, Poulsen,
Krebbers and Visser), so a decoded instruction is a value that can only
describe an instruction the Arm ARM defines. `Instr` nests the way the Arm
ARM's decode tree does (encoding group, class, instruction), and each field's
type may depend on the fields before it:

```lean
inductive Instr where                      -- Arm ARM C4.1: the encoding groups
  | dpImm (i : DpImm) | dpReg (i : DpReg) | branch (i : Branch)
  | ldst (i : Ldst) | system (i : System) | simd (i : Simd)

inductive DpImm where                      -- C4.1.2: its classes
  | addSub  (w : Width) (op : AddSub) (s : SetFlags)
            (d : DestOf s) (n : SpSlot) (imm : Imm12)
  | logical (w : Width) (op : Logic) (d : WSlot) (n : RSlot) (mask : BitMask w)
  | movWide (w : Width) (op : MovOp) (d : WSlot) (imm : UInt16) (hw : Fin w.hwCount)
```

- `hw : Fin w.hwCount`: a 32-bit MOVZ with `hw = 2`, UNDEFINED in the Arm
  ARM, cannot be built.
- `d : DestOf s`: register 31 is SP for `ADD (immediate)` and XZR for
  `ADDS`, so `Rd`'s type is computed from the S bit.
- `mask : BitMask w`: a logical immediate exists only as `DecodeBitMasks`'s
  accepted result, so it carries that witness.
- A pair load's two registers carry the proof that they differ (the Arm ARM
  makes them CONSTRAINED UNPREDICTABLE when equal), and an offset is scaled
  by its access size in its type.

Decoding is total: `decode : UInt32 → Decoded`, where
`Decoded := instr (i : Instr) | undefined (u : Undef)`, and every one of
the 2³² words lands in exactly one case. Encoding is its inverse,
`encode : Instr → UInt32`, with `decode (encode i) = .instr i` proved: a
field read from the wrong bits breaks the round trip.

The engine does not run `Instr`. Each level of nesting that has fields is
its own heap object, one dependent load per level, on the path every guest
instruction takes (Lean 4.35's C, 2026-11-06). A level with no fields costs
nothing: `op : AddSub` is a byte inside its parent. So the engine runs a
flat `Op`, with the hot split forms flat and one case `instr (i : Instr)`
for everything else, produced by `Op.lower : Instr → Op`. The theorem
`execOp (lower i) = execInstr i` ties the two together. Both sides are
total functions, so it is proved form by form, ahead of the `run`
refinement of L9.

**L16. Transitions are indexed by the states they connect.** A change of
processor state is typed by the states before and after it, after Brady's
indexed state machines (Idris, *Type-Driven Development*, chapters 13–14):

```lean
def takeException (st : St) (h : st.sys.el ≤ target) : { s : St // s.sys.el = target }
def exceptionReturn (st : St) (h : st.sys.el = .el1) : ReturnOutcome st
```

When the guest decides the result, the result's type says which case
happened. A return either lands in a legal state, with the proof, or sets
PSTATE.IL, with the reason. A caller cannot treat the second as the first.

**L17. The specification is Arm's, not ours.** Types stop a program from
contradicting its specification, not a specification from being wrong. So
the specification comes from Arm wherever Arm publishes one in a form that
can be checked:
- `Instr`'s field positions and its UNDEFINED and UNPREDICTABLE cases are
  checked against the Arm ARM's machine-readable encodings (the A64 ISA XML
  and AARCHMRS in `cpp_orig/arm-docs/`).
- Each `#guard`'s expected value comes from rusty_box's unit tests or an Arm
  document, never from Leanbox's own output.

The horizon is Arm's own pseudocode in Lean. Sail already has Armv8.5-A and
Armv9.3-A models generated from Arm's ASL, and work on a Lean backend for
Sail was presented in 2025, RISC-V first. With it, `Cpu.Step` would be
proved to refine the ASL itself.

## Hooks

**L18. A hook's condition is its type.** An instrumentation hook (after
rusty_box's `Instrumentation`, whose empty hook mask makes the processor
skip every dispatch) names what it wants with a `Selector`: a mnemonic, an
encoding group or class, a PC range, an exception level, an access kind, and
their conjunctions, disjunctions and negations. The selector computes the
type the hook receives, and the hook gets a proof that its condition holds,
so it cannot be handed an instruction its selector rules out. A hook on LDR
receives the LDR's fields, not an `Instr` it would have to take apart.

```lean
def Selector.Matches : Selector → Instr → Prop
def Selector.Payload : Selector → Type          -- .mnemonic .ldr ↦ LdrFields
def Selector.payload : (s : Selector) → (i : Instr) → s.Matches i → s.Payload
  -- total: every instruction the selector rules out is eliminated by the proof

structure Hook where
  sel : Selector
  run : sel.Payload → View → Event

inductive Op where                              -- decided when the trace is built
  | …
  | hooked (h : Hook) (i : Instr) (ok : h.sel.Matches i)
```

The lock and its twin, checked on Lean 4.35 (2026-11-06):

```lean
-- rejected: "fun f x => toString f.d has type AddFields → UInt64 → String
-- but is expected to have type (Selector.mnemonic Mnemonic.ldr).Payload → …"
#guard_msgs (drop info) in
example : Hook := { sel := .mnemonic .ldr, run := fun (f : AddFields) _ => toString f.d }
-- accepted
example : Hook := { sel := .mnemonic .ldr, run := fun (f : LdrFields) _ => toString f.t }
```

How hooks run:
- **Selectivity is paid once.** The parts of a selector fixed by the
  instruction word and its address (mnemonic, class, PC range) are decided
  when the trace is built, and a matching instruction is lowered to
  `Op.hooked` with its proof. With no hooks installed no trace holds a
  `hooked` op, so the engine runs exactly as without hooks (L13 measures
  it). The parts that depend on the machine state (the exception level, a
  register's value, the address of an access) are checked only when a
  hooked op runs.
- **A hooked op runs the reference step.** `Op.hooked` goes to the slow
  path, so a hook sees `execOp`'s view of the instruction, before and after.
- **Observing is not intervening.** An observing hook receives a read-only
  `View` and returns an `Event`, and its type gives it no machine state to
  return. That makes the theorem that observing hooks leave the machine's
  run unchanged a matter of definition. An intervening hook (setting a
  register, injecting a fault) returns a state, and the machine it runs on
  is `Mach .instrumented`. Lockstep's checks take only a `Mach .exact`, so
  an altered run cannot be reported as matching rusty_box (llvmkit D8's
  verified typestate).
- **The hook set indexes the trace cache.** The decode-time decisions belong
  to one hook set, so the cache is `TraceCache hooks`. Installing a
  different set gives a cache of a different type, and a stale trace cannot
  be run under it.

## What stays a runtime check

The rules make wrong programs unwritable; they cannot make the guest's
choices static. These are decided while the machine runs, and each decision
yields a witness (L6):
- The guest's page tables decide a translation, which yields `Granted` or a
  fault.
- The guest's registers decide its exception level, which the dispatcher
  turns into the L2 proof.
- Lean has no linear types, so whether an array is shared is checked when
  it is written (the exclusivity check behind every `ByteArray` store;
  `bytearray-word-access.md`).

## Claims

A number or an "every/none" claim in these documents names the command that
produced it and the date it was measured at, and a claim that something is
absent names its search (llvmkit's rule). Re-derive before quoting.

## Order of work

Each step is one commit, gated by L13:
0. **L0**: the environment audit in `lake test`, landing together with
   kernel-checked proofs of CLZ, RBIT and REV, so the audit passes the day
   it starts.
1. **L1 and L7**: the state codes become inductive types, starting with
   `Power`, `Activity`, `Flow` and `Fault`.
2. **L4**: slots and `RegFile`.
3. **L3 and L10**: `Val w` through `Isa.Alu`, with `Spec/` for the ALU and
   the flags.
4. **L15**: `Instr` along the Arm ARM's decode tree, with total `decode`,
   `encode` and the round trip; `Op.lower` with its theorem; and the
   encodings checked against the A64 ISA XML (L17).
5. **L2 and L16**: privilege evidence and indexed transitions in
   `Cpu.System`, `Cpu.Sysreg` and `Cpu.Exception`, and system registers as
   an enumeration with their lowest exception level.
6. **L5**: `VA`, `PA`, `Ticks` and `Count` through `Cpu.Mmu`, `Cpu.Memory`
   and `Cpu.Timer`.
7. **L6**: `Granted`, `InRam` and `Trace`.
8. **L11**: `run`, `drive` and `checkLoop` made total.
9. **L18**: selectors, typed payloads, `Op.hooked`, observing and
   intervening hooks, and the hook-indexed trace cache.

L12's locks land with the rule they lock, in the same commit.
