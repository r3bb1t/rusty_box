# Word access to a `ByteArray`: a proposal for Lean core

The emulator is pure Lean: no `@[extern]`, `implemented_by`, `unsafe` or
`sorry` in the project, so everything it runs is either proved or part of
Lean's own trusted runtime. This note records the one cost that pure Lean
cannot remove from inside the project, and what would remove it upstream.

## The cost

`ByteArray` (`Init/Prelude.lean`) is `structure ByteArray where data : Array UInt8`.
Each operation is a Lean definition, which proofs see, and a C primitive of
Lean's runtime, which compiled code runs (`Init/Data/ByteArray/Basic.lean`):

```lean
@[extern "lean_byte_array_uset"]
def uset : (a : ByteArray) → (i : USize) → UInt8 → (h : i.toNat < a.size) → ByteArray
  | ⟨bs⟩, i, v, h => ⟨bs.uset i v h⟩
```

A write keeps value semantics by checking that the array is not shared
(reference count 1) and copying it first if it is (Ullrich and de Moura,
*Counting Immutable Beans*). Lean has no write wider than one byte, so a
64-bit store (`st64` in `Leanbox/Bytes.lean`, used by every register write
and every guest store) is eight writes, each after its own check. The C
compiler cannot merge them: each byte store may alias the reference count
it checked. In `run` that is 1,207 cold copy paths, a large part of why
`run` is about 41,600 machine instructions and why about 32% of what it
executes is stack-frame traffic (Alpine's first 40M instructions, callgrind,
2026-11-06: 191 host instructions per guest instruction, rusty_box 99).

Reads are already one bounds check: `ld64` proves its eight byte indices
from one `ByteArray.WordIn` check, and clang joins the eight loads into one.

Lean 4.35 has `ByteArray.toUInt64LE!`/`toUInt64BE!` (`Init/Data/ByteArray/Extra.lean`),
which read a whole array of exactly 8 bytes and panic otherwise; nothing
reads or writes a word at an offset of a larger array. No issue or pull
request asking for one was found (2026-11-06).

## The proposal

A pair of primitives in the shape Lean already uses for `uset`: the Lean
definition is the byte-by-byte model, the C side checks exclusivity once and
does one 8-byte store.

```lean
@[extern "lean_byte_array_uset_u64le"]
def ByteArray.usetUInt64LE (a : ByteArray) (i : USize) (v : UInt64)
    (h : i.toNat + 8 ≤ a.size) : ByteArray :=
  -- eight `uset`s of v's bytes, low byte at i (the model `st64` is today)

@[extern "lean_byte_array_uget_u64le"]
def ByteArray.ugetUInt64LE (a : @& ByteArray) (i : USize)
    (h : i.toNat + 8 ≤ a.size) : UInt64 :=
  -- the eight bytes at i, low byte first (the model `ld64` is today)
```

with the lemmas a user needs: `(a.usetUInt64LE i v h).size = a.size`,
reading back the word written, and a byte outside `[i, i+8)` unchanged.
Probably also the 16- and 32-bit forms, and big-endian ones.

Nothing provable changes: the definitions are the models the emulator's
proofs use now, and the C joins the runtime Lean's maintainers already
review. Once released, `ld64`/`st64` (and the 16/32-bit forms) in
`Bytes.lean` call them, and the project stays pure Lean.

## The way there

1. Measure the gain first, with a throwaway experiment outside the
   repository (the primitive added to a copy of the runtime header, never
   committed), so the proposal carries a number. An 8-byte read helper
   measured that way earlier reached 268 host instructions per guest
   instruction against 300 for the proven reads of the time; stores have
   not been measured.
2. Post the case on the Lean Zulip (#lean4): the measured cost, the
   signature, the lemmas.
3. Open an issue on `leanprover/lean4`; write the pull request if the
   maintainers agree. New runtime primitives are added with care, so
   discussion comes before code.

## Pure-Lean work that does not wait on it

- **Invariants in types**: the register file as
  `{ b : ByteArray // b.size = RF_BYTES }` (`Isa/Op.lean`) and register
  slots a `Fin`, so the
  remaining bounds check on each register access becomes an erased proof.
- **Less loop state in `run`**: the instruction count as the trace's start
  count plus the op index rather than a separate loop parameter.
- **Longer or linked traces**, so fewer ops pass through `endTrace`.
- **`run` total rather than `partial`** (with `drive` in `Engine/Machine.lean` and
  `checkLoop` in `Lockstep.lean`): a `partial` function is opaque to proofs, which limits
  verification more than anything above.
