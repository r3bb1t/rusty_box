/-! A design sketch, checked by `lean docs/sketches/Hooks.lean` (Lean 4.35); not part of the library. -/
inductive El | el0 | el1 deriving DecidableEq, Repr

/-- The architectural view a condition is stated over. `read` is a byte through
the current translation, `none` where the access would fault. -/
structure State where
  el : El
  x : Fin 31 → UInt64
  read : UInt64 → Option UInt8

/-- `bs` is the NUL-terminated string at `a`: every byte readable, none of them
NUL, a NUL after the last. -/
inductive CStrAt (s : State) : UInt64 → List UInt8 → Prop
  | nil  {a} : s.read a = some 0 → CStrAt s a []
  | cons {a b bs} : s.read a = some b → b ≠ 0 → CStrAt s (a + 1) bs → CStrAt s a (b :: bs)

/-- A C string at `a` of fewer than `max` bytes, with the proof that it is one. -/
abbrev CStr (s : State) (a : UInt64) (max : Nat) :=
  { bs : List UInt8 // CStrAt s a bs ∧ bs.length < max }

/-- The scan's result is its own proof: it can only return a string that is there. -/
def scan (s : State) : (max : Nat) → (a : UInt64) → Option (CStr s a max)
  | 0, _ => none
  | max + 1, a =>
    match h : s.read a with
    | none => none
    | some b =>
      if hb : b = 0 then some ⟨[], .nil (hb ▸ h), by simp⟩
      else match scan s max (a + 1) with
        | none => none
        | some ⟨bs, hc, hl⟩ => some ⟨b :: bs, .cons h hb hc, by simp; omega⟩

/-- What a hook may wait for, over the machine's state. -/
inductive Cond where
  | userMode
  | regIs (r : Fin 31) (v : UInt64)
  | regPointsToCStr (r : Fin 31) (max : Nat)
  | someRegPointsToCStr (max : Nat)
  | and (p q : Cond)
  | or (p q : Cond)

/-- What a condition gives the hook when it holds: its type depends on the
condition and on the state it holds in. -/
def Cond.Witness : Cond → State → Type
  | .userMode, s => PLift (s.el = .el0)
  | .regIs r v, s => PLift (s.x r = v)
  | .regPointsToCStr r max, s => CStr s (s.x r) max
  | .someRegPointsToCStr max, s => (r : Fin 31) × CStr s (s.x r) max
  | .and p q, s => p.Witness s × q.Witness s
  | .or p q, s => p.Witness s ⊕ q.Witness s

def firstCStr (s : State) (max : Nat) : (n : Nat) → n ≤ 31 → Option ((r : Fin 31) × CStr s (s.x r) max)
  | 0, _ => none
  | n + 1, h => match scan s max (s.x ⟨30 - n, by omega⟩) with
    | some c => some ⟨⟨30 - n, by omega⟩, c⟩
    | none => firstCStr s max n (by omega)

/-- The certified check: `some w` only with a witness, which carries the proof. -/
def Cond.check : (p : Cond) → (s : State) → Option (p.Witness s)
  | .userMode, s => if h : s.el = .el0 then some ⟨h⟩ else none
  | .regIs r v, s => if h : s.x r = v then some ⟨h⟩ else none
  | .regPointsToCStr r max, s => scan s max (s.x r)
  | .someRegPointsToCStr max, s => firstCStr s max 31 (by omega)
  | .and p q, s => match p.check s, q.check s with
    | some a, some b => some (a, b) | _, _ => none
  | .or p q, s => match p.check s with
    | some a => some (.inl a) | none => (q.check s).map .inr

structure Hook where
  cond : Cond
  run : (s : State) → cond.Witness s → String

/-- A user-mode hook that fires when any register points to a C string and gets
the register and the string: the bytes the check read, not a second read. -/
def userString : Hook where
  cond := .and .userMode (.someRegPointsToCStr 256)
  run := fun _ (_, ⟨r, ⟨bs, _⟩⟩) => s!"x{r.val} -> {String.fromUTF8! ⟨bs.toArray⟩}"

def fire (h : Hook) (s : State) : Option String := (h.cond.check s).map (h.run s)

/-- "hi" at 0x1000, x3 pointing at it. -/
def demo (el : El) : State where
  el := el
  x := fun r => if r.val = 3 then 0x1000 else 0
  read := fun a => if a = 0x1000 then some 0x68 else if a = 0x1001 then some 0x69
                   else if a = 0x1002 then some 0 else none

#eval fire userString (demo .el0)
#eval fire userString (demo .el1)

/-- The witness is evidence the hook can use: the user-mode proof feeds a function
that only user-mode states may call. -/
def userOnly (s : State) (_ : s.el = .el0) : String := "user"
def chained : Hook where
  cond := .userMode
  run := fun s ⟨h⟩ => userOnly s h

#print axioms scan

/-- A string at an address is unique: two scans cannot disagree. -/
theorem CStrAt.unique {s : State} : ∀ {a bs bs'}, CStrAt s a bs → CStrAt s a bs' → bs = bs'
  | _, _, _, .nil h, .nil _ => rfl
  | _, _, _, .nil h, .cons h' hb _ => by rw [h] at h'; cases h'; exact absurd rfl hb
  | _, _, _, .cons h hb _, .nil h' => by rw [h] at h'; cases h'; exact absurd rfl hb
  | _, _, _, .cons h _ t, .cons h' _ t' => by
      rw [h] at h'; cases h'; rw [CStrAt.unique t t']

/-- Complete: whenever a short enough string is there, the scan finds it. -/
theorem scan_complete {s : State} : ∀ {a bs} (max : Nat), CStrAt s a bs → bs.length < max →
    ∃ c, scan s max a = some c ∧ c.val = bs
  | _, _, 0, _, hl => absurd hl (Nat.not_lt_zero _)
  | a, _, max + 1, .nil h, _ => by
      refine ⟨⟨[], .nil h, by simp⟩, ?_, rfl⟩
      unfold scan; split
      · simp_all
      · rename_i b' h'; rw [h] at h'; cases h'; simp
  | a, b :: bs, max + 1, .cons h hb t, hl => by
      obtain ⟨c, hc, hv⟩ := scan_complete max t (by simp at hl; omega)
      refine ⟨⟨b :: bs, .cons h hb t, hl⟩, ?_, rfl⟩
      unfold scan; split
      · simp_all
      · rename_i b' h'; rw [h] at h'; cases h'
        simp only [hb, dite_false, hc]
        obtain ⟨cv, _, _⟩ := c; subst hv; rfl

#print axioms CStrAt.unique
#print axioms scan_complete
