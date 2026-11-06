/-! A design sketch, checked by `lean docs/sketches/Budget.lean` (Lean 4.35); not part of the library.

A bounded claim about one replay: "within N steps from this start, P never
holds". With the inputs fixed and N bounded it is decidable. It reduces, by
one generic theorem, to a single computation (a monitor's count is 0), and
that computation is the only thing left to check: by the kernel (trusts only
the kernel, slow) or by compiled code (fast, and trusts Lean's compiler, which
`#print axioms` then reports as a `_native.native_decide` axiom).

Measured on this toy machine (2026-11-06; one multiply-add per step):
- `decide +kernel`: 1,000 steps in 0.6 s, 10,000 in 2.5 s, 100,000 in 23.7 s,
  about 4,200 steps a second, `propext` and `Quot.sound` only.
- `native_decide`: 100,000,000 steps in about 75 s under `lean`'s interpreter
  (a compiled library runs far faster), with the native axiom reported.
- A non-tail-recursive monitor overflows the stack long before that, so the
  monitor that runs is tail-recursive and proved equal to the definition. -/

section Generic
variable {S : Type} (step : S → S) (P : S → Prop) [DecidablePred P]

def runState : Nat → S → S
  | 0, s => s
  | n + 1, s => runState n (step s)

/-- The monitor: how many of the first `n` states satisfy `P`. -/
def count : Nat → S → Nat
  | 0, _ => 0
  | n + 1, s => (if P s then 1 else 0) + count n (step s)

/-- The one generic theorem: a count of zero means `P` held at no step. -/
theorem never_of_count_zero : ∀ n s, count step P n s = 0 → ∀ t < n, ¬ P (runState step t s)
  | 0, _, _, t, ht => absurd ht (Nat.not_lt_zero t)
  | n + 1, s, h, t, ht => by
    simp only [count] at h
    have h1 : (if P s then 1 else 0) = 0 := by omega
    have h2 : count step P n (step s) = 0 := by omega
    cases t with
    | zero => simp only [runState]; intro hp; simp [hp] at h1
    | succ t => simp only [runState]; exact never_of_count_zero n (step s) h2 t (by omega)

/-- The monitor as it runs: tail-recursive, so a long budget does not grow the
stack. -/
def countFast : Nat → S → Nat → Nat
  | 0, _, acc => acc
  | n + 1, s, acc => countFast n (step s) (acc + if P s then 1 else 0)

theorem countFast_eq : ∀ n s acc, countFast step P n s acc = acc + count step P n s
  | 0, _, _ => rfl
  | n + 1, s, acc => by
    simp only [countFast, count]; rw [countFast_eq n (step s)]; omega

/-- What is checked: the running monitor's count. -/
theorem never_of_countFast_zero (n : Nat) (s : S) (h : countFast step P n s 0 = 0) :
    ∀ t < n, ¬ P (runState step t s) :=
  never_of_count_zero step P n s (by rw [countFast_eq] at h; omega)

end Generic

/-! ## A machine to measure on -/

/-- A toy machine: a counter and a 64-bit register mixed every step. -/
structure M where
  pc : Nat
  x : UInt64

def M.step (m : M) : M := { pc := m.pc + 1, x := m.x * 6364136223846793005 + 1442695040888963407 }

/-- The event that must never happen within the budget. -/
def Bad (m : M) : Prop := m.x = 0

instance : DecidablePred Bad := fun m => inferInstanceAs (Decidable (m.x = 0))

def start : M := { pc := 0, x := 1 }

/-- Checked by the kernel, by evaluating 1000 steps. -/
theorem never_bad_1000 : ∀ t < 1000, ¬ Bad (runState M.step t start) :=
  never_of_countFast_zero M.step Bad 1000 start (by decide +kernel)

/-- Checked by compiled code, for ten million steps. -/
theorem never_bad_1e7 : ∀ t < 10000000, ¬ Bad (runState M.step t start) :=
  never_of_countFast_zero M.step Bad 10000000 start (by native_decide)

#print axioms never_bad_1000
#print axioms never_bad_1e7
