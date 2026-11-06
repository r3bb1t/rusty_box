/-! A design sketch, checked by `lean docs/sketches/Emulator.lean` (Lean 4.35); not part of the library.

Creating an emulator, installing hooks, running it, and reading what the hooks
saw, on a toy machine small enough to show the types: the machine's
instructions are `mov`, `svc` (user to kernel) and `eret` (kernel to user). -/

/-! ## A toy machine -/

inductive El | el0 | el1 deriving DecidableEq, Repr

structure State where
  el : El
  pc : Nat
  x : Fin 31 → UInt64
  read : UInt64 → Option UInt8

inductive Instr | mov (r : Fin 31) (v : UInt64) | svc | eret
inductive Mnemonic | mov | svc | eret deriving DecidableEq
def Instr.mnemonic : Instr → Mnemonic | .mov .. => .mov | .svc => .svc | .eret => .eret

def Instr.exec (s : State) : Instr → State
  | .mov r v => { s with x := fun q => if q = r then v else s.x q, pc := s.pc + 1 }
  | .svc => { s with el := .el1, pc := s.pc + 1 }
  | .eret => { s with el := .el0, pc := s.pc + 1 }

/-! ## Conditions and their witnesses (as in Hooks.lean) -/

inductive CStrAt (s : State) : UInt64 → List UInt8 → Prop
  | nil  {a} : s.read a = some 0 → CStrAt s a []
  | cons {a b bs} : s.read a = some b → b ≠ 0 → CStrAt s (a + 1) bs → CStrAt s a (b :: bs)

