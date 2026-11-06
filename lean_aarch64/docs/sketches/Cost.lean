/-! A design sketch, checked by `lean -c Cost.c docs/sketches/Cost.lean` (Lean 4.35); not part of the library.

Costs in types, two ways.
A. A graded type, `Cost n α` (Danielsson's `Thunk`): free at run time, costs
   that depend on values, budgets checked by `decide`. But its index is only
   asserted, so it can be forged (L14: privacy is no boundary).
B. Costs computed from the code: the code is data (a small language), its
   cost a function of its syntax, its evaluation proved to be what the cost
   counts. Nothing is asserted, so nothing can be forged. -/

/-! ## A. A graded type -/

/-- A computation said to cost `n` model units. The index exists only in the
type: compiled, a `Cost n α` is an `α`. -/
structure Cost (n : Nat) (α : Type) where
  val : α

@[inline] def Cost.ret (a : α) : Cost 0 α := ⟨a⟩
@[inline] def Cost.bind (x : Cost m α) (f : α → Cost n β) : Cost (m + n) β := ⟨(f x.val).val⟩
/-- A bound is an upper bound: it may be raised, with the proof that it rises. -/
@[inline] def Cost.weaken (x : Cost m α) (_ : m ≤ n) : Cost n α := ⟨x.val⟩

/-- The primitives, each with its cost: a 64-bit store is eight byte stores,
each after the runtime's exclusivity check (what callgrind showed). -/
@[inline] def load64 (rf : Array UInt64) (i : Fin rf.size) : Cost 1 UInt64 := ⟨rf[i]⟩
@[inline] def store64 (rf : Array UInt64) (i : Fin rf.size) (v : UInt64) : Cost 8 (Array UInt64) :=
  ⟨rf.set i v⟩

/-- A cost that depends on a value: copying `n` words costs `9 n`. -/
def copyWords (src : Array UInt64) : (n : Nat) → Array UInt64 → Cost (9 * n) (Array UInt64)
  | 0, dst => ⟨dst⟩
  | n + 1, dst => ⟨(copyWords src n (dst.push (src[n]?.getD 0))).val⟩

/-- An ADD's fast path, with its cost in its type: two loads and a store. -/
@[inline] def addFast (rf : Array UInt64) (d n m : Fin rf.size) : Cost 10 (Array UInt64) :=
  (load64 rf n).bind fun a => (load64 rf m).bind fun b => store64 rf d (a + b)

/-- A budget is a proof obligation, discharged by `decide`. -/
def withinBudget (budget : Nat) (_ : Cost n α) (ok : n ≤ budget := by decide) : Unit := let _ := ok; ()

example (rf : Array UInt64) (d n m : Fin rf.size) := withinBudget 12 (addFast rf d n m)

-- over budget: 10 is not at most 9
/-- error: could not synthesize default value for parameter 'ok' using tactics
---
error: Tactic `decide` proved that the proposition
  10 ≤ 9
is false -/
#guard_msgs in
example (rf : Array UInt64) (d n m : Fin rf.size) := withinBudget 9 (addFast rf d n m)

/-- The flaw: the index is asserted, not derived. Anything can claim cost 0. -/
def forged (rf : Array UInt64) : Cost 0 (Array UInt64) := ⟨(copyWords rf 1000 #[]).val⟩

/-! ## B. Costs computed from the code -/

/-- A tiny language for register-file code: the code is data. -/
inductive Prog where
  | load (dst src : Fin 4)           -- acc[dst] := rf[src]
  | add (dst a b : Fin 4)            -- acc[dst] := acc[a] + acc[b]
  | store (reg : Fin 4) (src : Fin 4) -- rf[reg] := acc[src]
  | seq (p q : Prog)
  | loop (n : Nat) (body : Prog)

/-- The cost model: a function of the program's syntax. -/
def Prog.cost : Prog → Nat
  | .load .. => 1
  | .add .. => 1
  | .store .. => 8
  | .seq p q => p.cost + q.cost
  | .loop n b => n * b.cost

structure Regs where
  rf : Fin 4 → UInt64
  acc : Fin 4 → UInt64

def upd (f : Fin 4 → UInt64) (i : Fin 4) (v : UInt64) : Fin 4 → UInt64 :=
  fun j => if j = i then v else f j

/-- Run `f` `n` times, adding up what each run counts. -/
def iter (f : Regs → Regs × Nat) : Nat → Regs → Regs × Nat
  | 0, r => (r, 0)
  | n + 1, r => let x := f r; let y := iter f n x.1; (y.1, x.2 + y.2)

/-- The semantics, counting as it goes. -/
def Prog.run : Prog → Regs → Regs × Nat
  | .load d s, r => ({ r with acc := upd r.acc d (r.rf s) }, 1)
  | .add d a b, r => ({ r with acc := upd r.acc d (r.acc a + r.acc b) }, 1)
  | .store g s, r => ({ r with rf := upd r.rf g (r.acc s) }, 8)
  | .seq p q, r => let x := p.run r; let y := q.run x.1; (y.1, x.2 + y.2)
  | .loop n b, r => iter b.run n r

theorem iter_cost (f : Regs → Regs × Nat) (c : Nat) (hf : ∀ r, (f r).2 = c) :
    ∀ n r, (iter f n r).2 = n * c
  | 0, _ => by simp [iter]
  | n + 1, r => by simp only [iter, hf, iter_cost f c hf n, Nat.succ_mul]; omega

/-- What the semantics counts is exactly the cost model, for every program and
every state: the bound is a theorem about the code, so it cannot be forged. -/
theorem run_cost : ∀ (p : Prog) (r : Regs), (p.run r).2 = p.cost
  | .load .., _ | .add .., _ | .store .., _ => rfl
  | .seq p q, r => by simp only [Prog.run, Prog.cost, run_cost p, run_cost q]
  | .loop n b, r => by simp only [Prog.run, Prog.cost]; exact iter_cost _ _ (run_cost b) n r

/-- ADD x0, x1, x2 as data: its cost is computed, 11. -/
def addProg : Prog := .seq (.load 0 1) (.seq (.load 1 2) (.seq (.add 0 0 1) (.store 0 0)))
example : addProg.cost ≤ 11 := by decide
example : (Prog.loop 100 addProg).cost = 1100 := by decide

#print axioms run_cost
