/-! A design sketch, checked by `lean docs/sketches/Further.lean` (Lean 4.35); not part of the library.

Four ways to push the types further, each on a toy:
A. the engine's caches carry their coherence proof, so a forgotten
   invalidation is a proof that cannot be written;
B. an architectural hook is a function on a quotient, so it cannot depend on
   anything the engine adds;
C. a hook owns a fragment of the machine, exactly its footprint, so it can
   neither read nor write outside it;
D. a hook's type is its specification (pre- and postcondition), and footprints
   are one shape of postcondition. -/

/-! ## A. Caches that carry their coherence -/

/-- RAM, and the engine's decoded copy of some of it (the trace cache's role). -/
structure Impl where
  mem : Fin 16 → UInt8
  cache : Fin 16 → Option UInt8

/-- Every cached copy agrees with memory. -/
def Coherent (m : Impl) : Prop := ∀ a v, m.cache a = some v → m.mem a = v

/-- The machine is an implementation state *with* the proof that its cache is
coherent: there is no machine without it. -/
structure Machine where
  impl : Impl
  coherent : Coherent impl

/-- A store must hand back a coherent machine, so it must invalidate the copy
of the byte it overwrites: the proof below is only possible because it does. -/
def Machine.store (m : Machine) (a : Fin 16) (v : UInt8) : Machine :=
  ⟨{ mem := fun b => if b = a then v else m.impl.mem b,
     cache := fun b => if b = a then none else m.impl.cache b },
   fun b w h => by
     simp only at h ⊢
     split at h
     · cases h
     · simp only [‹¬b = a›, ↓reduceIte]; exact m.coherent b w h⟩

/-- The store that forgets to invalidate. -/
def storeForgetting (m : Impl) (a : Fin 16) (v : UInt8) : Impl :=
  { m with mem := fun b => if b = a then v else m.mem b }

/-- It cannot be given the machine's type: there is no proof to give, because
the statement it would need is false. The classic emulator bug (stale decoded
code after a store into it) is a proof obligation that cannot be met. -/
theorem forgetting_breaks : ¬ ∀ m a v, Coherent m → Coherent (storeForgetting m a v) := by
  intro h
  have := h ⟨fun _ => 0, fun _ => some 0⟩ 0 1 (fun _ _ hv => by cases hv; rfl) 0 0 rfl
  simp [storeForgetting] at this

/-! ## B. Architectural hooks are functions on a quotient -/

/-- The engine's state: the architecture's registers, and a TLB hit counter
that only the engine has. -/
structure Engine where
  regs : Fin 4 → UInt64
  tlbHits : Nat

def abs (e : Engine) : Fin 4 → UInt64 := e.regs

instance archSetoid : Setoid Engine where
  r a b := abs a = abs b
  iseqv := ⟨fun _ => rfl, fun h => h.symm, fun h₁ h₂ => h₁.trans h₂⟩

/-- The architectural state: engine states identified when the architecture
cannot tell them apart. -/
abbrev Arch := Quotient archSetoid

/-- An architectural hook is a function on `Arch`. Defining one from the engine
state needs the proof that it respects the quotient: that it depends on the
architecture alone. -/
def archHook {α} (f : Engine → α) (h : ∀ a b, abs a = abs b → f a = f b) : Arch → α :=
  Quotient.lift f h

def readX0 : Arch → UInt64 := archHook (fun e => e.regs 0) (fun _ _ h => congrFun h 0)

/-- A hook that looks at the TLB has no such proof: the statement is false.
So engine-dependence cannot pass for an architectural observation. -/
theorem tlb_not_architectural : ¬ ∀ a b : Engine, abs a = abs b → a.tlbHits = b.tlbHits := by
  intro h
  have := h ⟨fun _ => 0, 0⟩ ⟨fun _ => 0, 1⟩ rfl
  simp at this

/-! ## C. A hook owns a fragment of the machine -/

inductive Loc | reg (i : Fin 4) | byte (a : Fin 16)

/-- What a location holds, by its kind. -/
def Loc.Val : Loc → Type | .reg _ => UInt64 | .byte _ => UInt8