abbrev CStr (s : State) (a : UInt64) (max : Nat) := { bs : List UInt8 // CStrAt s a bs ∧ bs.length < max }

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

inductive Cond | userMode | regIs (r : Fin 31) (v : UInt64) | regPointsToCStr (r : Fin 31) | and (p q : Cond)

def Cond.Witness : Cond → State → Type
  | .userMode, s => PLift (s.el = .el0)
  | .regIs r v, s => PLift (s.x r = v)
  | .regPointsToCStr r, s => CStr s (s.x r) 256
  | .and p q, s => p.Witness s × q.Witness s

def Cond.check : (p : Cond) → (s : State) → Option (p.Witness s)
  | .userMode, s => if h : s.el = .el0 then some ⟨h⟩ else none
  | .regIs r v, s => if h : s.x r = v then some ⟨h⟩ else none
  | .regPointsToCStr r, s => scan s 256 (s.x r)
  | .and p q, s => match p.check s, q.check s with
    | some a, some b => some (a, b) | _, _ => none

/-! ## Hooks: each declares its event type and what it may do -/

inductive Effect | observe | intervene deriving DecidableEq

/-- What a hook's run returns, by its effect. An observing hook returns only its
event. An intervening hook also returns a new state, which may differ from the
old one only in its registers: the type forbids it to change the exception
level, the PC or memory. -/
def Effect.Out : Effect → State → Type → Type
  | .observe, _, E => E
  | .intervene, s, E => E × { s' : State // s'.el = s.el ∧ s'.pc = s.pc ∧ s'.read = s.read }

structure Hook where
  name : String
  /-- the static half: decided once per instruction, before the machine runs -/
  on : Mnemonic
  /-- the dynamic half: checked when a selected instruction runs -/
  cond : Cond
  /-- this hook's own kind of event -/
  Event : Type
  effect : Effect
  run : (s : State) → cond.Witness s → effect.Out s Event

/-- Which hook of a list: an index that is data, not a proof, so a log can be
looked up by it. -/
inductive Member : Hook → List Hook → Type 1
  | here {h hs} : Member h (h :: hs)
  | there {h g hs} : Member h hs → Member h (g :: hs)

/-- One log per installed hook, each of that hook's event type. -/
inductive Logs : List Hook → Type 1
  | nil : Logs []
  | cons {h hs} : List h.Event → Logs hs → Logs (h :: hs)

def Logs.empty : (hs : List Hook) → Logs hs
  | [] => .nil
  | _ :: hs => .cons [] (Logs.empty hs)

def Logs.get : Logs hs → Member h hs → List h.Event
  | .cons l _, .here => l.reverse
  | .cons _ r, .there m => r.get m

def Logs.push : Logs hs → Member h hs → h.Event → Logs hs
  | .cons l r, .here, e => .cons (e :: l) r
  | .cons l r, .there m, e => .cons l (r.push m e)

/-- A run is exact (comparable with rusty_box in lockstep) when every hook only
observes. Computed from the hook list, so it is known where the list is. -/
def exact (hs : List Hook) : Bool := hs.all (·.effect == .observe)

/-! ## The emulator, indexed by its hooks -/

structure Emulator (hs : List Hook) where
  program : Array Instr
  state : State
  /-- per instruction, the hooks whose static half selects it: decided once,
  when the emulator is created (when the trace is built, in the real machine) -/
  selected : Array (List ((h : Hook) × Member h hs))
  logs : Logs hs

def members : (hs : List Hook) → List ((h : Hook) × Member h hs)
  | [] => []
  | h :: hs => ⟨h, .here⟩ :: (members hs).map fun ⟨g, m⟩ => ⟨g, .there m⟩

def Emulator.create (program : Array Instr) (start : State) (hs : List Hook) : Emulator hs :=
  { program, state := start, logs := .empty hs,
    selected := program.map fun i => (members hs).filter fun ⟨h, _⟩ => h.on = i.mnemonic }

/-- Run one installed hook, if its dynamic half holds. -/
def fireOne (s : State) (logs : Logs hs) : (h : Hook) → Member h hs → State × Logs hs
  | h, m => match h.cond.check s with
    | none => (s, logs)
    | some w => match h.effect, h.run s w with
      | .observe, e => (s, logs.push m e)
      | .intervene, (e, ⟨s', _⟩) => (s', logs.push m e)

/-- One instruction: its selected hooks see the state before it, then it runs. -/
def Emulator.step (e : Emulator hs) : Option (Emulator hs) :=
  match e.program[e.state.pc]? with
  | none => none
  | some i =>
    let hooked := e.selected[e.state.pc]?.getD []
    let (s, logs) := hooked.foldl (fun (s, l) ⟨h, m⟩ => fireOne s l h m) (e.state, e.logs)
    some { e with state := i.exec s, logs }

def Emulator.run (e : Emulator hs) : (fuel : Nat) → Emulator hs
  | 0 => e
  | n + 1 => match e.step with | some e' => e'.run n | none => e

/-- Run until a condition holds, and stop with its witness: what was found, and
the proof that it holds in the state the machine stopped in. -/
def Emulator.runUntil (c : Cond) (e : Emulator hs) : (fuel : Nat) →
    Option ((e' : Emulator hs) × c.Witness e'.state)
  | 0 => none
  | n + 1 => match c.check e.state with
    | some w => some ⟨e, w⟩
    | none => e.step.bind fun e' => e'.runUntil c n

/-- Lockstep with rusty_box takes only an exact run: the proof that no hook
intervened is an argument, and for a hook list written in the program `by
decide` finds it. -/
def lockstep (e : Emulator hs) (_ : exact hs = true) : String :=
  s!"lockstep at pc {e.state.pc}"

/-! ## Using it -/

def str (bs : List UInt8) : String := String.fromUTF8! ⟨bs.toArray⟩

/-- Every path a user program passes in x1 to a system call. -/
def paths : Hook where
  name := "paths"
  on := .svc
  cond := .and .userMode (.regPointsToCStr 1)
  Event := String
  effect := .observe
  run := fun _ (_, p) => str p.val

/-- The number in x8 of every system call from user mode. -/
def numbers : Hook where
  name := "numbers"
  on := .svc
  cond := .userMode
  Event := UInt64
  effect := .observe
  run := fun s _ => s.x 8

/-- Point an `openat` of anything at /tmp/decoy instead (0x4000). It may set
registers; its type gives it no way to change the level, the PC or memory. -/
def redirect : Hook where
  name := "redirect"
  on := .svc
  cond := .and .userMode (.regIs 8 56)
  Event := Unit
  effect := .intervene
  run := fun s _ => ((), ⟨{ s with x := fun r => if r = 1 then 0x4000 else s.x r }, rfl, rfl, rfl⟩)

/-- Memory: "/etc/passwd" at 0x2000, "/etc/hosts" at 0x3000, "/tmp/decoy" at 0x4000. -/
def mem (a : UInt64) : Option UInt8 :=
  let strs : List (UInt64 × String) := [(0x2000, "/etc/passwd"), (0x3000, "/etc/hosts"), (0x4000, "/tmp/decoy")]
  match strs.find? fun (b, t) => b ≤ a && a ≤ b + t.utf8ByteSize.toUInt64 with
  | some (b, t) => some (if a = b + t.utf8ByteSize.toUInt64 then 0 else t.toUTF8[(a - b).toNat]!)
  | none => none

/-- A user program: openat("/etc/passwd"), then openat("/etc/hosts"), each
`svc` followed by the kernel's `eret`. -/
def program : Array Instr :=
  #[.mov 8 56, .mov 1 0x2000, .svc, .eret, .mov 1 0x3000, .svc, .eret]

def start : State := { el := .el0, pc := 0, x := fun _ => 0, read := mem }

-- create, install two observing hooks, run, read each hook's log at its own type
def observed := (Emulator.create program start [paths, numbers]).run 100

#eval (observed.logs.get .here : List String)          -- paths' log: strings
#eval (observed.logs.get (.there .here) : List UInt64) -- numbers' log: numbers
#eval lockstep observed (by decide)                    -- exact: no hook intervened

-- with the intervening hook, the program's paths are redirected
def redirected := (Emulator.create program start [redirect, paths]).run 100
#eval (redirected.logs.get (.there .here) : List String)

-- run until a user-mode system call names a path, and stop with that path
#eval ((Emulator.create program start []).runUntil (.and .userMode (.regPointsToCStr 1)) 100).map
  fun ⟨e, (_, p)⟩ => s!"stopped at pc {e.state.pc} with {str p.val}"

-- lockstep refuses a run that a hook altered: there is no proof to give
/-- error: Tactic `decide` proved that the proposition
  exact [redirect, paths] = true
is false -/
#guard_msgs in
#eval lockstep redirected (by decide)

-- an intervening hook cannot change the exception level: the proof does not exist
/-- error: Application type mismatch: The argument
  rfl
has type
  ?m.10 = ?m.10
but is expected to have type
  { el := El.el1, pc := s.pc, x := s.x, read := s.read }.el = s.el
in the application
  And.intro rfl -/
#guard_msgs in
example : Hook where
  name := "escalate"; on := .svc; cond := .userMode; Event := Unit; effect := .intervene
  run := fun s _ => ((), ⟨{ s with el := .el1 }, rfl, rfl, rfl⟩)
