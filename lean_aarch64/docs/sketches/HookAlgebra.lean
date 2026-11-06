/-! A design sketch, checked by `lean docs/sketches/HookAlgebra.lean` (Lean 4.35); not part of the library.

Complicated hooks that keep their guarantees: hook points with their own
context types, a machine indexed by its CPU model, stateful hooks, declared
register writes that are the only ones a hook can make, frame hooks with
overlays the size of the frame, installation refused for conflicting hooks,
and the theorem that hooks writing nothing leave the machine's run as it is. -/

/-! ## A machine indexed by its CPU model -/

inductive Model | a57 | armv92 deriving DecidableEq
def Model.hasSME : Model → Bool | .a57 => false | .armv92 => true

inductive El | el0 | el1 deriving DecidableEq

/-- PSTATE.SM (streaming mode) exists only on a model with SME: on the A57 the
field's type is `Unit`, so there is no streaming state to read or set. -/
structure State (m : Model) where
  el : El
  pc : Nat
  x : Fin 31 → UInt64
  mem : UInt64 → UInt8
  sm : if m.hasSME then Bool else Unit

/-- `smstart` takes the proof that the model has SME: an A57 program cannot
contain it. `strb` stores a register's low byte; `present` shows a frame. -/
inductive Instr (m : Model) where
  | mov (r : Fin 31) (v : UInt64)
  | strb (r : Fin 31) (a : UInt64)
  | svc
  | smstart (h : m.hasSME = true)
  | present

def setSm (m : Model) (h : m.hasSME = true) (s : State m) : State m :=
  { s with sm := cast (by simp [h]) true }

def Instr.exec (s : State m) : Instr m → State m
  | .mov r v => { s with x := fun q => if q = r then v else s.x q, pc := s.pc + 1 }
  | .strb r a => { s with mem := fun b => if b = a then (s.x r).toUInt8 else s.mem b, pc := s.pc + 1 }
  | .svc => { s with el := .el1, pc := s.pc + 1 }
  | .smstart h => { setSm m h s with pc := s.pc + 1 }
  | .present => { s with pc := s.pc + 1 }

/-! ## Conditions, indexed by the model too -/

inductive Cond (m : Model) where
  | always
  | userMode
  | regIs (r : Fin 31) (v : UInt64)
  /-- in streaming mode: only on a model that has it -/
  | streaming (h : m.hasSME = true)
  | and (p q : Cond m)

def State.streaming (s : State m) (h : m.hasSME = true) : Bool := cast (by simp [h]) s.sm

def Cond.Witness : Cond m → State m → Type
  | .always, _ => Unit
  | .userMode, s => PLift (s.el = .el0)
  | .regIs r v, s => PLift (s.x r = v)
  | .streaming h, s => PLift (s.streaming h = true)
  | .and p q, s => p.Witness s × q.Witness s

def Cond.check : (p : Cond m) → (s : State m) → Option (p.Witness s)
  | .always, _ => some ()
  | .userMode, s => if h : s.el = .el0 then some ⟨h⟩ else none
  | .regIs r v, s => if h : s.x r = v then some ⟨h⟩ else none
  | .streaming h, s => if h' : s.streaming h = true then some ⟨h'⟩ else none
  | .and p q, s => match p.check s, q.check s with
    | some a, some b => some (a, b) | _, _ => none

/-! ## Hook points: each has its own context type -/

def W : Nat := 4
def H : Nat := 2
def FB : UInt64 := 0x8000
abbrev Frame := Vector (Vector UInt8 W) H
/-- An overlay is exactly the frame's size: a pixel outside it cannot be named. -/
abbrev Overlay := Vector (Vector (Option UInt8) W) H

def State.frame (s : State m) : Frame :=
  Vector.ofFn fun y => Vector.ofFn fun x => s.mem (FB + (y.val * W + x.val).toUInt64)

inductive Point | exec | store | frame

