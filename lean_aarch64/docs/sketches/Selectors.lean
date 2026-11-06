/-! A design sketch, checked by `lean docs/sketches/Selectors.lean` (Lean 4.35); not part of the library. -/
structure LdrFields where
  t : UInt8
  n : UInt8
  off : UInt64
structure AddFields where
  d : UInt8
  n : UInt8
  imm : UInt64
inductive Instr where
  | ldr (f : LdrFields) | add (f : AddFields) | eret | nop

inductive Mnemonic | ldr | add | eret | nop deriving DecidableEq

def Instr.mnemonic : Instr → Mnemonic
  | .ldr _ => .ldr | .add _ => .add | .eret => .eret | .nop => .nop

/-- What a hook may ask for. -/
inductive Selector where
  | mnemonic (m : Mnemonic)
  | any
  | both (a b : Selector)

def Selector.Matches : Selector → Instr → Prop
  | .mnemonic m, i => i.mnemonic = m
  | .any, _ => True
  | .both a b, i => a.Matches i ∧ b.Matches i

def Selector.decMatches : (s : Selector) → (i : Instr) → Decidable (s.Matches i)
  | .mnemonic _, _ => inferInstanceAs (Decidable (_ = _))
  | .any, _ => .isTrue trivial
  | .both a b, i =>
    have := a.decMatches i
    have := b.decMatches i
    inferInstanceAs (Decidable (_ ∧ _))
instance (s : Selector) (i : Instr) : Decidable (s.Matches i) := s.decMatches i

/-- What the hook receives: a hook on LDR gets the LDR's fields, nothing less specific. -/
def Selector.Payload : Selector → Type
  | .mnemonic .ldr => LdrFields
  | .mnemonic .add => AddFields
  | _ => Instr

/-- Total: the proof rules out every instruction the selector does not admit. -/
def Selector.payload : (s : Selector) → (i : Instr) → s.Matches i → s.Payload
  | .mnemonic .ldr, .ldr f, _ => f
  | .mnemonic .ldr, .add _, h | .mnemonic .ldr, .eret, h | .mnemonic .ldr, .nop, h => nomatch h
  | .mnemonic .add, .add f, _ => f
  | .mnemonic .add, .ldr _, h | .mnemonic .add, .eret, h | .mnemonic .add, .nop, h => nomatch h
  | .mnemonic .eret, i, _ => i
  | .mnemonic .nop, i, _ => i
  | .any, i, _ => i
  | .both _ _, i, _ => i

structure Hook where
  sel : Selector
  run : sel.Payload → UInt64 → String

/-- Decided once, when the instruction is decoded into a trace. -/
inductive Op where
  | plain (i : Instr)
  | hooked (h : Hook) (i : Instr) (ok : h.sel.Matches i)

def lower (hk : Hook) (i : Instr) : Op :=
  if ok : hk.sel.Matches i then .hooked hk i ok else .plain i

def fire : Op → UInt64 → Option String
  | .hooked h i ok, pc => some (h.run (h.sel.payload i ok) pc)
  | .plain _, _ => none

def onLoads : Hook := { sel := .mnemonic .ldr, run := fun f pc => s!"ldr x{f.t} at {pc}" }
#eval fire (lower onLoads (.ldr ⟨3, 1, 8⟩)) 0x1000
#eval fire (lower onLoads (.add ⟨3, 1, 8⟩)) 0x1004

-- a hook on LDR cannot be handed an ADD's fields
/-- error: Type mismatch
  fun f x => toString f.d
has type
  AddFields → UInt64 → String
but is expected to have type
  (Selector.mnemonic Mnemonic.ldr).Payload → UInt64 → String -/
#guard_msgs in
example : Hook := { sel := .mnemonic .ldr, run := fun (f : AddFields) _ => toString f.d }
-- the same hook taking LDR fields is accepted
example : Hook := { sel := .mnemonic .ldr, run := fun (f : LdrFields) _ => toString f.t }
