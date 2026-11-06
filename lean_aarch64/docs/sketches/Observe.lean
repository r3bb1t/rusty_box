import Lean
/-! A design sketch, checked by `lean -c Observe.c docs/sketches/Observe.lean` (Lean 4.35); not part of the library.

Observation reimagined: an observer is a function from one machine state to a
monoid, and observing a run is combining every state's contribution. The
machine runs on its own; observers are downstream of it. What the types then
give, each as a theorem:
- an observer cannot affect the machine (it never receives the step);
- the fast engine (one tail-recursive loop, fused with the machine) equals the
  plain definition, *because* the monoid's laws are in its type;
- two observers in one pass equal each in its own pass;
- a run observed in pieces, split anywhere, equals the run observed whole. -/


/-! ## A machine that knows nothing of observers -/

inductive El | el0 | el1

@[inline] instance : DecidableEq El
  | .el0, .el0 | .el1, .el1 => isTrue rfl
  | .el0, .el1 | .el1, .el0 => isFalse (by intro h; cases h)

structure State where
  el : El
  pc : UInt64
  x0 : UInt64
  x1 : UInt64

@[inline] def State.next (s : State) : State := { s with pc := s.pc + 1, x0 := s.x0 + s.x1 }

def runState : Nat → State → State
  | 0, s => s
  | n + 1, s => runState n s.next

/-! ## Monoids, with their laws in the type -/

/-- A way to combine contributions. The laws are fields: an instance without
their proofs cannot be written, and every theorem below rests on them. -/
class Monoid (σ : Type) where
  one : σ
  mul : σ → σ → σ
  one_mul : ∀ a, mul one a = a
  mul_one : ∀ a, mul a one = a
  mul_assoc : ∀ a b c, mul (mul a b) c = mul a (mul b c)

instance : Monoid Nat := ⟨0, (· + ·), Nat.zero_add, Nat.add_zero, Nat.add_assoc⟩

instance : Monoid (Array α) :=
  ⟨#[], (· ++ ·), fun _ => Array.empty_append, fun _ => Array.append_empty,
   fun _ _ _ => Array.append_assoc⟩

instance [Monoid σ] [Monoid τ] : Monoid (σ × τ) where
  one := (Monoid.one, Monoid.one)
  mul a b := (Monoid.mul a.1 b.1, Monoid.mul a.2 b.2)
  one_mul a := by simp [Monoid.one_mul]
  mul_one a := by simp [Monoid.mul_one]
  mul_assoc a b c := by simp [Monoid.mul_assoc]

open Monoid

/-! ## Observing a run -/

/-- The definition: each state's contribution, combined in order. -/
def observe [Monoid σ] (f : State → σ) : Nat → State → σ
  | 0, _ => one
  | n + 1, s => mul (f s) (observe f n s.next)

/-- The engine: one tail-recursive loop, the machine step and the observer
fused, specialized for each observer it is given. -/
@[specialize] def observeFast [Monoid σ] (f : State → σ) : Nat → State → σ → State × σ
  | 0, s, acc => (s, acc)
  | n + 1, s, acc => observeFast f n s.next (mul acc (f s))

/-- The fused loop computes the machine's state and the definition's
observation, from any accumulator; associativity is what makes it so. -/
theorem observeFast_eq [Monoid σ] (f : State → σ) :
    ∀ n s acc, observeFast f n s acc = (runState n s, mul acc (observe f n s))
  | 0, s, acc => by simp [observeFast, runState, observe, mul_one]
  | n + 1, s, acc => by
    simp only [observeFast, runState, observe]
    rw [observeFast_eq f n s.next, mul_assoc]

/-- The engine's state is the machine's, whatever is observed. -/
theorem observeFast_state [Monoid σ] (f : State → σ) (n : Nat) (s : State) :
    (observeFast f n s one).1 = runState n s := by rw [observeFast_eq]

/-- The engine's observation is the definition's. -/
theorem observeFast_value [Monoid σ] (f : State → σ) (n : Nat) (s : State) :
    (observeFast f n s one).2 = observe f n s := by rw [observeFast_eq, one_mul]

/-- Two observers in one pass are each observer in its own pass. -/
theorem observe_pair [Monoid σ] [Monoid τ] (f : State → σ) (g : State → τ) :
    ∀ n s, observe (fun s => (f s, g s)) n s = (observe f n s, observe g n s)
  | 0, _ => rfl
  | n + 1, s => by simp only [observe, observe_pair f g n s.next]; rfl

/-- A run observed in two pieces, split anywhere, is the run observed whole: so
pieces between checkpoints can be observed separately and combined. -/
theorem observe_split [Monoid σ] (f : State → σ) :
    ∀ n m s, observe f (n + m) s = mul (observe f n s) (observe f m (runState n s))
  | 0, m, s => by simp [observe, runState, one_mul]
  | n + 1, m, s => by
    rw [Nat.add_right_comm]
    simp only [observe, runState]
    rw [observe_split f n m s.next, mul_assoc]

/-! ## The user's surface: `on condition => value` -/

/-- A contribution where the condition holds, nothing elsewhere. The value
lifts into the monoid by `Lift`: a number counts or sums, an element collects. -/
class Lift (σ : Type) (α : outParam Type) where
  lift : α → σ

instance : Lift Nat Nat := ⟨id⟩
instance : Lift (Array α) α := ⟨fun a => #[a]⟩

open Lean Elab Term in
/-- The state structure's fields are read from the environment, so the names in
scope are exactly its fields: the surface cannot drift from the structure. -/
elab "on " c:term " => " e:term : term <= ty => do
  let fields := getStructureFields (← getEnv) ``_root_.State
  let ids : Array (TSyntax `term) := fields.map fun n => ⟨mkIdent n⟩
  let pat ← `(⟨$ids,*⟩)
  let stx ← `(fun s => match s with | $pat:term => if $c then Lift.lift $e else Monoid.one)
  elabTerm stx ty

/-- `observer name : σ := on condition => value` always defines an inlined
observer, so the engine's specialization can never be lost by forgetting it. -/
macro "observer " n:ident " : " t:term " := " body:term : command =>
  `(@[inline] def $n : _root_.State → $t := $body)

/-! ## User code -/

observer oddAtEl0 : Nat := on x0 % 2 = 1 ∧ el = .el0 => 1
observer thirds : Array UInt64 := on pc % 3 = 0 => pc

def st : State := { el := .el0, pc := 0, x0 := 1, x1 := 2 }

def both (n : Nat) : Nat × Array UInt64 :=
  (observeFast (fun s => (oddAtEl0 s, thirds s)) n st Monoid.one).2

#eval both 10              -- (10, #[0, 3, 6, 9]): two observers, one pass

#print axioms observeFast_eq
#print axioms observe_split
