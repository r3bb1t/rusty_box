# Hooks: conditions as types, witnesses as arguments

A hook runs when a condition on the machine holds, and receives what the
condition found. The condition is a value of a small language (`Cond`).
Its meaning is a proposition over the architectural state. The type of what
the hook receives is computed from the condition *and the state it holds
in*. The check that decides it is certified: it can only return evidence
that is true, and it is proven to return it whenever it exists.

Everything below except the closing list of further conditions is in
`docs/sketches/Hooks.lean`,
checked on Lean 4.35 (2026-11-06) by `lean docs/sketches/Hooks.lean`. `#print axioms` gives
`propext` for `scan` and `scan_complete`, and nothing for `CStrAt.unique`.

## One example, all the way through

*A hook that runs only in user mode, when some register points to a C
string, and gets the register and the string.*

```lean
def userString : Hook where
  cond := .and .userMode (.someRegPointsToCStr 256)
  run  := fun _ (_, ⟨r, ⟨bs, _⟩⟩) => s!"x{r.val} -> {String.fromUTF8! ⟨bs.toArray⟩}"
```

On a state at EL0 with `x3 = 0x1000` and `"hi"` at 0x1000 it gives
`some "x3 -> hi"`. The same state at EL1 gives `none`.

The hook does not search registers or read memory. The check already did
both, and what it found is the hook's argument: the register's index and
the bytes, nothing re-read. The proofs inside the witness are erased when
compiled, so what the hook is passed at run time is the register index and
the byte list.

## The pieces

**The condition language** names what a hook waits for:

```lean
inductive Cond where
  | userMode
  | regIs (r : Fin 31) (v : UInt64)
  | regPointsToCStr (r : Fin 31) (max : Nat)
  | someRegPointsToCStr (max : Nat)
  | and (p q : Cond)
  | or (p q : Cond)
```

**The witness type depends on the condition and the state**:

```lean
def Cond.Witness : Cond → State → Type
  | .userMode, s => PLift (s.el = .el0)                       -- a proof
  | .regIs r v, s => PLift (s.x r = v)
  | .regPointsToCStr r max, s => CStr s (s.x r) max          -- the string, with its proof
  | .someRegPointsToCStr max, s => (r : Fin 31) × CStr s (s.x r) max   -- which register, and its string
  | .and p q, s => p.Witness s × q.Witness s                  -- both
  | .or p q, s => p.Witness s ⊕ q.Witness s                   -- which one held
```

So `and` gives a pair, `or` says which side held, and "some register" gives
the register that matched, a dependent pair whose second half's type
mentions the first.

**What "is a C string" means** is an inductive proposition: every byte
readable through the current translation, none of them NUL, a NUL after
the last.

```lean
inductive CStrAt (s : State) : UInt64 → List UInt8 → Prop
  | nil  : s.read a = some 0 → CStrAt s a []
  | cons : s.read a = some b → b ≠ 0 → CStrAt s (a + 1) bs → CStrAt s a (b :: bs)

abbrev CStr (s : State) (a : UInt64) (max : Nat) :=
  { bs : List UInt8 // CStrAt s a bs ∧ bs.length < max }
```

**The certified check** returns `Option (p.Witness s)`, so `some` cannot be
returned without the evidence, and a wrong scan does not type-check:

```lean
def scan (s : State) : (max : Nat) → (a : UInt64) → Option (CStr s a max)
def Cond.check : (p : Cond) → (s : State) → Option (p.Witness s)
```

**Proved about it**, so that "the hook fired" and "the condition held" are
the same event:
- *sound*, by its type: every `some` carries its proof;
- *complete*: `scan_complete : CStrAt s a bs → bs.length < max → ∃ c, scan s max a = some c ∧ c.val = bs`;
- *unique*: `CStrAt.unique : CStrAt s a bs → CStrAt s a bs' → bs = bs'`, so
  there is one string at an address and the hook gets that one.

**Evidence flows on.** A witness's proofs are usable by the hook. The
`userMode` witness is the L2 proof `s.el = .el0`, so a hook can call a
function that only user-mode states may reach:

```lean
def userOnly (s : State) (_ : s.el = .el0) : String := …
def chained : Hook where
  cond := .userMode
  run  := fun s ⟨h⟩ => userOnly s h
```

## How it connects to the rest

The same vocabulary runs through every part, so the parts can be proved
about together:
- **The semantics.** `State` is the specification's `ArchState`
  (`semantics.md`): a condition means a fact about the architecture, not
  about the emulator's encoding of it. The check runs on the emulator's
  state, and its proof goes through `abs` and the same memory and register
  lemmas the instruction squares use.
- **The MMU.** `read` is a byte through the current translation, with the
  access permission of the current exception level. So "x3 points to a C
  string" in user mode means a string the user program could itself read:
  the hook sees exactly what P3's memory protection says that program can
  see.
- **Decode.** A hook's condition has a static half (L18's `Selector`:
  mnemonic, class, PC range), decided once when the trace is built, and the
  dynamic half here, checked only when an instruction the static half
  selected runs. The full witness is both, `(ok : sel.Matches i) ×
  cond.Witness s`.
- **The doctrine.** A witness is an L6 witness, and `userMode` is L2
  evidence. Observing hooks return only an `Event`, so a run with them is
  the same run (L18).

## Further conditions, by the same construction

None of these is built yet.

- **Typed views of guest memory.** C strings generalise to a universe of
  layouts with a meaning function, the universe pattern of dependently
  typed programming:

  ```lean
  inductive Layout where
    | u8 | u16le | u32le | u64le
    | cstr (max : Nat)
    | array (n : Nat) (elem : Layout)
    | struct (fields : List (String × Layout))
    | ptr (to : Layout)                     -- followed through translation
  def Layout.Value : Layout → Type           -- what reading one gives
  ```

  A condition `regPointsTo r L` gives a witness of type `L.Value`, with the
  proof that memory holds it. A hook on "x1 points to a `struct
  { u32 len; ptr (array len u8) }`" gets the decoded structure. A length
  field that sizes a later array is a dependent layout,
  `Layout.dep (f : UInt64 → Layout)`, the shape real file-system and network
  structures have.
- **Guest ABIs as types.** On top of layouts, an opt-in module per guest
  (`Leanbox.Guest.Linux`, kept out of the guest-neutral board) types a
  system call by its number: a hook on `openat` gets `(dirfd : Int32)
  (path : CStr) (flags : UInt32)`, decoded from x0–x2 when `svc` runs with
  `x8 = 56`.
- **Conditions over time.** A condition can be an automaton over events:
  "the first `svc` after x0 pointed to a string". Its states are types, so
  the witness carries the path that reached the accepting state.
- **Address-space conditions**: a fetch from a page that was written since
  it was last executed (self-modifying code), or a store to a page the
  current translation marks executable.

## What it costs

- A hook's static half is decided at decode time. A machine with no hooks
  has no `hooked` op in any trace, and runs exactly as without hooks
  (gated by L13).
- The dynamic half runs only on instructions the static half selected.
- A witness's proofs are erased, so a hook is passed exactly the data the
  check computed. The real implementation scans into a `ByteArray` slice
  rather than a `List`, which is the sketch's representation only.
