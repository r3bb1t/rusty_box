/-! A design sketch, checked by `lean docs/sketches/ProofGuided.lean` (Lean 4.35); not part of the library.

A hook takes the proof of its condition, and computes what it needs from it:
the proof is the termination argument, so there is no fuel, no length cap and
no `Option`. Compiled, the proof is erased: `IsCStr.bytes` takes the state and
the address (C: `l_IsCStr_bytes(lean_object*, uint64_t)`) and is a plain loop. -/
structure State where
  read : UInt64 → Option UInt8

/-- There is a C string at `a`: a proposition, no data. -/
inductive IsCStr (s : State) : UInt64 → Prop
  | nul  {a} : s.read a = some 0 → IsCStr s a
  | more {a} : (∃ b, s.read a = some b ∧ b ≠ 0) → IsCStr s (a + 1) → IsCStr s a

/-- One step along a string. -/
def Next (s : State) (a' a : { a // IsCStr s a }) : Prop :=
  a'.val = a.val + 1 ∧ ∃ b, s.read a.val = some b ∧ b ≠ 0

theorem next_acc (s : State) : ∀ {a} (h : IsCStr s a), Acc (Next s) ⟨a, h⟩
  | _, h@(.nul h0) => .intro _ fun ⟨_, _⟩ ⟨_, b, hb, hn⟩ => by rw [h0] at hb; cases hb; exact absurd rfl hn
  | _, .more e t => .intro _ fun ⟨a', h'⟩ ⟨ha, _⟩ => by
      simp only at ha; subst ha; exact next_acc s t

instance (s : State) : WellFoundedRelation { a // IsCStr s a } :=
  ⟨Next s, ⟨fun ⟨_, h⟩ => next_acc s h⟩⟩

/-- The string's bytes, computed from the proof that it is there: no fuel, no
maximum length, no `Option`. The proof is why the loop ends, and why every
read succeeds. -/
def IsCStr.bytes (s : State) (p : { a // IsCStr s a }) : List UInt8 :=
  match hr : s.read p.val with
  | none => absurd hr (by
      obtain ⟨a, h⟩ := p
      cases h with
      | nul h0 => simp_all
      | more e _ => obtain ⟨b, hb, _⟩ := e; simp_all)
  | some b =>
    if hb : b = 0 then []
    else b :: IsCStr.bytes s ⟨p.val + 1, by
      obtain ⟨a, h⟩ := p
      cases h with
      | nul h0 => simp_all
      | more _ t => exact t⟩
termination_by p
decreasing_by exact ⟨rfl, b, hr, hb⟩

/-- A hook only exists where its condition holds: its argument is the proof. -/
def showString (s : State) (a : UInt64) (h : IsCStr s a) : String :=
  String.fromUTF8! ⟨(IsCStr.bytes s ⟨a, h⟩).toArray⟩

def mem : State := ⟨fun a => if a = 0x10 then some 0x68 else if a = 0x11 then some 0x69 else if a = 0x12 then some 0 else none⟩
theorem here : IsCStr mem 0x10 :=
  .more ⟨0x68, rfl, by decide⟩ (.more ⟨0x69, rfl, by decide⟩ (.nul rfl))
#eval showString mem 0x10 here
#print axioms IsCStr.bytes
