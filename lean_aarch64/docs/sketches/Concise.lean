/-! A design sketch, checked by `lean -c Concise.c docs/sketches/Concise.lean` (Lean 4.35); not part of the library.

Hooks that cost what hand-written code costs, written in one line.
- A hook is a decidable proposition over the state's fields, named directly,
  and a value; the type the user asks for decides how values accumulate.
- The engine is specialized for the hooks it is given: in the generated C the
  condition is inline comparisons and the loop a jump, with no closure call,
  no instance call and no allocation per firing (checked in `Concise.c`).
- The proof that the condition holds is free at run time and can remove
  checks from the hook's own code. -/

inductive El | el0 | el1

/-- By match, and inlined, so `el = .el0` compiles to a test of the byte (the
derived instance compares constructor indices as `Nat` through a call). -/
@[inline] instance : DecidableEq El
  | .el0, .el0 | .el1, .el1 => isTrue rfl
  | .el0, .el1 | .el1, .el0 => isFalse (by intro h; cases h)

structure State where
  el : El
  pc : UInt64
  x0 : UInt64
  x1 : UInt64

/-- How a value joins an accumulator, chosen by the accumulator's type. -/
class Accum (σ : Type) (α : outParam Type) where
  add : σ → α → σ

instance : Accum Nat Nat := ⟨(· + ·)⟩            -- a `Hook Nat` sums (counts, with 1)
instance : Accum (Array α) α := ⟨Array.push⟩     -- a `Hook (Array α)` collects

/-- A hook: a decidable condition, and what to do with the accumulator where it
holds, given the proof. -/
structure Hook (σ : Type) where
  P : State → Prop
  dec : DecidablePred P
  step : (s : State) → P s → σ → σ

/-- `on condition => value`, with the state's fields in scope by name. (The real
library would generate this per state structure, with a deriving handler.) -/
syntax "on " term " => " term : term
set_option hygiene false in
macro_rules
  | `(on $c => $e) =>
    `(Hook.mk (fun ⟨el, pc, x0, x1⟩ => $c) inferInstance (fun ⟨el, pc, x0, x1⟩ _ acc => Accum.add acc $e))

/-- A toy machine step. -/
@[inline] def State.next (s : State) : State := { s with pc := s.pc + 1, x0 := s.x0 + s.x1 }

/-- The engine, specialized per hook: the decision is an instance argument and
the step a function argument, which is what Lean specializes on. -/
@[specialize] def runWith {P : State → Prop} [DecidablePred P]
    (step : (s : State) → P s → σ → σ) : (n : Nat) → State → σ → State × σ
  | 0, s, a => (s, a)
  | n + 1, s, a =>
    let a := if p : P s then step s p a else a
    runWith step n s.next a

@[inline] def Hook.run (h : Hook σ) (n : Nat) (s : State) (a : σ) : State × σ :=
  have := h.dec
  runWith h.step n s a

/-! ## User code -/

@[inline] def odd : Hook Nat := on x0 % 2 = 1 ∧ el = .el0 => 1   -- counts
@[inline] def pcs : Hook (Array UInt64) := on pc % 3 = 0 => pc  -- collects

def st : State := { el := .el0, pc := 0, x0 := 1, x1 := 2 }

def countOdd (n : Nat) : Nat := (odd.run n st 0).2
def everyThird (n : Nat) : Array UInt64 := (pcs.run n st #[]).2

#eval countOdd 10                 -- 10
#eval everyThird 10               -- #[0, 3, 6, 9]

/-- The proof is free at run time and can remove a check from the hook itself:
where `x0 < 4` holds, indexing the table needs no bounds check. -/
def table : Array Nat := #[10, 20, 30, 40]

@[inline] def lookup : Hook Nat :=
  ⟨fun s => s.x0 < 4, inferInstance, fun s h acc =>
    acc + table[s.x0.toNat]'(by have := UInt64.lt_iff_toNat_lt.mp h; simp at this; simp [table]; omega)⟩

def sumLookups (n : Nat) : Nat := (lookup.run n { st with x1 := 0 } 0).2
#eval sumLookups 3                -- 60: x0 stays 1, table[1] three times
