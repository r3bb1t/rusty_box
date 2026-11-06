/-! A design sketch, checked by `lean docs/sketches/Concise.lean` (Lean 4.35); not part of the library.

How little an end user writes: a condition is an ordinary proposition, its
decision procedure is found by instance search, the event type is inferred
from the handler, the proof is passed to the handler without being named, and
installation proofs are discharged by default arguments. -/

inductive El | el0 | el1 deriving DecidableEq

structure State where
  el : El
  pc : Nat
  x : Fin 31 → UInt64

/-- A hook: any decidable proposition over the state, and a handler that is
given the state and the proof. Nothing here names a particular condition. -/
structure Hook where
  P : State → Prop
  dec : DecidablePred P
  /-- The event's type may depend on the state it is taken in. -/
  Event : State → Type
  handle : (s : State) → P s → Event s

/-- The user's whole surface: `on s, condition => event`. The decision procedure
comes from instance search (`=`, `∧`, `∨`, `¬`, `<` on the state's fields all
have one), and the event type is whatever the handler returns. -/
syntax "on " ident ", " term " => " term : term
macro_rules
  | `(on $s, $c => $e) => `(Hook.mk (fun $s => $c) inferInstance _ (fun $s _ => $e))

/-- Fire a hook: its handler runs only inside the branch that holds the proof. -/
def Hook.fire (h : Hook) (s : State) : List (h.Event s) :=
  have := h.dec
  if p : h.P s then [h.handle s p] else []

/-! ## User code -/

def a := on s, s.x 0 = 5 ∧ s.el = .el0 => s.pc            -- event: Nat
def b := on s, s.pc < 3 ∨ s.x 1 ≠ 0 => (s.x 1, s.x 2)      -- event: UInt64 × UInt64

def st : State := { el := .el0, pc := 7, x := fun r => if r = 0 then 5 else 0 }

#eval a.fire st                                             -- [7]
#eval b.fire st                                             -- []
#eval (on s, s.x 0 = 5 => s.x 0 + 1).fire st                -- [6]

/-- The handler uses the condition's proof, and its event's type depends on the
state: a number with the proof that it is two less than this state's PC. The
event type family is inferred from the handler. -/
def d := Hook.mk (fun s => 2 ≤ s.pc) inferInstance _ fun s (h : 2 ≤ s.pc) =>
  (⟨s.pc - 2, by omega⟩ : { n : Nat // n + 2 = s.pc })
#eval (d.fire st).map (·.val)                               -- [5]