/-- What a hook at each point is given, as a type computed from the point, the
state before the instruction, and the instruction. -/
def Point.Ctx : Point → State m → Instr m → Type
  | .exec, s, i => { post : State m // post = i.exec s }       -- the step's result, proved
  | .store, s, .strb r a =>                                     -- the byte before and after
      (old new : UInt8) × PLift (old = s.mem a ∧ new = (s.x r).toUInt8)
  | .store, _, _ => Empty                                       -- not a store: no context
  | .frame, s, .present => { f : Frame // f = s.frame }         -- the picture being shown
  | .frame, _, _ => Empty

/-- Build the context where the point applies. -/
def Point.ctx? : (p : Point) → (s : State m) → (i : Instr m) → Option (p.Ctx s i)
  | .exec, s, i => some ⟨i.exec s, rfl⟩
  | .store, s, .strb r a => some ⟨s.mem a, (s.x r).toUInt8, ⟨rfl, rfl⟩⟩
  | .store, _, .mov .. | .store, _, .svc | .store, _, .smstart _ | .store, _, .present => none
  | .frame, s, .present => some ⟨s.frame, rfl⟩
  | .frame, _, .mov .. | .frame, _, .strb .. | .frame, _, .svc | .frame, _, .smstart _ => none

/-! ## What a hook may change: declared, and nothing else is expressible -/

structure Caps where
  /-- the registers it may set -/
  writes : List (Fin 31)
  /-- whether it draws on the screen -/
  draws : Bool

/-- A reply gives a value for each declared register and for no other (the
function's domain is `r ∈ writes`), and an overlay only if it draws. -/
abbrev Caps.Reply (c : Caps) : Type :=
  ((r : Fin 31) → r ∈ c.writes → UInt64) × (if c.draws then Overlay else Unit)

def Caps.overlay? (c : Caps) (r : c.Reply) : Option Overlay :=
  if h : c.draws then some (cast (by simp [h]) r.2) else none

/-- A hook: where, when, its own state, its own events, what it may change. -/
structure Hook (m : Model) where
  name : String
  point : Point
  cond : Cond m
  σ : Type
  init : σ
  Event : Type
  caps : Caps
  handle : (s : State m) → (i : Instr m) → point.Ctx s i → cond.Witness s → σ →
    σ × Event × caps.Reply

/-- Apply a reply: the declared registers take its values, every other register
keeps its own. -/
def apply (s : State m) (c : Caps) (r : c.Reply) : State m :=
  { s with x := fun q => if h : q ∈ c.writes then r.1 q h else s.x q }

/-- The frame condition: a register a hook did not declare keeps its value,
whatever the hook replies. -/
theorem apply_frame (s : State m) (c : Caps) (rep : c.Reply) (r : Fin 31) (h : r ∉ c.writes) :
    (apply s c rep).x r = s.x r := by
  simp [apply, h]

/-- And nothing but registers changes. -/
theorem apply_rest (s : State m) (c : Caps) (rep : c.Reply) :
    (apply s c rep).el = s.el ∧ (apply s c rep).pc = s.pc ∧ (apply s c rep).mem = s.mem := ⟨rfl, rfl, rfl⟩

theorem apply_of_nil (s : State m) (c : Caps) (h : c.writes = []) (r : c.Reply) :
    apply s c r = s := by
  cases s; unfold apply; congr 1
  funext q; split
  · rename_i hq; rw [h] at hq; cases hq
  · rfl

/-! ## Installing hooks: refused if two of them write the same register -/

def Compatible : List (Hook m) → Prop
  | [] => True
  | h :: hs => (∀ g ∈ hs, ∀ r ∈ h.caps.writes, r ∉ g.caps.writes) ∧ Compatible hs

def Compatible.dec : (hs : List (Hook m)) → Decidable (Compatible hs)
  | [] => inferInstanceAs (Decidable True)
  | h :: hs =>
    have := Compatible.dec hs
    inferInstanceAs (Decidable (_ ∧ _))
instance (hs : List (Hook m)) : Decidable (Compatible hs) := Compatible.dec hs

/-- Each installed hook's state and events, at that hook's own types. -/
inductive Track : List (Hook m) → Type 1
  | nil : Track []
  | cons {h hs} : h.σ → List h.Event → Track hs → Track (h :: hs)

def Track.init : (hs : List (Hook m)) → Track hs
  | [] => .nil
  | h :: hs => .cons h.init [] (Track.init hs)

/-- Run every hook whose point and condition hold, in order, each seeing the
state the previous one left. -/
def fire (s : State m) (i : Instr m) : (hs : List (Hook m)) → Track hs →
    State m × Track hs × List Overlay
  | [], .nil => (s, .nil, [])
  | h :: hs, .cons σ ev t =>
    match h.point.ctx? s i, h.cond.check s with
    | some c, some w =>
      let out := h.handle s i c w σ
      let rest := fire (apply s h.caps out.2.2) i hs t
      (rest.1, .cons out.1 (out.2.1 :: ev) rest.2.1, (h.caps.overlay? out.2.2).toList ++ rest.2.2)
    | _, _ =>
      let rest := fire s i hs t
      (rest.1, .cons σ ev rest.2.1, rest.2.2)

structure Emulator (m : Model) (hs : List (Hook m)) where
  program : Array (Instr m)
  state : State m
  track : Track hs
  /-- the screen's overlay: the GUI's, not the machine's -/
  screen : Overlay

def Emulator.create (program : Array (Instr m)) (start : State m) (hs : List (Hook m))
    (_ : Compatible hs) : Emulator m hs :=
  { program, state := start, track := .init hs, screen := Vector.replicate H (Vector.replicate W none) }

def Emulator.step (e : Emulator m hs) : Option (Emulator m hs) :=
  match e.program[e.state.pc]? with
  | none => none
  | some i =>
    let out := fire e.state i hs e.track
    some { e with state := i.exec out.1, track := out.2.1, screen := out.2.2.getLastD e.screen }

def Emulator.run (e : Emulator m hs) : Nat → Emulator m hs
  | 0 => e
  | n + 1 => match e.step with | some e' => e'.run n | none => e

/-- What the screen shows: the machine's frame with the overlay on top. -/
def Emulator.display (e : Emulator m hs) : Frame :=
  Vector.ofFn fun y => Vector.ofFn fun x => (e.screen[y][x]).getD (e.state.frame[y][x])

/-! ## The guarantee: hooks that write nothing change nothing -/

/-- Hooks that declare no register writes leave the state as it was. -/
theorem fire_state (s : State m) (i : Instr m) : ∀ (hs : List (Hook m)) (t : Track hs),
    (∀ h ∈ hs, h.caps.writes = []) → (fire s i hs t).1 = s
  | [], .nil, _ => rfl
  | h :: hs, .cons σ ev t, hw => by
    have hw' : ∀ g ∈ hs, g.caps.writes = [] := fun g hg => hw g (List.mem_cons_of_mem _ hg)
    simp only [fire]
    split
    · simp only; rw [fire_state _ i hs _ hw', apply_of_nil _ _ (hw h (List.mem_cons_self))]
    · exact fire_state s i hs t hw'

/-- The machine with no hooks at all. -/
def bareRun (p : Array (Instr m)) (s : State m) : Nat → State m
  | 0 => s
  | n + 1 => match p[s.pc]? with
    | none => s
    | some i => bareRun p (i.exec s) n

/-- A machine with any number of hooks that write nothing runs exactly as the
machine with none: the same states, whatever the hooks observe, keep or draw. -/
theorem observers_transparent {hs : List (Hook m)} (hw : ∀ h ∈ hs, h.caps.writes = []) :
    ∀ (n : Nat) (e : Emulator m hs), (e.run n).state = bareRun e.program e.state n
  | 0, _ => rfl
  | n + 1, e => by
    simp only [Emulator.run, Emulator.step, bareRun]
    cases hi : e.program[e.state.pc]? with
    | none => rfl
    | some i =>
      simp only
      rw [observers_transparent hw n, fire_state _ _ _ _ hw]

/-! ## Using it -/

/-- Stateful, at every store: counts stores and records the address and the
byte before and after, from its context. -/
def storeLog : Hook m where
  name := "stores"; point := .store; cond := .always
  σ := Nat; init := 0; Event := Nat × UInt8 × UInt8
  caps := { writes := [], draws := false }
  handle := fun _ i c _ n => match i, c with
    | .strb _ _, ⟨old, new, _⟩ => (n + 1, (n, old, new), ((fun _ h => nomatch h), ()))

/-- Stateless, at every executed instruction: the PC after, read from the
proved post-state, not recomputed. -/
def trace : Hook m where
  name := "trace"; point := .exec; cond := .always
  σ := Unit; init := (); Event := Nat
  caps := { writes := [], draws := false }
  handle := fun _ _ c _ _ => ((), c.val.pc, ((fun _ h => nomatch h), ()))

/-- At each frame: draws a marker in the top-left corner, an overlay of exactly
the frame's size. -/
def marker : Hook m where
  name := "marker"; point := .frame; cond := .always
  σ := Unit; init := (); Event := Unit
  caps := { writes := [], draws := true }
  handle := fun _ _ _ _ _ =>
    ((), (), ((fun _ h => nomatch h),
      (Vector.ofFn fun y => Vector.ofFn fun x => if y.val = 0 ∧ x.val = 0 then some 0xFF else none : Overlay)))

/-- In streaming mode only, and only on a model that has it: sets x0 to 1.
Declares x0, so x0 is the only register its reply can name. -/
def inStreaming : Hook .armv92 where
  name := "sme"; point := .exec; cond := .streaming rfl
  σ := Unit; init := (); Event := Unit
  caps := { writes := [0], draws := false }
  handle := fun s _ _ _ _ => ((), (), (fun _ _ => 1, ()))

def mem0 : UInt64 → UInt8 := fun _ => 0

def startA57 : State .a57 := { el := .el0, pc := 0, x := fun _ => 0, mem := mem0, sm := () }
def startV92 : State .armv92 := { el := .el0, pc := 0, x := fun _ => 0, mem := mem0, sm := cast (by simp [Model.hasSME]) false }

def progA57 : Array (Instr .a57) := #[.mov 1 0x41, .strb 1 0x8000, .strb 1 0x8005, .present]

def Track.get0 : Track (h :: hs) → List h.Event | .cons _ ev _ => ev.reverse
def Track.tail : Track (h :: hs) → Track hs | .cons _ _ t => t

def withHooks := (Emulator.create progA57 startA57 [storeLog, trace, marker] (by decide)).run 10
#eval withHooks.track.get0                                -- the stores: (count, before, after)
#eval withHooks.track.tail.get0                           -- the PCs
#eval withHooks.display.toList.map (·.toList)             -- the frame, the marker on top

def progV92 : Array (Instr .armv92) := #[.mov 0 7, .smstart rfl, .mov 2 3]
#eval ((Emulator.create progV92 startV92 [inStreaming] (by decide)).run 10).state.x 0   -- set by the hook

-- SMSTART on a Cortex-A57: there is no proof that it has SME
/-- error: Application type mismatch: The argument
  rfl
has type
  ?m.3 = ?m.3
but is expected to have type
  Model.a57.hasSME = true
in the application
  Instr.smstart ⋯ -/
#guard_msgs in
example : Instr .a57 := .smstart rfl
-- accepted twin
example : Instr .armv92 := .smstart rfl

-- two hooks that both write x0 cannot be installed together
def alsoX0 : Hook .armv92 := { inStreaming with name := "also" }
/-- error: Tactic `decide` proved that the proposition
  Compatible [inStreaming, alsoX0]
is false -/
#guard_msgs in
example := Emulator.create progV92 startV92 [inStreaming, alsoX0] (by decide)
-- accepted twin
example := Emulator.create progV92 startV92 [inStreaming, trace] (by decide)

#print axioms observers_transparent
#print axioms apply_frame
