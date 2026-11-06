# The type doctrine

Leanbox is written so that a wrong program is a program Lean rejects. A
test shows that one run went right; a type shows that no run can go wrong
in that way. Thirteen rules say how, adapted to dependent types from
llvmkit's type-safety doctrine (D1–D11) and rusty_box's safety doctrine
(R0–R9). Cite them by id (`L1`–`L13`) in commits and reviews.

The rules cost nothing at run time, and that is checked, not assumed (L13):
Lean erases proofs, compiles an enumeration to one byte, and compiles a
structure whose only non-proof field is a machine word to that word.

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
1. **L1 and L7**: the state codes become inductive types, starting with
   `Power`, `Activity`, `Flow` and `Fault`.
2. **L4**: slots and `RegFile`.
3. **L3 and L10**: `Val w` through `Isa.Alu`, with `Spec/` for the ALU and
   the flags.
4. **L2**: privilege evidence in `Cpu.System`, `Cpu.Sysreg` and
   `Cpu.Exception`, and system registers as an enumeration with their
   lowest exception level.
5. **L5**: `VA`, `PA`, `Ticks` and `Count` through `Cpu.Mmu`, `Cpu.Memory`
   and `Cpu.Timer`.
6. **L6**: `Granted`, `InRam` and `Trace`.
7. **L11**: `run`, `drive` and `checkLoop` made total.

L12's locks land with the rule they lock, in the same commit.