structure M where
  val : (l : Loc) → l.Val

/-- A footprint: which locations. -/
abbrev Footprint := Loc → Bool

/-- A fragment: a value for each location in the footprint, and for no other. -/
def Fragment (fp : Footprint) := (l : Loc) → fp l = true → l.Val

def split (m : M) (fp : Footprint) : Fragment fp × Fragment (fun l => !fp l) :=
  (fun l _ => m.val l, fun l _ => m.val l)

def join (fp : Footprint) (f : Fragment fp) (g : Fragment (fun l => !fp l)) : M :=
  ⟨fun l => if h : fp l = true then f l h else g l (by simp [h])⟩

theorem join_split (m : M) (fp : Footprint) : join fp (split m fp).1 (split m fp).2 = m := by
  cases m; simp only [join, split, M.mk.injEq]; funext l; split <;> rfl

/-- A hook is given its fragment and returns one of the same footprint: it is
never given anything else to read, and has nothing else to write. -/
def OwnHook (fp : Footprint) := Fragment fp → Fragment fp

def runOwn (fp : Footprint) (h : OwnHook fp) (m : M) : M :=
  join fp (h (split m fp).1) (split m fp).2

/-- Integrity: outside its footprint, nothing changes. -/
theorem runOwn_frame (fp : Footprint) (h : OwnHook fp) (m : M) (l : Loc) (hl : fp l = false) :
    (runOwn fp h m).val l = m.val l := by
  simp [runOwn, join, split, hl]

/-- Confidentiality: two machines that agree on the footprint give the hook the
same input, so whatever it computes cannot depend on anything outside it. -/
theorem runOwn_confidential (fp : Footprint) (h : OwnHook fp) (m m' : M)
    (agree : ∀ l, fp l = true → m.val l = m'.val l) :
    h (split m fp).1 = h (split m' fp).1 := by
  congr 1; funext l hl; exact agree l hl

/-! ## D. A hook's type is its specification -/

/-- From any state satisfying `P`, a state related to it by `Q`. -/
def Spec (P : M → Prop) (Q : M → M → Prop) := (m : M) → P m → { m' : M // Q m m' }

/-- Footprints are one shape of postcondition: agreement outside them. -/
def Frame (fp : Footprint) (m m' : M) : Prop := ∀ l, fp l = false → m'.val l = m.val l

/-- Every owning hook meets the frame specification, for any precondition. -/
def OwnHook.spec (fp : Footprint) (h : OwnHook fp) : Spec (fun _ => True) (Frame fp) :=
  fun m _ => ⟨runOwn fp h m, fun l hl => runOwn_frame fp h m l hl⟩

/-- Specifications compose: the second's precondition follows from the first's
postcondition, and the composite's postcondition is the relational product. -/
def Spec.seq {P Q P' Q'} (h₁ : Spec P Q) (h₂ : Spec P' Q')
    (link : ∀ m m', P m → Q m m' → P' m') :
    Spec P (fun m m'' => ∃ m', Q m m' ∧ Q' m' m'') :=
  fun m hp =>
    let ⟨m', hq⟩ := h₁ m hp
    let ⟨m'', hq'⟩ := h₂ m' (link m m' hp hq)
    ⟨m'', m', hq, hq'⟩

/-- Two hooks with disjoint footprints compose into one whose frame is the
union: the separation-logic frame rule, as a function. -/
def framedSeq (fp₁ fp₂ : Footprint) (h₁ : OwnHook fp₁) (h₂ : OwnHook fp₂) :
    Spec (fun _ => True) (Frame (fun l => fp₁ l || fp₂ l)) :=
  fun m _ =>
    let r := (Spec.seq (h₁.spec fp₁) (h₂.spec fp₂) (fun _ _ _ _ => trivial)) m trivial
    ⟨r.1, fun l hl => by
      obtain ⟨m', h1, h2⟩ := r.2
      simp only [Bool.or_eq_false_iff] at hl
      rw [h2 l hl.2, h1 l hl.1]⟩

#print axioms forgetting_breaks
#print axioms tlb_not_architectural
#print axioms runOwn_confidential
#print axioms framedSeq
