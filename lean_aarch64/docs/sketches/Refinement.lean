/-! A design sketch, checked by `lean docs/sketches/Refinement.lean` (Lean 4.35); not part of the library. -/
/-! Spec: the Arm ARM's view. Registers are a function, values are BitVec 64. -/
structure ArchState where
  x : Fin 31 → BitVec 64
  sp : BitVec 64

def upd (f : Fin 31 → BitVec 64) (i : Fin 31) (v : BitVec 64) : Fin 31 → BitVec 64 :=
  fun j => if j = i then v else f j

inductive SpecReg | r (i : Fin 31) | zr
def SpecReg.read (s : ArchState) : SpecReg → BitVec 64 | .r i => s.x i | .zr => 0
def ArchState.write (s : ArchState) : SpecReg → BitVec 64 → ArchState
  | .r i, v => { s with x := upd s.x i v } | .zr, _ => s

/-- ADD (shifted register), 64-bit, no shift: `X[d] = X[n] + X[m]` (Arm ARM, AddWithCarry with carry 0). -/
def specAdd (d n m : SpecReg) (s : ArchState) : ArchState := s.write d (n.read s + m.read s)

/-! Implementation: the emulator's view. Slots 0..30 X, 31 SP, 32 XZR (reads 0), 33 sink. -/
structure Impl where
  rf : Vector UInt64 34
  zr : rf[32] = 0

def Slot.ofSpec : SpecReg → Fin 34 | .r i => ⟨i, by omega⟩ | .zr => ⟨32, by decide⟩
def Slot.ofSpecW : SpecReg → Fin 34 | .r i => ⟨i, by omega⟩ | .zr => ⟨33, by decide⟩

def implAdd (d n m : Fin 34) (hd : d ≠ 32) (s : Impl) : Impl :=
  ⟨s.rf.set d (s.rf[n] + s.rf[m]), by
    rw [Vector.getElem_set]; split
    · exact absurd (Fin.ext (by omega)) hd
    · exact s.zr⟩

/-- The abstraction function: what Arm state an emulator state stands for. -/
def abs (s : Impl) : ArchState := { x := fun i => (s.rf[i.val]'(by omega)).toBitVec, sp := s.rf[31].toBitVec }

theorem read_abs (s : Impl) (r : SpecReg) : r.read (abs s) = (s.rf[Slot.ofSpec r]).toBitVec := by
  cases r
  · simp [SpecReg.read, abs, Slot.ofSpec]
  · show (0 : BitVec 64) = (s.rf[32]).toBitVec
    rw [s.zr]; rfl

/-- The commuting square: running the emulator's ADD, then abstracting, is the Arm ARM's ADD. -/
theorem add_refines (d n m : SpecReg) (s : Impl) :
    abs (implAdd (Slot.ofSpecW d) (Slot.ofSpec n) (Slot.ofSpec m)
          (by cases d <;> simp [Slot.ofSpecW, Fin.ext_iff] <;> omega) s)
      = specAdd d n m (abs s) := by
  cases d with
  | r i =>
    simp only [specAdd, ArchState.write, read_abs]
    simp only [abs, implAdd, Slot.ofSpecW, ArchState.mk.injEq]
    constructor
    · funext j
      by_cases h : j = i
      · subst h; simp [upd, Vector.getElem_set]
      · have h' : (i : Nat) ≠ j := fun e => h (Fin.ext e.symm)
        simp [upd, h, Vector.getElem_set, h']
    · simp only [Vector.getElem_set]; split
      · omega
      · rfl
  | zr =>
    simp only [specAdd, ArchState.write, abs, implAdd, Slot.ofSpecW, ArchState.mk.injEq]
    constructor
    · funext j; simp only [Vector.getElem_set]; split
      · omega
      · rfl
    · simp only [Vector.getElem_set]; split
      · omega
      · rfl

#print axioms add_refines
